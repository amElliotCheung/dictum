//! Transactional, single-project SQLite storage. The database holds reported work, not a scheduler.

use crate::model::*;
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

const SCHEMA_VERSION: i64 = 2;
const APPLICATION_ID: i64 = 0x4449_4354; // DICT
const SCHEMA: &str = include_str!("schema.sql");

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("stored data error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("{entity} {id} was not found")]
    NotFound { entity: &'static str, id: i64 },
    #[error(
        "{entity} {id} revision conflict: expected {expected}, found {actual}; reread before retrying"
    )]
    RevisionConflict {
        entity: &'static str,
        id: i64,
        expected: i64,
        actual: i64,
    },
    #[error(
        "unsupported database schema (version {version}, application ID {application_id}); use a new database path, preserving the existing file"
    )]
    UnsupportedSchema { version: i64, application_id: i64 },
}
pub type StoreResult<T> = Result<T, StoreError>;

/// Connections are short lived; clones share the same on-disk database, not a connection.
#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() || path == Path::new(":memory:") {
            return invalid("database path must identify an on-disk file");
        }
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let store = Self { path };
        store.initialize()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn initialize(&self) -> StoreResult<()> {
        let mut connection = self.connect()?;
        // Inspect before persistent pragmas or DDL; serialize empty-file initialization too.
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = transaction.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let application_id: i64 =
            transaction.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let objects: i64 = transaction.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        if version == 0 && application_id == 0 && objects == 0 {
            transaction.execute_batch(SCHEMA)?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if version != SCHEMA_VERSION || application_id != APPLICATION_ID {
            return Err(StoreError::UnsupportedSchema {
                version,
                application_id,
            });
        }
        transaction.commit()?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(())
    }

    fn connect(&self) -> StoreResult<Connection> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        Ok(connection)
    }

    pub fn create_task(&self, input: CreateTask) -> StoreResult<Task> {
        validate_brief(
            &input.title,
            &input.problem_or_goal,
            &input.design,
            &input.acceptance_criteria,
        )?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = timestamp(Utc::now());
        transaction.execute(
            "INSERT INTO tasks (title, problem_or_goal, design, acceptance_criteria, alternatives_rejected, status, revision, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 'todo', 1, ?6, ?6)",
            params![input.title, input.problem_or_goal, input.design, encode(&input.acceptance_criteria)?, input.alternatives_rejected, now],
        )?;
        let id = transaction.last_insert_rowid();
        replace_source_links(&transaction, id, "context", &input.context_source_ids)?;
        replace_dependencies(
            &transaction,
            id,
            "before_start",
            &input.prerequisite_task_ids,
        )?;
        replace_dependencies(&transaction, id, "before_done", &input.completion_task_ids)?;
        reject_cycle(&transaction, id)?;
        let task = task_by_id(&transaction, id)?;
        transaction.commit()?;
        Ok(task)
    }

    pub fn list_tasks(&self, input: ListTasks) -> StoreResult<Vec<TaskSummary>> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let ids = all_ids(&transaction, "SELECT id FROM tasks ORDER BY id")?;
        let query = input.query.as_deref().map(str::to_lowercase);
        let mut result = Vec::new();
        for id in ids {
            let task = task_by_id(&transaction, id)?;
            if input.status.is_some_and(|status| status != task.status) {
                continue;
            }
            if query.as_ref().is_some_and(|query| {
                ![
                    task.title.as_str(),
                    task.problem_or_goal.as_str(),
                    task.design.as_str(),
                ]
                .iter()
                .any(|value| value.to_lowercase().contains(query))
                    && !task
                        .acceptance_criteria
                        .iter()
                        .any(|value| value.to_lowercase().contains(query))
            }) {
                continue;
            }
            let dependency_state = dependency_state(&transaction, &task)?;
            if input
                .ready_to_start
                .is_some_and(|ready| ready != dependency_state.ready_to_start)
            {
                continue;
            }
            result.push(TaskSummary {
                id: task.id,
                title: task.title,
                status: task.status,
                progress: task.progress,
                dependency_state,
                revision: task.revision,
                updated_at: task.updated_at,
            });
        }
        transaction.commit()?;
        Ok(result)
    }

    pub fn get_task_context(&self, task_id: i64) -> StoreResult<TaskContext> {
        let mut connection = self.connect()?;
        // All components belong to the same read snapshot across concurrent writers.
        let transaction = connection.transaction()?;
        let task = task_by_id(&transaction, task_id)?;
        let dependency_state = dependency_state(&transaction, &task)?;
        let now = Utc::now();
        let source_ids = {
            let mut statement = transaction.prepare("SELECT id FROM sources WHERE context_policy = 'always' OR id IN (SELECT source_id FROM task_sources WHERE task_id = ?1 AND role = 'context') ORDER BY id")?;
            statement
                .query_map([task_id], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let sources = source_ids
            .into_iter()
            .map(|id| source_by_id(&transaction, id, now))
            .collect::<StoreResult<Vec<_>>>()?;
        let outputs = task
            .output_source_ids
            .iter()
            .map(|&id| source_by_id(&transaction, id, now).map(|source| descriptor(&source)))
            .collect::<StoreResult<Vec<_>>>()?;
        let current_decisions = decisions(&transaction, false)?;
        let result = TaskContext {
            task,
            dependency_state,
            current_decisions,
            sources,
            outputs,
        };
        transaction.commit()?;
        Ok(result)
    }

    pub fn update_task(&self, input: UpdateTask) -> StoreResult<Task> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut task = task_by_id(&transaction, input.task_id)?;
        check_revision("task", task.id, input.expected_revision, task.revision)?;
        let previous_status = task.status;
        if let Some(value) = input.title {
            task.title = value;
        }
        if let Some(value) = input.problem_or_goal {
            task.problem_or_goal = value;
        }
        if let Some(value) = input.design {
            task.design = value;
        }
        if let Some(value) = input.acceptance_criteria {
            task.acceptance_criteria = value;
        }
        if let Some(value) = input.alternatives_rejected {
            task.alternatives_rejected = value;
        }
        if let Some(value) = input.status {
            task.status = value;
        }
        if let Some(value) = input.progress {
            task.progress = value;
        }
        if let Some(value) = input.result {
            task.result = value;
        }
        validate_brief(
            &task.title,
            &task.problem_or_goal,
            &task.design,
            &task.acceptance_criteria,
        )?;
        if let Some(ids) = input.context_source_ids {
            replace_source_links(&transaction, task.id, "context", &ids)?;
        }
        if let Some(ids) = input.output_source_ids {
            replace_source_links(&transaction, task.id, "output", &ids)?;
        }
        if let Some(ids) = input.prerequisite_task_ids {
            replace_dependencies(&transaction, task.id, "before_start", &ids)?;
        }
        if let Some(ids) = input.completion_task_ids {
            replace_dependencies(&transaction, task.id, "before_done", &ids)?;
        }
        reject_cycle(&transaction, task.id)?;
        let state = dependency_state(&transaction, &task)?;
        if task.status == TaskStatus::InProgress
            && previous_status != TaskStatus::InProgress
            && !state.unmet_prerequisite_task_ids.is_empty()
        {
            return invalid(format!(
                "cannot start task while prerequisite tasks are unfinished: {:?}",
                state.unmet_prerequisite_task_ids
            ));
        }
        if task.status == TaskStatus::Done {
            required(
                task.result.as_deref().unwrap_or_default(),
                "done task result/verification evidence",
            )?;
            if !state.unmet_prerequisite_task_ids.is_empty()
                || !state.unmet_completion_task_ids.is_empty()
            {
                return invalid(format!(
                    "cannot finish task; unmet start dependencies: {:?}; unmet completion dependencies: {:?}",
                    state.unmet_prerequisite_task_ids, state.unmet_completion_task_ids
                ));
            }
        }
        transaction.execute(
            "UPDATE tasks SET title=?1, problem_or_goal=?2, design=?3, acceptance_criteria=?4, alternatives_rejected=?5, status=?6, progress=?7, result=?8, revision=revision+1, updated_at=?9 WHERE id=?10",
            params![task.title, task.problem_or_goal, task.design, encode(&task.acceptance_criteria)?, task.alternatives_rejected, status_name(task.status), task.progress, task.result, timestamp(Utc::now()), task.id],
        )?;
        let result = task_by_id(&transaction, task.id)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn create_source(&self, input: CreateSource) -> StoreResult<Source> {
        validate_source(
            &input.name,
            &input.description,
            &input.content,
            &input.locator,
            &input.action,
        )?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now();
        transaction.execute(
            "INSERT INTO sources (name, description, content, locator, context_policy, action, revision, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)",
            params![input.name, input.description, input.content, input.locator, policy_name(input.context_policy), encode_optional(&input.action)?, timestamp(now)],
        )?;
        let result = source_by_id(&transaction, transaction.last_insert_rowid(), now)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn list_sources(&self, input: ListSources) -> StoreResult<Vec<SourceDescriptor>> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let linked = if let Some(task_id) = input.task_id {
            ensure_exists(&transaction, "task", task_id)?;
            let mut statement = transaction
                .prepare("SELECT DISTINCT source_id FROM task_sources WHERE task_id=?1")?;
            Some(
                statement
                    .query_map([task_id], |row| row.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<BTreeSet<_>>>()?,
            )
        } else {
            None
        };
        let ids = all_ids(&transaction, "SELECT id FROM sources ORDER BY id")?;
        let now = Utc::now();
        let query = input.query.as_deref().map(str::to_lowercase);
        let mut result = Vec::new();
        for id in ids {
            if linked.as_ref().is_some_and(|ids| !ids.contains(&id)) {
                continue;
            }
            let source = source_by_id(&transaction, id, now)?;
            let item = descriptor(&source);
            if input
                .representation
                .is_some_and(|representation| representation != item.representation)
                || input.action_due.is_some_and(|due| due != item.action_due)
                || query.as_ref().is_some_and(|query| {
                    ![
                        source.name.as_str(),
                        source.description.as_str(),
                        source.locator.as_deref().unwrap_or_default(),
                    ]
                    .iter()
                    .any(|value| value.to_lowercase().contains(query))
                })
            {
                continue;
            }
            result.push(item);
        }
        transaction.commit()?;
        Ok(result)
    }

    pub fn get_source(&self, source_id: i64) -> StoreResult<Source> {
        source_by_id(&self.connect()?, source_id, Utc::now())
    }

    pub fn update_source(&self, input: UpdateSource) -> StoreResult<Source> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now();
        let mut source = source_by_id(&transaction, input.source_id, now)?;
        check_revision(
            "source",
            source.id,
            input.expected_revision,
            source.revision,
        )?;
        let previous_content = source.content.clone();
        let previous_locator = source.locator.clone();
        let previous_action = source.action.clone();
        if let Some(value) = input.name {
            source.name = value;
        }
        if let Some(value) = input.description {
            source.description = value;
        }
        if let Some(value) = input.content {
            source.content = value;
        }
        if let Some(value) = input.locator {
            source.locator = value;
        }
        if let Some(value) = input.context_policy {
            source.context_policy = value;
        }
        if let Some(value) = input.action {
            source.action = value;
        }
        validate_source(
            &source.name,
            &source.description,
            &source.content,
            &source.locator,
            &source.action,
        )?;
        if source.content != previous_content
            || source.locator != previous_locator
            || source.action != previous_action
        {
            source.snapshot = None;
            source.latest_attempt = None;
        }
        transaction.execute(
            "UPDATE sources SET name=?1, description=?2, content=?3, locator=?4, context_policy=?5, action=?6, snapshot=?7, latest_attempt=?8, revision=revision+1, updated_at=?9 WHERE id=?10",
            params![source.name, source.description, source.content, source.locator, policy_name(source.context_policy), encode_optional(&source.action)?, encode_optional(&source.snapshot)?, encode_optional(&source.latest_attempt)?, timestamp(now), source.id],
        )?;
        let result = source_by_id(&transaction, source.id, now)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn record_source_action(&self, input: RecordSourceAction) -> StoreResult<Source> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now();
        let mut source = source_by_id(&transaction, input.source_id, now)?;
        check_revision(
            "source",
            source.id,
            input.expected_revision,
            source.revision,
        )?;
        if source.action.is_none() {
            return invalid("source has no configured action to record");
        }
        let (outcome, error) = match input.result {
            SourceActionResult::Success {
                observed_at,
                summary,
                checkpoint,
                references,
            } => {
                required(&summary, "summary")?;
                if summary.chars().count() > 1_000 {
                    return invalid("summary must not exceed 1,000 characters");
                }
                for reference in &references {
                    required(reference, "reference")?;
                }
                let observed_at = observation_time(&observed_at, source.snapshot.as_ref(), now)?;
                source.snapshot = Some(SourceSnapshot {
                    observed_at,
                    summary,
                    checkpoint,
                    references,
                });
                (ActionOutcome::Success, None)
            }
            SourceActionResult::NoChange {
                observed_at,
                checkpoint,
            } => {
                let observed_at = observation_time(&observed_at, source.snapshot.as_ref(), now)?;
                let Some(snapshot) = source.snapshot.as_mut() else {
                    return invalid(
                        "no_change requires an existing successful snapshot; record success first",
                    );
                };
                snapshot.observed_at = observed_at;
                if let Some(value) = checkpoint {
                    snapshot.checkpoint = value;
                }
                (ActionOutcome::NoChange, None)
            }
            SourceActionResult::Failure { error } => {
                required(&error, "error")?;
                (ActionOutcome::Failure, Some(error))
            }
        };
        let latest_attempt = SourceAttempt {
            recorded_at: timestamp(now),
            outcome,
            error,
        };
        transaction.execute(
            "UPDATE sources SET snapshot=?1, latest_attempt=?2, revision=revision+1, updated_at=?3 WHERE id=?4",
            params![encode_optional(&source.snapshot)?, encode(&latest_attempt)?, timestamp(now), source.id],
        )?;
        let result = source_by_id(&transaction, source.id, now)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn record_decision(&self, input: RecordDecision) -> StoreResult<Decision> {
        required(&input.statement, "statement")?;
        required(&input.rationale, "rationale")?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(id) = input.supersedes_id {
            ensure_exists(&transaction, "decision", id)?;
            let superseded: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM decisions WHERE supersedes_id=?1)",
                [id],
                |row| row.get(0),
            )?;
            if superseded {
                return invalid(format!(
                    "decision {id} is already superseded; replace its current successor"
                ));
            }
        }
        transaction.execute("INSERT INTO decisions (statement, rationale, supersedes_id, created_at) VALUES (?1, ?2, ?3, ?4)", params![input.statement, input.rationale, input.supersedes_id, timestamp(Utc::now())])?;
        let result = transaction.query_row("SELECT id, statement, rationale, supersedes_id, created_at, 1 AS is_current FROM decisions WHERE id=?1", [transaction.last_insert_rowid()], map_decision)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn list_decisions(&self, input: ListDecisions) -> StoreResult<Vec<Decision>> {
        decisions(&self.connect()?, input.include_superseded)
    }
}

fn invalid<T>(message: impl Into<String>) -> StoreResult<T> {
    Err(StoreError::Validation(message.into()))
}

fn required(value: &str, field: &str) -> StoreResult<()> {
    if value.trim().is_empty() {
        invalid(format!("{field} must not be blank"))
    } else {
        Ok(())
    }
}

fn validate_brief(title: &str, goal: &str, design: &str, criteria: &[String]) -> StoreResult<()> {
    required(title, "title")?;
    required(goal, "problem_or_goal")?;
    required(design, "design")?;
    if criteria.is_empty() {
        return invalid("acceptance_criteria must contain at least one condition");
    }
    for criterion in criteria {
        required(criterion, "acceptance criterion")?;
    }
    Ok(())
}

fn validate_source(
    name: &str,
    description: &str,
    content: &Option<String>,
    locator: &Option<String>,
    action: &Option<SourceAction>,
) -> StoreResult<()> {
    required(name, "name")?;
    required(description, "description")?;
    match (content, locator) {
        (Some(_), None) => {} // Empty inline content is still a complete document.
        (None, Some(locator)) => required(locator, "locator")?,
        _ => return invalid("source requires exactly one of content or locator"),
    }
    if let Some(action) = action {
        required(&action.instruction, "action instruction")?;
        if let ActionTrigger::Interval { every_minutes } = action.trigger {
            if every_minutes <= 0 {
                return invalid("every_minutes must be positive");
            }
            let Some(duration) = chrono::Duration::try_minutes(every_minutes) else {
                return invalid("every_minutes exceeds supported timestamp range");
            };
            if Utc::now().checked_add_signed(duration).is_none() {
                return invalid("every_minutes exceeds supported timestamp range");
            }
        }
    }
    Ok(())
}

fn check_revision(entity: &'static str, id: i64, expected: i64, actual: i64) -> StoreResult<()> {
    if expected != actual {
        Err(StoreError::RevisionConflict {
            entity,
            id,
            expected,
            actual,
        })
    } else {
        Ok(())
    }
}

fn ensure_exists(connection: &Connection, entity: &'static str, id: i64) -> StoreResult<()> {
    let sql = match entity {
        "task" => "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1)",
        "source" => "SELECT EXISTS(SELECT 1 FROM sources WHERE id=?1)",
        "decision" => "SELECT EXISTS(SELECT 1 FROM decisions WHERE id=?1)",
        _ => unreachable!("internal entity name"),
    };
    let exists: bool = connection.query_row(sql, [id], |row| row.get(0))?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::NotFound { entity, id })
    }
}

