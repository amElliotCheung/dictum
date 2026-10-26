//! Recurring work and durable result history through actual MCP stdio processes.
mod support;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Barrier},
};
use support::Project;

fn call(client: &Project, name: &str, input: Value) -> Value {
    client.call(name, input).unwrap()
}

fn brief(title: &str, kind: &str) -> Value {
    json!({
        "title": title,
        "kind": kind,
        "problem_or_goal": "Keep useful evidence of each authorized execution.",
        "design": "Perform the work and report its outcome through MCP.",
        "acceptance_criteria": ["Evidence identifies what happened and the outcome."]
    })
}

fn create(client: &Project, title: &str, kind: &str) -> Value {
    call(client, "create_task", brief(title, kind))
}

fn context(client: &Project, task: &Value) -> Value {
    call(client, "get_task_context", json!({"task_id": task["id"]}))
}

fn update(client: &Project, task: &Value, mut patch: Value) -> Value {
    patch["task_id"] = task["id"].clone();
    patch["expected_revision"] = task["revision"].clone();
    call(client, "update_task", patch)
}

fn record(client: &Project, task: &Value, outcome: &str, evidence: &str, outputs: &[i64]) -> Value {
    let before = Utc::now();
    let result = call(
        client,
        "record_task_result",
        json!({
            "task_id": task["id"], "expected_revision": task["revision"],
            "outcome": outcome, "evidence": evidence, "output_source_ids": outputs
        }),
    );
    let after = Utc::now();
    assert_eq!(result["task_id"], task["id"]);
    assert_eq!(result["project_id"], task["project_id"]);
    assert_eq!(result["outcome"], outcome);
    assert_eq!(result["evidence"], evidence);
    assert_eq!(result["output_source_ids"], json!(outputs));
    assert_eq!(
        result["task_revision"],
        task["revision"].as_i64().unwrap() + 1
    );
    let timestamp = result["recorded_at"].as_str().unwrap();
    assert!(timestamp.ends_with('Z'));
    let recorded_at = DateTime::parse_from_rfc3339(timestamp).unwrap();
    assert_eq!(recorded_at.offset().local_minus_utc(), 0);
    // Compare at millisecond precision so harmless serialization precision differences do not matter.
    assert!(recorded_at.timestamp_millis() >= before.timestamp_millis());
    assert!(recorded_at.timestamp_millis() <= after.timestamp_millis());
    result
}

fn history(client: &Project, task: &Value, mut options: Value) -> Value {
    options["task_id"] = task["id"].clone();
    call(client, "list_task_results", options)
}

fn source(client: &Project, name: &str, content: &str) -> Value {
    call(
        client,
        "create_source",
        json!({"name": name, "description": "Supporting evidence for a reported execution", "content": content}),
    )
}

fn ids(values: &Value) -> Vec<i64> {
    values
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["id"].as_i64().unwrap())
        .collect()
}

fn snapshot(database: &Path) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    let connection = rusqlite::Connection::open(database).unwrap();
    let tables = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                .unwrap();
            let columns = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..columns).map(|column| row.get(column)).collect()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}

fn rejected(client: &Project, database: &Path, name: &str, input: Value) {
    let before = snapshot(database);
    assert!(
        client.call::<Value>(name, input.clone()).is_err(),
        "accepted {name}: {input}"
    );
    assert_eq!(
        snapshot(database),
        before,
        "rejected {name} changed durable data"
    );
}

fn rejected_update(client: &Project, database: &Path, task: &Value, mut patch: Value) {
    patch["task_id"] = task["id"].clone();
    patch["expected_revision"] = task["revision"].clone();
    rejected(client, database, "update_task", patch);
}

