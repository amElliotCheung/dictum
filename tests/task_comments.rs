//! General task discussion and execution reports through real MCP processes.
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
            "title": title, "kind": kind, "problem_or_goal": "Retain task discussion and evidence.",
            "design": "Append version-targeted comments without changing the task.",
            "acceptance_criteria": ["A new process can retrieve the complete discussion."]
        }),
    )
}

fn context(client: &Project, task: &Value) -> Value {
    call(client, "get_task_context", json!({"task_id": task["id"]}))
}

fn comment(
    client: &Project,
    task: &Value,
    body: &str,
    execution: Option<&str>,
    reply: Option<i64>,
) -> Value {
    let mut input = json!({"task_id": task["id"], "task_revision": task["revision"], "body": body});
    if let Some(outcome) = execution {
        input["execution_result"] = json!({"outcome": outcome});
    }
    if let Some(id) = reply {
        input["reply_to_comment_id"] = json!(id);
    }
    call(client, "record_task_comment", input)
}

fn snapshot(database: &Path) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    let connection = rusqlite::Connection::open(database).unwrap();
    let tables = connection.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
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

#[test]
fn comments_target_existing_versions_and_preserve_the_complete_task_across_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("discussion.sqlite3");
    let client = Project::open(&database).unwrap();
    let original = task(&client, "Discuss before executing", "one_time");
    let before = Utc::now();
    let note = comment(
        &client,
        &original,
        "## Question\n\nShould we preserve the existing endpoint?",
        None,
        None,
    );
    let after = Utc::now();
    assert_eq!(note["task_revision"], original["revision"]);
    assert_eq!(note["project_id"], original["project_id"]);
    assert!(note["execution_result"].is_null());
    assert!(note["reply_to_comment_id"].is_null());
    let recorded = DateTime::parse_from_rfc3339(note["recorded_at"].as_str().unwrap()).unwrap();
    assert_eq!(recorded.offset().local_minus_utc(), 0);
    assert!(recorded.timestamp_millis() >= before.timestamp_millis());
    assert!(recorded.timestamp_millis() <= after.timestamp_millis());
    assert_eq!(context(&client, &original)["task"], original);
    let updated = call(
        &client,
        "update_task",
        json!({
            "task_id": original["id"], "expected_revision": original["revision"], "design": "Preserve the endpoint and add a new field."
        }),
    );
    let reply = comment(
        &client,
        &original,
        "This reply concerns the original design.",
        None,
        note["id"].as_i64(),
    );
    let execution = comment(
        &client,
        &updated,
        "Checked revision abc123: compatibility checks passed.",
        Some("success"),
        reply["id"].as_i64(),
    );
    assert_eq!(context(&client, &original)["task"], updated);
    assert_eq!(reply["task_revision"], original["revision"]);
    assert_eq!(execution["task_revision"], updated["revision"]);
    let get = call(
        &client,
        "get_task_comment",
        json!({"task_id": original["id"], "comment_id": note["id"]}),
    );
    assert_eq!(get, note);
    let history = call(
        &client,
        "list_task_comments",
        json!({"task_id": original["id"]}),
    );
    assert_eq!(history["comments"], json!([execution, reply, note]));
    assert!(history["next_before_id"].is_null());
    drop(client);
    let restarted = Project::open(&database).unwrap();
    assert_eq!(context(&restarted, &original)["task"], updated);
    assert_eq!(
        call(
            &restarted,
            "list_task_comments",
            json!({"task_id": original["id"]})
        ),
        history
    );
}

