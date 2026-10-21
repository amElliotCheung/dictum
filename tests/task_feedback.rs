//! Result-specific feedback and handoffs through real MCP stdio processes.
mod support;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Barrier},
};
use support::Project;

fn call(client: &Project, name: &str, input: Value) -> Value {
    client.call(name, input).unwrap()
}

fn task(client: &Project, title: &str, kind: &str) -> Value {
    call(
        client,
        "create_task",
        json!({
            "title": title, "kind": kind,
            "problem_or_goal": "Carry review feedback into the next agent's work.",
            "design": "Keep execution evidence separate from comments about its quality.",
            "acceptance_criteria": ["A later agent sees why a previous result was unacceptable."]
        }),
    )
}

fn context(client: &Project, task: &Value) -> Value {
    call(client, "get_task_context", json!({"task_id": task["id"]}))
}

fn update(client: &Project, task: &Value, mut input: Value) -> Value {
    input["task_id"] = task["id"].clone();
    input["expected_revision"] = task["revision"].clone();
    call(client, "update_task", input)
}

fn result(client: &Project, task: &Value, outcome: &str, addressed: &[i64]) -> Value {
    call(
        client,
        "record_task_result",
        json!({
            "task_id": task["id"], "expected_revision": task["revision"],
            "outcome": outcome, "evidence": format!("Execution reported {outcome}."),
            "addressed_feedback_ids": addressed
        }),
    )
}

fn feedback(
    client: &Project,
    task: &Value,
    result: &Value,
    disposition: Option<&str>,
    body: &str,
) -> Value {
    let mut input = json!({
        "task_id": task["id"], "result_id": result["id"],
        "expected_revision": task["revision"], "body": body
    });
    if let Some(disposition) = disposition {
        input["disposition"] = json!(disposition);
    }
    let before = Utc::now();
    let stored = call(client, "record_task_feedback", input);
    let after = Utc::now();
    assert_eq!(stored["project_id"], task["project_id"]);
    assert_eq!(stored["task_id"], task["id"]);
    assert_eq!(stored["result_id"], result["id"]);
    assert_eq!(stored["body"], body);
    assert_eq!(stored["disposition"], disposition.unwrap_or("comment"));
    assert_eq!(
        stored["task_revision"],
        task["revision"].as_i64().unwrap() + 1
    );
    assert!(stored["addressed_by_result_id"].is_null());
    let timestamp = stored["recorded_at"].as_str().unwrap();
    assert!(timestamp.ends_with('Z'));
    let recorded = DateTime::parse_from_rfc3339(timestamp).unwrap();
    assert_eq!(recorded.offset().local_minus_utc(), 0);
    assert!(recorded.timestamp_millis() >= before.timestamp_millis());
    assert!(recorded.timestamp_millis() <= after.timestamp_millis());
    stored
}

fn history(client: &Project, task: &Value) -> Vec<Value> {
    call(client, "list_task_results", json!({"task_id": task["id"]}))["results"]
        .as_array()
        .unwrap()
        .clone()
}

