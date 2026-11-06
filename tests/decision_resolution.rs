//! Resolve accepted decision history through the real MCP stdio process.
mod support;

use std::{sync::Arc, sync::Barrier, time::Duration};

use rmcp::{
    ClientLifecycleMode, ClientServiceExt,
    model::{ClientConfig, ProtocolVersion},
    transport::TokioChildProcess,
};
use serde_json::{Value, json};
use support::Project;

fn call(client: &Project, name: &str, input: Value) -> Value {
    client.call(name, input).unwrap()
}

fn decision(client: &Project, statement: &str, rationale: &str, extra: Value) -> Value {
    let mut input = json!({"topics": ["Project workflow"],
        "statement": statement,
        "rationale": rationale,
        "alternatives_rejected": "Retain alternatives as recorded for this accepted choice."
    });
    input
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    call(client, "record_decision", input)
}

fn immutable_fields(mut record: Value) -> Value {
    record.as_object_mut().unwrap().remove("is_current");
    record
        .as_object_mut()
        .unwrap()
        .remove("current_decision_id");
    record
}

#[test]
fn repeated_reversals_resolve_every_historical_page_and_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("decisions.sqlite3");
    let client = Project::open(&database).unwrap();
    let task = call(
        &client,
        "create_task",
        json!({"topics": ["Project workflow"],
            "title": "Read the current database choice",
            "problem_or_goal": "Follow the latest accepted intent through reversals.",
            "design": "Read complete current choices without expanding historical decisions.",
            "acceptance_criteria": ["Every historical decision identifies the current choice."]
        }),
    );
    let first = decision(
        &client,
        "Use DynamoDB for primary application data.",
        "Expected access patterns are key-based.",
        json!({}),
    );
    assert_eq!(first["current_decision_id"], first["id"]);
    assert_eq!(first["is_current"], true);
    let independent = decision(
        &client,
        "Use PostgreSQL for analytics.",
        "Analytics requires relational queries independently of the application database.",
        json!({}),
    );
    let second = decision(
        &client,
        "Use PostgreSQL for primary application data.",
        "Application requirements now include relational queries.",
        json!({"supersedes_id": first["id"]}),
    );
    let third = decision(
        &client,
        "Use DynamoDB for primary application data.",
        "Revised application requirements removed relational queries; reduce operations.",
        json!({"supersedes_id": second["id"]}),
    );
    for current in [&independent, &second, &third] {
        assert_eq!(current["current_decision_id"], current["id"]);
        assert_eq!(current["is_current"], true);
    }

    // A different MCP process advances the chain after the reader fetched its first page.
    let first_page = call(
        &client,
        "list_decisions",
        json!({"include_superseded": true, "page_size": 1}),
    );
    assert_eq!(first_page["decisions"], json!([third]));
    assert_eq!(first_page["next_before_id"], third["id"]);
    let writer = Project::open(&database).unwrap();
    let fourth = decision(
        &writer,
        "Use PostgreSQL for primary application data.",
        "A newly accepted reporting requirement needs transactions across related records.",
        json!({"supersedes_id": third["id"]}),
    );
    assert_eq!(fourth["current_decision_id"], fourth["id"]);
    assert_eq!(fourth["is_current"], true);

    let mut cursor = first_page["next_before_id"].clone();
    let mut older = Vec::new();
    loop {
        let page = call(
            &client,
            "list_decisions",
            json!({"include_superseded": true, "page_size": 1, "before_id": cursor}),
        );
        older.extend(page["decisions"].as_array().unwrap().clone());
        cursor = page["next_before_id"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(older.len(), 3);
    for (actual, original) in older.iter().zip([&second, &independent, &first]) {
        assert_eq!(
            immutable_fields(actual.clone()),
            immutable_fields(original.clone())
        );
        let expected_current = if actual["id"] == independent["id"] {
            &independent["id"]
        } else {
            &fourth["id"]
        };
        assert_eq!(&actual["current_decision_id"], expected_current);
        assert_eq!(actual["is_current"], actual["id"] == *expected_current);
    }
    // Even a filtered page excluding the head resolves the entire chain beyond its cursor.
    let history = call(
        &client,
        "list_decisions",
        json!({"include_superseded": true}),
    );
    assert_eq!(history["decisions"].as_array().unwrap().len(), 5);
    for record in history["decisions"].as_array().unwrap() {
        let expected = if record["id"] == independent["id"] {
            &independent["id"]
        } else {
            &fourth["id"]
        };
        assert_eq!(&record["current_decision_id"], expected);
    }
    let current = call(&client, "list_decisions", json!({}));
    assert_eq!(current["decisions"], json!([fourth, independent]));
    let project_context = call(&client, "get_context", json!({}));
    let task_context = call(&client, "get_context", json!({"task_id": task["id"]}));
    for context in [&project_context, &task_context] {
        assert_eq!(context["current_decisions"], json!([independent, fourth]));
    }
    drop(writer);
    drop(client);

    let restarted = Project::open(&database).unwrap();
    assert_eq!(call(&restarted, "list_decisions", json!({})), current);
    assert_eq!(
        call(
            &restarted,
            "list_decisions",
            json!({"include_superseded": true})
        ),
        history
    );
    assert_eq!(call(&restarted, "get_context", json!({})), project_context);
    assert_eq!(
        call(&restarted, "get_context", json!({"task_id": task["id"]})),
        task_context
    );
}

#[test]
fn stale_and_foreign_replacements_are_rejected_without_redirecting_writes() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("isolation.sqlite3");
    let client = Project::open(&database).unwrap();
    let first = decision(
        &client,
        "Use DynamoDB.",
        "Initial accepted choice.",
        json!({}),
    );
    let second = decision(
        &client,
        "Use PostgreSQL.",
        "Accepted relational requirements.",
        json!({"supersedes_id": first["id"]}),
    );
    let current = decision(
        &client,
        "Use DynamoDB.",
        "Accepted requirements have changed again.",
        json!({"supersedes_id": second["id"]}),
    );
    let other = call(
        &client,
        "create_project",
        json!({"name": "Other", "description": "Independent project decisions"}),
    );
    let foreign = decision(
        &client,
        "Use PostgreSQL.",
        "This belongs to another project.",
        json!({"project_id": other["id"]}),
    );
    let before = call(
        &client,
        "list_decisions",
        json!({"include_superseded": true}),
    );
    for stale in [&first, &second] {
        let error = client
            .call::<Value>(
                "record_decision",
                json!({"topics": ["Project workflow"],
                    "statement": "A stale writer must not replace an unseen choice.",
                    "rationale": "This write must fail.", "supersedes_id": stale["id"]
                }),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("current_decision_id"), "{error}");
        assert!(error.contains(&current["id"].to_string()), "{error}");
    }
    for input in [
        json!({"supersedes_id": foreign["id"]}),
        json!({"project_id": other["id"], "supersedes_id": current["id"]}),
        json!({"current_decision_id": current["id"]}),
    ] {
        let mut request = json!({"topics": ["Project workflow"], "statement": "Rejected choice", "rationale": "Must not mutate."});
        request
            .as_object_mut()
            .unwrap()
            .extend(input.as_object().unwrap().clone());
        assert!(client.call::<Value>("record_decision", request).is_err());
    }
    assert_eq!(
        call(
            &client,
            "list_decisions",
            json!({"include_superseded": true})
        ),
        before
    );
    let foreign_history = call(
        &client,
        "list_decisions",
        json!({"project_id": other["id"], "include_superseded": true}),
    );
    assert_eq!(foreign_history["decisions"], json!([foreign]));
    assert_eq!(foreign["current_decision_id"], foreign["id"]);
    let context = call(&client, "get_context", json!({"project_id": other["id"]}));
    assert_eq!(context["current_decisions"], json!([foreign]));
}

