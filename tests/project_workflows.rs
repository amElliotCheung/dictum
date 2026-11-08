//! Behavioral scenarios against the real MCP binary. See support for transport setup.
mod support;

use chrono::{DateTime, Duration, Utc};
use dictum_mcp::model::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::{Arc, Barrier};
use support::Project;
use tempfile::TempDir;

fn database() -> (TempDir, Project) {
    let directory = tempfile::tempdir().unwrap();
    let store = Project::open(directory.path().join("project.sqlite3")).unwrap();
    (directory, store)
}

fn input<T: DeserializeOwned>(mut value: Value) -> T {
    value["project_id"] = json!(support::PROJECT_PLACEHOLDER);
    serde_json::from_value(value).unwrap()
}

fn task_input(title: &str) -> Value {
    json!({"topics": ["Project workflow"],
        "title": title,
        "problem_or_goal": "A new agent needs a durable execution brief.",
        "design": "Read the linked context, implement the change, and record verification.",
        "acceptance_criteria": ["The behavior is covered by a passing integration test."]
    })
}

fn task(store: &Project, title: &str) -> Task {
    store.create_task(input(task_input(title))).unwrap()
}

fn task_patch(task: &Task, mut patch: Value) -> UpdateTask {
    patch["task_id"] = json!(task.id);
    patch["expected_revision"] = json!(task.revision);
    input(patch)
}

fn complete(store: &Project, task: &Task) -> Task {
    let result: dictum_mcp::model::TaskComment = store
        .call(
            "record_task_comment",
            json!({"task_id": task.id, "task_revision": task.revision, "body": "Verified revision abc123: cargo test passed.", "execution_result": {"outcome": "success"}}),
        )
        .unwrap();
    let mut done_task = task.clone();
    done_task.revision = result.task_revision;
    store
        .update_task(task_patch(&done_task, json!({"status": "done"})))
        .unwrap()
}

fn inline(store: &Project, name: &str, content: &str, policy: &str) -> Source {
    store
        .create_source(input(json!({"topics": ["Project workflow"],
            "name": name,
            "description": "Project context and when to use it.",
            "content": content,
            "context_policy": policy
        })))
        .unwrap()
}

fn external(store: &Project, name: &str) -> Source {
    store
        .create_source(input(json!({"topics": ["Project workflow"],
            "name": name,
            "description": "Daily discussion of project requirements.",
            "locator": "slack://workspace/channel",
            "action": {
                "instruction": "Read current project-relevant messages."
            }
        })))
        .unwrap()
}

fn source_patch(source: &Source, mut patch: Value) -> UpdateSource {
    patch["source_id"] = json!(source.id);
    patch["expected_revision"] = json!(source.revision);
    input(patch)
}

