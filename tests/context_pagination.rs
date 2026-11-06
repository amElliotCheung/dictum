//! Project/task context and bounded discovery through real MCP processes.
mod support;

use serde_json::{Value, json};
use std::path::Path;
use support::Project;

fn call(client: &Project, name: &str, input: Value) -> Value {
    client.call(name, input).unwrap()
}

fn source(client: &Project, name: &str, extra: Value) -> Value {
    let mut input = json!({
        "name": name, "description": format!("Context for {name}"),
        "content": format!("# {name}\n\nPreserve the complete source content.")
    });
    input
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    call(client, "create_source", input)
}

fn task(client: &Project, title: &str, extra: Value) -> Value {
    let mut input = json!({
        "title": title, "problem_or_goal": "Read complete, explicitly scoped context.",
        "design": "Discover bounded pages and expand the current project or task.",
        "acceptance_criteria": ["Reads preserve durable records and project isolation."]
    });
    input
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    call(client, "create_task", input)
}

fn decision(client: &Project, statement: &str, extra: Value) -> Value {
    let mut input = json!({
        "statement": statement,
        "rationale": "## Rationale\n\nRetain the complete accepted choice, including Unicode: 项目。",
        "alternatives_rejected": "## Rejected\n\nDo not truncate accepted records."
    });
    input
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    call(client, "record_decision", input)
}

fn ids(result: &Value, field: &str) -> Vec<i64> {
    result[field]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["id"].as_i64().unwrap())
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

fn rejected(client: &Project, name: &str, input: Value) {
    assert!(
        client.raw_call::<Value>(name, input.clone()).is_err(),
        "accepted {name}: {input}"
    );
}

#[test]
fn source_and_decision_pages_have_exact_bounds_and_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("bounds.sqlite3");
    let client = Project::open(&database).unwrap();
    let mut sources = Vec::new();
    let mut decisions = Vec::new();
    for index in 0..101 {
        sources.push(source(&client, &format!("Source {index}"), json!({})));
        decisions.push(decision(&client, &format!("Choice {index}"), json!({})));
    }
    sources.reverse();
    decisions.reverse();
    let before = snapshot(&database);
    let mut saved_pages = Vec::new();
    for (tool, field, records) in [
        ("list_sources", "sources", &sources),
        ("list_decisions", "decisions", &decisions),
    ] {
        let expected: Vec<_> = records
            .iter()
            .map(|record| record["id"].as_i64().unwrap())
            .collect();
        let default_page = call(&client, tool, json!({}));
        assert_eq!(ids(&default_page, field), expected[..20]);
        assert_eq!(default_page["next_before_id"], expected[19]);

        let maximum = call(&client, tool, json!({"page_size": 100}));
        assert_eq!(ids(&maximum, field), expected[..100]);
        assert_eq!(maximum["next_before_id"], expected[99]);
        let tail = call(
            &client,
            tool,
            json!({"page_size": 100, "before_id": maximum["next_before_id"]}),
        );
        assert_eq!(ids(&tail, field), expected[100..]);
        assert!(tail["next_before_id"].is_null());

        let exact = call(
            &client,
            tool,
            json!({"page_size": 100, "before_id": expected[0]}),
        );
        assert_eq!(ids(&exact, field), expected[1..]);
        assert!(exact["next_before_id"].is_null());
        let empty = call(
            &client,
            tool,
            json!({"page_size": 1, "before_id": expected[100]}),
        );
        assert_eq!(empty[field], json!([]));
        assert!(empty["next_before_id"].is_null());

        if field == "decisions" {
            assert_eq!(default_page[field], json!(records[..20]));
        } else {
            let descriptor = &default_page[field][0];
            for name in [
                "id",
                "project_id",
                "name",
                "description",
                "revision",
                "context_policy",
            ] {
                assert_eq!(descriptor[name], records[0][name]);
            }
            assert_eq!(descriptor["representation"], "inline");
            assert!(descriptor.get("content").is_none());
        }
        saved_pages.push((tool, default_page));
    }
    assert_eq!(snapshot(&database), before);
    drop(client);
    let restarted = Project::open(&database).unwrap();
    for (tool, expected) in saved_pages {
        assert_eq!(call(&restarted, tool, json!({})), expected);
    }
    assert_eq!(snapshot(&database), before);
}