fn outstanding_ids(context: &Value) -> Vec<i64> {
    context["outstanding_feedback"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["feedback"]["id"].as_i64().unwrap())
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

fn rejected(client: &Project, database: &Path, name: &str, mut input: Value) {
    if input.get("project_id").is_none() {
        input["project_id"] = json!(client.project_id);
    }
    rejected_raw(client, database, name, input);
}

fn rejected_raw(client: &Project, database: &Path, name: &str, input: Value) {
    let before = snapshot(database);
    assert!(
        client.raw_call::<Value>(name, input.clone()).is_err(),
        "accepted {name}: {input}"
    );
    assert_eq!(
        snapshot(database),
        before,
        "rejected {name} changed durable data"
    );
}

#[test]
fn later_agents_see_old_result_feedback_after_brief_edits_new_attempts_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("handoff.sqlite3");
    let client = Project::open(&database).unwrap();
    let original = task(&client, "Revise a published result", "one_time");
    assert_eq!(
        outstanding_ids(&context(&client, &original)),
        Vec::<i64>::new()
    );
    let output = call(
        &client,
        "create_source",
        json!({"name": "Published report", "description": "The rejected document", "content": "# Report\nMissing the required reproduction steps."}),
    );
    let first = call(
        &client,
        "record_task_result",
        json!({
            "task_id": original["id"], "expected_revision": original["revision"],
            "outcome": "success", "evidence": "Published report; lint passed.",
            "output_source_ids": [output["id"]]
        }),
    );
    assert_eq!(first["feedback"], json!([]));
    assert_eq!(first["addressed_feedback_ids"], json!([]));
    let done = update(
        &client,
        &context(&client, &original)["task"],
        json!({"status": "done"}),
    );
    let request = feedback(
        &client,
        &done,
        &first,
        Some("changes_requested"),
        "The report omits reproduction steps. Include the exact failing input and expected output.",
    );
    let reopened = context(&client, &original);
    assert_eq!(reopened["task"]["status"], "todo");
    assert_eq!(reopened["task"]["revision"], request["task_revision"]);
    assert_eq!(reopened["latest_result"]["feedback"], json!([request]));
    let comment = feedback(
        &client,
        &reopened["task"],
        &first,
        None,
        "Use the original report as the reference when explaining the correction.",
    );
    let edited = update(
        &client,
        &context(&client, &original)["task"],
        json!({
            "design": "Include reproducible examples and preserve the review record.",
            "acceptance_criteria": ["The report contains the exact failing input and expected output."],
            "progress": "A replacement agent is preparing the correction."
        }),
    );
    let second = result(&client, &edited, "failure", &[]);
    let third = result(
        &client,
        &context(&client, &original)["task"],
        "success",
        &[],
    );
    let current = context(&client, &original);
    assert_eq!(current["latest_result"]["id"], third["id"]);
    assert_eq!(current["latest_successful_result"]["id"], third["id"]);
    assert_eq!(
        outstanding_ids(&current),
        vec![
            request["id"].as_i64().unwrap(),
            comment["id"].as_i64().unwrap()
        ]
    );
    for entry in current["outstanding_feedback"].as_array().unwrap() {
        assert_eq!(entry["result"]["id"], first["id"]);
        assert_eq!(entry["result"]["evidence"], first["evidence"]);
        assert_eq!(entry["result"]["output_source_ids"], json!([output["id"]]));
    }
    let results = history(&client, &original);
    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["id"], third["id"]);
    assert_eq!(results[1]["id"], second["id"]);
    assert_eq!(results[2]["feedback"], json!([request, comment]));
    drop(client);

    let replacement = Project::open(&database).unwrap();
    assert_eq!(context(&replacement, &original), current);
    assert_eq!(history(&replacement, &original), results);
    assert_eq!(
        call(
            &replacement,
            "get_source",
            json!({"source_id": output["id"]})
        )["content"],
        "# Report\nMissing the required reproduction steps."
    );
}