#[test]
fn full_inline_context_is_deduplicated_and_outputs_are_not_automatic_inputs() {
    let (_directory, store) = database();
    let full_content = format!(
        "# Detailed specification\n{}\nEND OF DOCUMENT\n",
        "完整内容 🦀\n".repeat(8_000)
    );
    let shared = inline(&store, "Shared specification", &full_content, "always");
    let context = inline(
        &store,
        "Task design",
        "# Task design\nAll details.\n",
        "when_relevant",
    );
    let output = inline(
        &store,
        "Research output",
        "# Findings\nKeep this as output.\n",
        "when_relevant",
    );
    let unrelated = inline(&store, "Unrelated", "Not required here.", "when_relevant");
    let implicit = inline(&store, "Project policy", "Always available.", "always");
    let mut brief = task_input("Implement the specification");
    brief["context_source_ids"] = json!([shared.id, context.id]);
    let task = store.create_task(input(brief)).unwrap();
    store
        .call::<dictum_mcp::model::TaskComment>(
            "record_task_comment",
            json!({"task_id": task.id, "task_revision": task.revision, "body": "Outputs are linked per execution result.", "execution_result": {"outcome": "success", "output_source_ids": [context.id, output.id]}}),
        )
        .unwrap();

    let expanded = store.get_context(task.id).unwrap();
    let mut source_ids: Vec<_> = expanded.sources.iter().map(|source| source.id).collect();
    source_ids.sort_unstable();
    let mut expected = vec![shared.id, context.id, implicit.id];
    expected.sort_unstable();
    assert_eq!(source_ids, expected);
    assert!(!source_ids.contains(&output.id));
    assert!(!source_ids.contains(&unrelated.id));
    assert_eq!(
        expanded
            .sources
            .iter()
            .find(|source| source.id == shared.id)
            .unwrap()
            .content
            .as_deref(),
        Some(full_content.as_str())
    );
    assert_eq!(
        store.get_source(shared.id).unwrap().content.as_deref(),
        Some(full_content.as_str())
    );
    assert_eq!(
        expanded
            .latest_execution
            .as_ref()
            .unwrap()
            .execution_result
            .as_ref()
            .unwrap()
            .output_source_ids,
        vec![context.id, output.id]
    );

    // Discovery is compact and task filtering uses explicit context links, not implicit always context.
    let descriptors = store
        .list_sources(input(json!({"task_id": task.id})))
        .unwrap();
    assert_eq!(descriptors.len(), 2);
    assert!(!descriptors.iter().any(|source| source.id == implicit.id));
    for descriptor in descriptors {
        assert!(
            serde_json::to_value(descriptor)
                .unwrap()
                .get("content")
                .is_none()
        );
    }
    let mut second_brief = task_input("Reuse shared knowledge");
    second_brief["context_source_ids"] = json!([context.id]);
    let second = store.create_task(input(second_brief)).unwrap();
    assert!(
        store
            .get_context(second.id)
            .unwrap()
            .sources
            .iter()
            .any(|source| source.id == context.id)
    );

    // Loading context leaves Source definitions unchanged.
    let reread = store.get_source(shared.id).unwrap();
    assert_eq!(reread.revision, shared.revision);
    assert_eq!(reread.updated_at, shared.updated_at);
}

#[test]
fn dependency_kinds_gate_start_and_completion_independently() {
    let (_directory, store) = database();
    let prerequisite = task(&store, "Approve design");
    let verification = task(&store, "Verify the implementation");
    let mut brief = task_input("Implement feature");
    brief["prerequisite_task_ids"] = json!([prerequisite.id]);
    brief["completion_task_ids"] = json!([verification.id]);
    let implementation = store.create_task(input(brief)).unwrap();
    let context = store.get_context(implementation.id).unwrap();
    assert!(!context.dependency_state.ready_to_start);
    assert_eq!(
        context.dependency_state.unmet_prerequisite_task_ids,
        vec![prerequisite.id]
    );
    assert_eq!(
        context.dependency_state.unmet_completion_task_ids,
        vec![verification.id]
    );
    assert!(
        store
            .update_task(task_patch(
                &implementation,
                json!({"status": "in_progress"})
            ))
            .is_err()
    );
    complete(&store, &prerequisite);

    assert!(
        store
            .get_context(implementation.id)
            .unwrap()
            .dependency_state
            .ready_to_start
    );
    let implementation = store
        .update_task(task_patch(
            &implementation,
            json!({"status": "in_progress"}),
        ))
        .unwrap();
    assert!(
        store
            .update_task(task_patch(&implementation, json!({"status": "done"})))
            .is_err()
    );
    complete(&store, &verification);
    assert_eq!(
        store.get_context(implementation.id).unwrap().task.status,
        TaskStatus::InProgress
    );
    assert!(
        store
            .update_task(task_patch(&implementation, json!({"status": "done"})))
            .is_err()
    );
    let implementation = complete(&store, &implementation);
    assert_eq!(implementation.status, TaskStatus::Done);
    assert!(
        !store
            .get_context(implementation.id)
            .unwrap()
            .dependency_state
            .ready_to_start
    );
}

