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
        description = "Create a project with a name, description, server-assigned UUIDv7 ID, revision 1, and UTC creation/update times. Names need not be unique. Create only when explicitly intended; discovery with no match does not authorize automatic creation. A project is a scope for records, not authorization to execute work or a work claim.",
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
        description = "Discover projects by optional name/description query. Returns IDs, names, descriptions, the compact project.topics directory, revisions, and UTC creation/update times. Topic creation does not advance the description revision. Use the latest revision as expected_revision for update_project. Resolve ambiguity or no match with the human; never implicitly choose or create a project. Once resolved, pass its project_id to list_tasks with ready_to_start=true, then get_context for the chosen task. Project selection is scope, not execution authorization or a work claim.",
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
        description = "Replace an existing project's description using project_id and the latest expected_revision from list_projects. Description is required and nonblank; null and unknown fields are rejected. Each accepted update, including identical text, increments revision and sets server UTC updated_at. Preserves ID, name, created_at and all related records. Stale revisions fail without mutation; reread and reassess before retrying. Project scope is not execution authorization or a work claim.",
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
        description = "Create a task in an existing project_id with its brief, required nonempty topics, and context links atomically. Reuse specific names from project.topics before creating new concepts (User authentication rather than authentication). Topic names are reused or created within this project, ignoring case and whitespace; duplicates are deduplicated. Optional parent_task_id attaches it beneath a same-kind root in this project; subtasks cannot have children. Put shared goals, constraints and overall approach in the parent; write child-specific work, design and acceptance criteria in the subtask. Avoid duplicating shared text and read both briefs together through get_context. Parentage is organizational: lifecycle, priority, context sources and dependencies stay explicit on each task. Priority defaults to medium. Immutable kind defaults to one_time (todo); recurring defaults to active and reuses one task across executions. Optional nonblank trigger_event is recurring-only external-event metadata, not a schedule or authorization. Dependencies must target one_time tasks. Creation starts task revision 1 and does not execute work.",
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
        description = "Requires an existing project_id. Discover compact task summaries with topic memberships by kind, exact priority, status, readiness, exact trigger_event label, topic, or text query. Optional topic matches a project-local name ignoring case and whitespace; unknown names match nothing and blank names are invalid. It combines with every other filter. Omit parent_task_id for all tasks, pass null for roots, or an existing task ID for its direct children; these filters combine with the others. Summaries retain their compact shape; get_context exposes parentage. Results sort high, medium, low priority, then ascending ID. ready_to_start means one_time todo with all explicit before-start dependencies done; parentage and completion gates do not block starting. Recurring tasks are never ready_to_start: discover with kind=recurring,status=active. Active is eligibility metadata, not a schedule or authorization.",
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
        description = "Read one consistent project context using an existing project_id. Returns the full project with its compact project.topics directory, all current decisions and always sources. Record topics are included. Topic membership supports discovery through list filters; it does not expand context or inherit sources. Optional task_id adds its full brief, parent_task, progress, dependency state, explicit context sources and bounded comment history. Without a task, task/parent/dependency_state and latest execution fields are null and recent_comments is empty. Read parent and child briefs together; parent sources, dependencies and comments are not inherited. Inline sources are full and untruncated; keep always sources short and task links relevant. recent_comments holds the ten newest comments; latest_execution and latest_successful_execution may repeat their IDs. Historical task briefs are not retained; compare comment task_revision with the current task and assess whether evidence applies. Fetch older comments/replies or output Sources only as needed with list_task_comments, get_task_comment or get_source. Poll comments independently of task revisions. Reading never records work or advances freshness.",
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
        description = "Requires an existing project_id and latest expected_revision. Optional nonempty topics replaces membership atomically, reusing or creating project-local names ignoring case and whitespace. Omission preserves topics; empty arrays and null are rejected. Every accepted update advances task revision and updated_at, including identical values; appending comments does neither. Kind is immutable. Omitted fields preserve values; null clears nullable text/trigger_event. For parent_task_id, null detaches and an ID reparents beneath a same-kind root in this project; a task with children cannot become a child. Parentage has no implicit lifecycle or dependency effects. Supplied context/dependency arrays replace that set ([] clears). Entering one_time in_progress checks start gates; done requires all start/completion gates satisfied and at least one success/no_change execution comment, including historical task versions. Recurring uses active/paused; trigger_event is recurring-only metadata. Comments never reopen or complete tasks; request rework by explicitly editing the brief or status. Dependencies must target one_time tasks.",
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
        description = "Append only information useful to later agents: a new finding, correction, blocker, actionable next step or execution evidence. Prefer 1-3 short sentences/bullets; omit routine narration, duplicated brief/metadata and empty polling. For execution_result, identify work/checks, tested implementation or artifact, outcome and remaining limitations. No_change can supply useful fresh verification; honor explicit reporting requirements and retain applicable success/no_change evidence before marking done. Link long durable artifacts as output Sources only when needed. Requires existing project_id, task_id and task_revision actually read (1..current); historical targets remain valid without retained brief snapshots. body is nonblank Markdown, max 4000 characters, a limit not a target. reply_to_comment_id must reference this task. Optional execution_result has outcome success/no_change/failure and same-project output_source_ids. Success/no_change checks all current dependency gates; failure/ordinary comments may report unmet gates. Appends and output links are atomic; task revision, updated_at and lifecycle never change, including done/paused tasks. Output Sources remain mutable. Evidence is reported, not independently verified.",
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
        description = "Register reusable information in an existing project_id with a name, relevance description, required nonempty topics, and exactly one of full inline Markdown content or external locator. Reuse specific names from project.topics first; missing names are created atomically within this project, ignoring case and whitespace. context_policy defaults to when_relevant; always sources appear in every task context. Reserve always for concise essentials, such as an environment-facts inventory with scope, exact identifiers and verification references/dates where useful. Facts describe current state; decisions record accepted choices. Keep long artifacts when_relevant and link only needed task context. Create Sources for reusable information or durable outputs, not routine notes. Optional action defines agent work with a manual or interval trigger; keep instructions focused and snapshots concise. Registration does not create the external resource.",
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
        description = "Discover source descriptors newest ID first within an existing project_id, without document bodies. page_size defaults to 20 (1..100); pass next_before_id as before_id for older pages. The cursor must identify a source in this project regardless of filters. Filter by topic, explicit task context links, representation, action_due or query. Optional topic matches a project-local name ignoring case and whitespace; unknown names match nothing and blank names are invalid. Descriptors include topic memberships; outputs are associated with task comments. Filters apply before the page limit. Later appends do not enter older ID pages; source edits or time-dependent due state can change filter membership. Expand needed content with get_source. Manual actions are never automatically due.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_sources(
        &self,
        Parameters(input): Parameters<ListSources>,
    ) -> Result<Json<SourceList>, String> {
        self.database(move |store| store.list_sources(input)).await
    }

    #[tool(
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Read topic memberships and complete inline source content or external locator, configured action, latest successful snapshot, latest attempt, and computed due state. Content is never truncated. A snapshot is a short current understanding, not a transcript. Use the agent's own external tools for further reading. This read does not record action completion.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Update a source definition with expected_revision. Optional nonempty topics replaces membership atomically, reusing or creating specific project-local names ignoring case and whitespace; omission preserves topics, empty arrays and null are rejected. Topic-only edits preserve action state. Omitted fields preserve values; null clears content, locator, or action. Exactly one of content/locator must remain; switch them in one call. Changing content, locator, or action invalidates its prior snapshot/attempt. Name, description, and context-policy edits preserve action state.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Record an agent's execution of the configured source action using expected_revision. Success replaces the snapshot with observed_at (RFC3339, not future or backwards), short summary (max 1000 characters), checkpoint and references. No_change requires a previous snapshot, advances observation time, and preserves summary/references; omitted checkpoint preserves it, null clears. Failure records an error while retaining the successful snapshot. Targeted task lookups must not count as completing this action. The server does not execute or independently verify it.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Record an accepted project-wide decision with required nonempty topics, a complete current statement and rationale, optional nullable Markdown alternatives_rejected, and server-assigned UTC creation time. Omitted or null alternatives_rejected stores null; empty text is allowed. Reuse specific names from project.topics first; missing names are created atomically in this project, ignoring case and whitespace. Decisions and their topic memberships are immutable; replacements supply their own topics. Optional supersedes_id replaces a current decision, preserving its original fields; each successor has its own alternatives_rejected value. A reversal creates a new decision with today's reasons. Each current decision can have only one successor. Results include server-derived current_decision_id, equal to the new record's id at creation. Replacing an already superseded decision fails with its current_decision_id; reread the current decision before retrying. This is not a proposal or voting workflow.",
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
        description = "Read full accepted decisions with topic memberships newest ID first in an existing project_id. Optional topic matches a project-local name ignoring case and whitespace and combines with history filtering before pagination; unknown names match nothing and blank names are invalid. page_size defaults to 20 (1..100); pass next_before_id as before_id for older pages. The cursor must identify a decision in this project, even if excluded by the current/history filter. Current decisions are the default; include_superseded also returns history with supersession links and each record's rationale and nullable alternatives_rejected. Each record includes server-derived current_decision_id identifying the current end of its entire chain at read time, even outside this page; it equals id when is_current is true. Follow that current decision for accepted intent. Filtering applies before the page limit. Later appends do not enter older ID pages, but supersession can change current membership and current_decision_id on subsequent reads. get_context always includes all current decisions, independently of pagination.",
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
