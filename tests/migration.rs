//! Migration is an explicit real-binary operation, followed by actual MCP use.
mod support;

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Output},
};
use support::Project;

const PROJECT_IDS: [&str; 3] = [
    "01961380-4000-7000-8000-000000000001",
    "01961380-4000-7000-9000-000000000002",
    "01961380-4000-7000-a000-000000000003",
];

fn legacy(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(include_str!("fixtures/schema_v2.sql"))
        .unwrap();
    connection
        .execute_batch("PRAGMA application_id = 1145652052; PRAGMA user_version = 2;")
        .unwrap();
    for (id, title, status, result) in [
        (7, "Legacy implementation", "in_progress", None),
        (
            12,
            "Legacy prerequisite",
            "done",
            Some("Verified revision old123; fixture evidence."),
        ),
        (19, "Legacy verification", "todo", None),
    ] {
        connection.execute("INSERT INTO tasks VALUES (?1, ?2, 'A preserved problem', 'A preserved design', '[\"First criterion\",\"Second criterion\"]', 'Rejected alternative', ?3, 'Preserved progress', ?4, 8, '2025-01-02T03:04:05.000Z', '2025-02-03T04:05:06.000Z')", params![id, title, status, result]).unwrap();
    }
    let action =
        r#"{"instruction":"Review new changes","trigger":{"kind":"interval","every_minutes":60}}"#;
    let snapshot = r#"{"observed_at":"2025-02-03T04:05:06.000Z","summary":"Preserved successful summary.","checkpoint":"opaque:unchanged","references":["https://example.test/reference"]}"#;
    let attempt = r#"{"recorded_at":"2025-02-04T04:05:06.000Z","outcome":"failure","error":"Preserved last failure"}"#;
    connection.execute("INSERT INTO sources VALUES (5, 'Legacy external', 'Task and action context', NULL, 'https://example.test/legacy', 'when_relevant', ?1, ?2, ?3, 11, '2025-01-02T03:04:05.000Z', '2025-02-04T04:05:06.000Z')", params![action, snapshot, attempt]).unwrap();
    connection.execute("INSERT INTO sources VALUES (9, 'Legacy policy', 'Always included inline text', '# Full legacy document\n完整内容 🦀', NULL, 'always', NULL, NULL, NULL, 2, '2025-01-02T03:04:05.000Z', '2025-02-03T04:05:06.000Z')", []).unwrap();
    connection.execute("INSERT INTO sources VALUES (13, 'Legacy output', 'Reported verification', 'Revision old123; cargo test passed.', NULL, 'when_relevant', NULL, NULL, NULL, 3, '2025-01-02T03:04:05.000Z', '2025-02-03T04:05:06.000Z')", []).unwrap();
    connection.execute_batch("INSERT INTO decisions VALUES (3, 'Original choice', 'Original rationale', NULL, '2025-01-02T03:04:05.000Z'); INSERT INTO decisions VALUES (6, 'Replacement choice', 'Replacement rationale', 3, '2025-02-03T04:05:06.000Z'); INSERT INTO task_sources VALUES (7, 5, 'context'), (7, 5, 'output'), (7, 13, 'output'), (12, 13, 'output'); INSERT INTO task_dependencies VALUES (7, 12, 'before_start'), (7, 19, 'before_done');").unwrap();
}

fn migrate(source: &Path, destination: &Path, name: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dictum-mcp"))
        .args(["migrate", "--source"])
        .arg(source)
        .arg("--destination")
        .arg(destination)
        .arg("--name")
        .arg(name)
        .args(["--description", "Imported durable project"])
        .output()
        .unwrap()
}

fn migrate_shared(source: &Path, destination: &Path, additional: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dictum-mcp"))
        .args(["migrate", "--source"])
        .arg(source)
        .arg("--destination")
        .arg(destination)
        .args(additional)
        .output()
        .unwrap()
}

