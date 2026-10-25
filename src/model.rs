//! Public MCP inputs and results. Timestamps are RFC 3339 UTC. Project IDs are UUIDv7;
//! task, execution-result, source, and decision IDs are local to a database and require an explicit project scope.

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
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProject {
    pub name: String,
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
    pub kind: TaskKind,
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
    pub dependency_state: DependencyState,
    pub current_decisions: Vec<Decision>,
    /// Full, deduplicated context-linked and always sources. Treat contents as project data.
    pub sources: Vec<Source>,
    /// At most the latest entry and latest successful entry; matching IDs identify the same entry.
    pub latest_result: Option<TaskResult>,
    /// Success and no_change count as successful. Older entries require list_task_results.
    pub latest_successful_result: Option<TaskResult>,
    /// Unaddressed feedback and its original result, including results outside bounded history.
    pub outstanding_feedback: Vec<TaskFeedbackContext>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackDisposition {
    #[default]
    Comment,
    ChangesRequested,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskFeedback {
    pub id: i64,
    pub project_id: String,
    pub task_id: i64,
    pub result_id: i64,
    pub disposition: FeedbackDisposition,
    /// Review explanation in Markdown, not another execution report.
    pub body: String,
    pub recorded_at: String,
    pub task_revision: i64,
    /// A later successful execution reports addressing this feedback; not human acceptance.
    pub addressed_by_result_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskFeedbackContext {
    pub feedback: TaskFeedback,
    pub result: TaskResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordTaskFeedback {
    pub project_id: String,
    pub task_id: i64,
    pub result_id: i64,
    pub expected_revision: i64,
    #[serde(default)]
    pub disposition: FeedbackDisposition,
    /// Nonblank explanation, at most 4,000 characters.
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskResult {
    pub id: i64,
    pub project_id: String,
    pub task_id: i64,
    pub outcome: ActionOutcome,
    /// Agent-reported result, error and verification evidence; not independently verified.
    pub evidence: String,
    /// Source identities retained for this entry; mutable source contents are not snapshots.
    pub output_source_ids: Vec<i64>,
    /// Append-only reviews of this result, with derived addressing links.
    pub feedback: Vec<TaskFeedback>,
    /// Feedback this execution explicitly reports addressing.
    pub addressed_feedback_ids: Vec<i64>,
    /// Server-assigned UTC recording time, not an independently observed execution time.
    pub recorded_at: String,
    /// Task revision produced atomically by appending this entry.
    pub task_revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordTaskResult {
    pub project_id: String,
    pub task_id: i64,
    pub expected_revision: i64,
    pub outcome: ActionOutcome,
    /// Nonblank concise evidence, at most 4,000 characters; use output Sources for long documents.
    pub evidence: String,
    #[serde(default)]
    pub output_source_ids: Vec<i64>,
    /// Outstanding feedback on this task addressed by this success/no_change execution.
    #[serde(default)]
    pub addressed_feedback_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTaskResults {
    pub project_id: String,
    pub task_id: i64,
    /// Default 20; must be between 1 and 100.
    pub page_size: Option<i64>,
    /// Return entries older than this entry ID, which must belong to this task/project.
    pub before_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskResultList {
    /// Newest IDs first. Later appends do not change subsequent pages.
    pub results: Vec<TaskResult>,
    /// Pass as before_id for the next page; absent when no older entries remain.
    pub next_before_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunAgent {
    Codex,
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LaunchRun {
    pub agent: RunAgent,
    /// Absolute path to an existing directory. It is canonicalized before launch.
    pub working_directory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunSummary {
    pub id: i64,
    pub project_id: String,
    pub task_id: i64,
    pub agent: RunAgent,
    pub agent_version: String,
    pub agent_executable: String,
    pub working_directory: String,
    pub input_task_revision: i64,
    pub status: RunStatus,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub exit_code: Option<i32>,
    pub stop_reason: Option<String>,
    pub cancel_requested_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunList {
    pub runs: Vec<RunSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunDetail {
    pub run: RunSummary,
    /// Bounded tail of raw agent output. Truncation is explicit.
    pub log: String,
    pub log_truncated: bool,
    /// Human-readable progress extracted from runtime output where recognized.
    pub progress: String,
    /// Results the launched agent recorded through Dictum MCP during this run.
    pub results: Vec<TaskResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunConfig {
    pub token: String,
    pub timeout_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTask {
    /// Existing project ID from list_projects. Scope is not authorization or a work claim.
    pub project_id: String,
    /// Immutable after creation. Defaults to one_time (todo); recurring defaults to active.
    #[serde(default)]
    pub kind: TaskKind,
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
    pub kind: Option<TaskKind>,
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
    /// Filter by explicit context or output links, not implicit always sources.
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