#[test]
fn recurring_history_survives_restart_and_context_keeps_only_the_latest_outcomes() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("history.sqlite3");
    let client = Project::open(&database).unwrap();
    let full_content = format!(
        "# Full execution evidence\n{}\nEND",
        "完整记录 🦀\n".repeat(8_000)
    );
    let output = source(&client, "Execution report", &full_content);
    let other_output = source(&client, "Second report", "Second execution evidence");
    let linked_context = source(&client, "Execution guide", &full_content);
    let always_context = call(
        &client,
        "create_source",
        json!({
            "name": "Project conventions", "description": "Context for every execution",
            "content": "# Conventions\nReport the inspected revision and actual outcome.",
            "context_policy": "always"
        }),
    );
    let mut input = brief("Review release signals", "recurring");
    input["trigger_event"] = json!("release-published");
    input["context_source_ids"] = json!([linked_context["id"], always_context["id"]]);
    let task = call(&client, "create_task", input);
    assert_eq!(task["status"], "active");
    assert_eq!(task["trigger_event"], "release-published");
    let initial = context(&client, &task);
    assert!(initial["latest_result"].is_null());
    assert!(initial["latest_successful_result"].is_null());
    assert_eq!(initial["sources"].as_array().unwrap().len(), 2);
    assert_eq!(
        initial["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["id"] == linked_context["id"])
            .unwrap()["content"],
        full_content
    );
    assert_eq!(history(&client, &task, json!({}))["results"], json!([]));

    let first = record(
        &client,
        &task,
        "success",
        "Release one inspected; checks passed.",
        &[output["id"].as_i64().unwrap()],
    );
    let after_first = context(&client, &task);
    assert_eq!(after_first["latest_result"], first);
    assert_eq!(after_first["latest_successful_result"], first);
    assert_eq!(after_first["sources"], initial["sources"]);
    assert_eq!(
        call(&client, "get_source", json!({"source_id": output["id"]}))["content"],
        full_content
    );
    let second = record(
        &client,
        &after_first["task"],
        "success",
        "Release two inspected; checks passed.",
        &[other_output["id"].as_i64().unwrap()],
    );
    let stored_task = context(&client, &task)["task"].clone();
    drop(client);

    let restarted = Project::open(&database).unwrap();
    let restored = context(&restarted, &task);
    assert_eq!(restored["task"], stored_task);
    assert_eq!(restored["latest_result"], second);
    assert_eq!(restored["latest_successful_result"], second);
    assert_eq!(restored["sources"], initial["sources"]);
    let failure = record(
        &restarted,
        &restored["task"],
        "failure",
        "Release three could not be checked: service unavailable.",
        &[],
    );
    let failed = context(&restarted, &task);
    assert_eq!(failed["latest_result"], failure);
    assert_eq!(failed["latest_successful_result"], second);
    assert_eq!(failed["task"]["status"], "active");
    let paused = update(&restarted, &failed["task"], json!({"status": "paused"}));
    let unchanged = record(
        &restarted,
        &paused,
        "no_change",
        "Report for work completed before pausing: no new release.",
        &[],
    );
    let final_context = context(&restarted, &task);
    assert_eq!(final_context["task"]["id"], task["id"]);
    assert_eq!(final_context["task"]["status"], "paused");
    assert_eq!(final_context["latest_result"], unchanged);
    assert_eq!(final_context["latest_successful_result"], unchanged);
    assert_eq!(final_context["sources"], initial["sources"]);
    for field in [
        "title",
        "problem_or_goal",
        "design",
        "acceptance_criteria",
        "context_source_ids",
        "trigger_event",
    ] {
        assert_eq!(final_context["task"][field], task[field]);
    }
    assert_eq!(
        ids(&call(&restarted, "list_tasks", json!({}))["tasks"]),
        vec![task["id"].as_i64().unwrap()]
    );
    assert!(final_context.get("results").is_none());
    assert!(!final_context.to_string().contains("Release one inspected"));
    assert!(!final_context.to_string().contains("Execution report"));
    let expected_history = json!([unchanged, failure, second, first]);
    assert_eq!(
        history(&restarted, &task, json!({}))["results"],
        expected_history
    );

    call(
        &restarted,
        "update_source",
        json!({"source_id": output["id"], "expected_revision": output["revision"], "content": "Updated report; source references do not freeze content."}),
    );
    assert_eq!(
        history(&restarted, &task, json!({}))["results"],
        expected_history
    );
    assert_eq!(
        call(&restarted, "get_source", json!({"source_id": output["id"]}))["content"],
        "Updated report; source references do not freeze content."
    );
    drop(restarted);
    let restarted_again = Project::open(&database).unwrap();
    assert_eq!(context(&restarted_again, &task), final_context);
    assert_eq!(
        history(&restarted_again, &task, json!({}))["results"],
        expected_history
    );
}