#[test]
fn concurrent_replacements_have_one_winner_and_report_its_current_id() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("concurrent.sqlite3");
    let reader = Project::open(&database).unwrap();
    let root = decision(&reader, "Initial choice.", "Initial rationale.", json!({}));
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|index| {
            let writer = Project::open(&database).unwrap();
            let barrier = barrier.clone();
            let root_id = root["id"].clone();
            std::thread::spawn(move || {
                barrier.wait();
                writer.call::<Value>(
                    "record_decision",
                    json!({"topics": ["Project workflow"],
                        "statement": format!("Concurrent choice {index}"),
                        "rationale": "Only one successor can be accepted.",
                        "supersedes_id": root_id
                    }),
                )
            })
        })
        .collect();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let winner = results
        .iter()
        .find_map(|result| result.as_ref().ok())
        .unwrap();
    let error = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .unwrap()
        .to_string();
    assert!(error.contains("current_decision_id"), "{error}");
    assert!(error.contains(&winner["id"].to_string()), "{error}");
    let history = call(
        &reader,
        "list_decisions",
        json!({"include_superseded": true}),
    );
    assert_eq!(history["decisions"].as_array().unwrap().len(), 2);
    for record in history["decisions"].as_array().unwrap() {
        assert_eq!(record["current_decision_id"], winner["id"]);
    }
}

fn resolve_schema<'a>(schema: &'a Value, field: &'a Value) -> &'a Value {
    if let Some(reference) = field["$ref"].as_str() {
        schema
            .pointer(reference.strip_prefix('#').unwrap())
            .unwrap()
    } else {
        field
    }
}

#[tokio::test]
async fn tool_discovery_advertises_current_decision_id_as_server_derived_output() {
    let directory = tempfile::tempdir().unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_dictum-mcp"));
    command
        .env("DICTUM_DATABASE", directory.path().join("schemas.sqlite3"))
        .env("TOKIO_WORKER_THREADS", "2");
    let client = tokio::time::timeout(
        Duration::from_secs(15),
        ClientConfig::default().serve_with_lifecycle(
            TokioChildProcess::new(command).unwrap(),
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let tools = client.list_all_tools().await.unwrap();
    for (name, collection) in [
        ("record_decision", None),
        ("list_decisions", Some("decisions")),
        ("get_context", Some("current_decisions")),
    ] {
        let tool = tools.iter().find(|tool| tool.name == name).unwrap();
        let schema = serde_json::to_value(&tool.output_schema).unwrap();
        let decision_schema = if let Some(collection) = collection {
            resolve_schema(&schema, &schema["properties"][collection]["items"])
        } else {
            &schema
        };
        assert_eq!(
            decision_schema["properties"]["current_decision_id"]["type"], "integer",
            "{name}: {schema}"
        );
        assert!(
            decision_schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("current_decision_id")),
            "{name}: {schema}"
        );
        assert!(
            tool.input_schema["properties"]
                .get("current_decision_id")
                .is_none()
        );
    }
    client.cancel().await.unwrap();
}