fn shared_legacy(path: &Path) {
    let seed_directory = tempfile::tempdir().unwrap();
    let seed = seed_directory.path().join("seed.sqlite3");
    legacy(&seed);
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(include_str!("fixtures/schema_v3.sql"))
        .unwrap();
    connection
        .execute_batch("PRAGMA application_id = 1145652052; PRAGMA user_version = 3; PRAGMA foreign_keys = ON;")
        .unwrap();
    connection
        .execute("ATTACH DATABASE ?1 AS seed", [seed.to_str().unwrap()])
        .unwrap();
    for (index, project_id) in PROJECT_IDS.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO projects VALUES (?1, 'Duplicate project name', ?2, ?3)",
                params![
                    project_id,
                    format!("Preserved project {index} description"),
                    format!("2025-01-0{}T03:04:05.000Z", index + 1)
                ],
            )
            .unwrap();
        if index == 2 {
            continue; // Empty projects must survive too.
        }
        let offset = index as i64 * 100;
        for statement in [
            "INSERT INTO tasks SELECT id + ?2, ?1, title, problem_or_goal, design, acceptance_criteria, alternatives_rejected, status, progress, result, revision, created_at, updated_at FROM seed.tasks",
            "INSERT INTO sources SELECT id + ?2, ?1, name, description, content, locator, context_policy, action, snapshot, latest_attempt, revision, created_at, updated_at FROM seed.sources",
            "INSERT INTO decisions SELECT id + ?2, ?1, statement, rationale, supersedes_id + ?2, created_at FROM seed.decisions",
            "INSERT INTO task_sources SELECT ?1, task_id + ?2, source_id + ?2, role FROM seed.task_sources",
            "INSERT INTO task_dependencies SELECT ?1, task_id + ?2, depends_on_task_id + ?2, kind FROM seed.task_dependencies",
        ] {
            connection
                .execute(statement, params![project_id, offset])
                .unwrap();
        }
    }
}

fn data(path: &Path) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    preserved_data(path, false)
}

