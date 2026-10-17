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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Create a self-contained execution brief with goal/problem, design, and a nonempty acceptance-criteria list. Optionally link context sources and tasks that must finish before starting or before completing. All links are created atomically. This registers work; it does not launch an agent.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Discover compact task briefs by status, readiness, or text query. Results include unmet start and completion dependencies. Ready means todo with all before-start dependencies done; completion gates do not block starting.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Load the full task brief, progress, agent-reported result, dependency state, current decisions, and deduplicated context-linked plus always sources. Inline documents are returned in full. External sources include locator, action, snapshot, latest attempt, and due state. Output links are separate. Reading never advances freshness. A fresh summary may still require targeted external reading.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Update a task using expected_revision from its latest read. Omitted fields preserve values; null clears nullable text. Supplied relationship arrays replace that set ([] clears). Entering in_progress checks start gates; done requires satisfied start/completion gates and nonblank result evidence. Evidence is reported by the agent, not verified by this server. Tasks never complete automatically.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Record an accepted project-wide decision with its rationale and server-assigned UTC creation time. Optional supersedes_id replaces a currently active decision, preserving history. Each current decision can have only one successor. This is not a proposal or voting workflow.",
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
        description = "Requires an explicit existing project_id; all records and relationships stay within that project. Read accepted project decisions. By default returns only current decisions; include_superseded also returns retained history with current flags and supersedes links.",
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