#[test]
fn keyset_history_pages_are_stable_when_other_clients_append() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("paging.sqlite3");
    let first = Project::open(&database).unwrap();
    let second = Project::open(&database).unwrap();
    let mut task = create(&first, "History paging", "recurring");
    let other_task = create(&first, "Unrelated history", "recurring");
    let mut expected = Vec::new();
    for index in 0..23 {
        let result = record(
            &first,
            &task,
            "success",
            &format!("Execution {index}: passing checks."),
            &[],
        );
        expected.push(result["id"].as_i64().unwrap());
        task = context(&first, &task)["task"].clone();
    }
    expected.reverse();
    let page = history(&first, &task, json!({"page_size": 7}));
    assert_eq!(ids(&page["results"]), expected[..7]);
    assert_eq!(page["next_before_id"], page["results"][6]["id"]);
    let interleaved = record(
        &second,
        &task,
        "no_change",
        "Appended after the first page was read.",
        &[],
    );
    let unrelated = record(
        &second,
        &other_task,
        "failure",
        "Unrelated execution creates an ID gap.",
        &[],
    );
    let mut received = ids(&page["results"]);
    let mut cursor = page["next_before_id"].clone();
    while !cursor.is_null() {
        let page = history(&first, &task, json!({"page_size": 7, "before_id": cursor}));
        assert!(page["results"].as_array().unwrap().len() <= 7);
        received.extend(ids(&page["results"]));
        cursor = page["next_before_id"].clone();
        if !cursor.is_null() {
            assert_eq!(
                cursor,
                page["results"].as_array().unwrap().last().unwrap()["id"]
            );
        }
    }
    assert_eq!(received, expected);
    assert_eq!(
        received.iter().collect::<BTreeSet<_>>().len(),
        received.len()
    );
    let default_page = history(&first, &task, json!({}));
    assert_eq!(default_page["results"].as_array().unwrap().len(), 20);
    assert_eq!(default_page["results"][0], interleaved);
    let all = history(&second, &task, json!({"page_size": 100}));
    assert_eq!(all["results"].as_array().unwrap().len(), 24);
    assert!(all["next_before_id"].is_null());
    assert_eq!(
        history(&first, &task, json!({"page_size": 1}))["results"],
        json!([interleaved])
    );
    let oldest = expected.last().unwrap();
    let empty = history(&first, &task, json!({"before_id": oldest}));
    assert_eq!(empty["results"], json!([]));
    assert!(empty["next_before_id"].is_null());

    let foreign_project = call(
        &first,
        "create_project",
        json!({"name": "Other project", "description": "Cursor isolation"}),
    );
    let mut foreign_input = brief("Foreign history", "recurring");
    foreign_input["project_id"] = foreign_project["id"].clone();
    let foreign_task = call(&first, "create_task", foreign_input);
    let foreign_result = call(
        &second,
        "record_task_result",
        json!({"project_id": foreign_project["id"], "task_id": foreign_task["id"], "expected_revision": foreign_task["revision"], "outcome": "success", "evidence": "Foreign execution."}),
    );
    for page_size in [json!(0), json!(-1), json!(101), json!(1.5), json!("2")] {
        rejected(
            &first,
            &database,
            "list_task_results",
            json!({"task_id": task["id"], "page_size": page_size}),
        );
    }
    for before_id in [
        json!(0),
        json!(-1),
        json!(999_999),
        json!(1.5),
        json!("2"),
        unrelated["id"].clone(),
        foreign_result["id"].clone(),
    ] {
        rejected(
            &first,
            &database,
            "list_task_results",
            json!({"task_id": task["id"], "before_id": before_id}),
        );
    }
}

