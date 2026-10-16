//! A synchronous test facade over an actual child process and its MCP stdio transport.
//! No store calls or in-process server shortcuts: every operation crosses the wire.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, ensure};
use dictum_mcp::model::*;
use rmcp::{
    ClientLifecycleMode, ClientServiceExt, RoleClient,
    model::{CallToolRequestParams, ClientConfig, ProtocolVersion},
    service::RunningService,
    transport::TokioChildProcess,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

pub struct Project {
    runtime: tokio::runtime::Runtime,
    client: Option<RunningService<RoleClient, ClientConfig>>,
}

impl Project {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_dictum-mcp"));
        command
            .env("DICTUM_DATABASE", path.as_ref())
            .env("TOKIO_WORKER_THREADS", "2");
        let client = runtime.block_on(async {
            let transport = TokioChildProcess::new(command)?;
            Ok::<_, anyhow::Error>(
                tokio::time::timeout(
                    Duration::from_secs(15),
                    ClientConfig::default().serve_with_lifecycle(
                        transport,
                        ClientLifecycleMode::Discover {
                            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                        },
                    ),
                )
                .await??,
            )
        })?;
        Ok(Self {
            runtime,
            client: Some(client),
        })
    }

    pub fn call<T: DeserializeOwned>(&self, name: &str, input: impl Serialize) -> Result<T> {
        let input = serde_json::to_value(input)?;
        let request = CallToolRequestParams::new(name.to_owned()).with_arguments(
            input
                .as_object()
                .context("tool input must be an object")?
                .clone(),
        );
        let client = self.client.as_ref().context("client already closed")?;
        let result = self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(15), client.call_tool(request)).await
        })??;
        ensure!(
            result.is_error != Some(true),
            "{name}: {:?}",
            result.content
        );
        Ok(serde_json::from_value(
            result
                .structured_content
                .context("missing structured result")?,
        )?)
    }

    pub fn create_task(&self, input: CreateTask) -> Result<Task> {
        self.call("create_task", input)
    }
    pub fn update_task(&self, input: UpdateTask) -> Result<Task> {
        self.call("update_task", input)
    }
    pub fn get_task_context(&self, task_id: i64) -> Result<TaskContext> {
        self.call("get_task_context", json!({"task_id": task_id}))
    }
    pub fn list_tasks(&self, input: ListTasks) -> Result<Vec<TaskSummary>> {
        Ok(self.call::<TaskList>("list_tasks", input)?.tasks)
    }
    pub fn create_source(&self, input: CreateSource) -> Result<Source> {
        self.call("create_source", input)
    }
    pub fn update_source(&self, input: UpdateSource) -> Result<Source> {
        self.call("update_source", input)
    }
    pub fn get_source(&self, source_id: i64) -> Result<Source> {
        self.call("get_source", json!({"source_id": source_id}))
    }
    pub fn list_sources(&self, input: ListSources) -> Result<Vec<SourceDescriptor>> {
        Ok(self.call::<SourceList>("list_sources", input)?.sources)
    }
    pub fn record_source_action(&self, input: RecordSourceAction) -> Result<Source> {
        self.call("record_source_action", input)
    }
    pub fn record_decision(&self, input: RecordDecision) -> Result<Decision> {
        self.call("record_decision", input)
    }
    pub fn list_decisions(&self, input: ListDecisions) -> Result<Vec<Decision>> {
        Ok(self
            .call::<DecisionList>("list_decisions", input)?
            .decisions)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            let _ = self.runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(5), client.cancel()).await
            });
        }
    }
}