#[test]
fn mixed_dependency_cycles_and_invalid_links_roll_back_the_entire_write() {
    let (_directory, store) = database();
    let first = task(&store, "First task");
    let mut brief = task_input("Second task");
    brief["prerequisite_task_ids"] = json!([first.id]);
    let second = store.create_task(input(brief)).unwrap();
    assert!(
        store
            .update_task(task_patch(
                &first,
                json!({
                    "title": "Must not be saved",
                    "completion_task_ids": [second.id]
                })
            ))
            .is_err()
    );
    assert!(
        store
            .update_task(task_patch(
                &first,
                json!({"prerequisite_task_ids": [first.id]})
            ))
            .is_err()
    );
    assert!(
        store
            .update_task(task_patch(
                &first,
                json!({"title": "Also not saved", "context_source_ids": ["ffffffff-ffff-7fff-bfff-ffffffffffff"]})
            ))
            .is_err()
    );
    let unchanged = store.get_context(first.id).unwrap().task;
    assert_eq!(unchanged.title, first.title);
    assert_eq!(unchanged.revision, first.revision);
    assert!(unchanged.completion_task_ids.is_empty());
    assert!(unchanged.context_source_ids.is_empty());

    let mut invalid = task_input("Missing dependency");
    invalid["completion_task_ids"] = json!(["ffffffff-ffff-7fff-bfff-ffffffffffff"]);
    assert!(store.create_task(input(invalid)).is_err());
    assert_eq!(store.list_tasks(input(json!({}))).unwrap().len(), 2);

    let second = store
        .update_task(task_patch(&second, json!({"prerequisite_task_ids": []})))
        .unwrap();
    assert!(second.prerequisite_task_ids.is_empty());
    let first = store
        .update_task(task_patch(
            &first,
            json!({"completion_task_ids": [second.id]}),
        ))
        .unwrap();
    assert_eq!(first.completion_task_ids, vec![second.id]);
}

#[test]
fn task_updates_distinguish_omission_null_empty_lists_and_stale_revisions() {
    let (_directory, store) = database();
    let source = inline(&store, "Reference", "Full reference.", "when_relevant");
    let mut brief = task_input("Maintain execution state");
    brief["alternatives_rejected"] = json!("A distributed database is unnecessary for the MVP.");
    brief["context_source_ids"] = json!([source.id]);
    let original = store.create_task(input(brief)).unwrap();
    let current = store
        .update_task(task_patch(
            &original,
            json!({"progress": "The persistence layer is implemented."}),
        ))
        .unwrap();
    assert_eq!(
        current.alternatives_rejected,
        original.alternatives_rejected
    );
    assert_eq!(current.context_source_ids, vec![source.id]);
    assert!(
        store
            .update_task(task_patch(&original, json!({"title": "Stale write"})))
            .is_err()
    );
    let cleared = store
        .update_task(task_patch(
            &current,
            json!({
                "alternatives_rejected": null,
                "progress": null,
                "context_source_ids": []
            }),
        ))
        .unwrap();
    assert!(cleared.alternatives_rejected.is_none());
    assert!(cleared.progress.is_none());
    assert!(cleared.context_source_ids.is_empty());
    assert_eq!(cleared.title, original.title);
    assert_eq!(cleared.revision, current.revision + 1);
}

#[test]
fn task_creation_rejects_empty_brief_sections_and_completion_conditions() {
    let (_directory, store) = database();
    for field in ["title", "problem_or_goal", "design"] {
        let mut brief = task_input("Invalid brief");
        brief[field] = json!(" \n\t ");
        assert!(
            store.create_task(input(brief)).is_err(),
            "accepted empty {field}"
        );
    }
    for criteria in [json!([]), json!(["   "])] {
        let mut brief = task_input("Invalid acceptance criteria");
        brief["acceptance_criteria"] = criteria;
        assert!(store.create_task(input(brief)).is_err());
    }
    assert!(store.list_tasks(input(json!({}))).unwrap().is_empty());
}