#[test]
fn invalid_and_stale_results_roll_back_evidence_outputs_and_revision() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("atomic.sqlite3");
    let first = Project::open(&database).unwrap();
    let second = Project::open(&database).unwrap();
    let task = create(&first, "Atomic reporting", "recurring");
    let output = source(&first, "Valid output", "Durable evidence");
    let foreign_project = call(
        &first,
        "create_project",
        json!({"name": "Other", "description": "Foreign records"}),
    );
    let foreign_output = call(
        &first,
        "create_source",
        json!({"project_id": foreign_project["id"], "name": "Foreign", "description": "Wrong project", "content": "Foreign evidence"}),
    );
    let valid = json!({"task_id": task["id"], "expected_revision": task["revision"], "outcome": "success", "evidence": "Checked revision abc123: all checks passed.", "output_source_ids": [output["id"]]});
    for evidence in [
        json!(""),
        json!(" \n\t"),
        json!("界".repeat(4_001)),
        Value::Null,
    ] {
        let mut input = valid.clone();
        input["evidence"] = evidence;
        rejected(&first, &database, "record_task_result", input);
    }
    for outcome in [json!("done"), json!("SUCCESS"), Value::Null] {
        let mut input = valid.clone();
        input["outcome"] = outcome;
        rejected(&first, &database, "record_task_result", input);
    }
    for field in ["task_id", "expected_revision", "outcome", "evidence"] {
        let mut input = valid.clone();
        input.as_object_mut().unwrap().remove(field);
        rejected(&first, &database, "record_task_result", input);
    }
    for (field, value) in [
        ("id", json!(123)),
        ("recorded_at", json!("2020-01-01T00:00:00Z")),
        ("task_revision", json!(40)),
    ] {
        let mut input = valid.clone();
        input[field] = value;
        rejected(&first, &database, "record_task_result", input);
    }
    for invalid_source in [json!(999_999), json!(0), foreign_output["id"].clone()] {
        let mut input = valid.clone();
        input["output_source_ids"] = json!([output["id"], invalid_source]);
        rejected(&first, &database, "record_task_result", input);
    }
    let max_evidence = "界".repeat(4_000);
    let accepted = record(
        &first,
        &task,
        "success",
        &max_evidence,
        &[output["id"].as_i64().unwrap()],
    );
    rejected(&second, &database, "record_task_result", valid);
    rejected_update(
        &second,
        &database,
        &task,
        json!({"status": "paused", "progress": "Stale update"}),
    );
    let updated = update(
        &second,
        &context(&second, &task)["task"],
        json!({"progress": "A newer client changed the brief."}),
    );
    let mut stale = json!({"task_id": task["id"], "expected_revision": accepted["task_revision"], "outcome": "failure", "evidence": "Stale failure", "output_source_ids": [output["id"]]});
    rejected(&first, &database, "record_task_result", stale.clone());
    stale["expected_revision"] = updated["revision"].clone();
    stale.as_object_mut().unwrap().remove("output_source_ids");
    let second_result = call(&first, "record_task_result", stale);
    assert_eq!(second_result["output_source_ids"], json!([]));
    assert_eq!(
        history(&second, &task, json!({}))["results"],
        json!([second_result, accepted])
    );
    assert_eq!(
        context(&second, &task)["task"]["revision"],
        updated["revision"].as_i64().unwrap() + 1
    );
}

