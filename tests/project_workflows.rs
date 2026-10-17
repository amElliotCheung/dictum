//! Behavioral scenarios against the real MCP binary. See support for transport setup.
mod support;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
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
    value["project_id"] = json!("");
    serde_json::from_value(value).unwrap()
}

fn task_input(title: &str) -> Value {
    json!({
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
    store
        .update_task(task_patch(
            task,
            json!({"status": "done", "result": "Verified revision abc123: cargo test passed."}),
        ))
        .unwrap()
}

fn inline(store: &Project, name: &str, content: &str, policy: &str) -> Source {
    store
        .create_source(input(json!({
            "name": name,
            "description": "Project context and when to use it.",
            "content": content,
            "context_policy": policy
        })))
        .unwrap()
}

fn external(store: &Project, name: &str) -> Source {
    store
        .create_source(input(json!({
            "name": name,
            "description": "Daily discussion of project requirements.",
            "locator": "slack://workspace/channel",
            "action": {
                "instruction": "Review new messages and summarize current project-relevant context.",
                "trigger": {"kind": "interval", "every_minutes": 1440}
            }
        })))
        .unwrap()
}

fn source_patch(source: &Source, mut patch: Value) -> UpdateSource {
    patch["source_id"] = json!(source.id);
    patch["expected_revision"] = json!(source.revision);
    input(patch)
}

fn action_input(source: &Source, result: Value) -> RecordSourceAction {
    input(json!({
        "source_id": source.id,
        "expected_revision": source.revision,
        "result": result
    }))
}

fn observed(minutes_ago: i64) -> String {
    (Utc::now() - Duration::minutes(minutes_ago)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn success(store: &Project, source: &Source, minutes_ago: i64) -> Source {
    store
        .record_source_action(action_input(
            source,
            json!({
                "outcome": "success",
                "observed_at": observed(minutes_ago),
                "summary": "The API is approved. Authentication remains an open question.",
                "checkpoint": "message:123",
                "references": ["slack://workspace/channel/thread123"]
            }),
        ))
        .unwrap()
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
    let task = store
        .update_task(task_patch(
            &task,
            json!({"output_source_ids": [context.id, output.id]}),
        ))
        .unwrap();

    let expanded = store.get_task_context(task.id).unwrap();
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
    assert_eq!(expanded.outputs.len(), 2);
    assert!(
        expanded
            .outputs
            .iter()
            .any(|source| source.id == context.id)
    );
    assert!(expanded.outputs.iter().any(|source| source.id == output.id));

    // Discovery is compact and task filtering uses explicit links, not implicit always context.
    let descriptors = store
        .list_sources(input(json!({"task_id": task.id})))
        .unwrap();
    assert_eq!(descriptors.len(), 3);
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
            .get_task_context(second.id)
            .unwrap()
            .sources
            .iter()
            .any(|source| source.id == context.id)
    );

    // Loading context is a read, not a source-action completion.
    let reread = store.get_source(shared.id).unwrap();
    assert_eq!(reread.revision, shared.revision);
    assert!(reread.latest_attempt.is_none());
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
    let context = store.get_task_context(implementation.id).unwrap();
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
            .get_task_context(implementation.id)
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
            .update_task(task_patch(
                &implementation,
                json!({"status": "done", "result": "Implementation is complete."})
            ))
            .is_err()
    );
    complete(&store, &verification);
    assert_eq!(
        store
            .get_task_context(implementation.id)
            .unwrap()
            .task
            .status,
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
            .get_task_context(implementation.id)
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
                json!({"title": "Also not saved", "context_source_ids": [999_999]})
            ))
            .is_err()
    );
    let unchanged = store.get_task_context(first.id).unwrap().task;
    assert_eq!(unchanged.title, first.title);
    assert_eq!(unchanged.revision, first.revision);
    assert!(unchanged.completion_task_ids.is_empty());
    assert!(unchanged.context_source_ids.is_empty());

    let mut invalid = task_input("Missing dependency");
    invalid["completion_task_ids"] = json!([999_999]);
    assert!(store.create_task(input(invalid)).is_err());
    assert_eq!(store.list_tasks(ListTasks::default()).unwrap().len(), 2);

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
    assert!(store.list_tasks(ListTasks::default()).unwrap().is_empty());
}

