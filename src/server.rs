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
        description = "Create a project with a name, description, server-assigned UUIDv7 ID, and UTC creation time. Names need not be unique. Create only when explicitly intended; discovery with no match does not authorize automatic creation. A project is a scope for records, not authorization to execute work or a work claim.",
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
        description = "Discover projects by optional name/description query. Returns IDs, names, descriptions, and creation times. Resolve ambiguity or no match with the human; never implicitly choose or create a project. Once resolved, pass its project_id to list_tasks with ready_to_start=true, then get_task_context for the chosen task. Project selection is scope, not execution authorization or a work claim.",
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
        description = "Requires an existing project_id; links stay within the project. Create a self-contained brief and context links atomically. Optional priority is low, medium or high and defaults to medium for both task kinds; it does not affect lifecycle or readiness. Immutable kind defaults to one_time with status todo; recurring uses the same brief and task ID across executions and defaults to active. Optional nonblank trigger_event is recurring-only external-event metadata, not a schedule, subscription or authorization. Dependencies must target one_time tasks. This registers work; it does not launch an agent.",
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
        description = "Requires an existing project_id. Discover compact task briefs by kind, exact priority, status, readiness, exact trigger_event label, or text query. Results include priority and unmet dependencies, ordered high, medium, low with ascending task ID within each priority. Filters preserve that order. ready_to_start means one_time todo with all before-start dependencies done; completion gates do not block starting. Priority never changes readiness. Recurring tasks are never ready_to_start: discover explicitly with kind=recurring and status=active. Active is eligibility metadata, not a run in progress, installed schedule or authorization.",
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
        description = "Requires an existing project_id. Load the full brief, progress, dependency state, current decisions and deduplicated context-linked plus always sources. Inline documents are full and untruncated; external sources include locator/action/state. Execution history is bounded to latest_result and latest_successful_result (success/no_change); identical IDs identify the same entry. outstanding_feedback always includes every unaddressed comment/change request and its original result, even outside bounded history. Read it before rework. Results include feedback and addressing links. Expand output Source IDs with get_source; use list_task_results for older history. Reading never records execution or advances freshness.",
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
        description = "Requires an existing project_id and latest expected_revision. Kind is immutable. Omitted fields preserve values; optional priority replaces low, medium or high under the same revision check and does not change lifecycle or readiness. Null clears nullable text/trigger_event; supplied relationship arrays replace that set ([] clears). One_time uses todo/in_progress/blocked/done: entering in_progress checks start gates; done requires satisfied start/completion gates, at least one successful execution result (success or no_change), and no outstanding changes_requested feedback. Task edits do not clear feedback. Recurring uses active/paused; trigger_event is recurring-only metadata. Execution results and output sources are append-only through record_task_result. Dependencies must target one_time tasks. Evidence is agent-reported, not independently verified.",
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
        description = "Append an execution result for either task kind using existing project_id, task_id and latest expected_revision. outcome is success/no_change/failure; evidence is nonblank, at most 4000 characters. Optional addressed_feedback_ids explicitly reports addressing outstanding feedback on this same task; only success/no_change can address it. Describe the fixes and verification in evidence. New results do not implicitly clear feedback; addressing is reported, not human acceptance. Output Source and feedback associations append atomically with a task revision increment. Success/no_change requires all dependency gates done; failure can report unmet gates. Recording never changes lifecycle, including when recurring is paused. Historical source identities persist but contents remain mutable. No scheduling, claiming, exactly-once execution or automatic completion.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn record_task_result(
        &self,
        Parameters(input): Parameters<RecordTaskResult>,
    ) -> Result<Json<TaskResult>, String> {
        self.database(move |store| store.record_task_result(input))
            .await
    }

    #[tool(
        description = "Append feedback on a specific execution result using existing project_id, task_id, result_id and latest expected_revision. body is a nonblank Markdown explanation, at most 4000 characters. disposition defaults to comment; changes_requested explains why the result is unacceptable and blocks one_time done until a later success/no_change explicitly addresses it. Changes requested on a done task atomically reopen it to todo; other statuses and recurring active/paused remain unchanged. Feedback increments the task revision, survives task edits/new attempts, and appears with the original result in outstanding_feedback until addressed. Historical result feedback is retained. This records reported review, not authenticated human approval or another execution.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn record_task_feedback(
        &self,
        Parameters(input): Parameters<RecordTaskFeedback>,
    ) -> Result<Json<TaskFeedback>, String> {
        self.database(move |store| store.record_task_feedback(input))
            .await
    }

    #[tool(
        description = "Read append-only execution history for an existing project_id and task_id, newest IDs first. page_size defaults to 20 and must be 1..100. Pass next_before_id as before_id to retrieve older entries; the cursor must identify an existing entry in the same task/project. Descending keyset membership stays stable when clients append; restart from the first page for new results. Entries include evidence, time, revision, output Source IDs, all feedback on that result and addressed_feedback_ids. Feedback annotations and addressing state reflect the current read snapshot; execution evidence is immutable. get_source expands complete source content.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_task_results(
        &self,
        Parameters(input): Parameters<ListTaskResults>,
    ) -> Result<Json<TaskResultList>, String> {
        self.database(move |store| store.list_task_results(input))
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Discover compact source descriptors without document bodies. Filter by explicitly linked task, inline/external representation, action_due, or name/description query. Task filtering includes context and output links but not unrelated always sources. Expand relevant sources with get_source. Manual actions are never automatically due.",
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
