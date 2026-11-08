//! Project promotion references exercised through real MCP processes.
mod support;

use serde_json::{Value, json};
use support::Project;

fn call(client: &Project, name: &str, input: Value) -> Value {
    client.call(name, input).unwrap()
}

fn decision(client: &Project, statement: &str, extra: Value) -> Value {
    let mut input = json!({
        "statement": statement,
        "rationale": format!("Accepted rationale for {statement}"),
        "alternatives_rejected": "Keep the complete decision available for review.",
        "topics": ["Application storage"]
    });
    input
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    call(client, "record_decision", input)
}

fn project(client: &Project) -> Value {
    call(client, "list_projects", json!({}))["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|project| project["id"] == client.project_id)
        .unwrap()
        .clone()
}

fn promote(client: &Project, revision: &Value, references: Value) -> Value {
    call(
        client,
        "update_project",
        json!({"expected_revision": revision, "promoted_decision_ids": references}),
    )
}

fn summary(reference: &Value, current: &Value) -> Value {
    json!({"reference_id": reference["id"], "id": current["id"], "statement": current["statement"]})
}

#[test]
fn promotions_preserve_reference_order_deduplicate_chains_and_keep_full_context() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("promoted.sqlite3");
    let client = Project::open(&database).unwrap();
    let original = project(&client);
    assert_eq!(original["promoted_decision_ids"], json!([]));
    assert!(original.get("promoted_decisions").is_none());
    assert_eq!(
        call(&client, "get_context", json!({}))["project"]["promoted_decisions"],
        json!([])
    );
    let first = decision(
        &client,
        "Use DynamoDB for primary application data.",
        json!({}),
    );
    let second = decision(
        &client,
        "Use PostgreSQL for primary application data.",
        json!({"supersedes_id": first["id"]}),
    );
    let independent = decision(&client, "Use SQLite for the local cache.", json!({}));
    let ordinary = decision(&client, "Keep audit events for ninety days.", json!({}));
    let task = call(
        &client,
        "create_task",
        json!({
            "title": "Improve the mobile interface", "problem_or_goal": "Keep navigation clear.",
            "design": "Follow the project-wide accepted choices.",
            "acceptance_criteria": ["Shared choices remain visible."], "topics": ["Mobile interface"]
        }),
    );
    let updated = promote(
        &client,
        &original["revision"],
        json!([
            independent["id"],
            first["id"],
            second["id"],
            independent["id"],
            first["id"]
        ]),
    );
    assert_eq!(updated["revision"], 2);
    assert_eq!(updated["description"], original["description"]);
    assert_eq!(
        updated["promoted_decision_ids"],
        json!([independent["id"], first["id"]])
    );
    assert!(updated.get("promoted_decisions").is_none());
    assert_eq!(project(&client), updated);
    let summaries = json!([
        summary(&independent, &independent),
        summary(&first, &second)
    ]);
    for input in [json!({}), json!({"task_id": task["id"]})] {
        let context = call(&client, "get_context", input);
        assert_eq!(context["project"]["promoted_decisions"], summaries);
        assert_eq!(
            context["project"]["promoted_decision_ids"],
            updated["promoted_decision_ids"]
        );
        // Promotion does not truncate or reorder the complete accepted decision set.
        assert_eq!(
            context["current_decisions"],
            json!([second, independent, ordinary])
        );
    }
    assert_eq!(
        call(
            &client,
            "list_decisions",
            json!({"topic": "Mobile interface"})
        )["decisions"],
        json!([])
    );
    // A historical or current chain member can be the first retained reference.
    let reordered = promote(
        &client,
        &updated["revision"],
        json!([second["id"], first["id"], independent["id"]]),
    );
    assert_eq!(
        reordered["promoted_decision_ids"],
        json!([second["id"], independent["id"]])
    );
    let third = decision(
        &client,
        first["statement"].as_str().unwrap(),
        json!({"supersedes_id": second["id"]}),
    );
    let context = call(&client, "get_context", json!({}));
    assert_eq!(
        context["project"]["promoted_decisions"],
        json!([
            summary(&second, &third),
            summary(&independent, &independent)
        ])
    );
    assert_eq!(
        context["current_decisions"],
        json!([independent, ordinary, third])
    );
    assert_eq!(project(&client), reordered);
    assert_eq!(call(&client, "get_context", json!({})), context);
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(project(&restarted), reordered);
    assert_eq!(call(&restarted, "get_context", json!({})), context);
}