#[test]
fn comments_preserve_done_but_change_requests_require_explicit_successful_resolution() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("acceptance.sqlite3");
    let client = Project::open(&database).unwrap();
    let original = task(&client, "Review completed work", "one_time");
    let first = result(&client, &original, "success", &[]);
    let done = update(
        &client,
        &context(&client, &original)["task"],
        json!({"status": "done"}),
    );
    let comment = feedback(
        &client,
        &done,
        &first,
        None,
        "The explanation could include another example.",
    );
    assert_eq!(context(&client, &original)["task"]["status"], "done");
    let done = update(
        &client,
        &context(&client, &original)["task"],
        json!({"progress": "Comment recorded."}),
    );
    let change = feedback(
        &client,
        &done,
        &first,
        Some("changes_requested"),
        "The example gives the wrong output; correct it before completion.",
    );
    let reopened = context(&client, &original);
    assert_eq!(reopened["task"]["status"], "todo");
    let still_open = result(&client, &reopened["task"], "success", &[]);
    assert_eq!(still_open["addressed_feedback_ids"], json!([]));
    let before_resolution = context(&client, &original);
    rejected(
        &client,
        &database,
        "update_task",
        json!({
            "task_id": original["id"], "expected_revision": before_resolution["task"]["revision"], "status": "done"
        }),
    );
    let resolution = result(
        &client,
        &before_resolution["task"],
        "no_change",
        &[change["id"].as_i64().unwrap()],
    );
    assert_eq!(resolution["addressed_feedback_ids"], json!([change["id"]]));
    let addressed = context(&client, &original);
    assert_eq!(
        outstanding_ids(&addressed),
        vec![comment["id"].as_i64().unwrap()]
    );
    let results = history(&client, &original);
    let reviewed = results
        .iter()
        .find(|entry| entry["id"] == first["id"])
        .unwrap();
    assert_eq!(reviewed["outcome"], "success");
    assert_eq!(reviewed["evidence"], first["evidence"]);
    assert_eq!(
        reviewed["feedback"][0]["addressed_by_result_id"],
        Value::Null
    );
    assert_eq!(
        reviewed["feedback"][1]["addressed_by_result_id"],
        resolution["id"]
    );
    let done = update(&client, &addressed["task"], json!({"status": "done"}));
    assert_eq!(done["status"], "done");
    assert_eq!(
        outstanding_ids(&context(&client, &original)),
        vec![comment["id"].as_i64().unwrap()]
    );
    let resolved_comment = result(
        &client,
        &done,
        "success",
        &[comment["id"].as_i64().unwrap()],
    );
    assert_eq!(
        resolved_comment["addressed_feedback_ids"],
        json!([comment["id"]])
    );
    let final_context = context(&client, &original);
    assert_eq!(final_context["task"]["status"], "done");
    assert_eq!(final_context["outstanding_feedback"], json!([]));
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(context(&restarted, &original), final_context);
    let results = history(&restarted, &original);
    let reviewed = results
        .iter()
        .find(|entry| entry["id"] == first["id"])
        .unwrap();
    assert_eq!(
        reviewed["feedback"][0]["addressed_by_result_id"],
        resolved_comment["id"]
    );
    assert_eq!(
        reviewed["feedback"][1]["addressed_by_result_id"],
        resolution["id"]
    );
}

#[test]
fn invalid_feedback_and_scope_errors_roll_back_feedback_reopening_and_revision() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("invalid-feedback.sqlite3");
    let client = Project::open(&database).unwrap();
    let other_client = Project::open(&database).unwrap();
    let original = task(&client, "Reject malformed feedback", "one_time");
    let first = result(&client, &original, "success", &[]);
    let done = update(
        &client,
        &context(&client, &original)["task"],
        json!({"status": "done"}),
    );
    let other_task = task(&client, "Other task", "one_time");
    let other_result = result(&client, &other_task, "success", &[]);
    let foreign_project = call(
        &client,
        "create_project",
        json!({"name": "Foreign project", "description": "Isolated feedback"}),
    );
    let foreign_task = call(
        &client,
        "create_task",
        json!({
            "project_id": foreign_project["id"], "title": "Foreign task", "problem_or_goal": "Stay isolated",
            "design": "Use explicit project scope", "acceptance_criteria": ["No cross-project references"]
        }),
    );
    let foreign_result = call(
        &client,
        "record_task_result",
        json!({
            "project_id": foreign_project["id"], "task_id": foreign_task["id"],
            "expected_revision": foreign_task["revision"], "outcome": "success", "evidence": "Foreign execution passed."
        }),
    );
    let valid = json!({
        "project_id": done["project_id"], "task_id": done["id"], "result_id": first["id"],
        "expected_revision": done["revision"], "disposition": "changes_requested", "body": "Correct the reported output."
    });
    for body in [
        json!(""),
        json!(" \n\t"),
        json!("界".repeat(4_001)),
        Value::Null,
    ] {
        let mut input = valid.clone();
        input["body"] = body;
        rejected(&client, &database, "record_task_feedback", input);
    }
    for disposition in [json!("rejected"), json!("success"), Value::Null] {
        let mut input = valid.clone();
        input["disposition"] = disposition;
        rejected(&client, &database, "record_task_feedback", input);
    }
    for field in [
        "project_id",
        "task_id",
        "result_id",
        "expected_revision",
        "body",
    ] {
        let mut input = valid.clone();
        input.as_object_mut().unwrap().remove(field);
        rejected_raw(&client, &database, "record_task_feedback", input);
    }
    for (field, value) in [
        ("id", json!(123)),
        ("recorded_at", json!("2020-01-01T00:00:00Z")),
        ("task_revision", json!(40)),
        ("addressed_by_result_id", first["id"].clone()),
        ("project_id", json!("00000000-0000-7000-8000-000000000000")),
        ("project_id", foreign_project["id"].clone()),
        ("task_id", other_task["id"].clone()),
        ("task_id", foreign_task["id"].clone()),
        ("result_id", other_result["id"].clone()),
        ("result_id", foreign_result["id"].clone()),
        ("result_id", json!(999_999)),
        ("result_id", json!(0)),
    ] {
        let mut input = valid.clone();
        input[field] = value;
        rejected(&client, &database, "record_task_feedback", input);
    }
    let max_body = "界".repeat(4_000);
    let stored = feedback(&client, &done, &first, Some("changes_requested"), &max_body);
    assert_eq!(context(&client, &original)["task"]["status"], "todo");
    rejected(&other_client, &database, "record_task_feedback", valid);
    let current = context(&client, &original);
    assert_eq!(current["task"]["revision"], stored["task_revision"]);
    assert_eq!(
        outstanding_ids(&current),
        vec![stored["id"].as_i64().unwrap()]
    );
}