#[test]
fn source_actions_are_instructions_without_result_bookkeeping() {
    let (directory, store) = database();
    let initial = external(&store, "Project Slack");
    let mut brief = task_input("Read channel context");
    brief["context_source_ids"] = json!([initial.id]);
    let reader = store.create_task(input(brief)).unwrap();
    let expected_action = json!({"instruction": "Read current project-relevant messages."});
    let original: Value = store
        .call("get_source", json!({"source_id": initial.id}))
        .unwrap();
    assert_eq!(original["action"], expected_action);
    assert_eq!(original["description"], initial.description);
    assert_eq!(
        original["locator"],
        initial.locator.as_ref().unwrap().as_str()
    );

    for _ in 0..2 {
        let context: Value = store
            .call("get_context", json!({"task_id": reader.id}))
            .unwrap();
        let full: Value = store
            .call("get_source", json!({"source_id": initial.id}))
            .unwrap();
        let listed: Value = store
            .call("list_sources", json!({"task_id": reader.id}))
            .unwrap();
        assert_eq!(full, original);
        assert_eq!(context["sources"][0], original);
        assert_eq!(listed["sources"][0]["has_action"], true);
        for response in [&full, &context["sources"][0], &listed["sources"][0]] {
            for field in ["snapshot", "latest_attempt", "action_due", "next_action_at"] {
                assert!(
                    response.get(field).is_none(),
                    "unexpected {field}: {response}"
                );
            }
        }
    }
    assert!(
        store
            .call::<Value>(
                "record_source_action",
                json!({
                    "source_id": initial.id,
                    "expected_revision": initial.revision,
                    "result": {"outcome": "failure", "error": "No result should be recorded."}
                })
            )
            .is_err()
    );
    assert_eq!(
        store
            .call::<Value>("get_source", json!({"source_id": initial.id}))
            .unwrap(),
        original
    );

    let connection = rusqlite::Connection::open(directory.path().join("project.sqlite3")).unwrap();
    let columns = connection
        .prepare("PRAGMA table_info(sources)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        !columns
            .iter()
            .any(|name| name == "snapshot" || name == "latest_attempt")
    );
}

#[test]
fn source_action_updates_preserve_omissions_clear_null_and_reject_invalid_definitions() {
    let (_directory, store) = database();
    let initial = external(&store, "Editable source");
    let metadata = store.update_source(source_patch(&initial, json!({
        "name": "Renamed source", "description": "A clearer explanation.", "context_policy": "always"
    }))).unwrap();
    assert_eq!(metadata.action, initial.action);
    assert_eq!(metadata.created_at, initial.created_at);
    assert_eq!(metadata.revision, initial.revision + 1);
    let retargeted = store
        .update_source(source_patch(
            &metadata,
            json!({
                "locator": "slack://workspace/other-channel"
            }),
        ))
        .unwrap();
    assert_eq!(retargeted.action, initial.action);
    let changed = store
        .update_source(source_patch(
            &retargeted,
            json!({
                "action": {"instruction": "Read current security changes."}
            }),
        ))
        .unwrap();
    assert_eq!(
        changed.action.as_ref().unwrap().instruction,
        "Read current security changes."
    );
    assert_eq!(changed.revision, retargeted.revision + 1);
    assert!(
        store
            .update_source(source_patch(&retargeted, json!({"action": null})))
            .is_err()
    );
    assert_eq!(store.get_source(changed.id).unwrap().action, changed.action);

    let invalid_actions = [
        json!({}),
        json!({"instruction": "   "}),
        json!({"instruction": null}),
        json!("Read this source"),
        json!({"instruction": "Read", "trigger": {"kind": "manual"}}),
        json!({"instruction": "Read", "trigger": {"kind": "interval", "every_minutes": 30}}),
    ];
    let before: Value = store
        .call("get_source", json!({"source_id": changed.id}))
        .unwrap();
    let before_list: Value = store.call("list_sources", json!({})).unwrap();
    for action in invalid_actions {
        assert!(
            store
                .call::<Value>(
                    "create_source",
                    json!({
                        "name": "Rejected action", "description": "Must not be saved.",
                        "content": "Input", "action": action
                    })
                )
                .is_err()
        );
        assert!(
            store
                .call::<Value>(
                    "update_source",
                    json!({
                        "source_id": changed.id, "expected_revision": changed.revision,
                        "name": "Must roll back", "action": action
                    })
                )
                .is_err()
        );
        assert_eq!(
            store
                .call::<Value>("get_source", json!({"source_id": changed.id}))
                .unwrap(),
            before
        );
        assert_eq!(
            store.call::<Value>("list_sources", json!({})).unwrap(),
            before_list
        );
    }
    assert!(
        store
            .call::<Value>("list_sources", json!({"action_due": true}))
            .is_err()
    );
    let cleared = store
        .update_source(source_patch(&changed, json!({"action": null})))
        .unwrap();
    assert!(cleared.action.is_none());
    assert_eq!(cleared.revision, changed.revision + 1);
    let descriptors = store.list_sources(input(json!({}))).unwrap();
    assert!(!descriptors[0].has_action);
    let recreated = store
        .update_source(source_patch(
            &cleared,
            json!({
                "action": {"instruction": "Inspect current messages again."}
            }),
        ))
        .unwrap();
    assert_eq!(
        recreated.action.as_ref().unwrap().instruction,
        "Inspect current messages again."
    );
    let context: Value = store.call("get_context", json!({})).unwrap();
    assert_eq!(
        context["sources"][0]["action"],
        json!({"instruction": "Inspect current messages again."})
    );
}