#[test]
fn simultaneous_result_writers_have_one_revision_winner_and_no_partial_output_links() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("concurrent.sqlite3");
    let client = Project::open(&database).unwrap();
    let task = create(&client, "Concurrent execution reports", "recurring");
    let first_output = source(&client, "First client's report", "First evidence");
    let second_output = source(&client, "Second client's report", "Second evidence");
    let barrier = Arc::new(Barrier::new(2));
    let writers: Vec<_> = [first_output, second_output]
        .into_iter()
        .enumerate()
        .map(|(index, output)| {
            let database = database.clone();
            let task = task.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let client = Project::open(database).unwrap();
                let input = json!({
                    "task_id": task["id"], "expected_revision": task["revision"],
                    "outcome": "success", "evidence": format!("Writer {index}: execution passed."),
                    "output_source_ids": [output["id"]]
                });
                barrier.wait();
                client.call::<Value>("record_task_result", input)
            })
        })
        .collect();
    let results: Vec<_> = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let rejection = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .unwrap();
    assert!(
        rejection.to_string().contains("revision"),
        "unexpected conflict: {rejection}"
    );
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(
        winner["task_revision"],
        task["revision"].as_i64().unwrap() + 1
    );
    assert_eq!(
        history(&client, &task, json!({}))["results"],
        json!([winner])
    );
    let final_context = context(&client, &task);
    assert_eq!(final_context["task"]["revision"], winner["task_revision"]);
    assert_eq!(final_context["task"]["status"], "active");
    assert_eq!(final_context["latest_result"], winner);
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(
        history(&restarted, &task, json!({}))["results"],
        json!([winner])
    );
}