fn replace_source_links(
    connection: &Connection,
    task_id: i64,
    role: &str,
    ids: &[i64],
) -> StoreResult<()> {
    for &id in ids {
        ensure_exists(connection, "source", id)?;
    }
    connection.execute(
        "DELETE FROM task_sources WHERE task_id=?1 AND role=?2",
        params![task_id, role],
    )?;
    for &id in ids.iter().collect::<BTreeSet<_>>() {
        connection.execute(
            "INSERT INTO task_sources (task_id, source_id, role) VALUES (?1, ?2, ?3)",
            params![task_id, id, role],
        )?;
    }
    Ok(())
}

fn replace_dependencies(
    connection: &Connection,
    task_id: i64,
    kind: &str,
    ids: &[i64],
) -> StoreResult<()> {
    for &id in ids {
        if id == task_id {
            return invalid("task cannot depend on itself");
        }
        ensure_exists(connection, "task", id)?;
    }
    connection.execute(
        "DELETE FROM task_dependencies WHERE task_id=?1 AND kind=?2",
        params![task_id, kind],
    )?;
    for &id in ids.iter().collect::<BTreeSet<_>>() {
        connection.execute(
            "INSERT INTO task_dependencies (task_id, depends_on_task_id, kind) VALUES (?1, ?2, ?3)",
            params![task_id, id, kind],
        )?;
    }
    Ok(())
}