#[test]
fn source_filters_apply_before_pagination_and_new_appends_preserve_older_pages() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("source-filters.sqlite3");
    let client = Project::open(&database).unwrap();
    let oldest = source(&client, "Keep oldest", json!({}));
    let irrelevant = source(&client, "Other", json!({"context_policy": "always"}));
    let middle = source(&client, "Keep middle", json!({}));
    let cursor = source(
        &client,
        "External cursor",
        json!({"content": null, "locator": "https://example.com/reference"}),
    );
    let newest = source(&client, "Keep newest", json!({}));
    let due = source(
        &client,
        "Keep due",
        json!({
            "action": {"instruction": "Inspect current release notes.", "trigger": {"kind": "interval", "every_minutes": 30}}
        }),
    );
    let linked_task = task(
        &client,
        "Linked sources",
        json!({"context_source_ids": [oldest["id"], middle["id"], newest["id"], due["id"]]}),
    );
    let filter = json!({"query": "KEEP", "representation": "inline", "task_id": linked_task["id"], "action_due": false, "page_size": 2});
    let first = call(&client, "list_sources", filter.clone());
    assert_eq!(
        ids(&first, "sources"),
        vec![
            newest["id"].as_i64().unwrap(),
            middle["id"].as_i64().unwrap()
        ]
    );
    assert_eq!(first["next_before_id"], middle["id"]);
    let writer = Project::open(&database).unwrap();
    let appended = source(&writer, "Keep appended", json!({}));
    call(
        &writer,
        "update_task",
        json!({"task_id": linked_task["id"], "expected_revision": linked_task["revision"], "context_source_ids": [oldest["id"], middle["id"], newest["id"], due["id"], appended["id"]]}),
    );
    let mut next = filter.clone();
    next["before_id"] = first["next_before_id"].clone();
    let older = call(&client, "list_sources", next);
    assert_eq!(ids(&older, "sources"), vec![oldest["id"].as_i64().unwrap()]);
    assert!(older["next_before_id"].is_null());
    assert_eq!(
        ids(&call(&client, "list_sources", filter.clone()), "sources"),
        vec![
            appended["id"].as_i64().unwrap(),
            newest["id"].as_i64().unwrap()
        ]
    );
    assert_eq!(
        call(&client, "list_sources", json!({"page_size": 1}))["sources"][0]["id"],
        appended["id"]
    );

    let mut outside_filter_cursor = filter;
    outside_filter_cursor["before_id"] = cursor["id"].clone();
    let exact = call(&client, "list_sources", outside_filter_cursor);
    assert_eq!(
        ids(&exact, "sources"),
        vec![
            middle["id"].as_i64().unwrap(),
            oldest["id"].as_i64().unwrap()
        ]
    );
    assert!(exact["next_before_id"].is_null());
    let action_page = call(
        &client,
        "list_sources",
        json!({"action_due": true, "task_id": linked_task["id"], "page_size": 1}),
    );
    assert_eq!(
        ids(&action_page, "sources"),
        vec![due["id"].as_i64().unwrap()]
    );
    assert!(action_page["next_before_id"].is_null());
    let linked = call(
        &client,
        "list_sources",
        json!({"task_id": linked_task["id"]}),
    );
    assert!(!ids(&linked, "sources").contains(&irrelevant["id"].as_i64().unwrap()));
}