#[test]
fn source_actions_preserve_one_successful_snapshot_across_no_change_and_failure() {
    let (_directory, store) = database();
    let initial = external(&store, "Project Slack");
    assert!(initial.action_due);
    assert!(initial.next_action_at.is_none());
    let mut brief = task_input("Read channel context");
    brief["context_source_ids"] = json!([initial.id]);
    let reader = store.create_task(input(brief)).unwrap();
    let context = store.get_task_context(reader.id).unwrap();
    assert_eq!(context.sources[0].revision, initial.revision);
    assert!(context.sources[0].snapshot.is_none());
    assert!(context.sources[0].latest_attempt.is_none());
    assert!(context.sources[0].action_due);
    assert!(
        store
            .record_source_action(action_input(
                &initial,
                json!({"outcome": "no_change", "observed_at": observed(5)})
            ))
            .is_err()
    );
    let failed = store
        .record_source_action(action_input(
            &initial,
            json!({"outcome": "failure", "error": "Slack is unavailable."}),
        ))
        .unwrap();
    assert!(failed.snapshot.is_none());
    assert!(failed.action_due);
    assert_eq!(
        failed.latest_attempt.as_ref().unwrap().outcome,
        ActionOutcome::Failure
    );

    let succeeded = success(&store, &failed, 4);
    assert!(!succeeded.action_due);
    assert!(succeeded.next_action_at.is_some());
    let snapshot = succeeded.snapshot.clone().unwrap();
    let unchanged = store
        .record_source_action(action_input(
            &succeeded,
            json!({
                "outcome": "no_change", "observed_at": observed(3)
            }),
        ))
        .unwrap();
    let retained = unchanged.snapshot.as_ref().unwrap();
    assert_eq!(retained.summary, snapshot.summary);
    assert_eq!(retained.references, snapshot.references);
    assert_eq!(retained.checkpoint, snapshot.checkpoint);
    assert_ne!(retained.observed_at, snapshot.observed_at);
    assert_eq!(
        unchanged.latest_attempt.as_ref().unwrap().outcome,
        ActionOutcome::NoChange
    );
    let advanced = store
        .record_source_action(action_input(
            &unchanged,
            json!({
                "outcome": "no_change", "observed_at": observed(2), "checkpoint": "message:456"
            }),
        ))
        .unwrap();
    assert_eq!(
        advanced.snapshot.as_ref().unwrap().checkpoint.as_deref(),
        Some("message:456")
    );
    let advanced = store
        .record_source_action(action_input(
            &advanced,
            json!({
                "outcome": "no_change", "observed_at": observed(1), "checkpoint": null
            }),
        ))
        .unwrap();
    assert!(advanced.snapshot.as_ref().unwrap().checkpoint.is_none());
    let failed_again = store
        .record_source_action(action_input(
            &advanced,
            json!({"outcome": "failure", "error": "Rate limited."}),
        ))
        .unwrap();
    assert_eq!(failed_again.snapshot, advanced.snapshot);
    assert_eq!(
        failed_again
            .latest_attempt
            .as_ref()
            .unwrap()
            .error
            .as_deref(),
        Some("Rate limited.")
    );
    assert!(!failed_again.action_due);

    let replaced = store.record_source_action(action_input(&failed_again, json!({
        "outcome": "success", "observed_at": observed(0), "summary": "Authentication has now been decided.",
        "references": ["slack://workspace/channel/new-thread"]
    }))).unwrap();
    assert_eq!(
        replaced.snapshot.as_ref().unwrap().summary,
        "Authentication has now been decided."
    );
    assert_eq!(
        replaced.snapshot.as_ref().unwrap().references,
        vec!["slack://workspace/channel/new-thread"]
    );
    assert!(replaced.snapshot.as_ref().unwrap().checkpoint.is_none());
    assert_eq!(
        replaced.latest_attempt.as_ref().unwrap().outcome,
        ActionOutcome::Success
    );
    assert!(replaced.latest_attempt.as_ref().unwrap().error.is_none());
}