#[test]
fn promotion_edits_preserve_omissions_clear_explicitly_and_reject_invalid_arrays_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("validation.sqlite3");
    let client = Project::open(&database).unwrap();
    let own = decision(&client, "Use a local cache.", json!({}));
    let other = call(
        &client,
        "create_project",
        json!({"name": "Other", "description": "Isolated scope"}),
    );
    assert_eq!(other["promoted_decision_ids"], json!([]));
    assert!(other.get("promoted_decisions").is_none());
    let foreign = decision(
        &client,
        "Use a remote cache.",
        json!({"project_id": other["id"]}),
    );
    let pinned = promote(&client, &json!(1), json!([own["id"]]));
    let before = call(&client, "list_projects", json!({}));
    let unchanged_context = call(&client, "get_context", json!({}));
    for references in [
        Value::Null,
        json!(own["id"]),
        json!([null]),
        json!([1]),
        json!([""]),
        json!(["00000000-0000-4000-8000-000000000000"]),
        json!(["00000000-0000-7000-8000-00000000000A"]),
        json!([own["id"], "00000000-0000-7000-8000-000000000000"]),
        json!([own["id"], foreign["id"]]),
        json!([own["id"], client.project_id]),
    ] {
        let input = json!({"expected_revision": pinned["revision"], "description": "Must roll back", "promoted_decision_ids": references});
        assert!(
            client
                .call::<Value>("update_project", input.clone())
                .is_err(),
            "accepted {input}"
        );
        assert_eq!(
            call(&client, "list_projects", json!({})),
            before,
            "invalid promotion mutated durable data: {input}"
        );
        assert_eq!(call(&client, "get_context", json!({})), unchanged_context);
    }
    let description_only = call(
        &client,
        "update_project",
        json!({"expected_revision": 2, "description": "Updated project purpose."}),
    );
    assert_eq!(description_only["revision"], 3);
    assert_eq!(
        description_only["promoted_decision_ids"],
        pinned["promoted_decision_ids"]
    );
    let omitted = call(&client, "update_project", json!({"expected_revision": 3}));
    assert_eq!(omitted["revision"], 4);
    for field in ["description", "promoted_decision_ids", "created_at", "name"] {
        assert_eq!(omitted[field], description_only[field]);
    }
    let identical = promote(
        &client,
        &omitted["revision"],
        pinned["promoted_decision_ids"].clone(),
    );
    assert_eq!(identical["revision"], 5);
    assert_eq!(
        identical["promoted_decision_ids"],
        pinned["promoted_decision_ids"]
    );
    let cleared = promote(&client, &identical["revision"], json!([]));
    assert_eq!(cleared["revision"], 6);
    assert_eq!(cleared["promoted_decision_ids"], json!([]));
    assert_eq!(cleared["description"], description_only["description"]);
    assert_eq!(
        call(&client, "get_context", json!({}))["project"]["promoted_decisions"],
        json!([])
    );
    assert_eq!(
        call(&client, "get_context", json!({}))["current_decisions"],
        json!([own])
    );
    let combined = call(
        &client,
        "update_project",
        json!({"expected_revision": 6, "description": "Combined edit", "promoted_decision_ids": [own["id"]]}),
    );
    assert_eq!(combined["revision"], 7);
    assert_eq!(combined["description"], "Combined edit");
    assert_eq!(combined["promoted_decision_ids"], json!([own["id"]]));
    assert!(client.call::<Value>("update_project", json!({"expected_revision": 6, "description": "Stale edit", "promoted_decision_ids": []})).is_err());
    assert_eq!(project(&client), combined);
}