#[test]
fn bounded_context_keeps_recent_discussion_and_independent_latest_execution_projections() {
    let directory = tempfile::tempdir().unwrap();
    let client = Project::open(directory.path().join("bounded.sqlite3")).unwrap();
    let task = task(&client, "Bound context", "recurring");
    let empty = context(&client, &task);
    assert_eq!(empty["recent_comments"], json!([]));
    assert!(empty["latest_execution"].is_null());
    assert!(empty["latest_successful_execution"].is_null());
    let success = comment(
        &client,
        &task,
        "Release inspected; all checks passed.",
        Some("no_change"),
        None,
    );
    let failure = comment(
        &client,
        &task,
        "Next release could not be inspected.",
        Some("failure"),
        None,
    );
    let mut notes = Vec::new();
    for index in 0..12 {
        notes.push(comment(
            &client,
            &task,
            &format!("Discussion note {index}"),
            None,
            None,
        ));
    }
    notes.reverse();
    let current = context(&client, &task);
    assert_eq!(current["task"], task);
    assert_eq!(current["recent_comments"], json!(notes[..10]));
    assert_eq!(current["latest_execution"], failure);
    assert_eq!(current["latest_successful_execution"], success);
    for removed in [
        "latest_result",
        "latest_successful_result",
        "outstanding_feedback",
    ] {
        assert!(current.get(removed).is_none());
    }
    let executions = call(
        &client,
        "list_task_comments",
        json!({"task_id": task["id"], "has_execution_result": true}),
    );
    assert_eq!(executions["comments"], json!([failure, success]));
    let discussion = call(
        &client,
        "list_task_comments",
        json!({"task_id": task["id"], "has_execution_result": false, "page_size": 100}),
    );
    assert_eq!(discussion["comments"], json!(notes));
}

#[test]
fn invalid_comments_versions_replies_outputs_and_legacy_fields_are_atomic() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("validation.sqlite3");
    let client = Project::open(&database).unwrap();
    let first = task(&client, "First task", "one_time");
    let second = task(&client, "Other task", "one_time");
    let other_note = comment(&client, &second, "Other task note", None, None);
    let foreign_project = call(
        &client,
        "create_project",
        json!({"name": "Other", "description": "Isolated scope"}),
    );
    let foreign_task = call(
        &client,
        "create_task",
        json!({
            "project_id": foreign_project["id"], "title": "Foreign discussion", "problem_or_goal": "Preserve scope isolation.",
            "design": "Replies must stay on one task.", "acceptance_criteria": ["Foreign replies are rejected."]
        }),
    );
    let foreign_note = call(
        &client,
        "record_task_comment",
        json!({
            "project_id": foreign_project["id"], "task_id": foreign_task["id"], "task_revision": foreign_task["revision"], "body": "Foreign note"
        }),
    );
    let foreign_source = call(
        &client,
        "create_source",
        json!({"project_id": foreign_project["id"], "name": "Foreign", "description": "Not this task's output", "content": "Evidence"}),
    );
    let source = call(
        &client,
        "create_source",
        json!({"name": "Valid", "description": "Valid execution output", "content": "Evidence"}),
    );
    let valid = json!({"task_id": first["id"], "task_revision": first["revision"], "body": "A complete comment."});
    for field in ["task_id", "task_revision", "body"] {
        let mut input = valid.clone();
        input.as_object_mut().unwrap().remove(field);
        rejected(&client, &database, "record_task_comment", input);
    }
    for version in [
        json!(0),
        json!(-1),
        json!(2),
        json!(1.5),
        json!("1"),
        Value::Null,
    ] {
        let mut input = valid.clone();
        input["task_revision"] = version;
        rejected(&client, &database, "record_task_comment", input);
    }
    for body in [
        json!(""),
        json!(" \n\t"),
        json!("界".repeat(4_001)),
        Value::Null,
    ] {
        let mut input = valid.clone();
        input["body"] = body;
        rejected(&client, &database, "record_task_comment", input);
    }
    for reply in [
        json!(0),
        json!(-1),
        json!(999_999),
        other_note["id"].clone(),
        foreign_note["id"].clone(),
    ] {
        let mut input = valid.clone();
        input["reply_to_comment_id"] = reply;
        rejected(&client, &database, "record_task_comment", input);
    }
    for (key, value) in [
        ("expected_revision", json!(1)),
        ("author", json!("agent")),
        ("disposition", json!("changes_requested")),
        ("addressed_feedback_ids", json!([])),
        ("outcome", json!("success")),
        ("evidence", json!("old shape")),
        ("recorded_at", json!("2020-01-01T00:00:00Z")),
    ] {
        let mut input = valid.clone();
        input[key] = value;
        rejected(&client, &database, "record_task_comment", input);
    }
    for execution in [
        json!({}),
        json!({"outcome": "done"}),
        json!({"outcome": "SUCCESS"}),
        json!({"outcome": "success", "addressed_comment_ids": []}),
    ] {
        let mut input = valid.clone();
        input["execution_result"] = execution;
        rejected(&client, &database, "record_task_comment", input);
    }
    for invalid in [json!(0), json!(999_999), foreign_source["id"].clone()] {
        let mut input = valid.clone();
        input["execution_result"] =
            json!({"outcome": "failure", "output_source_ids": [source["id"], invalid]});
        rejected(&client, &database, "record_task_comment", input);
    }
    let max_body = "界".repeat(4_000);
    let accepted = comment(&client, &first, &max_body, None, None);
    assert_eq!(accepted["body"], max_body);
    assert_eq!(context(&client, &first)["task"], first);
    for (task_id, comment_id) in [
        (first["id"].clone(), other_note["id"].clone()),
        (first["id"].clone(), json!(999_999)),
    ] {
        rejected(
            &client,
            &database,
            "get_task_comment",
            json!({"task_id": task_id, "comment_id": comment_id}),
        );
    }
    rejected(
        &client,
        &database,
        "get_task_comment",
        json!({"project_id": foreign_project["id"], "task_id": first["id"], "comment_id": accepted["id"]}),
    );
}