#[test]
fn source_action_cas_time_validation_and_summary_limit_leave_state_unchanged() {
    let (_directory, store) = database();
    let initial = external(&store, "Action validation");
    let current = success(&store, &initial, 5);
    assert!(
        store
            .record_source_action(action_input(
                &initial,
                json!({"outcome": "failure", "error": "Stale attempt"})
            ))
            .is_err()
    );
    for observation in [
        observed(10),
        "2099-01-01T00:00:00Z".to_owned(),
        "not a date".to_owned(),
    ] {
        assert!(
            store
                .record_source_action(action_input(
                    &current,
                    json!({"outcome": "no_change", "observed_at": observation})
                ))
                .is_err()
        );
    }
    assert!(
        store
            .record_source_action(action_input(
                &current,
                json!({
                    "outcome": "success", "observed_at": observed(4), "summary": "界".repeat(1001)
                })
            ))
            .is_err()
    );
    let reread = store.get_source(current.id).unwrap();
    assert_eq!(reread.revision, current.revision);
    assert_eq!(reread.snapshot, current.snapshot);
    assert_eq!(reread.latest_attempt, current.latest_attempt);
    let at_limit = store
        .record_source_action(action_input(
            &current,
            json!({
                "outcome": "success", "observed_at": observed(4), "summary": "界".repeat(1000)
            }),
        ))
        .unwrap();
    assert_eq!(at_limit.snapshot.unwrap().summary.chars().count(), 1000);
}

#[test]
fn source_definition_changes_invalidate_state_but_metadata_edits_do_not() {
    let (_directory, store) = database();
    let source = external(&store, "Editable source");
    let source = success(&store, &source, 5);
    let metadata = store.update_source(source_patch(&source, json!({
        "name": "Renamed source", "description": "A clearer explanation.", "context_policy": "always"
    }))).unwrap();
    assert_eq!(metadata.snapshot, source.snapshot);
    assert_eq!(metadata.latest_attempt, source.latest_attempt);
    let retargeted = store
        .update_source(source_patch(
            &metadata,
            json!({"locator": "slack://workspace/other-channel"}),
        ))
        .unwrap();
    assert!(retargeted.snapshot.is_none());
    assert!(retargeted.latest_attempt.is_none());
    assert!(retargeted.action_due);
    assert!(
        store
            .record_source_action(action_input(
                &metadata,
                json!({"outcome": "failure", "error": "Result from old target"})
            ))
            .is_err()
    );
    let retargeted = success(&store, &retargeted, 4);
    let changed_action = store.update_source(source_patch(&retargeted, json!({
        "action": {"instruction": "Review security changes only.", "trigger": {"kind": "interval", "every_minutes": 60}}
    }))).unwrap();
    assert!(changed_action.snapshot.is_none());
    assert!(changed_action.latest_attempt.is_none());
    let changed_action = success(&store, &changed_action, 3);
    let removed = store
        .update_source(source_patch(&changed_action, json!({"action": null})))
        .unwrap();
    assert!(removed.action.is_none());
    assert!(removed.snapshot.is_none());
    assert!(removed.latest_attempt.is_none());
    assert!(!removed.action_due);
    assert!(removed.next_action_at.is_none());
    assert!(
        store
            .record_source_action(action_input(
                &removed,
                json!({"outcome": "failure", "error": "No configured action"})
            ))
            .is_err()
    );
}