#[test]
fn decision_pages_filter_superseded_records_and_keep_cursors_valid() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("decision-filters.sqlite3");
    let client = Project::open(&database).unwrap();
    let oldest = decision(&client, "Keep oldest", json!({}));
    let superseded = decision(&client, "Original choice", json!({}));
    let middle = decision(&client, "Independent choice", json!({}));
    let replacement = decision(
        &client,
        "Replacement choice",
        json!({"supersedes_id": superseded["id"]}),
    );
    let first = call(&client, "list_decisions", json!({"page_size": 2}));
    assert_eq!(first["decisions"], json!([replacement, middle]));
    assert_eq!(first["next_before_id"], middle["id"]);

    let writer = Project::open(&database).unwrap();
    let appended = decision(&writer, "Later unrelated choice", json!({}));
    let older = call(
        &client,
        "list_decisions",
        json!({"page_size": 2, "before_id": first["next_before_id"]}),
    );
    assert_eq!(older["decisions"], json!([oldest]));
    assert!(older["next_before_id"].is_null());
    assert_eq!(
        call(&client, "list_decisions", json!({"page_size": 1}))["decisions"],
        json!([appended])
    );

    let excluded_cursor = call(
        &client,
        "list_decisions",
        json!({"before_id": superseded["id"], "page_size": 1}),
    );
    assert_eq!(excluded_cursor["decisions"], json!([oldest]));
    assert!(excluded_cursor["next_before_id"].is_null());
    let history = call(
        &client,
        "list_decisions",
        json!({"include_superseded": true, "page_size": 2, "before_id": middle["id"]}),
    );
    let mut historical = superseded;
    historical["is_current"] = json!(false);
    historical["current_decision_id"] = replacement["id"].clone();
    assert_eq!(history["decisions"], json!([historical, oldest]));
    assert!(history["next_before_id"].is_null());
}

#[test]
fn invalid_pages_cursors_and_context_scopes_do_not_mutate_records() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("isolation.sqlite3");
    let client = Project::open(&database).unwrap();
    let local_source = source(&client, "Local", json!({}));
    let local_decision = decision(&client, "Local", json!({}));
    let local_task = task(&client, "Local", json!({}));
    let other_project = call(
        &client,
        "create_project",
        json!({"name": "Other", "description": "Separate project context"}),
    );
    let foreign_source = source(
        &client,
        "Foreign",
        json!({"project_id": other_project["id"]}),
    );
    let foreign_decision = decision(
        &client,
        "Foreign",
        json!({"project_id": other_project["id"]}),
    );
    let foreign_task = task(
        &client,
        "Foreign",
        json!({"project_id": other_project["id"]}),
    );
    let before = snapshot(&database);
    for (tool, foreign_id) in [
        ("list_sources", &foreign_source["id"]),
        ("list_decisions", &foreign_decision["id"]),
    ] {
        for page_size in [json!(0), json!(-1), json!(101), json!(1.5), json!("2")] {
            rejected(
                &client,
                tool,
                json!({"project_id": client.project_id, "page_size": page_size}),
            );
        }
        for before_id in [json!(0), json!(-1), json!(999_999), foreign_id.clone()] {
            let mut input = json!({"project_id": client.project_id, "before_id": before_id});
            if tool == "list_sources" {
                input["query"] = json!("no matching records");
            } else {
                input["include_superseded"] = json!(true);
            }
            rejected(&client, tool, input);
        }
        rejected(&client, tool, json!({}));
        rejected(&client, tool, json!({"project_id": "missing-project"}));
    }
    rejected(
        &client,
        "list_sources",
        json!({"project_id": client.project_id, "task_id": foreign_task["id"]}),
    );
    for input in [
        json!({}),
        json!({"project_id": "missing-project"}),
        json!({"project_id": client.project_id, "task_id": foreign_task["id"]}),
        json!({"project_id": client.project_id, "task_id": 999_999}),
        json!({"project_id": client.project_id, "task_id": 0}),
    ] {
        rejected(&client, "get_context", input);
    }
    rejected(
        &client,
        "get_task_context",
        json!({"project_id": client.project_id, "task_id": local_task["id"]}),
    );
    assert_eq!(
        ids(&call(&client, "list_sources", json!({})), "sources"),
        vec![local_source["id"].as_i64().unwrap()]
    );
    assert_eq!(
        call(&client, "list_decisions", json!({}))["decisions"],
        json!([local_decision])
    );
    let foreign_context = call(
        &client,
        "get_context",
        json!({"project_id": other_project["id"]}),
    );
    assert_eq!(foreign_context["project"], other_project);
    assert_eq!(
        foreign_context["current_decisions"],
        json!([foreign_decision])
    );
    assert_eq!(foreign_context["sources"], json!([]));
    assert_eq!(snapshot(&database), before);
}