#[test]
fn resolution_rejects_foreign_missing_failed_or_reused_feedback_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("atomic-resolution.sqlite3");
    let client = Project::open(&database).unwrap();
    let original = task(&client, "Resolve the correct review", "one_time");
    let first = result(&client, &original, "success", &[]);
    let first_feedback = feedback(
        &client,
        &context(&client, &original)["task"],
        &first,
        Some("changes_requested"),
        "Fix the missing example.",
    );
    let second_feedback = feedback(
        &client,
        &context(&client, &original)["task"],
        &first,
        Some("changes_requested"),
        "Fix the wrong output.",
    );
    let other_task = task(&client, "Unrelated review", "one_time");
    let other_result = result(&client, &other_task, "success", &[]);
    let other_feedback = feedback(
        &client,
        &context(&client, &other_task)["task"],
        &other_result,
        None,
        "Unrelated task comment.",
    );
    let foreign_project = call(
        &client,
        "create_project",
        json!({"name": "Other project", "description": "Isolated reviews"}),
    );
    let foreign_task = call(
        &client,
        "create_task",
        json!({
            "project_id": foreign_project["id"], "title": "Foreign work", "problem_or_goal": "Scope reviews",
            "design": "Use project isolation", "acceptance_criteria": ["Feedback remains in its project"]
        }),
    );
    let foreign_result = call(
        &client,
        "record_task_result",
        json!({
            "project_id": foreign_project["id"], "task_id": foreign_task["id"], "expected_revision": foreign_task["revision"],
            "outcome": "success", "evidence": "Foreign work completed."
        }),
    );
    let foreign_feedback = call(
        &client,
        "record_task_feedback",
        json!({
            "project_id": foreign_project["id"], "task_id": foreign_task["id"], "result_id": foreign_result["id"],
            "expected_revision": foreign_result["task_revision"], "body": "Foreign comment."
        }),
    );
    let foreign_output = call(
        &client,
        "create_source",
        json!({
            "project_id": foreign_project["id"], "name": "Foreign output", "description": "Not allowed here", "content": "Foreign evidence"
        }),
    );
    let current = context(&client, &original);
    let valid = json!({
        "task_id": original["id"], "expected_revision": current["task"]["revision"],
        "outcome": "success", "evidence": "Corrected and checked both examples.",
        "addressed_feedback_ids": [first_feedback["id"], second_feedback["id"]]
    });
    for invalid_id in [
        json!(0),
        json!(999_999),
        other_feedback["id"].clone(),
        foreign_feedback["id"].clone(),
    ] {
        let mut input = valid.clone();
        input["addressed_feedback_ids"] = json!([first_feedback["id"], invalid_id]);
        rejected(&client, &database, "record_task_result", input);
    }
    let mut failed = valid.clone();
    failed["outcome"] = json!("failure");
    rejected(&client, &database, "record_task_result", failed);
    let mut invalid_output = valid.clone();
    invalid_output["output_source_ids"] = json!([foreign_output["id"]]);
    rejected(&client, &database, "record_task_result", invalid_output);
    assert_eq!(context(&client, &original), current);
    let partial = result(
        &client,
        &current["task"],
        "success",
        &[first_feedback["id"].as_i64().unwrap()],
    );
    let partial_context = context(&client, &original);
    assert_eq!(
        outstanding_ids(&partial_context),
        vec![second_feedback["id"].as_i64().unwrap()]
    );
    rejected(
        &client,
        &database,
        "update_task",
        json!({
            "task_id": original["id"], "expected_revision": partial_context["task"]["revision"], "status": "done"
        }),
    );
    let mut reused = valid.clone();
    reused["expected_revision"] = partial_context["task"]["revision"].clone();
    rejected(&client, &database, "record_task_result", reused);
    let mut stale = valid;
    stale["addressed_feedback_ids"] = json!([second_feedback["id"]]);
    rejected(&client, &database, "record_task_result", stale);
    assert_eq!(context(&client, &original), partial_context);
    let final_result = result(
        &client,
        &partial_context["task"],
        "success",
        &[second_feedback["id"].as_i64().unwrap()],
    );
    assert_eq!(
        context(&client, &original)["outstanding_feedback"],
        json!([])
    );
    let results = history(&client, &original);
    let reviewed = results
        .iter()
        .find(|entry| entry["id"] == first["id"])
        .unwrap();
    assert_eq!(
        reviewed["feedback"][0]["addressed_by_result_id"],
        partial["id"]
    );
    assert_eq!(
        reviewed["feedback"][1]["addressed_by_result_id"],
        final_result["id"]
    );
    assert_eq!(
        outstanding_ids(&context(&client, &other_task)),
        vec![other_feedback["id"].as_i64().unwrap()]
    );
}