#[test]
fn source_representation_switches_are_atomic_and_inline_edits_preserve_actions() {
    let (_directory, store) = database();
    for definition in [
        json!({"topics": ["Project workflow"], "name": "Neither", "description": "Invalid"}),
        json!({"topics": ["Project workflow"], "name": "Both", "description": "Invalid", "content": "Text", "locator": "https://example.test/doc"}),
    ] {
        assert!(store.create_source(input(definition)).is_err());
    }
    let source = external(&store, "Convertible document");
    assert!(
        store
            .update_source(source_patch(&source, json!({"content": "Inline document"})))
            .is_err()
    );
    assert_eq!(
        store.get_source(source.id).unwrap().revision,
        source.revision
    );
    let source = store
        .update_source(source_patch(
            &source,
            json!({"locator": null, "content": "# Inline document\nFull content."}),
        ))
        .unwrap();
    assert!(source.locator.is_none());
    assert_eq!(
        source.content.as_deref(),
        Some("# Inline document\nFull content.")
    );
    let source = store
        .update_source(source_patch(
            &source,
            json!({"content": "# Updated document\nNew content."}),
        ))
        .unwrap();
    assert_eq!(
        source.action.as_ref().unwrap().instruction,
        "Read current project-relevant messages."
    );
}

#[test]
fn decisions_are_timestamped_immutable_and_have_one_current_successor() {
    let (directory, store) = database();
    let before = Utc::now() - Duration::seconds(1);
    let first = store
        .record_decision(input(json!({"topics": ["Project workflow"],
            "statement": "One project per SQLite database/server process.",
            "rationale": "Keep the local MVP simple.",
            "alternatives_rejected": "- Shared process: defer until project scoping exists."
        })))
        .unwrap();
    let created = DateTime::parse_from_rfc3339(&first.created_at).unwrap();
    assert_eq!(created.offset().local_minus_utc(), 0);
    assert!(created >= before);
    assert!(created <= Utc::now());
    assert!(first.is_current);
    assert_eq!(
        first.alternatives_rejected.as_deref(),
        Some("- Shared process: defer until project scoping exists.")
    );
    let second = store
        .record_decision(input(json!({"topics": ["Project workflow"],
            "statement": "Keep one project per database, shared by local server processes.",
            "rationale": "Allow multiple local agents to share durable state.",
            "alternatives_rejected": "- Isolated databases: agents would lose shared context.",
            "supersedes_id": first.id
        })))
        .unwrap();
    let current = store.list_decisions(input(json!({}))).unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].id, second.id);
    assert_eq!(
        current[0].alternatives_rejected,
        second.alternatives_rejected
    );
    assert_ne!(first.alternatives_rejected, second.alternatives_rejected);
    assert!(store.record_decision(input(json!({"topics": ["Project workflow"], "statement": "Another replacement", "rationale": "Invalid branch.", "supersedes_id": first.id}))).is_err());
    assert!(store.record_decision(input(json!({"topics": ["Project workflow"], "statement": "Missing predecessor", "rationale": "Invalid reference.", "supersedes_id": "ffffffff-ffff-7fff-bfff-ffffffffffff"}))).is_err());
    let history = store
        .list_decisions(input(json!({"include_superseded": true})))
        .unwrap();
    assert_eq!(history.len(), 2);
    let retained = history
        .iter()
        .find(|decision| decision.id == first.id)
        .unwrap();
    assert_eq!(retained.statement, first.statement);
    assert_eq!(retained.alternatives_rejected, first.alternatives_rejected);
    assert_eq!(retained.created_at, first.created_at);
    assert!(!retained.is_current);
    let task = task(&store, "Use current decisions");
    let context = store.get_context(task.id).unwrap();
    assert_eq!(context.current_decisions.len(), 1);
    assert_eq!(context.current_decisions[0].id, second.id);
    assert_eq!(
        context.current_decisions[0].alternatives_rejected,
        second.alternatives_rejected
    );
    let third = store
        .record_decision(input(json!({"topics": ["Project workflow"],
            "statement": "Use the established scoped shared database.",
            "rationale": "No further alternatives were considered.",
            "supersedes_id": second.id
        })))
        .unwrap();
    assert!(
        third.alternatives_rejected.is_none(),
        "omission must not inherit its predecessor's alternatives"
    );
    let history = store
        .list_decisions(input(json!({"include_superseded": true})))
        .unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(
        history
            .iter()
            .find(|decision| decision.id == second.id)
            .unwrap()
            .alternatives_rejected,
        second.alternatives_rejected
    );
    let current_context = store.get_context(task.id).unwrap();
    assert_eq!(current_context.current_decisions[0].id, third.id);
    assert!(
        current_context.current_decisions[0]
            .alternatives_rejected
            .is_none()
    );
    drop(store);
    let restarted = Project::open(directory.path().join("project.sqlite3")).unwrap();
    let persisted = restarted
        .list_decisions(input(json!({"include_superseded": true})))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&persisted).unwrap(),
        serde_json::to_value(&history).unwrap()
    );
    let context = restarted.get_context(task.id).unwrap();
    assert_eq!(context.current_decisions[0].id, third.id);
    assert!(context.current_decisions[0].alternatives_rejected.is_none());
}