#[test]
fn concurrent_comments_and_task_edits_do_not_conflict_or_lose_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("concurrent.sqlite3");
    let client = Project::open(&database).unwrap();
    let original = task(&client, "Concurrent discussion", "one_time");
    let barrier = Arc::new(Barrier::new(3));
    let mut writers = Vec::new();
    for index in 0..3 {
        let database = database.clone();
        let task = original.clone();
        let barrier = Arc::clone(&barrier);
        writers.push(std::thread::spawn(move || {
            let client = Project::open(database).unwrap();
            barrier.wait();
            if index == 2 {
                call(&client, "update_task", json!({"task_id": task["id"], "expected_revision": task["revision"], "design": "A concurrently revised design."}))
            } else {
                comment(&client, &task, &format!("Writer {index} inspected version one."), Some("failure"), None)
            }
        }));
    }
    let values: Vec<_> = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect();
    assert_ne!(values[0]["id"], values[1]["id"]);
    assert_eq!(values[0]["task_revision"], 1);
    assert_eq!(values[1]["task_revision"], 1);
    let current = context(&client, &original);
    assert_eq!(current["task"], values[2]);
    assert_eq!(current["task"]["revision"], 2);
    assert_eq!(current["recent_comments"].as_array().unwrap().len(), 2);
}

#[test]
fn discussion_and_execution_comments_preserve_done_and_paused_lifecycles() {
    let directory = tempfile::tempdir().unwrap();
    let client = Project::open(directory.path().join("statuses.sqlite3")).unwrap();
    for (kind, status) in [("one_time", "done"), ("recurring", "paused")] {
        let original = task(&client, kind, kind);
        comment(&client, &original, "Checks passed.", Some("success"), None);
        let updated = call(
            &client,
            "update_task",
            json!({"task_id": original["id"], "expected_revision": original["revision"], "status": status}),
        );
        let note = comment(
            &client,
            &updated,
            "Review discussion needs another look.",
            None,
            None,
        );
        comment(
            &client,
            &original,
            "Late evidence about version one.",
            Some("failure"),
            note["id"].as_i64(),
        );
        assert_eq!(context(&client, &original)["task"], updated);
    }
}
#[test]
fn historical_execution_uses_current_dependency_gates_and_success_still_allows_done() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("historical-gates.sqlite3");
    let client = Project::open(&database).unwrap();
    let original = task(&client, "Versioned task", "one_time");
    let gate = task(&client, "Completion gate", "one_time");
    let gated = call(
        &client,
        "update_task",
        json!({
            "task_id": original["id"], "expected_revision": original["revision"],
            "completion_task_ids": [gate["id"]], "design": "A later design adds an explicit completion gate."
        }),
    );
    rejected(
        &client,
        &database,
        "record_task_comment",
        json!({
            "task_id": original["id"], "task_revision": original["revision"],
            "body": "Historical success cannot bypass the current completion gate.", "execution_result": {"outcome": "success"}
        }),
    );
    comment(
        &client,
        &original,
        "Historical attempt failed while the gate remains unmet.",
        Some("failure"),
        None,
    );
    comment(
        &client,
        &gate,
        "Completion gate checks passed.",
        Some("success"),
        None,
    );
    call(
        &client,
        "update_task",
        json!({"task_id": gate["id"], "expected_revision": gate["revision"], "status": "done"}),
    );
    let old_success = comment(
        &client,
        &original,
        "Late report: version one passed checks after all current gates were satisfied.",
        Some("success"),
        None,
    );
    assert_eq!(old_success["task_revision"], original["revision"]);
    let revised = call(
        &client,
        "update_task",
        json!({"task_id": original["id"], "expected_revision": gated["revision"], "design": "Another recorded design revision."}),
    );
    comment(
        &client,
        &revised,
        "A later attempt failed; earlier successful evidence remains retained.",
        Some("failure"),
        None,
    );
    let done = call(
        &client,
        "update_task",
        json!({"task_id": original["id"], "expected_revision": revised["revision"], "status": "done"}),
    );
    assert_eq!(done["status"], "done");
    assert_eq!(
        context(&client, &original)["latest_successful_execution"],
        old_success
    );
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
        error.contains("schema version 13"),
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