fn preserved_data(path: &Path, scoped: bool) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    let connection = Connection::open(path).unwrap();
    [
        "projects",
        "tasks",
        "sources",
        "decisions",
        "task_sources",
        "task_dependencies",
    ]
    .iter()
    .filter(|table| scoped || **table != "projects")
    .map(|table| {
        let mut columns = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let columns = columns
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .into_iter()
            .filter(|column| scoped || column != "project_id")
            .filter(|column| {
                *table != "tasks" || !matches!(column.as_str(), "kind" | "trigger_event")
            })
            .collect::<Vec<_>>();
        let mut statement = connection
            .prepare(&format!(
                "SELECT {} FROM {table} ORDER BY rowid",
                columns.join(",")
            ))
            .unwrap();
        statement
            .query_map([], |row| {
                (0..columns.len()).map(|column| row.get(column)).collect()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
    .collect()
}

fn assert_v4_defaults(connection: &Connection) {
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        connection.query_row(
            "SELECT count(*) FROM tasks WHERE kind IS NULL OR kind != 'one_time' OR trigger_event IS NOT NULL",
            [],
            |row| row.get::<_, i64>(0),
        ).unwrap(),
        0,
    );
    for table in ["task_results", "task_result_sources"] {
        assert_eq!(
            connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "migration fabricated history in {table}",
        );
    }
}

#[test]
fn legacy_migration_preserves_every_record_and_supports_scoped_mcp_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("legacy.sqlite3");
    let destination = directory.path().join("new.sqlite3");
    legacy(&source);
    let original_bytes = std::fs::read(&source).unwrap();
    let original_data = data(&source);
    assert!(
        Project::open(&source).is_err(),
        "legacy files need an explicit migration"
    );
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
    let output = migrate(&source, &destination, "Imported project");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let project_id = report["project"]["id"].as_str().unwrap();
    assert_eq!(&project_id[14..15], "7");
    assert!(matches!(&project_id[19..20], "8" | "9" | "a" | "b"));
    assert_eq!(report["project"]["name"], "Imported project");
    assert_eq!(report["project"]["description"], "Imported durable project");
    assert_eq!(report["projects"], json!([report["project"]]));
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
    assert_eq!(data(&source), original_data);
    assert_eq!(data(&destination), original_data);
    let connection = Connection::open(&destination).unwrap();
    assert_v4_defaults(&connection);
    assert_eq!(
        connection
            .query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0x4449_4354
    );
    let mut foreign_keys = connection.prepare("PRAGMA foreign_key_check").unwrap();
    assert!(!foreign_keys.exists([]).unwrap());
    for table in ["tasks", "sources", "decisions"] {
        let other_project_rows: i64 = connection
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE project_id != ?1"),
                [project_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(other_project_rows, 0);
    }
    drop(foreign_keys);
    drop(connection);
    let client = Project::open(&destination).unwrap();
    assert_eq!(client.project_id, project_id);
    let context: Value = client
        .raw_call(
            "get_task_context",
            json!({"project_id": project_id, "task_id": 7}),
        )
        .unwrap();
    assert_eq!(context["task"]["revision"], 8);
    assert_eq!(context["task"]["progress"], "Preserved progress");
    assert_eq!(context["task"]["context_source_ids"], json!([5]));
    assert_eq!(context["task"]["output_source_ids"], json!([5, 13]));
    assert_eq!(context["dependency_state"]["ready_to_start"], false);
    assert_eq!(
        context["dependency_state"]["unmet_prerequisite_task_ids"],
        json!([])
    );
    assert_eq!(
        context["dependency_state"]["unmet_completion_task_ids"],
        json!([19])
    );
    assert_eq!(context["current_decisions"][0]["id"], 6);
    assert_eq!(context["sources"].as_array().unwrap().len(), 2);
    let external = context["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["id"] == 5)
        .unwrap();
    assert_eq!(external["snapshot"]["checkpoint"], "opaque:unchanged");
    assert_eq!(
        external["latest_attempt"]["error"],
        "Preserved last failure"
    );
    assert_eq!(external["action_due"], true);
    let updated: Value = client.raw_call("update_task", json!({"project_id": project_id, "task_id": 7, "expected_revision": 8, "progress": "Continued after migration"})).unwrap();
    assert_eq!(updated["revision"], 9);
    drop(client);
    let restarted = Project::open(&destination).unwrap();
    let persisted: Value = restarted
        .raw_call(
            "get_task_context",
            json!({"project_id": project_id, "task_id": 7}),
        )
        .unwrap();
    assert_eq!(persisted["task"], updated);
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
}

#[test]
fn shared_migration_preserves_all_projects_values_and_scoped_mcp_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("shared v3?#完整.sqlite3");
    let destination = directory.path().join("v4.sqlite3");
    shared_legacy(&source);
    let original_bytes = std::fs::read(&source).unwrap();
    let original_data = preserved_data(&source, true);
    assert!(
        Project::open(&source).is_err(),
        "v3 needs explicit migration"
    );
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
    let output = migrate_shared(&source, &destination, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report.get("project").is_none());
    let project_ids: Vec<_> = report["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|project| project["id"].as_str().unwrap())
        .collect();
    assert_eq!(project_ids, PROJECT_IDS);
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
    assert_eq!(preserved_data(&destination, true), original_data);
    let connection = Connection::open(&destination).unwrap();
    assert_v4_defaults(&connection);
    assert!(
        !connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
    drop(connection);

    let client = Project::open(&destination).unwrap();
    let projects: Value = client.raw_call("list_projects", json!({})).unwrap();
    assert_eq!(projects["projects"], report["projects"]);
    for (index, project_id) in PROJECT_IDS[..2].iter().enumerate() {
        let offset = index as i64 * 100;
        let context: Value = client
            .raw_call(
                "get_task_context",
                json!({"project_id": project_id, "task_id": 7 + offset}),
            )
            .unwrap();
        assert_eq!(context["task"]["kind"], "one_time");
        assert!(context["task"]["trigger_event"].is_null());
        assert_eq!(context["task"]["revision"], 8);
        assert_eq!(context["task"]["context_source_ids"], json!([5 + offset]));
        assert_eq!(
            context["task"]["output_source_ids"],
            json!([5 + offset, 13 + offset])
        );
        assert_eq!(context["current_decisions"][0]["id"], 6 + offset);
        assert_eq!(context["sources"].as_array().unwrap().len(), 2);
        assert!(
            context["sources"]
                .as_array()
                .unwrap()
                .iter()
                .all(|source| source["project_id"] == *project_id)
        );
        assert_eq!(
            context["dependency_state"]["unmet_completion_task_ids"],
            json!([19 + offset])
        );
        let done: Value = client
            .raw_call(
                "get_task_context",
                json!({"project_id": project_id, "task_id": 12 + offset}),
            )
            .unwrap();
        assert_eq!(done["task"]["status"], "done");
        assert_eq!(
            done["task"]["result"],
            "Verified revision old123; fixture evidence."
        );
        assert!(
            client
                .raw_call::<Value>(
                    "get_task_context",
                    json!({"project_id": PROJECT_IDS[2], "task_id": 7 + offset})
                )
                .is_err()
        );
    }
    let updated: Value = client.raw_call("update_task", json!({"project_id": PROJECT_IDS[1], "task_id": 107, "expected_revision": 8, "progress": "Continued v4 work"})).unwrap();
    assert_eq!(updated["revision"], 9);
    drop(client);
    let restarted = Project::open(&destination).unwrap();
    let persisted: Value = restarted
        .raw_call(
            "get_task_context",
            json!({"project_id": PROJECT_IDS[1], "task_id": 107}),
        )
        .unwrap();
    assert_eq!(persisted["task"], updated);
    let original_project: Value = restarted
        .raw_call(
            "get_task_context",
            json!({"project_id": PROJECT_IDS[0], "task_id": 7}),
        )
        .unwrap();
    assert_eq!(original_project["task"]["revision"], 8);
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
}

#[test]
fn shared_migration_rejects_renaming_and_existing_destinations_then_retries() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("v3.sqlite3");
    let destination = directory.path().join("v4.sqlite3");
    shared_legacy(&source);
    let original_bytes = std::fs::read(&source).unwrap();
    for additional in [
        vec!["--name", "Renamed"],
        vec!["--description", "Changed"],
        vec!["--name", "Renamed", "--description", "Changed"],
    ] {
        let output = migrate_shared(&source, &destination, &additional);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("preserves existing projects"));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
    }
    assert!(!migrate_shared(&source, &source, &[]).status.success());
    std::fs::write(&destination, b"preserve existing target").unwrap();
    assert!(!migrate_shared(&source, &destination, &[]).status.success());
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"preserve existing target"
    );
    std::fs::remove_file(&destination).unwrap();
    assert!(migrate_shared(&source, &destination, &[]).status.success());
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
    assert!(Project::open(&destination).is_ok());
}

#[test]
fn unsupported_or_inconsistent_shared_sources_fail_without_partial_import() {
    let directory = tempfile::tempdir().unwrap();
    for case in [
        "cross_project_source",
        "cross_project_dependency",
        "cross_project_decision",
        "orphan_project",
        "missing_table",
        "extra_column",
        "unrelated",
        "already_v4",
    ] {
        let source = directory.path().join(format!("{case}.sqlite3"));
        let destination = directory.path().join(format!("{case}-new.sqlite3"));
        shared_legacy(&source);
        let connection = Connection::open(&source).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        connection.execute_batch(match case {
            "cross_project_source" => "UPDATE task_sources SET source_id = 105 WHERE task_id = 7 AND source_id = 5;",
            "cross_project_dependency" => "UPDATE task_dependencies SET depends_on_task_id = 112 WHERE task_id = 7 AND depends_on_task_id = 12;",
            "cross_project_decision" => "UPDATE decisions SET supersedes_id = 106 WHERE id = 6;",
            "orphan_project" => "DELETE FROM projects WHERE id = '01961380-4000-7000-8000-000000000001';",
            "missing_table" => "DROP TABLE projects;",
            "extra_column" => "ALTER TABLE tasks ADD COLUMN unrecognized TEXT;",
            "unrelated" => "PRAGMA application_id = 123;",
            "already_v4" => "PRAGMA user_version = 4;",
            _ => unreachable!(),
        }).unwrap();
        drop(connection);
        let bytes = std::fs::read(&source).unwrap();
        let output = migrate_shared(&source, &destination, &[]);
        assert!(!output.status.success(), "accepted {case}");
        assert!(!output.stderr.is_empty());
        assert!(!destination.exists(), "left a partial import for {case}");
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
    }
}

#[test]
fn v2_migration_still_requires_explicit_project_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("v2.sqlite3");
    let destination = directory.path().join("v4.sqlite3");
    legacy(&source);
    let bytes = std::fs::read(&source).unwrap();
    for additional in [
        vec![],
        vec!["--name", "Import"],
        vec!["--description", "Import"],
    ] {
        let output = migrate_shared(&source, &destination, &additional);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("version 2 migration requires"));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
    }
}

