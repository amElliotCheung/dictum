//! Explicit project scope and isolation through the actual MCP transport.
mod support;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, Barrier},
};
use support::Project;

fn call(client: &Project, name: &str, input: Value) -> Value {
    client.raw_call(name, input).unwrap()
}

fn scoped(client: &Project, project_id: &str, name: &str, mut input: Value) -> Value {
    input["project_id"] = json!(project_id);
    call(client, name, input)
}

fn brief() -> Value {
    json!({"topics": ["Project workflow"], "title": "Implement feature", "problem_or_goal": "Keep work isolated", "design": "Use explicit project scope", "acceptance_criteria": ["Only this project's records are used"]})
}

fn project(client: &Project, name: &str, description: &str) -> String {
    call(
        client,
        "create_project",
        json!({"name": name, "description": description}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn snapshot(database: &std::path::Path) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    let connection = rusqlite::Connection::open(database).unwrap();
    let tables = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    tables
        .iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                .unwrap();
            let columns = statement.column_count();
            statement
                .query_map([], |row| {
                    (0..columns).map(|column| row.get(column)).collect()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        })
        .collect()
}

fn rejected_without_mutation(
    client: &Project,
    database: &std::path::Path,
    name: &str,
    input: Value,
) {
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
fn project_discovery_is_explicit_uuidv7_and_persistent() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("shared.sqlite3");
    let client = Project::open(&database).unwrap();
    let before = Utc::now().timestamp_millis() as u64;
    let first = project(&client, "Same name", "Compiler service");
    let second = project(&client, "Same name", "Release service");
    let third = project(&client, "Other", "Compiler tools");
    assert_eq!(BTreeSet::from([&first, &second, &third]).len(), 3);
    let after = Utc::now().timestamp_millis() as u64;
    for id in [&first, &second, &third] {
        assert_eq!(id.len(), 36);
        for position in [8, 13, 18, 23] {
            assert_eq!(&id[position..position + 1], "-");
        }
        assert_eq!(&id[14..15], "7");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        let compact = id.replace('-', "");
        assert!(
            compact
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
        );
        let millis = u64::from_str_radix(&compact[..12], 16).unwrap();
        assert!((before..=after).contains(&millis));
    }
    let matching = call(&client, "list_projects", json!({"query": "SAME NAME"}));
    assert_eq!(matching["projects"].as_array().unwrap().len(), 2);
    assert_eq!(
        call(&client, "list_projects", json!({"query": "COMPILER"}))["projects"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let all = call(&client, "list_projects", json!({}));
    assert_eq!(
        call(
            &client,
            "list_projects",
            json!({"query": "nothing matches this"})
        )["projects"],
        json!([])
    );
    assert_eq!(call(&client, "list_projects", json!({})), all);
    for input in [
        json!({"name": " ", "description": "Valid"}),
        json!({"name": "Valid", "description": "\n"}),
        json!({"name": "Valid", "description": "Valid", "id": first}),
    ] {
        rejected_without_mutation(&client, &database, "create_project", input);
    }
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(call(&restarted, "list_projects", json!({})), all);
}

#[test]
fn project_description_edits_preserve_identity_context_and_restart_state() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("shared.sqlite3");
    let client = Project::open(&database).unwrap();
    let original = call(
        &client,
        "create_project",
        json!({"name": "Same name", "description": "Original compiler purpose"}),
    );
    let other = call(
        &client,
        "create_project",
        json!({"name": "Same name", "description": "Independent release purpose"}),
    );
    assert_eq!(original["revision"], 1);
    assert_eq!(original["updated_at"], original["created_at"]);
    let scope = original["id"].as_str().unwrap();
    let source = scoped(
        &client,
        scope,
        "create_source",
        json!({"topics": ["Project workflow"], "name": "Requirements", "description": "Project context", "content": "Keep these records", "context_policy": "always"}),
    );
    let dependency = scoped(&client, scope, "create_task", brief());
    let mut task_input = brief();
    task_input["context_source_ids"] = json!([source["id"]]);
    task_input["prerequisite_task_ids"] = json!([dependency["id"]]);
    let task = scoped(&client, scope, "create_task", task_input);
    let result = scoped(
        &client,
        scope,
        "record_task_comment",
        json!({"task_id": task["id"], "task_revision": task["revision"], "body": "Fixture prerequisite remains unmet", "execution_result": {"outcome": "failure", "output_source_ids": [source["id"]]}}),
    );
    scoped(
        &client,
        scope,
        "record_task_comment",
        json!({"task_id": task["id"], "reply_to_comment_id": result["id"], "task_revision": result["task_revision"], "body": "Preserve this feedback"}),
    );
    scoped(
        &client,
        scope,
        "record_decision",
        json!({"topics": ["Project workflow"], "statement": "Preserve project history", "rationale": "Description edits retain context"}),
    );
    let mut context = scoped(
        &client,
        scope,
        "get_context",
        json!({"task_id": task["id"]}),
    );
    let before = Utc::now();
    let description = "# Revised purpose\n\n支持协作 and preserve exact text.  ";
    let updated = call(
        &client,
        "update_project",
        json!({"project_id": scope, "expected_revision": original["revision"], "description": description}),
    );
    let after = Utc::now();
    assert_eq!(updated["description"], description);
    assert_eq!(updated["revision"], 2);
    for field in ["id", "name", "created_at"] {
        assert_eq!(updated[field], original[field]);
    }
    let edited_at = DateTime::parse_from_rfc3339(updated["updated_at"].as_str().unwrap()).unwrap();
    assert_eq!(edited_at.offset().local_minus_utc(), 0);
    assert!(edited_at >= before && edited_at <= after);
    context["project"] = updated.clone();
    assert_eq!(
        scoped(
            &client,
            scope,
            "get_context",
            json!({"task_id": task["id"]})
        ),
        context
    );
    assert_eq!(
        scoped(
            &client,
            scope,
            "get_source",
            json!({"source_id": source["id"]})
        ),
        source
    );
    assert_eq!(
        call(
            &client,
            "list_projects",
            json!({"query": "ORIGINAL COMPILER"})
        )["projects"],
        json!([])
    );
    assert_eq!(
        call(
            &client,
            "list_projects",
            json!({"query": "REVISED PURPOSE"})
        )["projects"],
        json!([updated])
    );
    let same_text = call(
        &client,
        "update_project",
        json!({"project_id": scope, "expected_revision": updated["revision"], "description": description}),
    );
    assert_eq!(same_text["revision"], 3);
    assert_eq!(same_text["description"], description);
    assert_eq!(same_text["created_at"], original["created_at"]);
    let listed = call(&client, "list_projects", json!({}));
    assert!(listed["projects"].as_array().unwrap().contains(&other));
    assert!(listed["projects"].as_array().unwrap().contains(&same_text));
    context["project"] = same_text;
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(call(&restarted, "list_projects", json!({})), listed);
    assert_eq!(
        scoped(
            &restarted,
            scope,
            "get_context",
            json!({"task_id": task["id"]})
        ),
        context
    );
}

#[test]
fn invalid_project_edits_do_not_mutate_durable_records() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("shared.sqlite3");
    let client = Project::open(&database).unwrap();
    let scope = client.project_id.clone();
    let valid =
        json!({"project_id": scope, "expected_revision": 1, "description": "Updated purpose"});
    for field in ["project_id", "expected_revision", "description"] {
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(field);
        rejected_without_mutation(&client, &database, "update_project", missing);
        let mut null = valid.clone();
        null[field] = Value::Null;
        rejected_without_mutation(&client, &database, "update_project", null);
    }
    for description in [json!(""), json!(" \t\n"), json!(14), json!(["purpose"])] {
        let mut input = valid.clone();
        input["description"] = description;
        rejected_without_mutation(&client, &database, "update_project", input);
    }
    for revision in [json!(0), json!(-1), json!(2), json!("1"), json!(1.5)] {
        let mut input = valid.clone();
        input["expected_revision"] = revision;
        rejected_without_mutation(&client, &database, "update_project", input);
    }
    for field in ["id", "name", "created_at", "updated_at", "revision"] {
        let mut input = valid.clone();
        input[field] = json!("Replacement");
        rejected_without_mutation(&client, &database, "update_project", input);
    }
    let updated = call(&client, "update_project", valid.clone());
    assert_eq!(updated["revision"], 2);
    rejected_without_mutation(&client, &database, "update_project", valid);
}

#[test]
fn simultaneous_project_edits_allow_one_winner_and_require_conflict_reread() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("shared.sqlite3");
    let client = Project::open(&database).unwrap();
    let scope = client.project_id.clone();
    let barrier = Arc::new(Barrier::new(2));
    let writers: Vec<_> = (0..2)
        .map(|index| {
            let database = database.clone();
            let scope = scope.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let client = Project::open(database).unwrap();
                barrier.wait();
                client.raw_call::<Value>(
                    "update_project",
                    json!({"project_id": scope, "expected_revision": 1, "description": format!("Writer {index}")}),
                )
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
        .unwrap()
        .to_string();
    assert!(conflict.contains(&scope), "unexpected conflict: {conflict}");
    assert!(
        conflict.contains("revision conflict"),
        "unexpected conflict: {conflict}"
    );
    assert!(
        conflict.contains("reread"),
        "unexpected conflict: {conflict}"
    );
    let winner = writes.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(winner["revision"], 2);
    assert_eq!(
        call(&client, "list_projects", json!({}))["projects"],
        json!([winner])
    );
    let retry = call(
        &client,
        "update_project",
        json!({"project_id": scope, "expected_revision": winner["revision"], "description": "Reconciled after rereading"}),
    );
    assert_eq!(retry["revision"], 3);
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(
        call(&restarted, "list_projects", json!({}))["projects"],
        json!([retry])
    );
}

#[test]
fn every_existing_tool_requires_a_known_explicit_project_without_side_effects() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("shared.sqlite3");
    let client = Project::open(&database).unwrap();
    let scope = client.project_id.clone();
    let source = scoped(
        &client,
        &scope,
        "create_source",
        json!({"topics": ["Project workflow"], "name": "Input", "description": "Scoped source", "locator": "https://example.test/input", "action": {"instruction": "Read", "trigger": {"kind": "manual"}}}),
    );
    let task = scoped(&client, &scope, "create_task", brief());
    let requests = [
        (
            "update_project",
            json!({"expected_revision": 1, "description": "Should never be saved"}),
        ),
        ("create_task", brief()),
        ("list_tasks", json!({})),
        ("get_context", json!({"task_id": task["id"]})),
        ("list_task_comments", json!({"task_id": task["id"]})),
        (
            "get_task_comment",
            json!({"task_id": task["id"], "comment_id": 1}),
        ),
        (
            "record_task_comment",
            json!({"task_id": task["id"], "task_revision": task["revision"], "body": "Should never be saved", "execution_result": {"outcome": "success"}}),
        ),
        (
            "update_task",
            json!({"task_id": task["id"], "expected_revision": task["revision"], "title": "Should never be saved"}),
        ),
        (
            "create_source",
            json!({"topics": ["Project workflow"], "name": "New", "description": "Scoped source", "content": "Document"}),
        ),
        ("list_sources", json!({})),
        ("get_source", json!({"source_id": source["id"]})),
        (
            "update_source",
            json!({"source_id": source["id"], "expected_revision": source["revision"], "name": "Should never be saved"}),
        ),
        (
            "record_source_action",
            json!({"source_id": source["id"], "expected_revision": source["revision"], "result": {"outcome": "failure", "error": "Should never be saved"}}),
        ),
        (
            "record_decision",
            json!({"topics": ["Project workflow"], "statement": "Should never be saved", "rationale": "Requires explicit scope"}),
        ),
        ("list_decisions", json!({})),
    ];
    for (name, input) in requests {
        rejected_without_mutation(&client, &database, name, input.clone());
        for project_id in [
            Value::Null,
            json!(4),
            json!(""),
            json!(" "),
            json!("malformed"),
            json!("00000000-0000-7000-8000-000000000001"),
        ] {
            let mut input = input.clone();
            input["project_id"] = project_id;
            rejected_without_mutation(&client, &database, name, input);
        }
    }
    for (name, mut input) in [
        (
            "get_context",
            json!({"task_id": "ffffffff-ffff-7fff-bfff-ffffffffffff"}),
        ),
        (
            "list_task_comments",
            json!({"task_id": "ffffffff-ffff-7fff-bfff-ffffffffffff"}),
        ),
        (
            "record_task_comment",
            json!({"task_id": "ffffffff-ffff-7fff-bfff-ffffffffffff", "task_revision": 1, "body": "Unknown task", "execution_result": {"outcome": "success"}}),
        ),
        (
            "update_task",
            json!({"task_id": "ffffffff-ffff-7fff-bfff-ffffffffffff", "expected_revision": 1, "title": "Unknown task"}),
        ),
        (
            "list_sources",
            json!({"task_id": "ffffffff-ffff-7fff-bfff-ffffffffffff"}),
        ),
        (
            "get_source",
            json!({"source_id": "ffffffff-ffff-7fff-bfff-ffffffffffff"}),
        ),
        (
            "update_source",
            json!({"source_id": "ffffffff-ffff-7fff-bfff-ffffffffffff", "expected_revision": 1, "name": "Unknown source"}),
        ),
        (
            "record_source_action",
            json!({"source_id": "ffffffff-ffff-7fff-bfff-ffffffffffff", "expected_revision": 1, "result": {"outcome": "failure", "error": "Unknown source"}}),
        ),
        (
            "record_decision",
            json!({"topics": ["Project workflow"], "statement": "Unknown predecessor", "rationale": "Invalid link", "supersedes_id": "ffffffff-ffff-7fff-bfff-ffffffffffff"}),
        ),
    ] {
        input["project_id"] = json!(scope);
        rejected_without_mutation(&client, &database, name, input);
    }
    for field in [
        "context_source_ids",
        "prerequisite_task_ids",
        "completion_task_ids",
    ] {
        let mut patch = json!({"project_id": scope, "task_id": task["id"], "expected_revision": task["revision"], "title": "Unknown link must roll back"});
        patch[field] = json!(["ffffffff-ffff-7fff-bfff-ffffffffffff"]);
        rejected_without_mutation(&client, &database, "update_task", patch);
        let mut input = brief();
        input["project_id"] = json!(scope);
        input[field] = json!(["ffffffff-ffff-7fff-bfff-ffffffffffff"]);
        rejected_without_mutation(&client, &database, "create_task", input);
    }
}

#[test]
fn projects_isolate_records_links_context_actions_and_supersession_across_clients() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("shared.sqlite3");
    let first = Project::open(&database).unwrap();
    let a = first.project_id.clone();
    let b = project(&first, "Second", "Independent project");
    let second = Project::open(&database).unwrap();
    let mut records = Vec::new();
    for (client, scope) in [(&first, &a), (&second, &b)] {
        let always = scoped(
            client,
            scope,
            "create_source",
            json!({"topics": ["Project workflow"], "name": "Policy", "description": "Always context", "content": format!("Policy for {scope}"), "context_policy": "always"}),
        );
        let external = scoped(
            client,
            scope,
            "create_source",
            json!({"topics": ["Project workflow"], "name": "Action", "description": "Own action state", "locator": format!("https://example.test/{scope}"), "action": {"instruction": "Read", "trigger": {"kind": "interval", "every_minutes": 60}}}),
        );
        let decision = scoped(
            client,
            scope,
            "record_decision",
            json!({"topics": ["Project workflow"], "statement": format!("Choice for {scope}"), "rationale": "Scoped decision", "alternatives_rejected": format!("- Rejected alternative private to {scope}")}),
        );
        let prerequisite = scoped(client, scope, "create_task", brief());
        let mut input = brief();
        input["context_source_ids"] = json!([external["id"]]);
        input["completion_task_ids"] = json!([prerequisite["id"]]);
        input["priority"] = json!(if scope == &a { "low" } else { "high" });
        let task = scoped(client, scope, "create_task", input);
        records.push((always, external, decision, prerequisite, task));
    }
    let (a_always, a_external, a_decision, a_dependency, a_task) = &records[0];
    let (_, b_external, b_decision, b_dependency, b_task) = &records[1];
    for (scope, foreign_task, foreign_source, foreign_decision) in [
        (&a, b_task, b_external, b_decision),
        (&b, a_task, a_external, a_decision),
    ] {
        for (name, mut input) in [
            ("get_context", json!({"task_id": foreign_task["id"]})),
            ("list_task_comments", json!({"task_id": foreign_task["id"]})),
            (
                "record_task_comment",
                json!({"task_id": foreign_task["id"], "task_revision": foreign_task["revision"], "body": "Foreign report", "execution_result": {"outcome": "failure"}}),
            ),
            (
                "update_task",
                json!({"task_id": foreign_task["id"], "expected_revision": foreign_task["revision"], "title": "Foreign edit", "priority": "medium"}),
            ),
            ("list_sources", json!({"task_id": foreign_task["id"]})),
            ("get_source", json!({"source_id": foreign_source["id"]})),
            (
                "update_source",
                json!({"source_id": foreign_source["id"], "expected_revision": foreign_source["revision"], "name": "Foreign edit"}),
            ),
            (
                "record_source_action",
                json!({"source_id": foreign_source["id"], "expected_revision": foreign_source["revision"], "result": {"outcome": "failure", "error": "Foreign edit"}}),
            ),
            (
                "record_decision",
                json!({"topics": ["Project workflow"], "statement": "Foreign successor", "rationale": "Invalid link", "alternatives_rejected": "This rejected write must not replace private alternatives.", "supersedes_id": foreign_decision["id"]}),
            ),
        ] {
            input["project_id"] = json!(scope);
            rejected_without_mutation(&first, &database, name, input);
        }
    }
    for field in [
        "context_source_ids",
        "prerequisite_task_ids",
        "completion_task_ids",
    ] {
        let foreign_id = if field.contains("source") {
            &b_external["id"]
        } else {
            &b_dependency["id"]
        };
        let mut patch = json!({"project_id": a, "task_id": a_task["id"], "expected_revision": a_task["revision"], "title": "Partial write must roll back"});
        patch[field] = json!([foreign_id]);
        rejected_without_mutation(&first, &database, "update_task", patch);
        let mut input = brief();
        input["project_id"] = json!(a);
        input[field] = json!([foreign_id]);
        rejected_without_mutation(&first, &database, "create_task", input);
    }
    for (scope, (_, external, decision, dependency, task)) in [(&a, &records[0]), (&b, &records[1])]
    {
        let context = scoped(
            &second,
            scope,
            "get_context",
            json!({"task_id": task["id"]}),
        );
        assert_eq!(context["task"]["project_id"], *scope);
        assert_eq!(
            context["task"]["priority"],
            if scope == &a { "low" } else { "high" }
        );
        let prioritized = scoped(
            &first,
            scope,
            "list_tasks",
            json!({"priority": if scope == &a { "low" } else { "high" }}),
        );
        assert_eq!(prioritized["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(prioritized["tasks"][0]["id"], task["id"]);
        assert_eq!(prioritized["tasks"][0]["project_id"], *scope);

        assert_eq!(context["sources"].as_array().unwrap().len(), 2);
        assert!(
            context["sources"]
                .as_array()
                .unwrap()
                .iter()
                .all(|source| source["project_id"] == *scope)
        );
        assert_eq!(context["current_decisions"][0]["id"], decision["id"]);
        assert_eq!(context["current_decisions"][0]["project_id"], *scope);
        assert_eq!(
            context["current_decisions"][0]["alternatives_rejected"],
            format!("- Rejected alternative private to {scope}")
        );
        assert_eq!(context["current_decisions"], json!([decision]));
        assert_eq!(
            context["dependency_state"]["unmet_completion_task_ids"],
            json!([dependency["id"]])
        );
        assert_eq!(
            scoped(&first, scope, "list_tasks", json!({}))["tasks"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let sources = scoped(
            &first,
            scope,
            "list_sources",
            json!({"task_id": task["id"]}),
        );
        assert_eq!(sources["sources"].as_array().unwrap().len(), 1);
        assert!(
            sources["sources"]
                .as_array()
                .unwrap()
                .iter()
                .all(|source| source["project_id"] == *scope)
        );
        assert_eq!(
            scoped(&first, scope, "list_sources", json!({"action_due": true}))["sources"][0]["id"],
            external["id"]
        );
    }
    let observed_at = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
    let changed = scoped(
        &second,
        &a,
        "record_source_action",
        json!({"source_id": a_external["id"], "expected_revision": a_external["revision"], "result": {"outcome": "success", "observed_at": observed_at, "summary": "Only A is refreshed.", "references": []}}),
    );
    assert_eq!(changed["project_id"], a);
    assert_eq!(
        scoped(&first, &a, "list_sources", json!({"action_due": true}))["sources"],
        json!([])
    );
    assert_eq!(
        scoped(
            &first,
            &b,
            "get_source",
            json!({"source_id": b_external["id"]})
        ),
        *b_external
    );
    let replacement = scoped(
        &first,
        &a,
        "record_decision",
        json!({"topics": ["Project workflow"], "statement": "New A choice", "rationale": "Applies only to A", "alternatives_rejected": "- Keep the former A choice: superseded for A only.", "supersedes_id": a_decision["id"]}),
    );
    assert_eq!(
        scoped(&second, &b, "list_decisions", json!({}))["decisions"],
        json!([b_decision])
    );
    assert_eq!(
        scoped(&second, &a, "list_decisions", json!({}))["decisions"],
        json!([replacement])
    );
    let a_history = scoped(
        &second,
        &a,
        "list_decisions",
        json!({"include_superseded": true}),
    );
    let retained = a_history["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == a_decision["id"])
        .unwrap();
    assert_eq!(
        retained["alternatives_rejected"],
        a_decision["alternatives_rejected"]
    );
    assert_ne!(
        retained["alternatives_rejected"],
        replacement["alternatives_rejected"]
    );
    assert!(
        a_history["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["project_id"] == a)
    );
    assert_eq!(
        scoped(
            &second,
            &a,
            "get_source",
            json!({"source_id": a_always["id"]})
        ),
        *a_always
    );
    assert_eq!(
        scoped(
            &second,
            &a,
            "get_context",
            json!({"task_id": a_dependency["id"]})
        )["task"]["project_id"],
        a
    );
    rejected_without_mutation(&second, &database, "list_tasks", json!({}));
    drop(first);
    drop(second);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(
        call(&restarted, "list_projects", json!({}))["projects"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        scoped(
            &restarted,
            &a,
            "get_source",
            json!({"source_id": a_external["id"]})
        ),
        changed
    );
    assert_eq!(
        scoped(
            &restarted,
            &b,
            "get_context",
            json!({"task_id": b_task["id"]})
        )["task"],
        *b_task
    );
    assert_eq!(
        scoped(
            &restarted,
            &a,
            "list_decisions",
            json!({"include_superseded": true})
        ),
        a_history
    );
    assert_eq!(
        scoped(&restarted, &b, "list_decisions", json!({}))["decisions"],
        json!([b_decision])
    );
}

#[test]
fn ready_filter_is_project_scoped_and_only_reports_startable_todo_tasks() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("shared.sqlite3");
    let client = Project::open(&database).unwrap();
    let a = client.project_id.clone();
    let b = project(&client, "Second", "Another readiness graph");
    let mut expected = Vec::new();
    for scope in [&a, &b] {
        let ready = scoped(&client, scope, "create_task", brief());
        let prerequisite = scoped(&client, scope, "create_task", brief());
        for status in ["in_progress", "blocked", "done"] {
            let task = scoped(&client, scope, "create_task", brief());
            let revision = if status == "done" {
                let result = scoped(
                    &client,
                    scope,
                    "record_task_comment",
                    json!({"task_id": task["id"], "task_revision": task["revision"], "body": "Fixture verification result", "execution_result": {"outcome": "success"}}),
                );
                result["task_revision"].clone()
            } else {
                task["revision"].clone()
            };
            scoped(
                &client,
                scope,
                "update_task",
                json!({"task_id": task["id"], "expected_revision": revision, "status": status}),
            );
        }
        let mut start_blocked = brief();
        start_blocked["prerequisite_task_ids"] = json!([prerequisite["id"]]);
        scoped(&client, scope, "create_task", start_blocked);
        let mut completion_blocked = brief();
        completion_blocked["completion_task_ids"] = json!([prerequisite["id"]]);
        let completion_blocked = scoped(&client, scope, "create_task", completion_blocked);
        expected.push(BTreeSet::from([
            ready["id"].as_str().unwrap().to_owned(),
            prerequisite["id"].as_str().unwrap().to_owned(),
            completion_blocked["id"].as_str().unwrap().to_owned(),
        ]));
    }
    for (scope, expected) in [&a, &b].into_iter().zip(expected) {
        let result = scoped(
            &client,
            scope,
            "list_tasks",
            json!({"ready_to_start": true}),
        );
        let tasks = result["tasks"].as_array().unwrap();
        assert_eq!(
            tasks
                .iter()
                .map(|task| task["id"].as_str().unwrap().to_owned())
                .collect::<BTreeSet<_>>(),
            expected
        );
        assert!(tasks.iter().all(|task| task["status"] == "todo"
            && task["project_id"] == *scope
            && task["dependency_state"]["ready_to_start"] == true));
    }
}

#[test]
fn database_foreign_keys_also_prevent_cross_project_relationships() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("shared.sqlite3");
    let client = Project::open(&database).unwrap();
    let a = client.project_id.clone();
    let b = project(&client, "Second", "Foreign key fixture");
    let a_task = scoped(&client, &a, "create_task", brief());
    let b_task = scoped(&client, &b, "create_task", brief());
    let b_source = scoped(
        &client,
        &b,
        "create_source",
        json!({"topics": ["Project workflow"], "name": "Foreign source", "description": "Fixture", "content": "Document"}),
    );
    let b_decision = scoped(
        &client,
        &b,
        "record_decision",
        json!({"topics": ["Project workflow"], "statement": "Foreign choice", "rationale": "Fixture"}),
    );
    let before = snapshot(&database);
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    assert!(connection.execute("INSERT INTO task_context_sources (project_id, task_id, source_id) VALUES (?1, ?2, ?3)", rusqlite::params![a, a_task["id"].as_str().unwrap().to_owned(), b_source["id"].as_str().unwrap().to_owned()]).is_err());
    for kind in ["before_start", "before_done"] {
        assert!(connection.execute("INSERT INTO task_dependencies (project_id, task_id, depends_on_task_id, kind) VALUES (?1, ?2, ?3, ?4)", rusqlite::params![a, a_task["id"].as_str().unwrap().to_owned(), b_task["id"].as_str().unwrap().to_owned(), kind]).is_err());
    }
    assert!(connection.execute("INSERT INTO decisions (id, project_id, statement, rationale, supersedes_id, created_at) VALUES ('ffffffff-ffff-7fff-bfff-ffffffffffff', ?1, 'Foreign replacement', 'Fixture', ?2, '2026-01-01T00:00:00Z')", rusqlite::params![a, b_decision["id"].as_str().unwrap().to_owned()]).is_err());
    assert_eq!(snapshot(&database), before);
}

#[test]
fn all_record_and_relationship_ids_are_canonical_uuidv7_and_persist() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("uuidv7.sqlite3");
    let client = Project::open(&database).unwrap();
    let scope = client.project_id.clone();
    let before = Utc::now().timestamp_millis() as u64;
    let source = scoped(
        &client,
        &scope,
        "create_source",
        json!({
            "topics": ["UUID identity"], "name": "UUID source", "description": "Relationship target", "content": "Evidence"
        }),
    );
    let parent = scoped(&client, &scope, "create_task", brief());
    let gate = scoped(&client, &scope, "create_task", brief());
    let mut child_input = brief();
    child_input["parent_task_id"] = parent["id"].clone();
    child_input["context_source_ids"] = json!([source["id"]]);
    child_input["prerequisite_task_ids"] = json!([gate["id"]]);
    let child = scoped(&client, &scope, "create_task", child_input);
    let note = scoped(
        &client,
        &scope,
        "record_task_comment",
        json!({
            "task_id": child["id"], "task_revision": 1, "body": "UUID output and reply target.",
            "execution_result": {"outcome": "failure", "output_source_ids": [source["id"]]}
        }),
    );
    let reply = scoped(
        &client,
        &scope,
        "record_task_comment",
        json!({
            "task_id": child["id"], "task_revision": 1, "body": "UUID reply.", "reply_to_comment_id": note["id"]
        }),
    );
    let first = scoped(
        &client,
        &scope,
        "record_decision",
        json!({
            "topics": ["UUID identity"], "statement": "Use UUIDv7", "rationale": "One ID contract"
        }),
    );
    let replacement = scoped(
        &client,
        &scope,
        "record_decision",
        json!({
            "topics": ["UUID identity"], "statement": "Use canonical UUIDv7 everywhere", "rationale": "Unify consumption", "supersedes_id": first["id"]
        }),
    );
    let after = Utc::now().timestamp_millis() as u64;
    let context = scoped(
        &client,
        &scope,
        "get_context",
        json!({"task_id": child["id"]}),
    );
    assert_eq!(context["parent_task"], parent);
    assert_eq!(context["task"]["context_source_ids"], json!([source["id"]]));
    assert_eq!(
        context["dependency_state"]["unmet_prerequisite_task_ids"],
        json!([gate["id"]])
    );
    assert_eq!(context["latest_execution"], note);
    assert_eq!(context["recent_comments"][0], reply);
    assert_eq!(
        context["current_decisions"][0]["current_decision_id"],
        replacement["id"]
    );
    assert_eq!(
        context["current_decisions"][0]["supersedes_id"],
        first["id"]
    );

    let connection = rusqlite::Connection::open(&database).unwrap();
    let mut identities = BTreeSet::new();
    for table in [
        "projects",
        "tasks",
        "topics",
        "sources",
        "decisions",
        "task_comments",
    ] {
        let ids = connection
            .prepare(&format!("SELECT id FROM {table} ORDER BY id"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(!ids.is_empty(), "missing identity coverage for {table}");
        for id in ids {
            let uuid = uuid::Uuid::parse_str(&id).unwrap();
            assert_eq!(uuid.get_version_num(), 7, "{table}: {id}");
            assert_eq!(uuid.to_string(), id);
            assert!(
                identities.insert(id.clone()),
                "duplicate identity across records: {id}"
            );
            if table != "projects" {
                let millis = u64::from_str_radix(&id.replace('-', "")[..12], 16).unwrap();
                assert!((before..=after).contains(&millis), "{table}: {id}");
            }
        }
    }
    for table in [
        "projects",
        "tasks",
        "topics",
        "sources",
        "decisions",
        "task_comments",
        "task_topics",
        "source_topics",
        "decision_topics",
        "task_context_sources",
        "task_dependencies",
        "task_comment_sources",
    ] {
        let columns = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for (name, kind) in columns {
            if name == "id" || name.ends_with("_id") {
                assert_eq!(kind, "TEXT", "{table}.{name}");
                let values = connection
                    .prepare(&format!(
                        "SELECT {name} FROM {table} WHERE {name} IS NOT NULL"
                    ))
                    .unwrap()
                    .query_map([], |row| row.get::<_, String>(0))
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                assert!(
                    values.iter().all(|id| identities.contains(id)),
                    "{table}.{name}"
                );
            }
        }
    }
    drop(connection);
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(
        scoped(
            &restarted,
            &scope,
            "get_context",
            json!({"task_id": child["id"]})
        ),
        context
    );
    let next = scoped(&restarted, &scope, "create_task", brief());
    assert!(next["id"].as_str().unwrap() > child["id"].as_str().unwrap());
}

#[test]
fn every_id_input_rejects_numbers_malformed_and_non_v7_strings_without_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("uuid-inputs.sqlite3");
    let client = Project::open(&database).unwrap();
    let scope = client.project_id.clone();
    let task = scoped(&client, &scope, "create_task", brief());
    let comment = scoped(
        &client,
        &scope,
        "record_task_comment",
        json!({"task_id": task["id"], "task_revision": 1, "body": "Valid target"}),
    );
    for invalid in [
        json!(1),
        json!("1"),
        json!("not-a-uuid"),
        json!("0199b3d8-f600-7000-8000-000000000001\n"),
        json!("0199b3d8-f600-4000-8000-000000000001"),
        json!("0199B3D8-F600-7000-8000-000000000001"),
        json!("0199b3d8f60070008000000000000001"),
        json!("0199b3d8-f600-7000-c000-000000000001"),
    ] {
        for (name, mut input, field) in [
            ("get_context", json!({}), "project_id"),
            ("get_context", json!({}), "task_id"),
            ("get_source", json!({}), "source_id"),
            (
                "get_task_comment",
                json!({"task_id": task["id"]}),
                "comment_id",
            ),
            (
                "list_task_comments",
                json!({"task_id": task["id"]}),
                "before_id",
            ),
            ("list_sources", json!({}), "before_id"),
            ("list_decisions", json!({}), "before_id"),
            ("list_tasks", json!({}), "parent_task_id"),
            ("create_task", brief(), "parent_task_id"),
            (
                "update_task",
                json!({"task_id": task["id"], "expected_revision": 1}),
                "parent_task_id",
            ),
            (
                "record_task_comment",
                json!({"task_id": task["id"], "task_revision": 1, "body": "Invalid reply"}),
                "reply_to_comment_id",
            ),
            (
                "record_decision",
                json!({"topics": ["UUID identity"], "statement": "Invalid reference", "rationale": "Reject invalid IDs"}),
                "supersedes_id",
            ),
        ] {
            input["project_id"] = json!(scope);
            input[field] = invalid.clone();
            rejected_without_mutation(&client, &database, name, input);
        }
        for field in [
            "context_source_ids",
            "prerequisite_task_ids",
            "completion_task_ids",
        ] {
            let mut input = brief();
            input["project_id"] = json!(scope);
            input[field] = json!([invalid]);
            rejected_without_mutation(&client, &database, "create_task", input);
        }
        rejected_without_mutation(
            &client,
            &database,
            "record_task_comment",
            json!({
                "project_id": scope, "task_id": task["id"], "task_revision": 1,
                "body": "Invalid output", "reply_to_comment_id": comment["id"],
                "execution_result": {"outcome": "failure", "output_source_ids": [invalid]}
            }),
        );
    }
}

#[test]
fn uuidv7_generation_advances_past_persisted_future_ids_across_clients_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("uuid-clock.sqlite3");
    let client = Project::open(&database).unwrap();
    let scope = client.project_id.clone();
    let partial_carry = "fffffffd-ffff-7000-bfff-ffffffffffff";
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute("INSERT INTO projects (id, name, description, revision, created_at, updated_at) VALUES (?1, 'Suffix carry fixture', 'Exercise carry across the variant bits', 1, '2026-11-06T00:00:00Z', '2026-11-06T00:00:00Z')", [partial_carry]).unwrap();
    drop(connection);
    let carried = client
        .call::<Value>(
            "create_project",
            json!({"name": "Suffix carry", "description": "Carry into rand_a"}),
        )
        .unwrap();
    assert_eq!(carried["id"], "fffffffd-ffff-7001-8000-000000000000");
    let future = "fffffffe-ffff-7fff-bfff-ffffffffffff";
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute("INSERT INTO projects (id, name, description, revision, created_at, updated_at) VALUES (?1, 'Future clock fixture', 'Simulate a regressed clock and suffix carry', 1, '2026-11-06T00:00:00Z', '2026-11-06T00:00:00Z')", [future]).unwrap();
    drop(connection);
    let first = client.call::<Value>("create_task", brief()).unwrap();
    assert_eq!(first["id"], "ffffffff-0000-7000-8000-000000000000");
    let barrier = Arc::new(Barrier::new(2));
    let writers = (0..2)
        .map(|_| {
            let database = database.clone();
            let barrier = barrier.clone();
            let scope = scope.clone();
            std::thread::spawn(move || {
                let client = Project::open(database).unwrap();
                barrier.wait();
                scoped(&client, &scope, "create_task", brief())
            })
        })
        .collect::<Vec<_>>();
    let mut concurrent = writers
        .into_iter()
        .map(|writer| writer.join().unwrap()["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    concurrent.sort();
    assert_eq!(
        concurrent,
        [
            "ffffffff-0000-7000-8000-000000000002",
            "ffffffff-0000-7000-8000-000000000003"
        ]
    );
    // Creating the first task also created a topic through the same generator.
    drop(client);
    let restarted = Project::open(&database).unwrap();
    let next = scoped(&restarted, &scope, "create_task", brief());
    assert_eq!(next["id"], "ffffffff-0000-7000-8000-000000000004");
    let tasks = scoped(&restarted, &scope, "list_tasks", json!({}));
    let actual = tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        [
            "ffffffff-0000-7000-8000-000000000000",
            "ffffffff-0000-7000-8000-000000000002",
            "ffffffff-0000-7000-8000-000000000003",
            "ffffffff-0000-7000-8000-000000000004"
        ]
    );
}