#[test]
fn recurring_feedback_preserves_active_or_paused_lifecycle_and_can_be_addressed_while_paused() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("recurring-feedback.sqlite3");
    let client = Project::open(&database).unwrap();
    let recurring = task(&client, "Review repeated reports", "recurring");
    let first = result(&client, &recurring, "failure", &[]);
    let active_request = feedback(
        &client,
        &context(&client, &recurring)["task"],
        &first,
        Some("changes_requested"),
        "Include the failed endpoint in the report.",
    );
    assert_eq!(context(&client, &recurring)["task"]["status"], "active");
    let paused = update(
        &client,
        &context(&client, &recurring)["task"],
        json!({"status": "paused"}),
    );
    let paused_request = feedback(
        &client,
        &paused,
        &first,
        Some("changes_requested"),
        "Also state the next investigation step.",
    );
    assert_eq!(context(&client, &recurring)["task"]["status"], "paused");
    let resolved = result(
        &client,
        &context(&client, &recurring)["task"],
        "success",
        &[
            active_request["id"].as_i64().unwrap(),
            paused_request["id"].as_i64().unwrap(),
        ],
    );
    assert_eq!(
        resolved["addressed_feedback_ids"],
        json!([active_request["id"], paused_request["id"]])
    );
    let final_context = context(&client, &recurring);
    assert_eq!(final_context["task"]["status"], "paused");
    assert_eq!(final_context["outstanding_feedback"], json!([]));
    assert_eq!(final_context["latest_result"]["id"], resolved["id"]);
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(context(&restarted, &recurring), final_context);
}

#[test]
fn simultaneous_feedback_writers_have_one_revision_winner_and_one_atomic_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("concurrent-feedback.sqlite3");
    let client = Project::open(&database).unwrap();
    let original = task(&client, "Concurrent review", "one_time");
    let first = result(&client, &original, "success", &[]);
    let done = update(
        &client,
        &context(&client, &original)["task"],
        json!({"status": "done"}),
    );
    let barrier = Arc::new(Barrier::new(2));
    let writers: Vec<_> = (0..2)
        .map(|index| {
            let database = database.clone();
            let done = done.clone();
            let first = first.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let client = Project::open(database).unwrap();
                let input = json!({
                    "task_id": done["id"], "result_id": first["id"], "expected_revision": done["revision"],
                    "disposition": "changes_requested", "body": format!("Reviewer {index}: correct the example.")
                });
                barrier.wait();
                client.call::<Value>("record_task_feedback", input)
            })
        })
        .collect();
    let writes: Vec<_> = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect();
    assert_eq!(writes.iter().filter(|result| result.is_ok()).count(), 1);
    let conflict = writes
        .iter()
        .find_map(|result| result.as_ref().err())
        .unwrap();
    assert!(
        conflict.to_string().contains("revision"),
        "unexpected conflict: {conflict}"
    );
    let winner = writes.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(
        winner["task_revision"],
        done["revision"].as_i64().unwrap() + 1
    );
    let final_context = context(&client, &original);
    assert_eq!(final_context["task"]["status"], "todo");
    assert_eq!(final_context["task"]["revision"], winner["task_revision"]);
    assert_eq!(
        outstanding_ids(&final_context),
        vec![winner["id"].as_i64().unwrap()]
    );
    assert_eq!(final_context["latest_result"]["feedback"], json!([winner]));
    assert_eq!(history(&client, &original).len(), 1);
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(context(&restarted, &original), final_context);
}