#[test]
fn recent_unsupported_versions_are_rejected_without_changing_existing_records() {
    for version in [8, 9, 10, 11, 12] {
        let directory = tempfile::tempdir().unwrap();
        let database = directory
            .path()
            .join(format!("preserved-v{version}.sqlite3"));
        let connection = rusqlite::Connection::open(&database).unwrap();
        // Version rejection happens before schema interpretation. Keep a small,
        // fixed fixture with existing records rather than historical server code.
        connection.execute_batch(r#"
            CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT, description TEXT, created_at TEXT);
            CREATE TABLE tasks (id INTEGER PRIMARY KEY, project_id TEXT, title TEXT, progress TEXT);
            CREATE TABLE decisions (id INTEGER PRIMARY KEY, project_id TEXT, statement TEXT, rationale TEXT);
            CREATE TABLE task_results (id INTEGER PRIMARY KEY, task_id INTEGER, evidence TEXT);
            PRAGMA application_id = 1145652052;
            INSERT INTO projects VALUES ('0199b3d8-f600-7000-8000-000000000001', 'Keep project', 'Existing workspace', '2026-10-23T00:00:00Z');
            INSERT INTO tasks VALUES (1, '0199b3d8-f600-7000-8000-000000000001', 'Keep task', 'Original progress');
            INSERT INTO decisions VALUES (1, '0199b3d8-f600-7000-8000-000000000001', 'Keep accepted choice', 'Original rationale');
            INSERT INTO task_results VALUES (1, 1, 'Original verification evidence.');
        "#).unwrap();
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        let schema = connection
            .prepare("SELECT type, name, sql FROM sqlite_schema ORDER BY type, name")
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        drop(connection);
        let before = std::fs::read(&database).unwrap();
        let records = snapshot(&database);

        for arguments in [vec![], vec!["dashboard", "--port", "0"]] {
            let output = std::process::Command::new(env!("CARGO_BIN_EXE_dictum-mcp"))
                .args(arguments)
                .env("DICTUM_DATABASE", &database)
                .output()
                .unwrap();
            assert!(!output.status.success());
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(
                error.contains(&format!("version {version}")),
                "unexpected error: {error}"
            );
            assert!(
                error.contains("schema version 13"),
                "unexpected error: {error}"
            );
            assert_eq!(std::fs::read(&database).unwrap(), before);
            assert_eq!(snapshot(&database), records);

            let connection = rusqlite::Connection::open(&database).unwrap();
            let actual_version: i64 = connection
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(actual_version, version);
            let after_schema = connection
                .prepare("SELECT type, name, sql FROM sqlite_schema ORDER BY type, name")
                .unwrap()
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(after_schema, schema);
        }
    }
}