#[test]
fn source_representation_switches_are_atomic_and_inline_edits_invalidate_actions() {
    let (_directory, store) = database();
    for definition in [
        json!({"name": "Neither", "description": "Invalid"}),
        json!({"name": "Both", "description": "Invalid", "content": "Text", "locator": "https://example.test/doc"}),
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
    let source = success(&store, &source, 2);
    let source = store
        .update_source(source_patch(
            &source,
            json!({"content": "# Updated document\nNew content."}),
        ))
        .unwrap();
    assert!(source.snapshot.is_none());
    assert!(source.latest_attempt.is_none());
    assert!(source.action_due);
}

#[test]
fn manual_actions_are_unscheduled_and_interval_freshness_is_computed() {
    let (_directory, store) = database();
    let manual = store.create_source(input(json!({
        "name": "Manual reference", "description": "Inspect only when needed.", "locator": "https://example.test/doc",
        "action": {"instruction": "Inspect the current document.", "trigger": {"kind": "manual"}}
    }))).unwrap();
    assert!(!manual.action_due);
    assert!(manual.next_action_at.is_none());
    let manual = success(&store, &manual, 10_000);
    assert!(!manual.action_due);
    assert!(manual.next_action_at.is_none());

    let overdue = external(&store, "Overdue source");
    let overdue = success(&store, &overdue, 2 * 1440);
    assert!(overdue.action_due);
    let recent = external(&store, "Recently observed source");
    let recent = success(&store, &recent, 2);
    assert!(!recent.action_due);
    let due = store
        .list_sources(input(json!({"action_due": true})))
        .unwrap();
    assert_eq!(
        due.iter().map(|source| source.id).collect::<Vec<_>>(),
        vec![overdue.id]
    );
    for minutes in [0, -1, i64::MAX] {
        assert!(store.create_source(input(json!({
            "name": "Invalid interval", "description": "Must be positive.", "locator": "https://example.test/doc",
            "action": {"instruction": "Review document.", "trigger": {"kind": "interval", "every_minutes": minutes}}
        }))).is_err());
    }
}

#[test]
fn decisions_are_timestamped_immutable_and_have_one_current_successor() {
    let (_directory, store) = database();
    let before = Utc::now() - Duration::seconds(1);
    let first = store
        .record_decision(input(json!({
            "statement": "One project per SQLite database/server process.",
            "rationale": "Keep the local MVP simple."
        })))
        .unwrap();
    let created = DateTime::parse_from_rfc3339(&first.created_at).unwrap();
    assert_eq!(created.offset().local_minus_utc(), 0);
    assert!(created >= before);
    assert!(created <= Utc::now());
    assert!(first.is_current);
    let second = store
        .record_decision(input(json!({
            "statement": "Keep one project per database, shared by local server processes.",
            "rationale": "Allow multiple local agents to share durable state.",
            "supersedes_id": first.id
        })))
        .unwrap();
    let current = store.list_decisions(ListDecisions::default()).unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].id, second.id);
    assert!(store.record_decision(input(json!({"statement": "Another replacement", "rationale": "Invalid branch.", "supersedes_id": first.id}))).is_err());
    assert!(store.record_decision(input(json!({"statement": "Missing predecessor", "rationale": "Invalid reference.", "supersedes_id": 999_999}))).is_err());
    let history = store
        .list_decisions(input(json!({"include_superseded": true})))
        .unwrap();
    assert_eq!(history.len(), 2);
    let retained = history
        .iter()
        .find(|decision| decision.id == first.id)
        .unwrap();
    assert_eq!(retained.statement, first.statement);
    assert_eq!(retained.created_at, first.created_at);
    assert!(!retained.is_current);
    let task = task(&store, "Use current decisions");
    let context = store.get_task_context(task.id).unwrap();
    assert_eq!(context.current_decisions.len(), 1);
    assert_eq!(context.current_decisions[0].id, second.id);
}

#[test]
fn mutable_records_persist_and_revisions_coordinate_separate_processes() {
    let (directory, first_store) = database();
    let source = external(&first_store, "Persistent source");
    let source = success(&first_store, &source, 3);
    let mut brief = task_input("Persistent task");
    brief["context_source_ids"] = json!([source.id]);
    let task = first_store.create_task(input(brief)).unwrap();
    let second_store = Project::open(directory.path().join("project.sqlite3")).unwrap();
    assert_eq!(
        second_store.get_source(source.id).unwrap().snapshot,
        source.snapshot
    );
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
    let context = reopened.get_task_context(task.id).unwrap();
    assert_eq!(context.task.progress, changed.progress);
    assert_eq!(context.sources.len(), 1);
    assert_eq!(context.sources[0].snapshot, source.snapshot);
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
    let persisted = store.get_task_context(original.id).unwrap().task;
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
    let waiting = store.create_task(input(brief)).unwrap();
    let matches = store
        .list_tasks(input(
            json!({"query": "AUTHENTICATION", "ready_to_start": true, "status": "todo"}),
        ))
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, ready.id);
    let blocked = store
        .list_tasks(input(json!({"ready_to_start": false})))
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
