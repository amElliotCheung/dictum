//! Transactional, project-scoped SQLite storage. The database holds reported work, not a scheduler.

use crate::model::*;
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

const SCHEMA_VERSION: i64 = 13;
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
    #[error("project {0} was not found")]
    ProjectNotFound(String),
    #[error("{entity} {id} was not found in this project")]
    NotFound { entity: &'static str, id: i64 },
    #[error(
        "{entity} {id} revision conflict: expected {expected}, found {actual}; reread before retrying"
    )]
    RevisionConflict {
        entity: &'static str,
        id: String,
        expected: i64,
        actual: i64,
    },
    #[error(
        "unsupported database schema (version {version}, application ID {application_id}); this build supports schema version 13 only; preserve the existing file"
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

    pub fn create_project(&self, input: CreateProject) -> StoreResult<Project> {
        required(&input.name, "name")?;
        required(&input.description, "description")?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = timestamp(Utc::now());
        let project = Project {
            id: new_project_id(&transaction)?,
            name: input.name,
            description: input.description,
            revision: 1,
            created_at: now.clone(),
            updated_at: now,
        };
        transaction.execute(
            "INSERT INTO projects (id, name, description, revision, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                project.id,
                project.name,
                project.description,
                project.revision,
                project.created_at,
                project.updated_at
            ],
        )?;
        transaction.commit()?;
        Ok(project)
    }

    pub fn list_projects(&self, input: ListProjects) -> StoreResult<Vec<Project>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare("SELECT * FROM projects ORDER BY created_at, id")?;
        let query = input.query.as_deref().map(str::to_lowercase);
        let projects = statement
            .query_map([], map_project)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(projects
            .into_iter()
            .filter(|project| {
                query.as_ref().is_none_or(|query| {
                    project.name.to_lowercase().contains(query)
                        || project.description.to_lowercase().contains(query)
                })
            })
            .collect())
    }

    pub fn update_project(&self, input: UpdateProject) -> StoreResult<Project> {
        required(&input.project_id, "project_id")?;
        required(&input.description, "description")?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut project = transaction
            .query_row(
                "SELECT * FROM projects WHERE id=?1",
                [&input.project_id],
                map_project,
            )
            .optional()?
            .ok_or_else(|| StoreError::ProjectNotFound(input.project_id.clone()))?;
        check_revision(
            "project",
            &project.id,
            input.expected_revision,
            project.revision,
        )?;
        project.description = input.description;
        project.revision += 1;
        project.updated_at = timestamp(Utc::now());
        transaction.execute(
            "UPDATE projects SET description=?2, revision=?3, updated_at=?4 WHERE id=?1",
            params![
                project.id,
                project.description,
                project.revision,
                project.updated_at
            ],
        )?;
        transaction.commit()?;
        Ok(project)
    }

    pub fn create_task(&self, input: CreateTask) -> StoreResult<Task> {
        validate_brief(
            &input.title,
            &input.problem_or_goal,
            &input.design,
            &input.acceptance_criteria,
        )?;
        let status = match input.kind {
            TaskKind::OneTime => TaskStatus::Todo,
            TaskKind::Recurring => TaskStatus::Active,
        };
        validate_task_lifecycle(input.kind, status, input.trigger_event.as_deref())?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_project(&transaction, &input.project_id)?;
        validate_parent(
            &transaction,
            &input.project_id,
            None,
            input.kind,
            input.parent_task_id,
        )?;
        let now = timestamp(Utc::now());
        transaction.execute(
            "INSERT INTO tasks (project_id, title, problem_or_goal, design, acceptance_criteria, alternatives_rejected, status, revision, created_at, updated_at, kind, trigger_event, priority, parent_task_id) VALUES (?7, ?1, ?2, ?3, ?4, ?5, ?8, 1, ?6, ?6, ?9, ?10, ?11, ?12)",
            params![input.title, input.problem_or_goal, input.design, encode(&input.acceptance_criteria)?, input.alternatives_rejected, now, input.project_id, status_name(status), kind_name(input.kind), input.trigger_event, priority_name(input.priority), input.parent_task_id],
        )?;
        let id = transaction.last_insert_rowid();
        replace_context_sources(
            &transaction,
            &input.project_id,
            id,
            &input.context_source_ids,
        )?;
        replace_dependencies(
            &transaction,
            &input.project_id,
            id,
            "before_start",
            &input.prerequisite_task_ids,
        )?;
        replace_dependencies(
            &transaction,
            &input.project_id,
            id,
            "before_done",
            &input.completion_task_ids,
        )?;
        reject_cycle(&transaction, &input.project_id, id)?;
        let task = task_by_id(&transaction, &input.project_id, id)?;
        transaction.commit()?;
        Ok(task)
    }

    pub fn list_tasks(&self, input: ListTasks) -> StoreResult<Vec<TaskSummary>> {
        if let Some(label) = &input.trigger_event {
            required(label, "trigger_event")?;
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        ensure_project(&transaction, &input.project_id)?;
        if let Some(Some(parent_id)) = input.parent_task_id {
            ensure_exists(&transaction, &input.project_id, "task", parent_id)?;
        }
        let ids = {
            let mut statement = transaction.prepare(
                "SELECT id FROM tasks WHERE project_id=?1 AND (?2 IS NULL OR priority=?2) ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'medium' THEN 1 WHEN 'low' THEN 2 END, id",
            )?;
            statement
                .query_map(
                    params![input.project_id, input.priority.map(priority_name)],
                    |row| row.get(0),
                )?
                .collect::<rusqlite::Result<Vec<i64>>>()?
        };
        let query = input.query.as_deref().map(str::to_lowercase);
        let mut result = Vec::new();
        for id in ids {
            let task = task_by_id(&transaction, &input.project_id, id)?;
            if input
                .parent_task_id
                .is_some_and(|parent_id| task.parent_task_id != parent_id)
            {
                continue;
            }
            if input.kind.is_some_and(|kind| kind != task.kind)
                || input
                    .trigger_event
                    .as_ref()
                    .is_some_and(|label| task.trigger_event.as_ref() != Some(label))
            {
                continue;
            }
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
                project_id: task.project_id,
                kind: task.kind,
                priority: task.priority,
                trigger_event: task.trigger_event,
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

    pub fn get_task_context(&self, project_id: &str, task_id: i64) -> StoreResult<TaskContext> {
        let mut connection = self.connect()?;
        // All components belong to the same read snapshot across concurrent writers.
        let transaction = connection.transaction()?;
        ensure_project(&transaction, project_id)?;
        let task = task_by_id(&transaction, project_id, task_id)?;
        let dependency_state = dependency_state(&transaction, &task)?;
        let now = Utc::now();
        let source_ids = {
            let mut statement = transaction.prepare("SELECT id FROM sources WHERE project_id=?2 AND (context_policy = 'always' OR id IN (SELECT source_id FROM task_context_sources WHERE task_id = ?1 AND project_id=?2)) ORDER BY id")?;
            statement
                .query_map(params![task_id, project_id], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let sources = source_ids
            .into_iter()
            .map(|id| source_by_id(&transaction, project_id, id, now))
            .collect::<StoreResult<Vec<_>>>()?;
        let current_decisions = decisions(&transaction, project_id, false)?;
        let parent_task = task
            .parent_task_id
            .map(|id| task_by_id(&transaction, project_id, id))
            .transpose()?;
        let recent_comments = task_comments(&transaction, project_id, task_id, None, None, 10)?;
        let latest_execution = latest_task_execution(&transaction, project_id, task_id, false)?;
        let latest_successful_execution =
            latest_task_execution(&transaction, project_id, task_id, true)?;
        let result = TaskContext {
            task,
            parent_task,
            dependency_state,
            current_decisions,
            sources,
            recent_comments,
            latest_execution,
            latest_successful_execution,
        };
        transaction.commit()?;
        Ok(result)
    }

    pub fn update_task(&self, input: UpdateTask) -> StoreResult<Task> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_project(&transaction, &input.project_id)?;
        let mut task = task_by_id(&transaction, &input.project_id, input.task_id)?;
        check_revision("task", task.id, input.expected_revision, task.revision)?;
        if let Some(parent_task_id) = input.parent_task_id {
            validate_parent(
                &transaction,
                &input.project_id,
                Some(task.id),
                task.kind,
                parent_task_id,
            )?;
            task.parent_task_id = parent_task_id;
        }
        if let Some(priority) = input.priority {
            task.priority = priority;
        }
        if let Some(value) = input.trigger_event {
            if task.kind != TaskKind::Recurring {
                return invalid("trigger_event is only supported for recurring tasks");
            }
            task.trigger_event = value;
        }
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
        validate_brief(
            &task.title,
            &task.problem_or_goal,
            &task.design,
            &task.acceptance_criteria,
        )?;
        validate_task_lifecycle(task.kind, task.status, task.trigger_event.as_deref())?;
        if let Some(ids) = input.context_source_ids {
            replace_context_sources(&transaction, &input.project_id, task.id, &ids)?;
        }
        if let Some(ids) = input.prerequisite_task_ids {
            replace_dependencies(
                &transaction,
                &input.project_id,
                task.id,
                "before_start",
                &ids,
            )?;
        }
        if let Some(ids) = input.completion_task_ids {
            replace_dependencies(
                &transaction,
                &input.project_id,
                task.id,
                "before_done",
                &ids,
            )?;
        }
        reject_cycle(&transaction, &input.project_id, task.id)?;
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
            let has_success: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM task_comments WHERE project_id=?1 AND task_id=?2 AND outcome IN ('success', 'no_change'))",
                params![input.project_id, task.id],
                |row| row.get(0),
            )?;
            if !has_success {
                return invalid(
                    "cannot finish task without a successful execution result; append success or no_change with record_task_comment",
                );
            }
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
            "UPDATE tasks SET title=?1, problem_or_goal=?2, design=?3, acceptance_criteria=?4, alternatives_rejected=?5, status=?6, progress=?7, revision=revision+1, updated_at=?8, trigger_event=?11, priority=?12, parent_task_id=?13 WHERE id=?9 AND project_id=?10",
            params![task.title, task.problem_or_goal, task.design, encode(&task.acceptance_criteria)?, task.alternatives_rejected, status_name(task.status), task.progress, timestamp(Utc::now()), task.id, input.project_id, task.trigger_event, priority_name(task.priority), task.parent_task_id],
        )?;
        let result = task_by_id(&transaction, &input.project_id, task.id)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn record_task_comment(&self, input: RecordTaskComment) -> StoreResult<TaskComment> {
        required(&input.body, "body")?;
        if input.body.chars().count() > 4_000 {
            return invalid(
                "comment body must not exceed 4,000 characters; use output Sources for documents",
            );
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_project(&transaction, &input.project_id)?;
        let task = task_by_id(&transaction, &input.project_id, input.task_id)?;
        if input.task_revision < 1 || input.task_revision > task.revision {
            return invalid(format!(
                "task_revision must target an existing task version between 1 and {}",
                task.revision
            ));
        }
        if let Some(id) = input.reply_to_comment_id {
            task_comment_by_id(&transaction, &input.project_id, task.id, id)?;
        }
        if let Some(result) = &input.execution_result {
            for &id in &result.output_source_ids {
                ensure_exists(&transaction, &input.project_id, "source", id)?;
            }
            if result.outcome != ActionOutcome::Failure {
                let state = dependency_state(&transaction, &task)?;
                if !state.unmet_prerequisite_task_ids.is_empty()
                    || !state.unmet_completion_task_ids.is_empty()
                {
                    return invalid(format!(
                        "cannot record successful execution; unmet start dependencies: {:?}; unmet completion dependencies: {:?}",
                        state.unmet_prerequisite_task_ids, state.unmet_completion_task_ids
                    ));
                }
            }
        }
        let outcome = input
            .execution_result
            .as_ref()
            .map(|result| outcome_name(result.outcome));
        transaction.execute(
            "INSERT INTO task_comments (project_id, task_id, task_revision, body, reply_to_comment_id, outcome, recorded_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![input.project_id, task.id, input.task_revision, input.body, input.reply_to_comment_id, outcome, timestamp(Utc::now())],
        )?;
        let id = transaction.last_insert_rowid();
        if let Some(result) = input.execution_result {
            for source_id in result
                .output_source_ids
                .into_iter()
                .collect::<BTreeSet<_>>()
            {
                transaction.execute(
                    "INSERT INTO task_comment_sources (project_id, comment_id, source_id) VALUES (?1, ?2, ?3)",
                    params![input.project_id, id, source_id],
                )?;
            }
        }
        let comment = task_comment_by_id(&transaction, &input.project_id, task.id, id)?;
        transaction.commit()?;
        Ok(comment)
    }

    pub fn list_task_comments(&self, input: ListTaskComments) -> StoreResult<TaskCommentList> {
        let page_size = input.page_size.unwrap_or(20);
        if !(1..=100).contains(&page_size) {
            return invalid("page_size must be between 1 and 100");
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        ensure_project(&transaction, &input.project_id)?;
        ensure_exists(&transaction, &input.project_id, "task", input.task_id)?;
        if let Some(id) = input.before_id {
            if id <= 0 {
                return invalid("before_id must be a positive comment ID belonging to this task");
            }
            task_comment_by_id(&transaction, &input.project_id, input.task_id, id)?;
        }
        let mut comments = task_comments(
            &transaction,
            &input.project_id,
            input.task_id,
            input.before_id,
            input.has_execution_result,
            page_size + 1,
        )?;
        let has_more = comments.len() > page_size as usize;
        comments.truncate(page_size as usize);
        let next_before_id = has_more.then(|| comments.last().expect("nonempty page").id);
        transaction.commit()?;
        Ok(TaskCommentList {
            comments,
            next_before_id,
        })
    }

    pub fn get_task_comment(&self, input: GetTaskComment) -> StoreResult<TaskComment> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        ensure_project(&transaction, &input.project_id)?;
        ensure_exists(&transaction, &input.project_id, "task", input.task_id)?;
        let comment = task_comment_by_id(
            &transaction,
            &input.project_id,
            input.task_id,
            input.comment_id,
        )?;
        transaction.commit()?;
        Ok(comment)
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
        ensure_project(&transaction, &input.project_id)?;
        let now = Utc::now();
        transaction.execute(
            "INSERT INTO sources (project_id, name, description, content, locator, context_policy, action, revision, created_at, updated_at) VALUES (?8, ?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)",
            params![input.name, input.description, input.content, input.locator, policy_name(input.context_policy), encode_optional(&input.action)?, timestamp(now), input.project_id],
        )?;
        let result = source_by_id(
            &transaction,
            &input.project_id,
            transaction.last_insert_rowid(),
            now,
        )?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn list_sources(&self, input: ListSources) -> StoreResult<Vec<SourceDescriptor>> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        ensure_project(&transaction, &input.project_id)?;
        let linked = if let Some(task_id) = input.task_id {
            ensure_exists(&transaction, &input.project_id, "task", task_id)?;
            let mut statement = transaction.prepare(
                "SELECT DISTINCT source_id FROM task_context_sources WHERE task_id=?1 AND project_id=?2",
            )?;
            Some(
                statement
                    .query_map(params![task_id, input.project_id], |row| {
                        row.get::<_, i64>(0)
                    })?
                    .collect::<rusqlite::Result<BTreeSet<_>>>()?,
            )
        } else {
            None
        };
        let ids = all_ids(
            &transaction,
            "SELECT id FROM sources WHERE project_id=?1 ORDER BY id",
            &input.project_id,
        )?;
        let now = Utc::now();
        let query = input.query.as_deref().map(str::to_lowercase);
        let mut result = Vec::new();
        for id in ids {
            if linked.as_ref().is_some_and(|ids| !ids.contains(&id)) {
                continue;
            }
            let source = source_by_id(&transaction, &input.project_id, id, now)?;
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

    pub fn get_source(&self, project_id: &str, source_id: i64) -> StoreResult<Source> {
        let connection = self.connect()?;
        ensure_project(&connection, project_id)?;
        source_by_id(&connection, project_id, source_id, Utc::now())
    }

    pub fn update_source(&self, input: UpdateSource) -> StoreResult<Source> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_project(&transaction, &input.project_id)?;
        let now = Utc::now();
        let mut source = source_by_id(&transaction, &input.project_id, input.source_id, now)?;
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
            "UPDATE sources SET name=?1, description=?2, content=?3, locator=?4, context_policy=?5, action=?6, snapshot=?7, latest_attempt=?8, revision=revision+1, updated_at=?9 WHERE id=?10 AND project_id=?11",
            params![source.name, source.description, source.content, source.locator, policy_name(source.context_policy), encode_optional(&source.action)?, encode_optional(&source.snapshot)?, encode_optional(&source.latest_attempt)?, timestamp(now), source.id, input.project_id],
        )?;
        let result = source_by_id(&transaction, &input.project_id, source.id, now)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn record_source_action(&self, input: RecordSourceAction) -> StoreResult<Source> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_project(&transaction, &input.project_id)?;
        let now = Utc::now();
        let mut source = source_by_id(&transaction, &input.project_id, input.source_id, now)?;
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
            "UPDATE sources SET snapshot=?1, latest_attempt=?2, revision=revision+1, updated_at=?3 WHERE id=?4 AND project_id=?5",
            params![encode_optional(&source.snapshot)?, encode(&latest_attempt)?, timestamp(now), source.id, input.project_id],
        )?;
        let result = source_by_id(&transaction, &input.project_id, source.id, now)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn record_decision(&self, input: RecordDecision) -> StoreResult<Decision> {
        required(&input.statement, "statement")?;
        required(&input.rationale, "rationale")?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_project(&transaction, &input.project_id)?;
        if let Some(id) = input.supersedes_id {
            ensure_exists(&transaction, &input.project_id, "decision", id)?;
            let superseded: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM decisions WHERE supersedes_id=?1 AND project_id=?2)",
                params![id, input.project_id],
                |row| row.get(0),
            )?;
            if superseded {
                return invalid(format!(
                    "decision {id} is already superseded; replace its current successor"
                ));
            }
        }
        transaction.execute(
            "INSERT INTO decisions (project_id, statement, rationale, alternatives_rejected, supersedes_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![input.project_id, input.statement, input.rationale, input.alternatives_rejected, input.supersedes_id, timestamp(Utc::now())],
        )?;
        let result = transaction.query_row("SELECT id, project_id, statement, rationale, alternatives_rejected, supersedes_id, created_at, 1 AS is_current FROM decisions WHERE id=?1 AND project_id=?2", params![transaction.last_insert_rowid(), input.project_id], map_decision)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn list_decisions(&self, input: ListDecisions) -> StoreResult<Vec<Decision>> {
        let connection = self.connect()?;
        ensure_project(&connection, &input.project_id)?;
        decisions(&connection, &input.project_id, input.include_superseded)
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

fn validate_task_lifecycle(
    kind: TaskKind,
    status: TaskStatus,
    trigger: Option<&str>,
) -> StoreResult<()> {
    let valid = match kind {
        TaskKind::OneTime => matches!(
            status,
            TaskStatus::Todo | TaskStatus::InProgress | TaskStatus::Blocked | TaskStatus::Done
        ),
        TaskKind::Recurring => matches!(status, TaskStatus::Active | TaskStatus::Paused),
    };
    if !valid {
        return invalid(
            "one_time tasks use todo/in_progress/blocked/done; recurring tasks use active/paused",
        );
    }
    if let Some(label) = trigger {
        if kind != TaskKind::Recurring {
            return invalid("trigger_event is only supported for recurring tasks");
        }
        required(label, "trigger_event")?;
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

fn check_revision(
    entity: &'static str,
    id: impl ToString,
    expected: i64,
    actual: i64,
) -> StoreResult<()> {
    if expected != actual {
        Err(StoreError::RevisionConflict {
            entity,
            id: id.to_string(),
            expected,
            actual,
        })
    } else {
        Ok(())
    }
}

fn ensure_project(connection: &Connection, project_id: &str) -> StoreResult<()> {
    required(project_id, "project_id")?;
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
        [project_id],
        |row| row.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::ProjectNotFound(project_id.into()))
    }
}

fn map_project(row: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get("id")?,
        name: row.get("name")?,
        description: row.get("description")?,
        revision: row.get("revision")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// Prefer a real SQLite built-in when available; never register an emulation/extension.
pub(crate) fn new_project_id(connection: &Connection) -> StoreResult<String> {
    let builtin: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_function_list WHERE name='uuidv7' AND builtin=1 AND narg IN (0, -1))", [], |row| row.get(0))?;
    if builtin {
        Ok(connection.query_row("SELECT uuidv7()", [], |row| row.get(0))?)
    } else {
        Ok(uuid::Uuid::now_v7().to_string())
    }
}

fn ensure_exists(
    connection: &Connection,
    project_id: &str,
    entity: &'static str,
    id: i64,
) -> StoreResult<()> {
    let sql = match entity {
        "task" => "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND project_id=?2)",
        "source" => "SELECT EXISTS(SELECT 1 FROM sources WHERE id=?1 AND project_id=?2)",
        "decision" => "SELECT EXISTS(SELECT 1 FROM decisions WHERE id=?1 AND project_id=?2)",
        _ => unreachable!("internal entity name"),
    };
    let exists: bool = connection.query_row(sql, params![id, project_id], |row| row.get(0))?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::NotFound { entity, id })
    }
}

fn validate_parent(
    connection: &Connection,
    project_id: &str,
    task_id: Option<i64>,
    kind: TaskKind,
    parent_task_id: Option<i64>,
) -> StoreResult<()> {
    let Some(parent_id) = parent_task_id else {
        return Ok(());
    };
    if task_id == Some(parent_id) {
        return invalid("task cannot be its own parent");
    }
    let parent = task_by_id(connection, project_id, parent_id)?;
    if parent.kind != kind {
        return invalid("parent and child tasks must have the same kind");
    }
    if parent.parent_task_id.is_some() {
        return invalid("parent must be a root task; subtasks cannot have children");
    }
    if let Some(id) = task_id {
        let has_children: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE project_id=?1 AND parent_task_id=?2)",
            params![project_id, id],
            |row| row.get(0),
        )?;
        if has_children {
            return invalid(
                "a task with children cannot become a subtask; detach its children first",
            );
        }
    }
    Ok(())
}

