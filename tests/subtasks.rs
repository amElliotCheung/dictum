//! One-level organizational subtasks through separate real MCP processes.
mod support;

use serde_json::{Value, json};
use std::sync::{Arc, Barrier};
use support::Project;

fn call(client: &Project, name: &str, input: Value) -> Value {
    client.call(name, input).unwrap()
}

fn brief(title: &str, kind: &str, parent: Option<i64>) -> Value {
    json!({"title": title, "kind": kind, "parent_task_id": parent,
        "problem_or_goal": format!("Deliver {title}."), "design": "Keep each brief independently executable.",
        "acceptance_criteria": ["The selected task is independently verified."]})
}

fn create(client: &Project, title: &str, kind: &str, parent: Option<i64>) -> Value {
    call(client, "create_task", brief(title, kind, parent))
}

fn update(client: &Project, task: &Value, mut patch: Value) -> Value {
    patch["task_id"] = task["id"].clone();
    patch["expected_revision"] = task["revision"].clone();
    call(client, "update_task", patch)
}

fn context(client: &Project, task: &Value) -> Value {
    call(client, "get_task_context", json!({"task_id": task["id"]}))
}

fn ids(list: &Value) -> Vec<i64> {
    list["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["id"].as_i64().unwrap())
        .collect()
}

#[test]
fn subtask_context_has_the_full_current_parent_without_inheriting_its_records() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("context.sqlite3");
    let client = Project::open(&database).unwrap();
    let source = call(
        &client,
        "create_source",
        json!({"name": "Parent only", "description": "Explicit parent context", "content": "Parent source contents"}),
    );
    let mut input = brief("Root brief", "one_time", None);
    input["context_source_ids"] = json!([source["id"]]);
    let parent = call(&client, "create_task", input);
    let child = create(&client, "Child brief", "one_time", parent["id"].as_i64());
    call(
        &client,
        "record_task_comment",
        json!({"task_id": parent["id"], "task_revision": parent["revision"], "body": "Parent discussion"}),
    );
    let current = context(&client, &child);
    assert_eq!(current["task"], child);
    assert_eq!(current["parent_task"], parent);
    assert_eq!(current["sources"], json!([]));
    assert_eq!(current["recent_comments"], json!([]));
    assert_eq!(child["context_source_ids"], json!([]));
    assert_eq!(child["prerequisite_task_ids"], json!([]));
    assert_eq!(child["completion_task_ids"], json!([]));
    assert!(context(&client, &parent)["parent_task"].is_null());
    let revised = update(
        &client,
        &parent,
        json!({"design": "The root design has changed."}),
    );
    let refreshed = context(&client, &child);
    assert_eq!(refreshed["parent_task"], revised);
    assert_eq!(refreshed["task"], child);
    let summaries = call(&client, "list_tasks", json!({}));
    for summary in summaries["tasks"].as_array().unwrap() {
        assert!(summary.get("parent_task_id").is_none());
    }
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(context(&restarted, &child), refreshed);
}

