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
        description = "Discover projects by optional name/description query. Returns IDs, names, descriptions, revisions, and UTC creation/update times. Use the latest revision as expected_revision for update_project. Resolve ambiguity or no match with the human; never implicitly choose or create a project. Once resolved, pass its project_id to list_tasks with ready_to_start=true, then get_task_context for the chosen task. Project selection is scope, not execution authorization or a work claim.",
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
        description = "Create a task in an existing project_id with its brief and context links atomically. Optional parent_task_id attaches it beneath a same-kind root in this project; subtasks cannot have children. Put shared goals, constraints and overall approach in the parent; write child-specific work, design and acceptance criteria in the subtask. Avoid duplicating shared text and read both briefs together through get_task_context. Parentage is organizational: lifecycle, priority, context sources and dependencies stay explicit on each task. Priority defaults to medium. Immutable kind defaults to one_time (todo); recurring defaults to active and reuses one task across executions. Optional nonblank trigger_event is recurring-only external-event metadata, not a schedule or authorization. Dependencies must target one_time tasks. Creation starts task revision 1 and does not execute work.",
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
        description = "Requires an existing project_id. Discover compact task summaries by kind, exact priority, status, readiness, exact trigger_event label, or text query. Omit parent_task_id for all tasks, pass null for roots, or an existing task ID for its direct children; these filters combine with the others. Summaries retain their compact shape; get_task_context exposes parentage. Results sort high, medium, low priority, then ascending ID. ready_to_start means one_time todo with all explicit before-start dependencies done; parentage and completion gates do not block starting. Recurring tasks are never ready_to_start: discover with kind=recurring,status=active. Active is eligibility metadata, not a schedule or authorization.",
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
        description = "Requires an existing project_id. Load the full task brief, current parent_task brief when present, progress, explicit dependency state, current decisions and deduplicated task-linked plus always sources. Read both briefs together: the parent holds shared goals, constraints and overall approach, and the subtask holds child-specific work, design and acceptance criteria. Parent sources, dependencies and comments are not inherited. Inline documents are full and untruncated. recent_comments contains the ten newest comments; latest_execution and latest_successful_execution (success/no_change) may duplicate those IDs. Each comment retains its targeted task_revision, so compare with task.revision to distinguish historical context. Historical brief snapshots are not stored. Expand output Sources with get_source and older comments/reply targets with list_task_comments or get_task_comment. Reading never records work or advances freshness.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_task_context(
        &self,
        Parameters(input): Parameters<GetTaskContext>,
    ) -> Result<Json<TaskContext>, String> {
        self.database(move |store| store.get_task_context(&input.project_id, input.task_id))
            .await
    }

    #[tool(
        description = "Requires an existing project_id and latest expected_revision. Every accepted update advances task revision and updated_at, including identical values; appending comments does neither. Kind is immutable. Omitted fields preserve values; null clears nullable text/trigger_event. For parent_task_id, null detaches and an ID reparents beneath a same-kind root in this project; a task with children cannot become a child. Parentage has no implicit lifecycle or dependency effects. Supplied relationship arrays replace that set ([] clears). Entering one_time in_progress checks start gates; done requires all start/completion gates satisfied and at least one success/no_change execution comment, including historical task versions. Recurring uses active/paused; trigger_event is recurring-only metadata. Comments never reopen or complete tasks; request rework by explicitly editing the brief or status. Dependencies must target one_time tasks.",
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
        description = "Append an immutable task comment using existing project_id, task_id and required task_revision targeting any version from 1 through the current task revision. Historical versions are valid even if the task changes concurrently; no historical brief snapshot is stored. body is nonblank Markdown, at most 4000 characters. Optional reply_to_comment_id references an existing comment on this same task. Optional execution_result contains outcome success/no_change/failure and output_source_ids; body then supplies reported execution evidence. Success/no_change requires all current dependency gates done, even for historical targets; failure and ordinary comments may report unmet gates. Output associations append atomically. Comments never change task version, updated_at or lifecycle, including when done or paused. Source identities persist but their contents remain mutable. Evidence is reported, not independently verified.",
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
        description = "Read task comments newest ID first using existing project_id and task_id. page_size defaults to 20 (1..100). Pass next_before_id as before_id for older entries; the cursor must identify a comment on this same task/project. Omit has_execution_result for all comments, use true for executions or false for ordinary comments. Cursor validation is independent of that filter. Later appends do not alter subsequent page membership. Comments include their targeted task_revision, full body, optional reply ID and optional execution result with output Source IDs. Replies remain flat; get_task_comment expands a target directly and get_source expands output content. Reads never mutate task state.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Register reusable project information with a name, relevance description, and exactly one of full inline Markdown content or external locator. context_policy defaults to when_relevant; always sources appear in every task context. Optional action defines work for an agent and a manual or interval trigger. Registration does not create any external channel or document.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Discover compact source descriptors without document bodies. Filter by task context links, inline/external representation, action_due, or name/description query. Task filtering covers explicit context links; execution output associations are available through task comments. Expand relevant sources with get_source. Manual actions are never automatically due.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_sources(
        &self,
        Parameters(input): Parameters<ListSources>,
    ) -> Result<Json<SourceList>, String> {
        self.database(move |store| {
            Ok(SourceList {
                sources: store.list_sources(input)?,
            })
        })
        .await
    }

    #[tool(
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Read complete inline source content or external locator, configured action, latest successful snapshot, latest attempt, and computed due state. Content is never truncated. A snapshot is a short current understanding, not a transcript. Use the agent's own external tools for further reading. This read does not record action completion.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Update a source definition with expected_revision. Omitted fields preserve values; null clears content, locator, or action. Exactly one of content/locator must remain; switch them in one call. Changing content, locator, or action invalidates its prior snapshot/attempt. Name, description, and context-policy edits preserve action state.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Record an accepted project-wide decision with its rationale, optional nullable Markdown alternatives_rejected, and server-assigned UTC creation time. Omitted or null alternatives_rejected stores null; empty text is allowed. Decisions are immutable. Optional supersedes_id replaces a currently active decision, preserving its original fields; each successor has its own alternatives_rejected value. Each current decision can have only one successor. This is not a proposal or voting workflow.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Read accepted project decisions, including each decision's nullable alternatives_rejected Markdown. By default returns only current decisions; include_superseded also returns retained history with current flags, supersedes links and each predecessor's original alternatives_rejected value.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_decisions(
        &self,
        Parameters(input): Parameters<ListDecisions>,
    ) -> Result<Json<DecisionList>, String> {
        self.database(move |store| {
            Ok(DecisionList {
                decisions: store.list_decisions(input)?,
            })
        })
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