#[test]
fn mutable_records_persist_and_revisions_coordinate_separate_processes() {
    let (directory, first_store) = database();
    let source = external(&first_store, "Persistent source");
    let mut brief = task_input("Persistent task");
    brief["context_source_ids"] = json!([source.id]);
    let task = first_store.create_task(input(brief)).unwrap();
    let second_store = Project::open(directory.path().join("project.sqlite3")).unwrap();
    assert_eq!(
        second_store.get_source(source.id).unwrap().action,
        source.action
    );
    let current_source = second_store
        .update_source(source_patch(
            &source,
            json!({
                "action": {"instruction": "Read the current release discussion."}
            }),
        ))
        .unwrap();
    assert!(
        first_store
            .update_source(source_patch(&source, json!({"action": null})))
            .is_err()
    );
    assert_eq!(
        first_store.get_source(source.id).unwrap().action,
        current_source.action
    );
    let source = current_source;
    let changed = second_store
        .update_task(task_patch(
            &task,
            json!({"progress": "Work survived reconnect."}),
        ))
        .unwrap();
    assert!(
        first_store
            .update_task(task_patch(&task, json!({"progress": "Stale client"})))
            .is_err()
    );
    drop(first_store);
    drop(second_store);
    let reopened = Project::open(directory.path().join("project.sqlite3")).unwrap();
    let context = reopened.get_context(task.id).unwrap();
    assert_eq!(context.task.progress, changed.progress);
    assert_eq!(context.sources.len(), 1);
    assert_eq!(context.sources[0].action, source.action);
    assert_eq!(context.sources[0].revision, source.revision);
    assert_eq!(context.sources[0].updated_at, source.updated_at);
}

