//! Explicit, non-destructive imports of supported v2 and v3 databases.

use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::{
    model::Project,
    store::{Store, new_project_id},
};

const APPLICATION_ID: i64 = 0x4449_4354;
const PROJECT_COLUMNS: &[&str] = &["id", "name", "description", "created_at"];
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
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MigrationReport {
    pub source: PathBuf,
    pub destination: PathBuf,
    /// Retained for callers of the v2 single-project migration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<Project>,
    pub projects: Vec<Project>,
}

/// Copy every supported value and relationship, preserving all existing projects.
/// Only a completely imported, checked, closed database is published at the destination.
pub fn migrate(options: MigrationOptions) -> Result<MigrationReport> {
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
    Store::open(temporary.path()).context("cannot initialize migration database")?;
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
    let (project, projects) = {
        let transaction = connection.transaction()?;
        let version = validate_source(&transaction)?;
        // Retain all IDs, including a decision whose predecessor was inserted later.
        transaction.pragma_update(None, "defer_foreign_keys", "ON")?;
        let project = if version == 2 {
            let name = options
                .name
                .context("version 2 migration requires --name")?;
            let description = options
                .description
                .context("version 2 migration requires --description")?;
            ensure!(!name.trim().is_empty(), "project name must not be blank");
            ensure!(
                !description.trim().is_empty(),
                "project description must not be blank"
            );
            let project = Project {
                id: new_project_id(&transaction)?,
                name,
                description,
                created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            };
            transaction.execute(
                "INSERT INTO main.projects (id, name, description, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![project.id, project.name, project.description, project.created_at],
            )?;
            for (table, columns) in LEGACY_COLUMNS {
                let columns = columns.join(", ");
                let sql = format!(
                    "INSERT INTO main.{table} (project_id, {columns}) SELECT ?1, {columns} FROM legacy.{table}"
                );
                transaction
                    .execute(&sql, params![project.id])
                    .with_context(|| format!("cannot preserve legacy {table}"))?;
            }
            Some(project)
        } else {
            ensure!(
                options.name.is_none() && options.description.is_none(),
                "version 3 migration preserves existing projects; omit --name and --description"
            );
            for (table, columns) in source_columns(version) {
                let columns = columns.join(", ");
                let sql = format!(
                    "INSERT INTO main.{table} ({columns}) SELECT {columns} FROM legacy.{table}"
                );
                transaction
                    .execute(&sql, [])
                    .with_context(|| format!("cannot preserve legacy {table}"))?;
            }
            None
        };
        let violations = transaction
            .prepare("PRAGMA main.foreign_key_check")?
            .exists([])?;
        ensure!(
            !violations,
            "legacy relationships violate the shared database constraints"
        );
        let projects = transaction
            .prepare("SELECT id, name, description, created_at FROM main.projects ORDER BY created_at, id")?
            .query_map([], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        transaction
            .commit()
            .context("cannot commit migrated records")?;
        (project, projects)
    };
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
        projects,
    })
}

fn source_columns(version: i64) -> Vec<(&'static str, Vec<&'static str>)> {
    let mut columns = Vec::new();
    if version == 3 {
        columns.push(("projects", PROJECT_COLUMNS.to_vec()));
    }
    for (table, legacy) in LEGACY_COLUMNS {
        let mut fields = legacy.to_vec();
        if version == 3 {
            let position = usize::from(fields[0] == "id");
            fields.insert(position, "project_id");
        }
        columns.push((*table, fields));
    }
    columns
}

fn validate_source(connection: &Connection) -> Result<i64> {
    let version: i64 =
        connection.pragma_query_value(Some("legacy"), "user_version", |row| row.get(0))?;
    let application_id: i64 =
        connection.pragma_query_value(Some("legacy"), "application_id", |row| row.get(0))?;
    ensure!(
        matches!(version, 2 | 3) && application_id == APPLICATION_ID,
        "unsupported migration source schema (version {version}, application ID {application_id}); preserve the source and use a supported version 2 or 3 database"
    );
    let mut tables = connection.prepare("SELECT name FROM legacy.sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
    let actual = tables
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let columns = source_columns(version);
    let mut expected: Vec<_> = columns.iter().map(|(table, _)| *table).collect();
    expected.sort_unstable();
    ensure!(
        actual == expected,
        "unsupported migration source tables; preserve the source"
    );
    for (table, expected) in columns {
        let mut columns = connection.prepare(&format!("PRAGMA legacy.table_info({table})"))?;
        let actual = columns
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            actual == expected,
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
    ensure!(
        !connection
            .prepare("PRAGMA legacy.foreign_key_check")?
            .exists([])?,
        "migration source has invalid relationships; preserve the source"
    );
    Ok(version)
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
