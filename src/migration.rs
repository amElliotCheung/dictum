//! Explicit, non-destructive import of the supported single-project database.

use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::{
    model::{CreateProject, Project},
    store::Store,
};

const LEGACY_VERSION: i64 = 2;
const APPLICATION_ID: i64 = 0x4449_4354;
const LEGACY_COLUMNS: &[(&str, &[&str])] = &[
    (
        "tasks",
        &[
            "id",
            "title",
            "problem_or_goal",
            "design",
            "acceptance_criteria",
            "alternatives_rejected",
            "status",
            "progress",
            "result",
            "revision",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "sources",
        &[
            "id",
            "name",
            "description",
            "content",
            "locator",
            "context_policy",
            "action",
            "snapshot",
            "latest_attempt",
            "revision",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "decisions",
        &[
            "id",
            "statement",
            "rationale",
            "supersedes_id",
            "created_at",
        ],
    ),
    ("task_sources", &["task_id", "source_id", "role"]),
    (
        "task_dependencies",
        &["task_id", "depends_on_task_id", "kind"],
    ),
];

pub struct MigrationOptions {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct MigrationReport {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub project: Project,
}

/// Copy every supported legacy value and relationship into one explicitly named project.
/// Only a completely imported, checked, closed database is published at the destination.
pub fn migrate(options: MigrationOptions) -> Result<MigrationReport> {
    ensure!(
        !options.name.trim().is_empty(),
        "project name must not be blank"
    );
    ensure!(
        !options.description.trim().is_empty(),
        "project description must not be blank"
    );
    let source = fs::canonicalize(&options.source)
        .with_context(|| format!("cannot read migration source {}", options.source.display()))?;
    ensure!(source.is_file(), "migration source must be a database file");
    ensure!(
        !options.destination.as_os_str().is_empty(),
        "migration destination must identify a new file"
    );
    let destination = if options.destination.is_absolute() {
        options.destination
    } else {
        std::env::current_dir()?.join(options.destination)
    };
    match fs::symlink_metadata(&destination) {
        Ok(_) => bail!("migration destination already exists; choose a new path"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("cannot inspect migration destination"),
    }
    let parent = destination
        .parent()
        .context("migration destination has no parent directory")?;
    fs::create_dir_all(parent).context("cannot create migration destination directory")?;
    let temporary = tempfile::Builder::new()
        .prefix(".dictum-migration-")
        .tempfile_in(parent)
        .context("cannot create temporary migration database")?;
    let store = Store::open(temporary.path()).context("cannot initialize migration database")?;
    let project = store.create_project(CreateProject {
        name: options.name,
        description: options.description,
    })?;
    let mut connection = Connection::open(temporary.path())?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    // mode=ro also applies to ATTACH: the legacy database is never opened for writes.
    let source_uri = readonly_uri(&source);
    connection.execute("ATTACH DATABASE ?1 AS legacy", [&source_uri])?;
    ensure!(
        connection.is_readonly("legacy")?,
        "migration source was not opened read-only"
    );
    {
        let transaction = connection.transaction()?;
        validate_legacy(&transaction)?;
        // Retain all IDs, including a decision whose predecessor was inserted later.
        transaction.pragma_update(None, "defer_foreign_keys", "ON")?;
        for (table, columns) in LEGACY_COLUMNS {
            let columns = columns.join(", ");
            let sql = format!(
                "INSERT INTO main.{table} (project_id, {columns}) SELECT ?1, {columns} FROM legacy.{table}"
            );
            transaction
                .execute(&sql, params![project.id])
                .with_context(|| format!("cannot preserve legacy {table}"))?;
        }
        let violations = transaction
            .prepare("PRAGMA main.foreign_key_check")?
            .exists([])?;
        ensure!(
            !violations,
            "legacy relationships violate the shared database constraints"
        );
        transaction
            .commit()
            .context("cannot commit migrated records")?;
    }
    connection.execute_batch("DETACH DATABASE legacy")?;
    // Store initialization uses WAL. Fold every page into the final standalone file.
    let mode: String =
        connection.query_row("PRAGMA main.journal_mode = DELETE", [], |row| row.get(0))?;
    ensure!(
        mode.eq_ignore_ascii_case("delete"),
        "cannot finalize migration journal"
    );
    connection.close().map_err(|(_, error)| error)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(&destination)
        .map_err(|error| error.error)
        .context("cannot publish migration destination without replacing an existing file")?;
    Ok(MigrationReport {
        source,
        destination,
        project,
    })
}

fn validate_legacy(connection: &Connection) -> Result<()> {
    let version: i64 =
        connection.pragma_query_value(Some("legacy"), "user_version", |row| row.get(0))?;
    let application_id: i64 =
        connection.pragma_query_value(Some("legacy"), "application_id", |row| row.get(0))?;
    ensure!(
        version == LEGACY_VERSION && application_id == APPLICATION_ID,
        "unsupported migration source schema (version {version}, application ID {application_id}); preserve the source and use a supported version 2 database"
    );
    let mut tables = connection.prepare("SELECT name FROM legacy.sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
    let actual = tables
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut expected: Vec<_> = LEGACY_COLUMNS.iter().map(|(table, _)| *table).collect();
    expected.sort_unstable();
    ensure!(
        actual == expected,
        "unsupported migration source tables; preserve the source"
    );
    for (table, expected) in LEGACY_COLUMNS {
        let mut columns = connection.prepare(&format!("PRAGMA legacy.table_info({table})"))?;
        let actual = columns
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            actual == *expected,
            "unsupported migration source columns in {table}; preserve the source"
        );
    }
    let mut integrity = connection.prepare("PRAGMA legacy.quick_check")?;
    let results = integrity
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(
        results == ["ok"],
        "migration source failed integrity checks; preserve the source"
    );
    Ok(())
}

fn readonly_uri(path: &std::path::Path) -> String {
    use std::fmt::Write;
    let mut uri = String::from("file:");
    for byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(byte) {
            uri.push(char::from(*byte));
        } else {
            write!(uri, "%{byte:02X}").expect("writing a string cannot fail");
        }
    }
    uri.push_str("?mode=ro");
    uri
}