#[test]
fn parent_filters_distinguish_omission_null_and_id_and_compose_with_existing_filters() {
    let directory = tempfile::tempdir().unwrap();
    let client = Project::open(directory.path().join("filters.sqlite3")).unwrap();
    let first = create(&client, "First root", "one_time", None);
    let second = create(&client, "Second root", "one_time", None);
    let low = create(&client, "Child low", "one_time", first["id"].as_i64());
    let high = create(&client, "Child high", "one_time", first["id"].as_i64());
    let low = update(&client, &low, json!({"priority": "low"}));
    let high = update(&client, &high, json!({"priority": "high"}));
    assert_eq!(
        ids(&call(
            &client,
            "list_tasks",
            json!({"parent_task_id": null})
        )),
        vec![
            first["id"].as_i64().unwrap(),
            second["id"].as_i64().unwrap()
        ]
    );
    assert_eq!(
        ids(&call(
            &client,
            "list_tasks",
            json!({"parent_task_id": first["id"]})
        )),
        vec![high["id"].as_i64().unwrap(), low["id"].as_i64().unwrap()]
    );
    assert_eq!(
        ids(&call(
            &client,
            "list_tasks",
            json!({"parent_task_id": first["id"], "priority": "high", "query": "CHILD", "status": "todo", "kind": "one_time", "ready_to_start": true})
        )),
        vec![high["id"].as_i64().unwrap()]
    );
    assert_eq!(
        call(&client, "list_tasks", json!({}))["tasks"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    let moved = update(&client, &high, json!({"parent_task_id": second["id"]}));
    assert_eq!(moved["parent_task_id"], second["id"]);
    let omitted = update(
        &client,
        &moved,
        json!({"progress": "Parent is preserved when omitted."}),
    );
    assert_eq!(omitted["parent_task_id"], second["id"]);
    let detached = update(&client, &omitted, json!({"parent_task_id": null}));
    assert!(detached["parent_task_id"].is_null());
    assert!(context(&client, &detached)["parent_task"].is_null());
    assert_eq!(context(&client, &first)["task"], first);
    assert_eq!(context(&client, &second)["task"], second);
}

#[test]
fn invalid_parentage_is_rejected_atomically_including_child_roots_and_foreign_kinds() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("validation.sqlite3");
    let client = Project::open(&database).unwrap();
    let parent = create(&client, "Root", "one_time", None);
    let child = create(&client, "Child", "one_time", parent["id"].as_i64());
    let other = create(&client, "Other root", "one_time", None);
    let recurring = create(&client, "Recurring root", "recurring", None);
    let foreign_project = call(
        &client,
        "create_project",
        json!({"name": "Foreign", "description": "Separate hierarchy"}),
    );
    let mut input = brief("Foreign root", "one_time", None);
    input["project_id"] = foreign_project["id"].clone();
    let foreign = call(&client, "create_task", input);
    let before = context(&client, &parent);
    for parent_id in [
        json!(0),
        json!(-1),
        json!(999_999),
        recurring["id"].clone(),
        foreign["id"].clone(),
        child["id"].clone(),
    ] {
        let mut input = brief("Must not be created", "one_time", None);
        input["parent_task_id"] = parent_id;
        assert!(client.call::<Value>("create_task", input).is_err());
    }
    for (task, parent_id) in [
        (&child, child["id"].clone()),
        (&parent, parent["id"].clone()),
        (&parent, other["id"].clone()),
        (&parent, child["id"].clone()),
        (&child, recurring["id"].clone()),
        (&child, foreign["id"].clone()),
        (&child, json!(999_999)),
    ] {
        let previous = context(&client, task);
        assert!(client.call::<Value>("update_task", json!({"task_id": task["id"], "expected_revision": task["revision"], "parent_task_id": parent_id, "title": "Must roll back"})).is_err());
        assert_eq!(context(&client, task), previous);
    }
    assert_eq!(context(&client, &parent), before);
    assert_eq!(
        call(&client, "list_tasks", json!({}))["tasks"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    for parent_id in [
        json!(0),
        json!(-1),
        json!(999_999),
        foreign["id"].clone(),
        json!("1"),
    ] {
        assert!(
            client
                .call::<Value>("list_tasks", json!({"parent_task_id": parent_id}))
                .is_err()
        );
    }
}

#[test]
fn parentage_does_not_gate_completion_copy_status_or_restrict_same_kind_recurrence() {
    let directory = tempfile::tempdir().unwrap();
    let client = Project::open(directory.path().join("lifecycle.sqlite3")).unwrap();
    let parent = create(&client, "Independent parent", "one_time", None);
    let child = create(
        &client,
        "Unfinished child",
        "one_time",
        parent["id"].as_i64(),
    );
    call(
        &client,
        "record_task_comment",
        json!({"task_id": parent["id"], "task_revision": parent["revision"], "body": "Parent checks passed.", "execution_result": {"outcome": "success"}}),
    );
    let done = update(&client, &parent, json!({"status": "done"}));
    assert_eq!(context(&client, &child)["task"], child);
    let added = create(
        &client,
        "Another child of done parent",
        "one_time",
        parent["id"].as_i64(),
    );
    assert_eq!(added["status"], "todo");
    assert_eq!(context(&client, &parent)["task"], done);
    let recurring = create(&client, "Recurring parent", "recurring", None);
    let recurring_child = create(
        &client,
        "Recurring child",
        "recurring",
        recurring["id"].as_i64(),
    );
    let paused = update(&client, &recurring, json!({"status": "paused"}));
    assert_eq!(
        context(&client, &recurring_child)["task"]["status"],
        "active"
    );
    assert_eq!(context(&client, &recurring_child)["parent_task"], paused);
}

#[test]
fn reciprocal_moves_and_child_creation_races_cannot_introduce_grandchildren() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("concurrency.sqlite3");
    let client = Project::open(&database).unwrap();
    for create_child in [false, true] {
        let first = create(&client, "First root", "one_time", None);
        let second = create(&client, "Second root", "one_time", None);
        let barrier = Arc::new(Barrier::new(2));
        let mut writers = Vec::new();
        for index in 0..2 {
            let database = database.clone();
            let first = first.clone();
            let second = second.clone();
            let barrier = Arc::clone(&barrier);
            writers.push(std::thread::spawn(move || {
                let client = Project::open(database).unwrap();
                barrier.wait();
                if create_child && index == 1 {
                    client.call::<Value>("create_task", brief("Concurrent child", "one_time", first["id"].as_i64()))
                } else {
                    let (task, parent) = if index == 0 { (&first, &second) } else { (&second, &first) };
                    client.call::<Value>("update_task", json!({"task_id": task["id"], "expected_revision": task["revision"], "parent_task_id": parent["id"]}))
                }
            }));
        }
        let results: Vec<_> = writers
            .into_iter()
            .map(|writer| writer.join().unwrap())
            .collect();
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            1,
            "race results: {results:?}"
        );
        for task in [&first, &second] {
            let state = context(&client, task);
            if !state["parent_task"].is_null() {
                assert!(state["parent_task"]["parent_task_id"].is_null());
            }
        }
    }
}
