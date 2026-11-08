//! Project/task context and bounded discovery through real MCP processes.
mod support;

use serde_json::{Value, json};
use std::path::Path;
use support::Project;

fn call(client: &Project, name: &str, input: Value) -> Value {
    client.call(name, input).unwrap()
}

fn source(client: &Project, name: &str, extra: Value) -> Value {
    let mut input = json!({"topics": ["Project workflow"],
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
    let mut input = json!({"topics": ["Project workflow"],
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
    let mut input = json!({"topics": ["Project workflow"],
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

fn ids(result: &Value, field: &str) -> Vec<String> {
    result[field]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["id"].as_str().unwrap().to_owned())
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
        let topics = if index % 2 == 0 {
            json!(["Project workflow", "User authentication"])
        } else {
            json!(["Project workflow", "Dashboard comment security"])
        };
        sources.push(source(
            &client,
            &format!("Source {index}"),
            json!({"topics": topics}),
        ));
        decisions.push(decision(
            &client,
            &format!("Choice {index}"),
            json!({"topics": topics}),
        ));
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
            .map(|record| record["id"].as_str().unwrap().to_owned())
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

        // Topic membership combines with pagination before limiting the result.
        let matching: Vec<_> = records
            .iter()
            .step_by(2)
            .map(|record| record["id"].as_str().unwrap().to_owned())
            .collect();
        let filtered = call(
            &client,
            tool,
            json!({"topic": "  USER  authentication  ", "page_size": 20}),
        );
        assert_eq!(ids(&filtered, field), matching[..20]);
        assert_eq!(filtered["next_before_id"], matching[19]);
        let filtered_tail = call(
            &client,
            tool,
            json!({"topic": "User authentication", "page_size": 100, "before_id": matching[19]}),
        );
        assert_eq!(ids(&filtered_tail, field), matching[20..]);
        assert!(filtered_tail["next_before_id"].is_null());
        assert_eq!(
            call(&client, tool, json!({"topic": "Missing topic"}))[field],
            json!([])
        );
    }
    assert_eq!(snapshot(&database), before);
    drop(client);
    let restarted = Project::open(&database).unwrap();
    for (tool, expected) in saved_pages {
        assert_eq!(call(&restarted, tool, json!({})), expected);
        let field = if tool == "list_sources" {
            "sources"
        } else {
            "decisions"
        };
        assert_eq!(
            call(
                &restarted,
                tool,
                json!({"topic": "User authentication", "page_size": 100})
            )[field]
                .as_array()
                .unwrap()
                .len(),
            51
        );
    }
    assert_eq!(snapshot(&database), before);
}

#[test]
fn source_filters_apply_before_pagination_and_new_appends_preserve_older_pages() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("source-filters.sqlite3");
    let client = Project::open(&database).unwrap();
    let oldest = source(
        &client,
        "Keep oldest",
        json!({"topics": ["User authentication"]}),
    );
    let irrelevant = source(&client, "Other", json!({"context_policy": "always"}));
    let middle = source(
        &client,
        "Keep middle",
        json!({"topics": ["User authentication"]}),
    );
    let cursor = source(
        &client,
        "External cursor",
        json!({"content": null, "locator": "https://example.com/reference"}),
    );
    let newest = source(
        &client,
        "Keep newest",
        json!({"topics": ["User authentication"]}),
    );
    let due = source(
        &client,
        "Keep due",
        json!({
            "topics": ["User authentication"],
            "action": {"instruction": "Inspect current release notes.", "trigger": {"kind": "interval", "every_minutes": 30}}
        }),
    );
    let linked_task = task(
        &client,
        "Linked sources",
        json!({"context_source_ids": [oldest["id"], middle["id"], newest["id"], due["id"]]}),
    );
    let filter = json!({"topic": "User authentication", "query": "KEEP", "representation": "inline", "task_id": linked_task["id"], "action_due": false, "page_size": 2});
    let first = call(&client, "list_sources", filter.clone());
    assert_eq!(
        ids(&first, "sources"),
        vec![
            newest["id"].as_str().unwrap().to_owned(),
            middle["id"].as_str().unwrap().to_owned()
        ]
    );
    assert_eq!(first["next_before_id"], middle["id"]);
    let writer = Project::open(&database).unwrap();
    let appended = source(
        &writer,
        "Keep appended",
        json!({"topics": ["User authentication"]}),
    );
    call(
        &writer,
        "update_task",
        json!({"task_id": linked_task["id"], "expected_revision": linked_task["revision"], "context_source_ids": [oldest["id"], middle["id"], newest["id"], due["id"], appended["id"]]}),
    );
    let mut next = filter.clone();
    next["before_id"] = first["next_before_id"].clone();
    let older = call(&client, "list_sources", next);
    assert_eq!(
        ids(&older, "sources"),
        vec![oldest["id"].as_str().unwrap().to_owned()]
    );
    assert!(older["next_before_id"].is_null());
    assert_eq!(
        ids(&call(&client, "list_sources", filter.clone()), "sources"),
        vec![
            appended["id"].as_str().unwrap().to_owned(),
            newest["id"].as_str().unwrap().to_owned()
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
            middle["id"].as_str().unwrap().to_owned(),
            oldest["id"].as_str().unwrap().to_owned()
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
        vec![due["id"].as_str().unwrap().to_owned()]
    );
    assert!(action_page["next_before_id"].is_null());
    let linked = call(
        &client,
        "list_sources",
        json!({"task_id": linked_task["id"]}),
    );
    assert!(!ids(&linked, "sources").contains(&irrelevant["id"].as_str().unwrap().to_owned()));
}

#[test]
fn decision_pages_filter_superseded_records_and_keep_cursors_valid() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("decision-filters.sqlite3");
    let client = Project::open(&database).unwrap();
    let oldest = decision(&client, "Keep oldest", json!({}));
    let superseded = decision(
        &client,
        "Original choice",
        json!({"topics": ["User authentication"]}),
    );
    let middle = decision(&client, "Independent choice", json!({}));
    let replacement = decision(
        &client,
        "Replacement choice",
        json!({"supersedes_id": superseded["id"], "topics": ["Dashboard comment security"]}),
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
    assert_eq!(
        call(
            &client,
            "list_decisions",
            json!({"topic": "User authentication"})
        )["decisions"],
        json!([])
    );
    assert_eq!(
        call(
            &client,
            "list_decisions",
            json!({"topic": "User authentication", "include_superseded": true})
        )["decisions"],
        json!([historical])
    );
    assert_eq!(
        call(
            &client,
            "list_decisions",
            json!({"topic": "Dashboard comment security"})
        )["decisions"],
        json!([replacement])
    );
    assert_eq!(history["decisions"], json!([historical, oldest]));
    assert!(history["next_before_id"].is_null());
}

#[test]
fn optional_topics_preserve_relational_identity_and_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("optional-topics.sqlite3");
    let client = Project::open(&database).unwrap();
    let initial_project = call(&client, "get_context", json!({}))["project"].clone();

    for (tool, input) in [
        (
            "create_task",
            json!({"title": "Unclassified task", "problem_or_goal": "Create without classification", "design": "Keep topics optional", "acceptance_criteria": ["Creation succeeds"]}),
        ),
        (
            "create_source",
            json!({"name": "Unclassified source", "description": "Optional classification", "content": "A durable fact"}),
        ),
        (
            "record_decision",
            json!({"statement": "Unclassified decision", "rationale": "Classification is optional"}),
        ),
    ] {
        let omitted = call(&client, tool, input.clone());
        assert_eq!(omitted["topics"], json!([]), "{tool} omitted topics");
        let mut input = input;
        input["topics"] = json!([]);
        let empty = call(&client, tool, input);
        assert_eq!(empty["topics"], json!([]), "{tool} explicit empty topics");
    }
    assert_eq!(
        call(&client, "get_context", json!({}))["project"],
        initial_project
    );
    let connection = rusqlite::Connection::open(&database).unwrap();
    let topic_count: i64 = connection
        .query_row("SELECT count(*) FROM topics", [], |row| row.get(0))
        .unwrap();
    assert_eq!(topic_count, 0, "unclassified records created no topics");
    drop(connection);

    let tagged_task = task(
        &client,
        "Classified task",
        json!({"topics": ["  User \t authentication ", "user authentication", "Task only"]}),
    );
    assert_eq!(tagged_task["topics"].as_array().unwrap().len(), 2);
    let shared_topic = tagged_task["topics"][0].clone();
    assert_eq!(shared_topic["name"], "User authentication");
    assert_eq!(shared_topic["project_id"], json!(client.project_id));
    let tagged_source = source(
        &client,
        "Classified source",
        json!({"topics": ["USER authentication", "Source only"], "action": {"instruction": "Read this fixture", "trigger": {"kind": "manual"}}}),
    );
    assert_eq!(tagged_source["topics"][0], shared_topic);
    let observed_source = call(
        &client,
        "record_source_action",
        json!({"source_id": tagged_source["id"], "expected_revision": tagged_source["revision"], "result": {"outcome": "success", "observed_at": tagged_source["created_at"], "summary": "Verified fixture", "checkpoint": "retained", "references": []}}),
    );
    let historical = decision(
        &client,
        "Keep historical classification",
        json!({"topics": ["user authentication"]}),
    );
    assert_eq!(historical["topics"], json!([shared_topic]));
    let replacement = call(
        &client,
        "record_decision",
        json!({"statement": "Replacement without topics", "rationale": "No inheritance", "supersedes_id": historical["id"]}),
    );
    assert_eq!(replacement["topics"], json!([]));
    let final_choice = call(
        &client,
        "record_decision",
        json!({"statement": "Explicitly unclassified replacement", "rationale": "An empty array is valid", "supersedes_id": replacement["id"], "topics": []}),
    );
    assert_eq!(final_choice["topics"], json!([]));
    let directory_before = call(&client, "get_context", json!({}))["project"]["topics"].clone();
    assert_eq!(directory_before.as_array().unwrap().len(), 3);

    let preserved_task = call(
        &client,
        "update_task",
        json!({"task_id": tagged_task["id"], "expected_revision": tagged_task["revision"], "progress": "Preserve classification"}),
    );
    assert_eq!(preserved_task["topics"], tagged_task["topics"]);
    let preserved_source = call(
        &client,
        "update_source",
        json!({"source_id": tagged_source["id"], "expected_revision": observed_source["revision"], "description": "Preserve classification and freshness"}),
    );
    assert_eq!(preserved_source["topics"], tagged_source["topics"]);
    let before_conflict = snapshot(&database);
    rejected(
        &client,
        "update_task",
        json!({"project_id": client.project_id, "task_id": tagged_task["id"], "expected_revision": tagged_task["revision"], "topics": []}),
    );
    assert_eq!(snapshot(&database), before_conflict);
    let cleared_task = call(
        &client,
        "update_task",
        json!({"task_id": tagged_task["id"], "expected_revision": preserved_task["revision"], "topics": []}),
    );
    let cleared_source = call(
        &client,
        "update_source",
        json!({"source_id": tagged_source["id"], "expected_revision": preserved_source["revision"], "topics": []}),
    );
    assert_eq!(cleared_task["topics"], json!([]));
    assert_eq!(cleared_source["topics"], json!([]));
    assert_eq!(
        cleared_task["revision"],
        preserved_task["revision"].as_i64().unwrap() + 1
    );
    assert_eq!(
        cleared_source["revision"],
        preserved_source["revision"].as_i64().unwrap() + 1
    );
    for field in ["snapshot", "latest_attempt"] {
        assert_eq!(cleared_source[field], observed_source[field]);
    }
    for (tool, field) in [
        ("list_tasks", "tasks"),
        ("list_sources", "sources"),
        ("list_decisions", "decisions"),
    ] {
        assert_eq!(
            call(&client, tool, json!({"topic": "USER authentication"}))[field],
            json!([])
        );
    }
    let history = call(
        &client,
        "list_decisions",
        json!({"topic": "USER authentication", "include_superseded": true}),
    );
    assert_eq!(history["decisions"][0]["topics"], historical["topics"]);
    let mut expected_project = initial_project;
    expected_project["topics"] = directory_before.clone();
    assert_eq!(
        call(&client, "get_context", json!({}))["project"],
        expected_project
    );
    drop(client);

    let client = Project::open(&database).unwrap();
    assert_eq!(
        call(&client, "get_context", json!({}))["project"],
        expected_project
    );
    assert_eq!(
        call(
            &client,
            "get_context",
            json!({"task_id": tagged_task["id"]})
        )["task"]["topics"],
        json!([])
    );
    assert_eq!(
        call(
            &client,
            "get_source",
            json!({"source_id": tagged_source["id"]})
        )["topics"],
        json!([])
    );
    let reused = source(
        &client,
        "Reuse retained topic",
        json!({"topics": ["task ONLY"]}),
    );
    assert_eq!(reused["topics"][0], directory_before[1]);
}

#[test]
fn invalid_pages_cursors_and_context_scopes_do_not_mutate_records() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("isolation.sqlite3");
    let client = Project::open(&database).unwrap();
    let local_source = source(
        &client,
        "Local",
        json!({"topics": ["  User \t authentication  ", "Dashboard comment security", "user authentication"]}),
    );
    let local_decision = decision(&client, "Local", json!({"topics": ["USER authentication"]}));
    let local_task = task(&client, "Local", json!({"topics": ["user authentication"]}));
    assert_eq!(local_source["topics"].as_array().unwrap().len(), 2);
    assert_eq!(local_source["topics"][0]["name"], "User authentication");
    assert_eq!(local_decision["topics"][0], local_source["topics"][0]);
    assert_eq!(local_task["topics"][0], local_source["topics"][0]);
    let other_project = call(
        &client,
        "create_project",
        json!({"name": "Other", "description": "Separate project context"}),
    );
    let foreign_source = source(
        &client,
        "Foreign",
        json!({"project_id": other_project["id"], "topics": ["User authentication"]}),
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
    assert_ne!(
        foreign_source["topics"][0]["id"],
        local_source["topics"][0]["id"]
    );
    assert_eq!(
        foreign_source["topics"][0]["project_id"],
        other_project["id"]
    );
    let before = snapshot(&database);
    for (tool, mut input) in [
        (
            "create_task",
            json!({"project_id": client.project_id, "title": "Invalid classification", "problem_or_goal": "Reject invalid topics", "design": "Validate atomically", "acceptance_criteria": ["No rows persist"]}),
        ),
        (
            "create_source",
            json!({"project_id": client.project_id, "name": "Invalid classification", "description": "Reject invalid topics", "content": "No rows persist"}),
        ),
        (
            "record_decision",
            json!({"project_id": client.project_id, "statement": "Invalid classification", "rationale": "Reject invalid topics"}),
        ),
    ] {
        for topics in [
            json!(null),
            json!([""]),
            json!(["New topic must roll back", "  "]),
            json!([1]),
        ] {
            input["topics"] = topics;
            rejected(&client, tool, input.clone());
            assert_eq!(snapshot(&database), before);
        }
    }
    for (tool, record, id_field) in [
        ("update_task", &local_task, "task_id"),
        ("update_source", &local_source, "source_id"),
    ] {
        for topics in [json!(null), json!(["New topic must roll back", "  "])] {
            let mut input = json!({"project_id": client.project_id, "expected_revision": record["revision"], "topics": topics});
            input[id_field] = record["id"].clone();
            rejected(&client, tool, input);
            assert_eq!(snapshot(&database), before);
        }
    }
    for tool in ["list_tasks", "list_sources", "list_decisions"] {
        rejected(
            &client,
            tool,
            json!({"project_id": client.project_id, "topic": " \t "}),
        );
        let field = match tool {
            "list_tasks" => "tasks",
            "list_sources" => "sources",
            _ => "decisions",
        };
        assert_eq!(
            call(&client, tool, json!({"topic": "Unknown topic"}))[field],
            json!([])
        );
        assert_eq!(
            call(&client, tool, json!({"topic": "  user AUTHENTICATION "}))[field]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
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
        for before_id in [
            json!(0),
            json!(-1),
            json!("ffffffff-ffff-7fff-bfff-ffffffffffff"),
            foreign_id.clone(),
        ] {
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
        json!({"project_id": client.project_id, "task_id": "ffffffff-ffff-7fff-bfff-ffffffffffff"}),
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
        vec![local_source["id"].as_str().unwrap().to_owned()]
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
    assert_eq!(foreign_context["project"]["id"], other_project["id"]);
    assert_eq!(
        foreign_context["project"]["revision"],
        other_project["revision"]
    );
    assert_eq!(
        foreign_context["project"]["topics"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        foreign_context["current_decisions"],
        json!([foreign_decision])
    );
    assert_eq!(foreign_context["sources"], json!([]));
    assert_eq!(snapshot(&database), before);

    let reclassified = call(
        &client,
        "update_task",
        json!({"task_id": local_task["id"], "expected_revision": local_task["revision"], "topics": ["Dashboard comment security"]}),
    );
    assert_eq!(reclassified["revision"], 2);
    assert_eq!(reclassified["topics"], json!([local_source["topics"][1]]));
    assert_eq!(
        call(
            &client,
            "list_tasks",
            json!({"topic": "User authentication"})
        )["tasks"],
        json!([])
    );
    let unchanged_topics = call(
        &client,
        "update_task",
        json!({"task_id": local_task["id"], "expected_revision": reclassified["revision"], "progress": "Topic membership survives unrelated edits."}),
    );
    assert_eq!(unchanged_topics["topics"], reclassified["topics"]);
    let reclassified_source = call(
        &client,
        "update_source",
        json!({"source_id": local_source["id"], "expected_revision": local_source["revision"], "topics": ["User authentication"]}),
    );
    assert_eq!(
        reclassified_source["topics"],
        json!([local_source["topics"][0]])
    );
    assert_eq!(
        call(
            &client,
            "list_sources",
            json!({"topic": "Dashboard comment security"})
        )["sources"],
        json!([])
    );
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
    let mut project = call(
        &client,
        "update_project",
        json!({"expected_revision": 1, "description": "# Current purpose\n\nInclude the full project description in every context."}),
    );
    project["promoted_decisions"] = json!([]);
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
