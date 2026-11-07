//! MCP boundary: descriptive tools and structured results over the local store.

use rmcp::{
    Json, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};

use crate::{
    model::*,
    store::{Store, StoreResult},
};

#[derive(Clone)]
pub struct DictumServer {
    store: Store,
    tool_router: ToolRouter<Self>,
}

impl DictumServer {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            tool_router: Self::tool_router(),
        }
    }

    // SQLite may wait on another local writer. Keep that work off the async reactor.
    async fn database<T, F>(&self, operation: F) -> Result<Json<T>, String>
    where
        T: Send + 'static,
        F: FnOnce(Store) -> StoreResult<T> + Send + 'static,
    {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || operation(store))
            .await
            .map_err(|error| format!("database worker failed: {error}"))?
            .map(Json)
            .map_err(|error| error.to_string())
    }
}

#[tool_router]
impl DictumServer {
    #[tool(
        description = "Create an explicitly intended project with a name and description. Names need not be unique. Returns a server-assigned ID, revision 1 and equal UTC creation/update times.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_project(
        &self,
        Parameters(input): Parameters<CreateProject>,
    ) -> Result<Json<Project>, String> {
        self.database(move |store| store.create_project(input))
            .await
    }

    #[tool(
        description = "Discover projects by optional name/description query. Returns unpaginated projects with IDs, names, descriptions, complete project.topics directories, revisions and UTC creation/update times. Topic creation leaves the description revision unchanged. Use the latest revision as expected_revision for update_project. Resolve ambiguous or missing matches with the human before selecting or creating a project.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_projects(
        &self,
        Parameters(input): Parameters<ListProjects>,
    ) -> Result<Json<ProjectList>, String> {
        self.database(move |store| {
            Ok(ProjectList {
                projects: store.list_projects(input)?,
            })
        })
        .await
    }

    #[tool(
        description = "Replace only an existing project's description using project_id and the latest expected_revision. Description is required and nonblank; null and unknown fields are rejected. Every accepted update, including identical text, increments revision and sets server UTC updated_at. Preserves ID, name, created_at and related records. Stale revisions fail without mutation; reread and reassess before retrying.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn update_project(
        &self,
        Parameters(input): Parameters<UpdateProject>,
    ) -> Result<Json<Project>, String> {
        self.database(move |store| store.update_project(input))
            .await
    }

    #[tool(
        description = "Create a task's brief, optional topics and context links atomically in an existing project_id. Topics default to []; null and blank names fail. Reuse project.topics names when helpful; missing names are created within this project, ignoring case/whitespace and deduplicating repeats. Optional parent_task_id requires a same-kind root in this project; subtasks cannot have children. Put shared goals, constraints and approach in the parent, specific work/design/criteria in the child; read both briefs through get_context. Lifecycle, priority, context sources and dependencies remain explicit on each task. Priority defaults to medium. Immutable kind defaults to one_time (todo); recurring defaults to active and reuses one task across executions. Optional nonblank trigger_event is recurring-only external-event metadata, not a schedule or authorization. Dependencies target one_time tasks. Creation starts revision 1 without executing work.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_task(
        &self,
        Parameters(input): Parameters<CreateTask>,
    ) -> Result<Json<Task>, String> {
        self.database(move |store| store.create_task(input)).await
    }

    #[tool(
        description = "Discover compact task summaries with topics in an existing project_id. Filters combine: kind, exact priority, status, readiness, exact trigger_event label, topic and text query. Topic matches project-local names ignoring case/whitespace; unknown names match nothing and blank names are invalid. Omit parent_task_id for all tasks, pass null for roots or an existing task ID for direct children. Use get_context for full briefs and parentage. Results sort high, medium, low priority, then ascending ID. ready_to_start selects one_time todo with all before-start dependencies done; parentage and completion gates do not block starting. For recurring work use kind=recurring,status=active. Active is eligibility metadata, not a schedule or authorization.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_tasks(
        &self,
        Parameters(input): Parameters<ListTasks>,
    ) -> Result<Json<TaskList>, String> {
        self.database(move |store| {
            Ok(TaskList {
                tasks: store.list_tasks(input)?,
            })
        })
        .await
    }

    #[tool(
        description = "Read one consistent snapshot using an existing project_id: full project with complete project.topics directory, all current decisions and always sources. Record topics are included; membership supports discovery, without expanding context or inheriting sources. Optional task_id adds full task/parent briefs, progress, dependency state, explicit context sources and bounded comments. Without a task, task/parent_task/dependency_state and latest execution fields are null; recent_comments is empty. Read both briefs; parent sources, dependencies and comments are not inherited. Inline sources are returned in full; keep always sources short and task links relevant. recent_comments holds ten newest comments; latest_execution and latest_successful_execution may repeat their IDs. Historical briefs are not retained: compare comment task_revision with the current task and assess evidence applicability. Fetch older comments/replies or outputs as needed with list_task_comments, get_task_comment or get_source. Poll comments independently of task revisions. Reading never records work or advances freshness.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_context(
        &self,
        Parameters(input): Parameters<GetContext>,
    ) -> Result<Json<Context>, String> {
        self.database(move |store| store.get_context(&input.project_id, input.task_id))
            .await
    }

    #[tool(
        description = "Update a task using existing project_id and latest expected_revision. Every accepted update advances revision/updated_at, including identical values. Kind is immutable. Omitted fields preserve values; null clears nullable text/trigger_event. Supplied topics atomically replace membership, reusing or creating project-local names ignoring case/whitespace; [] clears membership; null and blank names are rejected. For parent_task_id, null detaches; an ID reparents beneath a same-kind root in this project. A task with children cannot become a child; parentage has no implicit lifecycle or dependency effects. Supplied context/dependency arrays replace that set ([] clears). Entering one_time in_progress checks start gates; done requires satisfied start/completion gates and at least one success/no_change execution comment targeting any task version. Recurring uses active/paused; trigger_event is recurring-only metadata. Comments preserve task revision/time/status; request rework by editing the brief or status. Dependencies target one_time tasks.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_task(
        &self,
        Parameters(input): Parameters<UpdateTask>,
    ) -> Result<Json<Task>, String> {
        self.database(move |store| store.update_task(input)).await
    }

    #[tool(
        description = "Every comment, with or without execution_result, must help later humans or agents understand, review or resume the task. Use 1-3 short sentences/bullets with new findings, corrections, blockers, next steps or evidence. Executions identify work/checks, tested implementation or artifact, outcome and relevant limitations. Honor required reports and completion evidence; fresh verification can warrant no_change. Link long artifacts as output Sources when needed. Requires existing project_id, task_id and task_revision actually read (1..current); historical targets remain valid without retained brief snapshots. body is nonblank Markdown, max 4000 characters, a limit not a target. reply_to_comment_id must reference this task. Optional execution_result has outcome success/no_change/failure and same-project output_source_ids. Success/no_change checks all current dependency gates; failure/ordinary comments may report unmet gates. Appends and output links are atomic; task revision, updated_at and lifecycle stay unchanged, including done/paused tasks. Output Sources remain mutable. Evidence is reported, not independently verified.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn record_task_comment(
        &self,
        Parameters(input): Parameters<RecordTaskComment>,
    ) -> Result<Json<TaskComment>, String> {
        self.database(move |store| store.record_task_comment(input))
            .await
    }

    #[tool(
        description = "Read full task comments newest ID first in an existing project_id/task_id. Fetch history only for a concrete question; prefer small pages (e.g. page_size=5) and stop when answered. page_size defaults to 20 (1..100). Pass next_before_id as before_id for older entries; the cursor must belong to this task/project regardless of filter. Omit has_execution_result for all comments, true for executions, false for ordinary comments. Later appends preserve older page membership. Comments change without task revisions; when polling, start without before_id, then page backward to a previously seen ID or history end so bursts are not missed. Track IDs per filter to avoid reprocessing. Each entry includes task_revision, full body, optional reply ID and execution result/output Source IDs. Replies are flat; use get_task_comment for a known target and get_source for needed outputs. Reads never mutate task state.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_task_comments(
        &self,
        Parameters(input): Parameters<ListTaskComments>,
    ) -> Result<Json<TaskCommentList>, String> {
        self.database(move |store| store.list_task_comments(input))
            .await
    }

    #[tool(
        description = "Read one complete immutable comment using existing project_id, task_id and comment_id. The comment must belong to this task/project. Returns full Markdown body, targeted task_revision, recording time, optional reply_to_comment_id and optional execution result with output Source IDs. Reply ancestors are not expanded recursively; call this tool for an earlier target and get_source for output documents. A task revision identifies its version but historical brief snapshots are not retained.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_task_comment(
        &self,
        Parameters(input): Parameters<GetTaskComment>,
    ) -> Result<Json<TaskComment>, String> {
        self.database(move |store| store.get_task_comment(input))
            .await
    }

    #[tool(
        description = "Register reusable information or a durable output in an existing project_id with name, description, optional topics (default []), and exactly one of inline Markdown content or external locator. Reuse project.topics when helpful; missing names are created atomically, ignoring case and whitespace. Null arrays and blank names fail. context_policy defaults to when_relevant. always Sources enter every context; reserve them for concise essentials such as environment scope, exact identifiers and verification dates/references. Facts describe current state; decisions record accepted choices. Keep long artifacts when_relevant; link needed task context. Optional action defines agent work with a manual or interval trigger. Registration does not create an external resource.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_source(
        &self,
        Parameters(input): Parameters<CreateSource>,
    ) -> Result<Json<Source>, String> {
        self.database(move |store| store.create_source(input)).await
    }

    #[tool(
        description = "List source descriptors with topics, without bodies, newest ID first in project_id. page_size defaults to 20 (1..100); pass next_before_id as before_id for older pages. Cursors must identify a Source in this project regardless of filters. Filter by topic, explicit task context links, representation, action_due or query before pagination. Topic names ignore case/whitespace; unknown names match nothing and blanks fail. Outputs belong to task comments. Later appends stay outside older pages; edits and due state can change membership. Expand needed content with get_source. Manual actions are never automatically due.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_sources(
        &self,
        Parameters(input): Parameters<ListSources>,
    ) -> Result<Json<SourceList>, String> {
        self.database(move |store| store.list_sources(input)).await
    }

    #[tool(
        description = "Read a Source in project_id: topics, full inline content or external locator, action, latest successful snapshot, latest attempt and computed due state. A snapshot summarizes current understanding. Use external tools for further reading. Reading leaves action state unchanged.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_source(
        &self,
        Parameters(input): Parameters<GetSource>,
    ) -> Result<Json<Source>, String> {
        self.database(move |store| store.get_source(&input.project_id, input.source_id))
            .await
    }

    #[tool(
        description = "Update a Source in project_id with expected_revision. Supplied topics replace membership atomically, reusing or creating project-local names ignoring case/whitespace; [] clears membership; null and blank names fail. Omitted fields preserve values; null clears content, locator or action. Exactly one of content/locator must remain; switch them in one call. Changing content, locator or action clears its snapshot/attempt. Topic, name, description and context_policy edits preserve action state.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_source(
        &self,
        Parameters(input): Parameters<UpdateSource>,
    ) -> Result<Json<Source>, String> {
        self.database(move |store| store.update_source(input)).await
    }

    #[tool(
        description = "Record an execution attempt of the configured Source action in project_id using expected_revision. Success replaces the snapshot with observed_at (RFC3339, neither future nor backwards), summary (max 1000 characters), checkpoint and references. No_change requires a snapshot, advances observation time and preserves summary/references; omitted checkpoint preserves it, null clears. Failure records an error while retaining the snapshot. Only completing the configured action advances freshness; targeted task lookups do not. Execution and evidence are agent-reported.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn record_source_action(
        &self,
        Parameters(input): Parameters<RecordSourceAction>,
    ) -> Result<Json<Source>, String> {
        self.database(move |store| store.record_source_action(input))
            .await
    }

    #[tool(
        description = "Record an accepted project-wide choice in an existing project_id with optional topics (default []), a complete current statement, rationale and optional Markdown alternatives_rejected. Keep proposals in discussion until accepted. Omitted or null alternatives_rejected stores null; empty text is allowed. Reuse project.topics when helpful; missing names are created atomically, ignoring case and whitespace. Null arrays and blank names fail. Decisions and topic memberships are immutable. An accepted change or reversal creates a replacement with supersedes_id, its own optional topics (no inheritance), alternatives and current reasons, preserving the predecessor. Each current decision can have only one successor. Returns server UTC created_at and server-derived current_decision_id (the new record's id). Stale replacements fail with the chain's current_decision_id; reread before retrying.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn record_decision(
        &self,
        Parameters(input): Parameters<RecordDecision>,
    ) -> Result<Json<Decision>, String> {
        self.database(move |store| store.record_decision(input))
            .await
    }

    #[tool(
        description = "Read full accepted decisions with topic memberships newest ID first in an existing project_id. Optional topic matches a project-local name ignoring case and whitespace and combines with history filtering before pagination; unknown names match nothing and blank names are invalid. page_size defaults to 20 (1..100); pass next_before_id as before_id for older pages. The cursor must identify a decision in this project, even if excluded by the current/history filter. Current decisions are the default; include_superseded also returns history with supersession links and each record's rationale and nullable alternatives_rejected. Server-derived current_decision_id locates the latest recorded intent in each chain, even outside this page; it equals id when is_current is true. Later appends do not enter older ID pages, but supersession can change current membership and current_decision_id on subsequent reads. get_context always includes all current decisions, independently of pagination.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_decisions(
        &self,
        Parameters(input): Parameters<ListDecisions>,
    ) -> Result<Json<DecisionList>, String> {
        self.database(move |store| store.list_decisions(input))
            .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DictumServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("dictum-mcp", env!("CARGO_PKG_VERSION")))
            // Embed guidance so the binary needs no runtime Markdown file or retrieval tool.
            .with_instructions(include_str!("../WORKFLOW.md"))
    }
}