#[test]
fn concurrent_task_writers_have_exactly_one_revision_winner() {
    let (directory, store) = database();
    let original = task(&store, "Concurrent update");
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = (0..2)
        .map(|writer| {
            let writer_store = Project::open(directory.path().join("project.sqlite3")).unwrap();
            let original = original.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                writer_store.update_task(task_patch(
                    &original,
                    json!({
                        "title": format!("Writer {writer} won"),
                        "progress": format!("Evidence saved by writer {writer}")
                    }),
                ))
            })
        })
        .collect();
    barrier.wait();
    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(outcomes.iter().filter(|result| result.is_err()).count(), 1);
    let winner = outcomes
        .iter()
        .find_map(|result| result.as_ref().ok())
        .unwrap();
    let loser = outcomes
        .iter()
        .find_map(|result| result.as_ref().err())
        .unwrap();
    assert!(loser.to_string().contains("revision conflict"));
    let persisted = store.get_context(original.id).unwrap().task;
    assert_eq!(persisted.revision, original.revision + 1);
    assert_eq!(persisted.title, winner.title);
    assert_eq!(persisted.progress, winner.progress);
}

#[test]
fn old_or_unrelated_databases_are_rejected_without_losing_existing_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("old.sqlite3");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TABLE projects (name TEXT NOT NULL); INSERT INTO projects VALUES ('preserve me'); PRAGMA user_version = 1;").unwrap();
    drop(connection);
    assert!(Project::open(&path).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let retained: String = connection
        .query_row("SELECT name FROM projects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(retained, "preserve me");
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1);
}

#[test]
fn filters_use_discovery_fields_and_task_readiness() {
    let (_directory, store) = database();
    let ready = task(&store, "Implement authentication");
    let dependency = task(&store, "Publish architecture");
    let mut brief = task_input("Blocked implementation");
    brief["prerequisite_task_ids"] = json!([dependency.id]);
    brief["priority"] = json!("high");
    let waiting = store.create_task(input(brief)).unwrap();
    let matches = store
        .list_tasks(input(
            json!({"query": "AUTHENTICATION", "ready_to_start": true, "status": "todo", "priority": "medium"}),
        ))
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, ready.id);
    let blocked = store
        .list_tasks(input(json!({"ready_to_start": false, "priority": "high"})))
        .unwrap();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].id, waiting.id);
    let local = inline(
        &store,
        "Authentication guide",
        "All details.",
        "when_relevant",
    );
    external(&store, "Remote discussion");
    let matches = store
        .list_sources(input(
            json!({"representation": "inline", "query": "AUTHENTICATION"}),
        ))
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, local.id);
}