fn replace_context_sources(
    connection: &Connection,
    project_id: &str,
    task_id: i64,
    ids: &[i64],
) -> StoreResult<()> {
    for &id in ids {
        ensure_exists(connection, project_id, "source", id)?;
    }
    connection.execute(
        "DELETE FROM task_context_sources WHERE task_id=?1 AND project_id=?2",
        params![task_id, project_id],
    )?;
    for &id in ids.iter().collect::<BTreeSet<_>>() {
        connection.execute(
            "INSERT INTO task_context_sources (task_id, source_id, project_id) VALUES (?1, ?2, ?3)",
            params![task_id, id, project_id],
        )?;
    }
    Ok(())
}

fn replace_dependencies(
    connection: &Connection,
    project_id: &str,
    task_id: i64,
    kind: &str,
    ids: &[i64],
) -> StoreResult<()> {
    for &id in ids {
        if id == task_id {
            return invalid("task cannot depend on itself");
        }
        ensure_exists(connection, project_id, "task", id)?;
        let kind: String = connection.query_row(
            "SELECT kind FROM tasks WHERE id=?1 AND project_id=?2",
            params![id, project_id],
            |row| row.get(0),
        )?;
        if kind != "one_time" {
            return invalid(
                "recurring tasks cannot be dependency targets because they never reach done",
            );
        }
    }
    connection.execute(
        "DELETE FROM task_dependencies WHERE task_id=?1 AND kind=?2 AND project_id=?3",
        params![task_id, kind, project_id],
    )?;
    for &id in ids.iter().collect::<BTreeSet<_>>() {
        connection.execute(
            "INSERT INTO task_dependencies (task_id, depends_on_task_id, kind, project_id) VALUES (?1, ?2, ?3, ?4)",
            params![task_id, id, kind, project_id],
        )?;
    }
    Ok(())
}

