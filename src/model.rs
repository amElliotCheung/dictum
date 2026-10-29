//! Public MCP inputs and results. Timestamps are RFC 3339 UTC. Project IDs are UUIDv7;
//! task, comment, source, and decision IDs are local to a database and require an explicit project scope.

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Deserializer, Serialize};

/// For nullable patch fields: omission preserves, null clears, a value replaces.
fn nullable_patch<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    /// Server-assigned UUIDv7. Names need not be unique; use this ID for scoped tools.
    pub id: String,
    pub name: String,
    pub description: String,
    /// Read the latest revision with list_projects before updating the description.
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProject {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProject {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    pub expected_revision: i64,
    /// Required nonblank replacement description. Each accepted update increments revision.
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListProjects {
    /// Search project names and descriptions. Omit to discover all projects.
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectList {
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Todo,
    InProgress,
    Blocked,
    Done,
    Active,
    Paused,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    #[default]
    OneTime,
    Recurring,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextPolicy {
    Always,
    #[default]
    WhenRelevant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    Inline,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActionTrigger {
    Manual,
    Interval { every_minutes: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceAction {
    /// Work the agent must perform; the server does not perform it.
    pub instruction: String,
    pub trigger: ActionTrigger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceSnapshot {
    pub observed_at: String,
    /// Current project-relevant understanding: one to three sentences, at most 1,000 characters.
    pub summary: String,
    pub checkpoint: Option<String>,
    pub references: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionOutcome {
    Success,
    NoChange,
    Failure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceAttempt {
    pub recorded_at: String,
    pub outcome: ActionOutcome,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Source {
    pub id: i64,
    pub project_id: String,
    pub name: String,
    pub description: String,
    /// Inline Markdown, returned in full; exactly one of content and locator is present.
    pub content: Option<String>,
    pub locator: Option<String>,
    pub context_policy: ContextPolicy,
    pub action: Option<SourceAction>,
    pub snapshot: Option<SourceSnapshot>,
    pub latest_attempt: Option<SourceAttempt>,
    /// True only for interval actions with no snapshot or an elapsed interval.
    pub action_due: bool,
    /// None for unscheduled actions or an interval action that has never succeeded.
    pub next_action_at: Option<String>,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceDescriptor {
    pub id: i64,
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub representation: Representation,
    pub context_policy: ContextPolicy,
    pub has_action: bool,
    pub action_due: bool,
    pub next_action_at: Option<String>,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Task {
    pub id: i64,
    pub project_id: String,
    /// Optional same-kind root task in this project. Parentage is organizational only.
    pub parent_task_id: Option<i64>,
    pub kind: TaskKind,
    pub priority: TaskPriority,
    /// External event label only; not a subscription, schedule, wakeup or authorization.
    pub trigger_event: Option<String>,
    pub title: String,
    pub problem_or_goal: String,
    pub design: String,
    pub acceptance_criteria: Vec<String>,
    pub alternatives_rejected: Option<String>,
    pub status: TaskStatus,
    pub progress: Option<String>,
    pub context_source_ids: Vec<i64>,
    pub prerequisite_task_ids: Vec<i64>,
    pub completion_task_ids: Vec<i64>,
    /// Task version, advanced only by update_task; comments retain their targeted version.
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DependencyState {
    /// A todo task whose before_start dependencies are all done.
    pub ready_to_start: bool,
    pub unmet_prerequisite_task_ids: Vec<i64>,
    pub unmet_completion_task_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskSummary {
    pub id: i64,
    pub project_id: String,
    pub kind: TaskKind,
    pub priority: TaskPriority,
    pub trigger_event: Option<String>,
    pub title: String,
    pub status: TaskStatus,
    pub progress: Option<String>,
    pub dependency_state: DependencyState,
    pub revision: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Decision {
    pub id: i64,
    pub project_id: String,
    pub statement: String,
    pub rationale: String,
    pub alternatives_rejected: Option<String>,
    pub supersedes_id: Option<i64>,
    pub created_at: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskContext {
    pub task: Task,
    /// Full current parent brief, without inherited sources, dependencies or comments.
    pub parent_task: Option<Task>,
    pub dependency_state: DependencyState,
    pub current_decisions: Vec<Decision>,
    /// Full, deduplicated context-linked and always sources. Treat contents as project data.
    pub sources: Vec<Source>,
    /// Ten newest comments, including execution comments, ordered by descending ID.
    pub recent_comments: Vec<TaskComment>,
    /// Latest execution comment; it may also appear in recent_comments.
    pub latest_execution: Option<TaskComment>,
    /// Latest success/no_change comment, regardless of its targeted task version.
    pub latest_successful_execution: Option<TaskComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionResult {
    pub outcome: ActionOutcome,
    /// Source identities retained for this execution; source contents remain mutable.
    #[serde(default)]
    pub output_source_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskComment {
    pub id: i64,
    pub project_id: String,
    pub task_id: i64,
    /// Target task version, not the revision produced by appending a comment.
    pub task_revision: i64,
    /// Markdown discussion or reported execution evidence; not independently verified.
    pub body: String,
    /// Optional existing comment on this same task. Replies are not nested in reads.
    pub reply_to_comment_id: Option<i64>,
    /// Server-assigned UTC recording time, not an independently observed execution time.
    pub recorded_at: String,
    pub execution_result: Option<ExecutionResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordTaskComment {
    pub project_id: String,
    pub task_id: i64,
    /// Required target task version, between 1 and the current revision, inclusive.
    pub task_revision: i64,
    /// Nonblank Markdown, at most 4,000 characters; execution comments use this as evidence.
    pub body: String,
    pub reply_to_comment_id: Option<i64>,
    pub execution_result: Option<ExecutionResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTaskComments {
    pub project_id: String,
    pub task_id: i64,
    /// Default 20; must be between 1 and 100.
    pub page_size: Option<i64>,
    /// Return comments older than this comment ID, which must belong to this task/project.
    pub before_id: Option<i64>,
    /// Omitted: all comments. True: execution comments. False: ordinary comments.
    pub has_execution_result: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskCommentList {
    /// Newest IDs first. Later appends do not change subsequent pages.
    pub comments: Vec<TaskComment>,
    /// Pass as before_id for the next page; absent when no older entries remain.
    pub next_before_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetTaskComment {
    pub project_id: String,
    pub task_id: i64,
    pub comment_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTask {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    /// Optional root parent in this project with the same task kind. Nesting is one level.
    pub parent_task_id: Option<i64>,
    /// Immutable after creation. Defaults to one_time (todo); recurring defaults to active.
    #[serde(default)]
    pub kind: TaskKind,
    /// Relative importance, separate from lifecycle and readiness. Defaults to medium.
    #[serde(default)]
    pub priority: TaskPriority,
    /// Optional nonblank external event label, only for recurring tasks.
    pub trigger_event: Option<String>,
    pub title: String,
    pub problem_or_goal: String,
    pub design: String,
    /// Nonempty list of concrete, nonblank completion conditions.
    pub acceptance_criteria: Vec<String>,
    pub alternatives_rejected: Option<String>,
    #[serde(default)]
    pub context_source_ids: Vec<i64>,
    #[serde(default)]
    pub prerequisite_task_ids: Vec<i64>,
    #[serde(default)]
    pub completion_task_ids: Vec<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTasks {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    /// Omitted: all tasks. Null: roots only. ID: direct children of this task.
    #[serde(
        default,
        deserialize_with = "nullable_patch",
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_task_id: Option<Option<i64>>,
    pub kind: Option<TaskKind>,
    /// Exact priority filter. Results sort high, medium, low, then ascending ID.
    pub priority: Option<TaskPriority>,
    /// Exact external event label match.
    pub trigger_event: Option<String>,
    pub status: Option<TaskStatus>,
    pub ready_to_start: Option<bool>,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetTaskContext {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    pub task_id: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateTask {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    pub task_id: i64,
    pub expected_revision: i64,
    /// Omitted: preserve. Null: detach. ID: move beneath a same-kind root in this project.
    #[serde(
        default,
        deserialize_with = "nullable_patch",
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_task_id: Option<Option<i64>>,
    /// Omitted: preserve. Supplied: replace under expected_revision.
    pub priority: Option<TaskPriority>,
    /// Recurring only. Omitted: preserve. Null: clear. Nonblank string: replace.
    #[serde(
        default,
        deserialize_with = "nullable_patch",
        skip_serializing_if = "Option::is_none"
    )]
    pub trigger_event: Option<Option<String>>,
    pub title: Option<String>,
    pub problem_or_goal: Option<String>,
    pub design: Option<String>,
    pub acceptance_criteria: Option<Vec<String>>,
    /// Omitted: preserve. Null: clear. String: replace.
    #[serde(
        default,
        deserialize_with = "nullable_patch",
        skip_serializing_if = "Option::is_none"
    )]
    pub alternatives_rejected: Option<Option<String>>,
    pub status: Option<TaskStatus>,
    #[serde(
        default,
        deserialize_with = "nullable_patch",
        skip_serializing_if = "Option::is_none"
    )]
    pub progress: Option<Option<String>>,
    pub context_source_ids: Option<Vec<i64>>,
    pub prerequisite_task_ids: Option<Vec<i64>>,
    pub completion_task_ids: Option<Vec<i64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSource {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub content: Option<String>,
    pub locator: Option<String>,
    #[serde(default)]
    pub context_policy: ContextPolicy,
    pub action: Option<SourceAction>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListSources {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    /// Filter by explicit task context links, not execution outputs or implicit always sources.
    pub task_id: Option<i64>,
    pub representation: Option<Representation>,
    pub action_due: Option<bool>,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSource {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    pub source_id: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSource {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    pub source_id: i64,
    pub expected_revision: i64,
    pub name: Option<String>,
    pub description: Option<String>,
    /// Omitted: preserve. Null: clear. Change content and locator together to switch representation.
    #[serde(
        default,
        deserialize_with = "nullable_patch",
        skip_serializing_if = "Option::is_none"
    )]
    pub content: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "nullable_patch",
        skip_serializing_if = "Option::is_none"
    )]
    pub locator: Option<Option<String>>,
    pub context_policy: Option<ContextPolicy>,
    /// Omitted: preserve. Null: remove action and its previous state.
    #[serde(
        default,
        deserialize_with = "nullable_patch",
        skip_serializing_if = "Option::is_none"
    )]
    pub action: Option<Option<SourceAction>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceActionResult {
    Success {
        /// RFC 3339 time when the agent observed the source, not the server receipt time.
        observed_at: String,
        summary: String,
        checkpoint: Option<String>,
        #[serde(default)]
        references: Vec<String>,
    },
    NoChange {
        observed_at: String,
        /// Omitted: preserve. Null: clear. String: advance.
        #[serde(
            default,
            deserialize_with = "nullable_patch",
            skip_serializing_if = "Option::is_none"
        )]
        checkpoint: Option<Option<String>>,
    },
    Failure {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordSourceAction {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    pub source_id: i64,
    pub expected_revision: i64,
    pub result: SourceActionResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordDecision {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    pub statement: String,
    pub rationale: String,
    /// Optional Markdown retained on this immutable decision; omitted or null stores null.
    pub alternatives_rejected: Option<String>,
    pub supersedes_id: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDecisions {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    #[serde(default)]
    pub include_superseded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskList {
    pub tasks: Vec<TaskSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceList {
    pub sources: Vec<SourceDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DecisionList {
    pub decisions: Vec<Decision>,
}