#[test]
fn kinds_lifecycles_triggers_and_filters_remain_distinct() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("lifecycle.sqlite3");
    let client = Project::open(&database).unwrap();
    let mut ordinary_input = brief("Ordinary work", "one_time");
    ordinary_input.as_object_mut().unwrap().remove("kind");
    let ordinary = call(&client, "create_task", ordinary_input);
    assert_eq!(ordinary["kind"], "one_time");
    assert_eq!(ordinary["status"], "todo");
    assert!(ordinary["trigger_event"].is_null());
    let recurring = create(&client, "Recurring work", "recurring");
    assert_eq!(recurring["kind"], "recurring");
    assert_eq!(recurring["status"], "active");
    assert!(recurring["trigger_event"].is_null());
    for (task, statuses) in [
        (&ordinary, vec!["active", "paused"]),
        (&recurring, vec!["todo", "in_progress", "blocked", "done"]),
    ] {
        for status in statuses {
            rejected_update(
                &client,
                &database,
                task,
                json!({"status": status, "title": "Must roll back"}),
            );
        }
        rejected_update(&client, &database, task, json!({"kind": "recurring"}));
        rejected_update(&client, &database, task, json!({"kind": "one_time"}));
    }
    for patch in [
        json!({"result": "An overwritten result"}),
        json!({"result": null}),
        json!({"output_source_ids": []}),
        json!({"output_source_ids": null}),
    ] {
        rejected_update(&client, &database, &recurring, patch);
    }
    for trigger in [json!(""), json!(" \n\t")] {
        let mut input = brief("Invalid trigger", "recurring");
        input["trigger_event"] = trigger.clone();
        rejected(&client, &database, "create_task", input);
        rejected_update(
            &client,
            &database,
            &recurring,
            json!({"trigger_event": trigger}),
        );
    }
    let mut invalid = brief("Ordinary trigger", "one_time");
    invalid["trigger_event"] = json!("release");
    rejected(&client, &database, "create_task", invalid);
    for trigger in [json!("release"), Value::Null] {
        rejected_update(
            &client,
            &database,
            &ordinary,
            json!({"trigger_event": trigger}),
        );
    }
    for kind in [json!("routine"), Value::Null] {
        let mut input = brief("Invalid kind", "recurring");
        input["kind"] = kind;
        rejected(&client, &database, "create_task", input);
    }

    let recurring = update(
        &client,
        &recurring,
        json!({"trigger_event": "release", "progress": "Persistent notes"}),
    );
    let recurring = update(
        &client,
        &recurring,
        json!({"status": "paused", "title": "Paused recurring work"}),
    );
    assert_eq!(recurring["trigger_event"], "release");
    assert_eq!(recurring["progress"], "Persistent notes");
    let mut prefix_input = brief("Prefix must not match", "recurring");
    prefix_input["trigger_event"] = json!("release-candidate");
    let prefix = call(&client, "create_task", prefix_input);
    let mut case_input = brief("Case must not match", "recurring");
    case_input["trigger_event"] = json!("Release");
    let case = call(&client, "create_task", case_input);
    let selected = call(
        &client,
        "list_tasks",
        json!({"kind": "recurring", "status": "paused", "trigger_event": "release"}),
    );
    assert_eq!(
        ids(&selected["tasks"]),
        vec![recurring["id"].as_i64().unwrap()]
    );
    assert_eq!(selected["tasks"][0]["kind"], "recurring");
    assert_eq!(selected["tasks"][0]["trigger_event"], "release");
    assert_eq!(
        ids(&call(&client, "list_tasks", json!({"trigger_event": "release"}))["tasks"]),
        ids(&selected["tasks"])
    );
    assert_eq!(
        call(
            &client,
            "list_tasks",
            json!({"kind": "one_time", "trigger_event": "release"})
        )["tasks"],
        json!([])
    );
    assert_eq!(
        ids(&call(&client, "list_tasks", json!({"kind": "one_time"}))["tasks"]),
        vec![ordinary["id"].as_i64().unwrap()]
    );
    assert_eq!(
        ids(&call(&client, "list_tasks", json!({"kind": "recurring"}))["tasks"])
            .into_iter()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            recurring["id"].as_i64().unwrap(),
            prefix["id"].as_i64().unwrap(),
            case["id"].as_i64().unwrap()
        ])
    );
    assert_eq!(
        ids(&call(&client, "list_tasks", json!({"ready_to_start": true}))["tasks"]),
        vec![ordinary["id"].as_i64().unwrap()]
    );
    assert_eq!(
        call(
            &client,
            "list_tasks",
            json!({"kind": "recurring", "ready_to_start": true})
        )["tasks"],
        json!([])
    );
    for task in [&recurring, &prefix, &case] {
        assert_eq!(
            context(&client, task)["dependency_state"]["ready_to_start"],
            false
        );
    }
    let recurring = update(
        &client,
        &recurring,
        json!({"trigger_event": null, "status": "active"}),
    );
    assert!(recurring["trigger_event"].is_null());
    assert_eq!(recurring["status"], "active");
    assert_eq!(
        call(&client, "list_tasks", json!({"trigger_event": "release"}))["tasks"],
        json!([])
    );
}