#[test]
fn schema_v5_is_rejected_without_changing_its_data_schema_or_file() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("preserved-v5.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    // Freeze the previous schema as a fixture so future schema changes cannot update it.
    connection.execute_batch(r#"
        CREATE TABLE projects (
            id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND substr(id, 15, 1) = '7' AND substr(id, 20, 1) IN ('8','9','a','b')),
            name TEXT NOT NULL CHECK (length(trim(name)) > 0),
            description TEXT NOT NULL CHECK (length(trim(description)) > 0), created_at TEXT NOT NULL
        );
        CREATE TABLE tasks (
            id INTEGER PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id),
            kind TEXT NOT NULL DEFAULT 'one_time' CHECK (kind IN ('one_time', 'recurring')),
            trigger_event TEXT CHECK (trigger_event IS NULL OR (kind = 'recurring' AND length(trim(trigger_event)) > 0)),
            title TEXT NOT NULL CHECK (length(trim(title)) > 0),
            problem_or_goal TEXT NOT NULL CHECK (length(trim(problem_or_goal)) > 0),
            design TEXT NOT NULL CHECK (length(trim(design)) > 0),
            acceptance_criteria TEXT NOT NULL CHECK (json_valid(acceptance_criteria)), alternatives_rejected TEXT,
            status TEXT NOT NULL CHECK ((kind = 'one_time' AND status IN ('todo', 'in_progress', 'blocked', 'done')) OR (kind = 'recurring' AND status IN ('active', 'paused'))),
            progress TEXT, revision INTEGER NOT NULL CHECK (revision >= 1),
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL, UNIQUE (project_id, id)
        );
        CREATE TABLE sources (
            id INTEGER PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id),
            name TEXT NOT NULL CHECK (length(trim(name)) > 0),
            description TEXT NOT NULL CHECK (length(trim(description)) > 0), content TEXT, locator TEXT,
            context_policy TEXT NOT NULL CHECK (context_policy IN ('always', 'when_relevant')),
            action TEXT CHECK (action IS NULL OR json_valid(action)),
            snapshot TEXT CHECK (snapshot IS NULL OR json_valid(snapshot)),
            latest_attempt TEXT CHECK (latest_attempt IS NULL OR json_valid(latest_attempt)),
            revision INTEGER NOT NULL CHECK (revision >= 1), created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
            UNIQUE (project_id, id), CHECK ((content IS NOT NULL) != (locator IS NOT NULL))
        );
        CREATE TABLE decisions (
            id INTEGER PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id),
            statement TEXT NOT NULL CHECK (length(trim(statement)) > 0),
            rationale TEXT NOT NULL CHECK (length(trim(rationale)) > 0), supersedes_id INTEGER UNIQUE,
            created_at TEXT NOT NULL, UNIQUE (project_id, id),
            FOREIGN KEY (project_id, supersedes_id) REFERENCES decisions(project_id, id),
            CHECK (supersedes_id IS NULL OR supersedes_id != id)
        );
        CREATE TABLE task_context_sources (
            project_id TEXT NOT NULL REFERENCES projects(id), task_id INTEGER NOT NULL, source_id INTEGER NOT NULL,
            FOREIGN KEY (project_id, task_id) REFERENCES tasks(project_id, id),
            FOREIGN KEY (project_id, source_id) REFERENCES sources(project_id, id), PRIMARY KEY (task_id, source_id)
        );
        CREATE TABLE task_dependencies (
            project_id TEXT NOT NULL REFERENCES projects(id), task_id INTEGER NOT NULL, depends_on_task_id INTEGER NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('before_start', 'before_done')),
            FOREIGN KEY (project_id, task_id) REFERENCES tasks(project_id, id),
            FOREIGN KEY (project_id, depends_on_task_id) REFERENCES tasks(project_id, id),
            PRIMARY KEY (task_id, depends_on_task_id, kind), CHECK (task_id != depends_on_task_id)
        );
        CREATE TABLE task_results (
            id INTEGER PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id), task_id INTEGER NOT NULL,
            outcome TEXT NOT NULL CHECK (outcome IN ('success', 'no_change', 'failure')),
            evidence TEXT NOT NULL CHECK (length(trim(evidence)) > 0 AND length(evidence) <= 4000),
            recorded_at TEXT NOT NULL, task_revision INTEGER NOT NULL CHECK (task_revision > 1),
            UNIQUE (project_id, id), UNIQUE (task_id, task_revision),
            FOREIGN KEY (project_id, task_id) REFERENCES tasks(project_id, id)
        );
        CREATE TABLE task_result_sources (
            project_id TEXT NOT NULL REFERENCES projects(id), result_id INTEGER NOT NULL, source_id INTEGER NOT NULL,
            FOREIGN KEY (project_id, result_id) REFERENCES task_results(project_id, id),
            FOREIGN KEY (project_id, source_id) REFERENCES sources(project_id, id), PRIMARY KEY (result_id, source_id)
        );
        CREATE INDEX tasks_status ON tasks(project_id, status);
        CREATE INDEX tasks_kind ON tasks(project_id, kind);
        CREATE INDEX task_results_task ON task_results(project_id, task_id, id);
        CREATE INDEX task_results_success ON task_results(project_id, task_id, id) WHERE outcome IN ('success', 'no_change');
        CREATE INDEX task_result_sources_source ON task_result_sources(source_id);
        CREATE INDEX task_context_sources_source ON task_context_sources(source_id);
        CREATE INDEX task_dependencies_target ON task_dependencies(depends_on_task_id);
        CREATE INDEX sources_project ON sources(project_id);
        CREATE INDEX decisions_project ON decisions(project_id);
        CREATE INDEX projects_name ON projects(name);
        PRAGMA application_id = 1145652052;
        PRAGMA user_version = 5;
        INSERT INTO projects VALUES ('0199b3d8-f600-7000-8000-000000000001', 'Keep this project', 'Existing v5 workspace', '2026-10-19T00:00:00Z');
        INSERT INTO tasks VALUES (1, '0199b3d8-f600-7000-8000-000000000001', 'one_time', NULL, 'Keep this task', 'Preserve the work', 'Existing design', '["Preserve evidence"]', NULL, 'done', 'Existing progress', 3, '2026-10-19T00:00:00Z', '2026-10-19T00:00:00Z');
        INSERT INTO sources VALUES (1, '0199b3d8-f600-7000-8000-000000000001', 'Keep this output', 'Existing evidence', 'Complete original report', NULL, 'when_relevant', NULL, NULL, NULL, 1, '2026-10-19T00:00:00Z', '2026-10-19T00:00:00Z');
        INSERT INTO task_results VALUES (1, '0199b3d8-f600-7000-8000-000000000001', 1, 'success', 'Original verification passed.', '2026-10-19T00:00:00Z', 2);
        INSERT INTO task_result_sources VALUES ('0199b3d8-f600-7000-8000-000000000001', 1, 1);
    "#).unwrap();
    drop(connection);
    let before = std::fs::read(&database).unwrap();
    let records = snapshot(&database);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_dictum-mcp"))
        .env("DICTUM_DATABASE", &database)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("version 5"), "unexpected error: {error}");
    assert!(
        error.contains("schema version 7"),
        "unexpected error: {error}"
    );
    assert_eq!(std::fs::read(&database).unwrap(), before);
    assert_eq!(snapshot(&database), records);
    let connection = rusqlite::Connection::open(&database).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 5);
    let feedback_tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name LIKE 'task_feedback%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(feedback_tables, 0);
}