fn reject_cycle(connection: &Connection, task_id: i64) -> StoreResult<()> {
    let cycle: bool = connection.query_row(
        "WITH RECURSIVE reachable(id) AS (SELECT depends_on_task_id FROM task_dependencies WHERE task_id=?1 UNION SELECT d.depends_on_task_id FROM task_dependencies d JOIN reachable r ON d.task_id=r.id) SELECT EXISTS(SELECT 1 FROM reachable WHERE id=?1)",
        [task_id], |row| row.get(0),
    )?;
    if cycle {
        invalid("task dependency would create a cycle across before_start/before_done links")
    } else {
        Ok(())
    }
}

fn linked_ids(
    connection: &Connection,
    sql: &str,
    task_id: i64,
    kind: &str,
) -> StoreResult<Vec<i64>> {
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map(params![task_id, kind], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn all_ids(connection: &Connection, sql: &str) -> StoreResult<Vec<i64>> {
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn task_by_id(connection: &Connection, id: i64) -> StoreResult<Task> {
    let mut task = connection
        .query_row("SELECT * FROM tasks WHERE id=?1", [id], |row| {
            Ok(Task {
                id: row.get("id")?,
                title: row.get("title")?,
                problem_or_goal: row.get("problem_or_goal")?,
                design: row.get("design")?,
                acceptance_criteria: json_column(row, "acceptance_criteria")?,
                alternatives_rejected: row.get("alternatives_rejected")?,
                status: enum_column(row, "status")?,
                progress: row.get("progress")?,
                result: row.get("result")?,
                context_source_ids: vec![],
                output_source_ids: vec![],
                prerequisite_task_ids: vec![],
                completion_task_ids: vec![],
                revision: row.get("revision")?,
                created_at: row.get("created_at")?,
                updated_at: row.get("updated_at")?,
            })
        })
        .optional()?
        .ok_or(StoreError::NotFound { entity: "task", id })?;
    let source_sql =
        "SELECT source_id FROM task_sources WHERE task_id=?1 AND role=?2 ORDER BY source_id";
    task.context_source_ids = linked_ids(connection, source_sql, id, "context")?;
    task.output_source_ids = linked_ids(connection, source_sql, id, "output")?;
    let dependency_sql = "SELECT depends_on_task_id FROM task_dependencies WHERE task_id=?1 AND kind=?2 ORDER BY depends_on_task_id";
    task.prerequisite_task_ids = linked_ids(connection, dependency_sql, id, "before_start")?;
    task.completion_task_ids = linked_ids(connection, dependency_sql, id, "before_done")?;
    Ok(task)
}

fn dependency_state(connection: &Connection, task: &Task) -> StoreResult<DependencyState> {
    let sql = "SELECT d.depends_on_task_id FROM task_dependencies d JOIN tasks t ON d.depends_on_task_id=t.id WHERE d.task_id=?1 AND d.kind=?2 AND t.status!='done' ORDER BY d.depends_on_task_id";
    let unmet_prerequisite_task_ids = linked_ids(connection, sql, task.id, "before_start")?;
    let unmet_completion_task_ids = linked_ids(connection, sql, task.id, "before_done")?;
    Ok(DependencyState {
        ready_to_start: task.status == TaskStatus::Todo && unmet_prerequisite_task_ids.is_empty(),
        unmet_prerequisite_task_ids,
        unmet_completion_task_ids,
    })
}

fn source_by_id(connection: &Connection, id: i64, now: DateTime<Utc>) -> StoreResult<Source> {
    let mut source = connection
        .query_row("SELECT * FROM sources WHERE id=?1", [id], |row| {
            Ok(Source {
                id: row.get("id")?,
                name: row.get("name")?,
                description: row.get("description")?,
                content: row.get("content")?,
                locator: row.get("locator")?,
                context_policy: enum_column(row, "context_policy")?,
                action: optional_json_column(row, "action")?,
                snapshot: optional_json_column(row, "snapshot")?,
                latest_attempt: optional_json_column(row, "latest_attempt")?,
                action_due: false,
                next_action_at: None,
                revision: row.get("revision")?,
                created_at: row.get("created_at")?,
                updated_at: row.get("updated_at")?,
            })
        })
        .optional()?
        .ok_or(StoreError::NotFound {
            entity: "source",
            id,
        })?;
    if let Some(SourceAction {
        trigger: ActionTrigger::Interval { every_minutes },
        ..
    }) = &source.action
    {
        if let Some(snapshot) = &source.snapshot {
            let observed = parse_time(&snapshot.observed_at)?;
            let next = chrono::Duration::try_minutes(*every_minutes)
                .and_then(|duration| observed.checked_add_signed(duration))
                .ok_or_else(|| {
                    StoreError::Validation("stored action interval exceeds timestamp range".into())
                })?;
            source.action_due = now >= next;
            source.next_action_at = Some(timestamp(next));
        } else {
            source.action_due = true;
        }
    }
    Ok(source)
}

fn descriptor(source: &Source) -> SourceDescriptor {
    SourceDescriptor {
        id: source.id,
        name: source.name.clone(),
        description: source.description.clone(),
        representation: if source.content.is_some() {
            Representation::Inline
        } else {
            Representation::External
        },
        context_policy: source.context_policy,
        has_action: source.action.is_some(),
        action_due: source.action_due,
        next_action_at: source.next_action_at.clone(),
        revision: source.revision,
    }
}

fn decisions(connection: &Connection, include_superseded: bool) -> StoreResult<Vec<Decision>> {
    let mut statement = connection.prepare("SELECT d.id, d.statement, d.rationale, d.supersedes_id, d.created_at, NOT EXISTS(SELECT 1 FROM decisions successor WHERE successor.supersedes_id=d.id) AS is_current FROM decisions d WHERE ?1 OR NOT EXISTS(SELECT 1 FROM decisions successor WHERE successor.supersedes_id=d.id) ORDER BY d.id")?;
    Ok(statement
        .query_map([include_superseded], map_decision)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn map_decision(row: &Row<'_>) -> rusqlite::Result<Decision> {
    Ok(Decision {
        id: row.get("id")?,
        statement: row.get("statement")?,
        rationale: row.get("rationale")?,
        supersedes_id: row.get("supersedes_id")?,
        created_at: row.get("created_at")?,
        is_current: row.get("is_current")?,
    })
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn parse_time(value: &str) -> StoreResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| {
            StoreError::Validation("observed_at must be a valid RFC 3339 timestamp".into())
        })
}

fn observation_time(
    value: &str,
    previous: Option<&SourceSnapshot>,
    now: DateTime<Utc>,
) -> StoreResult<String> {
    let observed = parse_time(value)?;
    if observed > now {
        return invalid("observed_at must not be in the future");
    }
    if let Some(snapshot) = previous
        && observed < parse_time(&snapshot.observed_at)?
    {
        return invalid("observed_at must not move backwards from the last successful snapshot");
    }
    Ok(timestamp(observed))
}

fn status_name(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Todo => "todo",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Done => "done",
    }
}

fn policy_name(policy: ContextPolicy) -> &'static str {
    match policy {
        ContextPolicy::Always => "always",
        ContextPolicy::WhenRelevant => "when_relevant",
    }
}

fn encode(value: &impl Serialize) -> StoreResult<String> {
    Ok(serde_json::to_string(value)?)
}

fn encode_optional<T: Serialize>(value: &Option<T>) -> StoreResult<Option<String>> {
    value.as_ref().map(encode).transpose()
}

fn decode_column<T: DeserializeOwned>(
    row: &Row<'_>,
    column: &str,
    value: &str,
) -> rusqlite::Result<T> {
    serde_json::from_str(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            row.as_ref().column_index(column).unwrap_or(0),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn json_column<T: DeserializeOwned>(row: &Row<'_>, column: &str) -> rusqlite::Result<T> {
    decode_column(row, column, &row.get::<_, String>(column)?)
}

fn optional_json_column<T: DeserializeOwned>(
    row: &Row<'_>,
    column: &str,
) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<String>>(column)?
        .map(|value| decode_column(row, column, &value))
        .transpose()
}

fn enum_column<T: DeserializeOwned>(row: &Row<'_>, column: &str) -> rusqlite::Result<T> {
    let value: String = row.get(column)?;
    decode_column(row, column, &serde_json::Value::String(value).to_string())
}