#[test]
fn recurring_tasks_cannot_be_dependency_targets_and_success_obeys_all_ordinary_gates() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("dependencies.sqlite3");
    let client = Project::open(&database).unwrap();
    let target = create(&client, "Never completes", "recurring");
    for kind in ["one_time", "recurring"] {
        let owner = create(&client, "Dependency owner", kind);
        for field in ["prerequisite_task_ids", "completion_task_ids"] {
            let mut input = brief("Invalid dependency", kind);
            input[field] = json!([target["id"]]);
            rejected(&client, &database, "create_task", input);
            let mut patch = json!({"title": "Invalid relationship must roll back"});
            patch[field] = json!([target["id"]]);
            rejected_update(&client, &database, &owner, patch);
        }
    }
    let start_gate = create(&client, "Approve start", "one_time");
    let finish_gate = create(&client, "Complete verification", "one_time");
    let output = source(
        &client,
        "Blocked report",
        "Evidence references cannot escape rollback",
    );
    for kind in ["one_time", "recurring"] {
        let mut input = brief("Gated report", kind);
        input["prerequisite_task_ids"] = json!([start_gate["id"]]);
        input["completion_task_ids"] = json!([finish_gate["id"]]);
        let task = call(&client, "create_task", input);
        let initial_status = task["status"].clone();
        let state = context(&client, &task);
        assert_eq!(
            state["dependency_state"]["unmet_prerequisite_task_ids"],
            json!([start_gate["id"]])
        );
        assert_eq!(
            state["dependency_state"]["unmet_completion_task_ids"],
            json!([finish_gate["id"]])
        );
        for outcome in ["success", "no_change"] {
            rejected(
                &client,
                &database,
                "record_task_result",
                json!({"task_id": task["id"], "expected_revision": task["revision"], "outcome": outcome, "evidence": "Gates must prevent success", "output_source_ids": [output["id"]]}),
            );
        }
        let failure = record(
            &client,
            &task,
            "failure",
            "Execution could not proceed because dependencies are incomplete.",
            &[],
        );
        let failed = context(&client, &task);
        assert_eq!(failed["task"]["status"], initial_status);
        assert_eq!(failed["latest_result"], failure);
        assert!(failed["latest_successful_result"].is_null());
    }
    let gated = call(&client, "list_tasks", json!({"query": "Gated report"}))["tasks"]
        .as_array()
        .unwrap()
        .clone();
    record(
        &client,
        &start_gate,
        "success",
        "Start conditions inspected and approved.",
        &[],
    );
    let start_gate = context(&client, &start_gate)["task"].clone();
    update(&client, &start_gate, json!({"status": "done"}));
    for task in &gated {
        let task = context(&client, task)["task"].clone();
        assert_eq!(
            context(&client, &task)["dependency_state"]["unmet_prerequisite_task_ids"],
            json!([])
        );
        for outcome in ["success", "no_change"] {
            rejected(
                &client,
                &database,
                "record_task_result",
                json!({"task_id": task["id"], "expected_revision": task["revision"], "outcome": outcome, "evidence": "Completion gate is still unmet."}),
            );
        }
    }
    record(
        &client,
        &finish_gate,
        "success",
        "Final revision tested: checks passed.",
        &[],
    );
    let finish_gate = context(&client, &finish_gate)["task"].clone();
    update(&client, &finish_gate, json!({"status": "done"}));
    for task in &gated {
        let task = context(&client, task)["task"].clone();
        let initial_status = task["status"].clone();
        record(
            &client,
            &task,
            "success",
            "Both start and completion conditions are now satisfied.",
            &[],
        );
        let after_success = context(&client, &task);
        assert_eq!(after_success["task"]["status"], initial_status);
        record(
            &client,
            &after_success["task"],
            "no_change",
            "Rechecked the completed conditions; no change.",
            &[],
        );
        assert_eq!(context(&client, &task)["task"]["status"], initial_status);
    }
}