#[test]
fn task_priorities_sort_filter_and_survive_revision_checked_updates_and_restart() {
    let (directory, store) = database();
    let database_path = directory.path().join("project.sqlite3");
    let default = task(&store, "Priority default");
    assert_eq!(default.priority, TaskPriority::Medium);
    // Inspect the MCP-created fixture's constraints without calling Store directly.
    let connection = rusqlite::Connection::open(&database_path).unwrap();
    for priority in [Some("urgent"), None] {
        assert!(
            connection
                .execute(
                    "UPDATE tasks SET priority = ?1 WHERE id = ?2",
                    rusqlite::params![priority, default.id]
                )
                .is_err()
        );
    }
    drop(connection);
    assert_eq!(
        store.get_context(default.id).unwrap().task.priority,
        TaskPriority::Medium
    );
    let make = |title: &str, priority: &str| {
        let mut brief = task_input(title);
        brief["priority"] = json!(priority);
        store.create_task(input(brief)).unwrap()
    };
    let low = make("Priority low", "low");
    let high = make("Priority high first", "high");
    let high_second = make("Priority high second", "high");
    let ordered = store.list_tasks(input(json!({}))).unwrap();
    assert_eq!(
        ordered.iter().map(|task| task.id).collect::<Vec<_>>(),
        vec![high.id, high_second.id, default.id, low.id]
    );
    assert_eq!(
        ordered.iter().map(|task| task.priority).collect::<Vec<_>>(),
        vec![
            TaskPriority::High,
            TaskPriority::High,
            TaskPriority::Medium,
            TaskPriority::Low
        ]
    );
    assert_eq!(store.list_tasks(input(json!({"priority": "high", "kind": "one_time", "status": "todo", "ready_to_start": true, "query": "PRIORITY HIGH"}))).unwrap().iter().map(|task| task.id).collect::<Vec<_>>(), vec![high.id, high_second.id]);
    assert!(
        store
            .list_tasks(input(json!({"priority": "low", "query": "high"})))
            .unwrap()
            .is_empty()
    );
    let high = store
        .update_task(task_patch(
            &high,
            json!({"progress": "Priority must be preserved when omitted."}),
        ))
        .unwrap();
    assert_eq!(high.priority, TaskPriority::High);
    let low = store
        .update_task(task_patch(
            &low,
            json!({"priority": "high", "title": "Priority promoted"}),
        ))
        .unwrap();
    assert_eq!(low.priority, TaskPriority::High);
    assert_eq!(
        store
            .list_tasks(input(json!({})))
            .unwrap()
            .iter()
            .map(|task| task.id)
            .collect::<Vec<_>>(),
        vec![low.id, high.id, high_second.id, default.id]
    );
    store
        .update_task(task_patch(&default, json!({"priority": "low"})))
        .unwrap();
    let default = store.get_context(default.id).unwrap().task;
    assert_eq!(default.priority, TaskPriority::Low);
    let stale = store.update_task(input(json!({"task_id": default.id, "expected_revision": default.revision - 1, "priority": "high", "title": "Must not overwrite"})));
    assert!(stale.is_err());
    assert_eq!(
        store.get_context(default.id).unwrap().task.priority,
        TaskPriority::Low
    );
    assert_eq!(
        store.get_context(default.id).unwrap().task.title,
        default.title
    );
    assert_eq!(
        store.get_context(low.id).unwrap().task.priority,
        TaskPriority::High
    );

    for priority in [
        json!("urgent"),
        json!("HIGH"),
        json!(""),
        json!(3),
        Value::Null,
    ] {
        let before = store.list_tasks(input(json!({}))).unwrap();
        let mut invalid = task_input("Invalid priority must not create a task");
        invalid["priority"] = priority;
        assert!(store.call::<Value>("create_task", invalid).is_err());
        assert_eq!(
            serde_json::to_value(store.list_tasks(input(json!({}))).unwrap()).unwrap(),
            serde_json::to_value(before).unwrap()
        );
    }
    for priority in [json!("urgent"), json!("HIGH"), json!(""), json!(3)] {
        let before = store.get_context(high.id).unwrap();
        assert!(store.call::<Value>("update_task", json!({"task_id": high.id, "expected_revision": high.revision, "priority": priority, "title": "A valid edit must roll back", "progress": "Not applied"})).is_err());
        assert_eq!(
            serde_json::to_value(store.get_context(high.id).unwrap()).unwrap(),
            serde_json::to_value(before).unwrap()
        );
        assert!(
            store
                .call::<Value>("list_tasks", json!({"priority": priority}))
                .is_err()
        );
    }
    let expected = store.list_tasks(input(json!({}))).unwrap();
    drop(store);
    let restarted = Project::open(&database_path).unwrap();
    assert_eq!(
        serde_json::to_value(restarted.list_tasks(input(json!({}))).unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(
        restarted.get_context(low.id).unwrap().task.priority,
        TaskPriority::High
    );
}