#[test]
fn project_and_task_contexts_remain_complete_read_only_and_persisted() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("context.sqlite3");
    let client = Project::open(&database).unwrap();
    let mut always = Vec::new();
    let mut choices = Vec::new();
    for index in 0..21 {
        always.push(source(
            &client,
            &format!("Environment fact {index}"),
            json!({"context_policy": "always"}),
        ));
        choices.push(decision(
            &client,
            &format!("Current choice {index}"),
            json!({}),
        ));
    }
    let replacement = decision(
        &client,
        "Replacement first choice",
        json!({"supersedes_id": choices[0]["id"]}),
    );
    choices.remove(0);
    choices.push(replacement);
    let linked = source(&client, "Child-specific context", json!({}));
    let parent_only = source(&client, "Parent-only context", json!({}));
    let parent = task(
        &client,
        "Shared parent",
        json!({"context_source_ids": [parent_only["id"]]}),
    );
    let dependency = task(&client, "Pending dependency", json!({}));
    let original = task(
        &client,
        "Selected child",
        json!({"parent_task_id": parent["id"], "context_source_ids": [always[0]["id"], linked["id"]]}),
    );
    let success = call(
        &client,
        "record_task_comment",
        json!({"task_id": original["id"], "task_revision": 1, "body": "Verified implementation abc123; all checks passed.", "execution_result": {"outcome": "success", "output_source_ids": [linked["id"]]}}),
    );
    let updated = call(
        &client,
        "update_task",
        json!({"task_id": original["id"], "expected_revision": 1, "prerequisite_task_ids": [dependency["id"]], "completion_task_ids": [dependency["id"]], "progress": "A later dependency now blocks the next execution."}),
    );
    let failure = call(
        &client,
        "record_task_comment",
        json!({"task_id": original["id"], "task_revision": updated["revision"], "body": "The follow-up check cannot run until the dependency is complete.", "execution_result": {"outcome": "failure"}}),
    );
    let reply = call(
        &client,
        "record_task_comment",
        json!({"task_id": original["id"], "task_revision": updated["revision"], "reply_to_comment_id": failure["id"], "body": "Preserve the earlier success as historical evidence only."}),
    );
    let project = call(
        &client,
        "update_project",
        json!({"expected_revision": 1, "description": "# Current purpose\n\nInclude the full project description in every context."}),
    );
    let before = snapshot(&database);
    let project_context = call(&client, "get_context", json!({}));
    assert_eq!(project_context["project"], project);
    for field in [
        "task",
        "parent_task",
        "dependency_state",
        "latest_execution",
        "latest_successful_execution",
    ] {
        assert!(project_context[field].is_null(), "unexpected {field}");
    }
    assert_eq!(project_context["sources"], json!(always));
    assert_eq!(project_context["current_decisions"], json!(choices));
    assert_eq!(project_context["recent_comments"], json!([]));
    assert_eq!(
        call(&client, "get_context", json!({"task_id": null})),
        project_context
    );

    let task_context = call(&client, "get_context", json!({"task_id": original["id"]}));
    assert_eq!(task_context["project"], project);
    assert_eq!(task_context["task"], updated);
    assert_eq!(task_context["parent_task"], parent);
    assert_eq!(
        task_context["dependency_state"],
        json!({"ready_to_start": false, "unmet_prerequisite_task_ids": [dependency["id"]], "unmet_completion_task_ids": [dependency["id"]]})
    );
    let mut task_sources = always;
    task_sources.push(linked);
    assert_eq!(task_context["sources"], json!(task_sources));
    assert_eq!(task_context["current_decisions"], json!(choices));
    assert_eq!(
        task_context["recent_comments"],
        json!([reply, failure, success])
    );
    assert_eq!(task_context["latest_execution"], failure);
    assert_eq!(task_context["latest_successful_execution"], success);
    assert_eq!(snapshot(&database), before);
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(call(&restarted, "get_context", json!({})), project_context);
    assert_eq!(
        call(
            &restarted,
            "get_context",
            json!({"task_id": original["id"]})
        ),
        task_context
    );
    assert_eq!(snapshot(&database), before);
}