fn reject_cycle(connection: &Connection, project_id: &str, task_id: i64) -> StoreResult<()> {
    let cycle: bool = connection.query_row(
        "WITH RECURSIVE reachable(id) AS (SELECT depends_on_task_id FROM task_dependencies WHERE task_id=?1 AND project_id=?2 UNION SELECT d.depends_on_task_id FROM task_dependencies d JOIN reachable r ON d.task_id=r.id WHERE d.project_id=?2) SELECT EXISTS(SELECT 1 FROM reachable WHERE id=?1)",
        params![task_id, project_id], |row| row.get(0),
    )?;
    if cycle {
        invalid("task dependency would create a cycle across before_start/before_done links")
    } else {
        Ok(())
    }
}

fn linked_ids(
    connection: &Connection,
    project_id: &str,
    sql: &str,
    task_id: i64,
    kind: &str,
) -> StoreResult<Vec<i64>> {
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map(params![task_id, kind, project_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn all_ids(connection: &Connection, sql: &str, project_id: &str) -> StoreResult<Vec<i64>> {
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map([project_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn task_by_id(connection: &Connection, project_id: &str, id: i64) -> StoreResult<Task> {
    let mut task = connection
        .query_row(
            "SELECT * FROM tasks WHERE id=?1 AND project_id=?2",
            params![id, project_id],
            |row| {
                Ok(Task {
                    project_id: row.get("project_id")?,
                    parent_task_id: row.get("parent_task_id")?,
                    kind: enum_column(row, "kind")?,
                    priority: enum_column(row, "priority")?,
                    trigger_event: row.get("trigger_event")?,
                    id: row.get("id")?,
                    title: row.get("title")?,
                    problem_or_goal: row.get("problem_or_goal")?,
                    design: row.get("design")?,
                    acceptance_criteria: json_column(row, "acceptance_criteria")?,
                    alternatives_rejected: row.get("alternatives_rejected")?,
                    status: enum_column(row, "status")?,
                    progress: row.get("progress")?,
                    context_source_ids: vec![],
                    prerequisite_task_ids: vec![],
                    completion_task_ids: vec![],
                    revision: row.get("revision")?,
                    created_at: row.get("created_at")?,
                    updated_at: row.get("updated_at")?,
                })
            },
        )
        .optional()?
        .ok_or(StoreError::NotFound { entity: "task", id })?;
    let mut statement = connection.prepare(
        "SELECT source_id FROM task_context_sources WHERE task_id=?1 AND project_id=?2 ORDER BY source_id",
    )?;
    task.context_source_ids = statement
        .query_map(params![id, project_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let dependency_sql = "SELECT depends_on_task_id FROM task_dependencies WHERE task_id=?1 AND kind=?2 AND project_id=?3 ORDER BY depends_on_task_id";
    task.prerequisite_task_ids =
        linked_ids(connection, project_id, dependency_sql, id, "before_start")?;
    task.completion_task_ids =
        linked_ids(connection, project_id, dependency_sql, id, "before_done")?;
    Ok(task)
}

fn dependency_state(connection: &Connection, task: &Task) -> StoreResult<DependencyState> {
    let sql = "SELECT d.depends_on_task_id FROM task_dependencies d JOIN tasks t ON d.depends_on_task_id=t.id WHERE d.task_id=?1 AND d.kind=?2 AND d.project_id=?3 AND t.project_id=?3 AND t.status!='done' ORDER BY d.depends_on_task_id";
    let unmet_prerequisite_task_ids =
        linked_ids(connection, &task.project_id, sql, task.id, "before_start")?;
    let unmet_completion_task_ids =
        linked_ids(connection, &task.project_id, sql, task.id, "before_done")?;
    Ok(DependencyState {
        ready_to_start: task.kind == TaskKind::OneTime
            && task.status == TaskStatus::Todo
            && unmet_prerequisite_task_ids.is_empty(),
        unmet_prerequisite_task_ids,
        unmet_completion_task_ids,
    })
}

fn task_comment_by_id(
    connection: &Connection,
    project_id: &str,
    task_id: i64,
    id: i64,
) -> StoreResult<TaskComment> {
    let mut comment = connection
        .query_row(
            "SELECT * FROM task_comments WHERE id=?1 AND project_id=?2 AND task_id=?3",
            params![id, project_id, task_id],
            |row| {
                let execution_result = if row.get::<_, Option<String>>("outcome")?.is_some() {
                    Some(ExecutionResult {
                        outcome: enum_column(row, "outcome")?,
                        output_source_ids: vec![],
                    })
                } else {
                    None
                };
                Ok(TaskComment {
                    id: row.get("id")?,
                    project_id: row.get("project_id")?,
                    task_id: row.get("task_id")?,
                    task_revision: row.get("task_revision")?,
                    body: row.get("body")?,
                    reply_to_comment_id: row.get("reply_to_comment_id")?,
                    recorded_at: row.get("recorded_at")?,
                    execution_result,
                })
            },
        )
        .optional()?
        .ok_or(StoreError::NotFound {
            entity: "task comment",
            id,
        })?;
    if let Some(result) = &mut comment.execution_result {
        let mut statement = connection.prepare(
            "SELECT source_id FROM task_comment_sources WHERE comment_id=?1 AND project_id=?2 ORDER BY source_id",
        )?;
        result.output_source_ids = statement
            .query_map(params![id, project_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
    }
    Ok(comment)
}

fn task_comments(
    connection: &Connection,
    project_id: &str,
    task_id: i64,
    before_id: Option<i64>,
    has_execution_result: Option<bool>,
    limit: i64,
) -> StoreResult<Vec<TaskComment>> {
    let mut statement = connection.prepare(
        "SELECT id FROM task_comments WHERE project_id=?1 AND task_id=?2 AND (?3 IS NULL OR id < ?3) AND (?4 IS NULL OR (outcome IS NOT NULL)=?4) ORDER BY id DESC LIMIT ?5",
    )?;
    let ids = statement
        .query_map(
            params![project_id, task_id, before_id, has_execution_result, limit],
            |row| row.get::<_, i64>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ids.into_iter()
        .map(|id| task_comment_by_id(connection, project_id, task_id, id))
        .collect()
}

fn latest_task_execution(
    connection: &Connection,
    project_id: &str,
    task_id: i64,
    successful: bool,
) -> StoreResult<Option<TaskComment>> {
    let sql = if successful {
        "SELECT id FROM task_comments WHERE project_id=?1 AND task_id=?2 AND outcome IN ('success', 'no_change') ORDER BY id DESC LIMIT 1"
    } else {
        "SELECT id FROM task_comments WHERE project_id=?1 AND task_id=?2 AND outcome IS NOT NULL ORDER BY id DESC LIMIT 1"
    };
    let id = connection
        .query_row(sql, params![project_id, task_id], |row| {
            row.get::<_, i64>(0)
        })
        .optional()?;
    id.map(|id| task_comment_by_id(connection, project_id, task_id, id))
        .transpose()
}

fn source_by_id(
    connection: &Connection,
    project_id: &str,
    id: i64,
    now: DateTime<Utc>,
) -> StoreResult<Source> {
    let mut source = connection
        .query_row(
            "SELECT * FROM sources WHERE id=?1 AND project_id=?2",
            params![id, project_id],
            |row| {
                Ok(Source {
                    project_id: row.get("project_id")?,
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
            },
        )
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
        project_id: source.project_id.clone(),
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

fn decisions(
    connection: &Connection,
    project_id: &str,
    include_superseded: bool,
) -> StoreResult<Vec<Decision>> {
    let mut statement = connection.prepare("SELECT d.id, d.project_id, d.statement, d.rationale, d.alternatives_rejected, d.supersedes_id, d.created_at, NOT EXISTS(SELECT 1 FROM decisions successor WHERE successor.supersedes_id=d.id) AS is_current FROM decisions d WHERE d.project_id=?2 AND (?1 OR NOT EXISTS(SELECT 1 FROM decisions successor WHERE successor.supersedes_id=d.id)) ORDER BY d.id")?;
    Ok(statement
        .query_map(params![include_superseded, project_id], map_decision)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn map_decision(row: &Row<'_>) -> rusqlite::Result<Decision> {
    Ok(Decision {
        project_id: row.get("project_id")?,
        id: row.get("id")?,
        statement: row.get("statement")?,
        rationale: row.get("rationale")?,
        alternatives_rejected: row.get("alternatives_rejected")?,
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
        TaskStatus::Active => "active",
        TaskStatus::Paused => "paused",
    }
}

fn kind_name(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::OneTime => "one_time",
        TaskKind::Recurring => "recurring",
    }
}

fn priority_name(priority: TaskPriority) -> &'static str {
    match priority {
        TaskPriority::Low => "low",
        TaskPriority::Medium => "medium",
        TaskPriority::High => "high",
    }
}

fn outcome_name(outcome: ActionOutcome) -> &'static str {
    match outcome {
        ActionOutcome::Success => "success",
        ActionOutcome::NoChange => "no_change",
        ActionOutcome::Failure => "failure",
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