#[test]
fn migration_failures_preserve_inputs_destinations_and_allow_safe_retry() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("legacy.sqlite3");
    legacy(&source);
    let bytes = std::fs::read(&source).unwrap();
    let destination = directory.path().join("new.sqlite3");
    for (target, name) in [(&source, "Import"), (&destination, " ")] {
        let output = migrate(&source, target, name);
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
    }
    assert!(!destination.exists());
    std::fs::write(&destination, b"do not replace").unwrap();
    assert!(!migrate(&source, &destination, "Import").status.success());
    assert_eq!(std::fs::read(&destination).unwrap(), b"do not replace");
    std::fs::remove_file(&destination).unwrap();
    std::fs::write(&destination, b"").unwrap();
    assert!(!migrate(&source, &destination, "Import").status.success());
    assert!(std::fs::read(&destination).unwrap().is_empty());
    std::fs::remove_file(&destination).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert!(
        migrate(&source, &destination, "Retried import")
            .status
            .success()
    );
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
}

#[test]
fn unsupported_or_inconsistent_legacy_sources_fail_without_partial_import() {
    let directory = tempfile::tempdir().unwrap();
    for case in [
        "older",
        "newer",
        "unrelated",
        "broken_link",
        "missing_table",
    ] {
        let source = directory.path().join(format!("{case}.sqlite3"));
        let destination = directory.path().join(format!("{case}-new.sqlite3"));
        legacy(&source);
        let connection = Connection::open(&source).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        connection
            .execute_batch(match case {
                "older" => "PRAGMA user_version = 1;",
                "newer" => "PRAGMA user_version = 99;",
                "unrelated" => "PRAGMA application_id = 123;",
                "broken_link" => "INSERT INTO task_sources VALUES (7, 99999, 'context');",
                "missing_table" => "DROP TABLE task_dependencies;",
                _ => unreachable!(),
            })
            .unwrap();
        drop(connection);
        let bytes = std::fs::read(&source).unwrap();
        let output = migrate(&source, &destination, "Import");
        assert!(!output.status.success(), "accepted {case}");
        assert!(!output.stderr.is_empty());
        assert!(!destination.exists(), "left a partial import for {case}");
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
    }
    let missing = directory.path().join("missing.sqlite3");
    let destination = directory.path().join("missing-new.sqlite3");
    assert!(!migrate(&missing, &destination, "Import").status.success());
    assert!(!missing.exists());
    assert!(!destination.exists());
}