#[test]
fn ordinary_tasks_require_successful_results_before_done_and_preserve_history() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("ordinary.sqlite3");
    let client = Project::open(&database).unwrap();
    let historical_output = source(&client, "Historic report", "Old evidence");
    let task = create(&client, "Ordinary task", "one_time");
    let task = update(&client, &task, json!({"status": "in_progress"}));
    let failure = record(&client, &task, "failure", "The verification failed.", &[]);
    let after_failure = context(&client, &task);
    assert_eq!(after_failure["task"]["status"], "in_progress");
    assert_eq!(after_failure["latest_result"], failure);
    assert!(after_failure["latest_successful_result"].is_null());
    rejected_update(
        &client,
        &database,
        &after_failure["task"],
        json!({"status": "done"}),
    );
    let success = record(
        &client,
        &after_failure["task"],
        "success",
        "Rechecked after the fix: checks passed.",
        &[historical_output["id"].as_i64().unwrap()],
    );
    let after_success = context(&client, &task);
    assert_eq!(after_success["task"]["status"], "in_progress");
    assert_eq!(after_success["latest_result"], success);
    assert_eq!(after_success["latest_successful_result"], success);
    let done = update(&client, &after_success["task"], json!({"status": "done"}));
    assert_eq!(done["status"], "done");
    let history_value = history(&client, &done, json!({}));
    assert_eq!(history_value["results"], json!([success, failure]));
    assert_eq!(
        history_value["results"][0]["output_source_ids"],
        json!([historical_output["id"]])
    );
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(context(&restarted, &done)["task"], done);
    assert_eq!(
        history(&restarted, &done, json!({}))["results"][0]["output_source_ids"],
        json!([historical_output["id"]])
    );
}

#[test]
fn recurring_priority_is_independent_of_lifecycle_and_composes_with_discovery_filters() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("priority.sqlite3");
    let client = Project::open(&database).unwrap();
    let default = create(&client, "Default recurring priority", "recurring");
    assert_eq!(default["priority"], "medium");
    let mut low_input = brief("Release low", "recurring");
    low_input["priority"] = json!("low");
    low_input["trigger_event"] = json!("release");
    let low = call(&client, "create_task", low_input);
    let mut high_input = brief("Release high", "recurring");
    high_input["priority"] = json!("high");
    high_input["trigger_event"] = json!("release");
    let high = call(&client, "create_task", high_input);
    let high = update(&client, &high, json!({"status": "paused"}));
    assert_eq!(high["priority"], "high");
    assert_eq!(context(&client, &high)["task"]["priority"], "high");
    assert_eq!(
        ids(&call(&client, "list_tasks", json!({"kind": "recurring"}))["tasks"]),
        vec![
            high["id"].as_i64().unwrap(),
            default["id"].as_i64().unwrap(),
            low["id"].as_i64().unwrap()
        ]
    );
    let selected = call(
        &client,
        "list_tasks",
        json!({"kind": "recurring", "status": "paused", "trigger_event": "release", "query": "HIGH", "priority": "high"}),
    );
    assert_eq!(ids(&selected["tasks"]), vec![high["id"].as_i64().unwrap()]);
    assert_eq!(selected["tasks"][0]["priority"], "high");
    assert!(
        call(
            &client,
            "list_tasks",
            json!({"kind": "recurring", "priority": "high", "ready_to_start": true})
        )["tasks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let low = update(&client, &low, json!({"priority": "high"}));
    assert_eq!(low["status"], "active");
    assert_eq!(
        ids(&call(
            &client,
            "list_tasks",
            json!({"kind": "recurring", "priority": "high"})
        )["tasks"]),
        vec![low["id"].as_i64().unwrap(), high["id"].as_i64().unwrap()]
    );
    let low = update(
        &client,
        &low,
        json!({"priority": "medium", "status": "paused"}),
    );
    assert_eq!(low["priority"], "medium");
    assert_eq!(low["status"], "paused");
    for priority in [json!("urgent"), json!(false)] {
        let mut input = brief("Invalid recurring priority", "recurring");
        input["priority"] = priority.clone();
        rejected(&client, &database, "create_task", input);
        rejected_update(
            &client,
            &database,
            &low,
            json!({"priority": priority, "status": "active", "progress": "Must not apply"}),
        );
    }
    let result = record(
        &client,
        &low,
        "failure",
        "Fixture failure after pausing retains priority and status.",
        &[],
    );
    let final_context = context(&client, &low);
    assert_eq!(final_context["task"]["status"], "paused");
    assert_eq!(final_context["task"]["priority"], "medium");
    assert_eq!(final_context["latest_result"], result);
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(context(&restarted, &low), final_context);
}
