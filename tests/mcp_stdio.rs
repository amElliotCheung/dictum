//! Exercise the actual binary over MCP stdio, not direct calls to server methods.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, ensure};
use chrono::{TimeDelta, Utc};
use rmcp::{
    ClientLifecycleMode, ClientServiceExt, RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ClientConfig, ProtocolVersion},
    service::RunningService,
    transport::TokioChildProcess,
};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ClientConfig>;

async fn connect(database: &Path, discover: bool) -> Result<Client> {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_dictum-mcp"));
    command.env("DICTUM_DATABASE", database).current_dir(
        database
            .parent()
            .context("database needs a project directory")?,
    );
    let transport = TokioChildProcess::new(command)?;
    if discover {
        Ok(ClientConfig::default()
            .serve_with_lifecycle(
                transport,
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                },
            )
            .await?)
    } else {
        Ok(ClientConfig::default().serve(transport).await?)
    }
}

fn assert_workflow_instructions(client: &Client) -> Result<()> {
    let server = client
        .peer_info()
        .context("expected server handshake info")?;
    let instructions = server
        .instructions
        .as_deref()
        .context("expected embedded workflow instructions")?;
    assert_eq!(instructions, include_str!("../WORKFLOW.md"));
    assert!(!instructions.trim().is_empty());
    assert!(instructions.chars().count() <= 2_000);
    Ok(())
}

async fn raw(client: &Client, name: &str, input: Value) -> Result<CallToolResult> {
    Ok(client
        .call_tool(
            CallToolRequestParams::new(name.to_owned()).with_arguments(
                input
                    .as_object()
                    .context("tool input must be object")?
                    .clone(),
            ),
        )
        .await?)
}

async fn call(client: &Client, name: &str, input: Value) -> Result<Value> {
    let result = raw(client, name, input).await?;
    ensure!(
        result.is_error != Some(true),
        "{name} failed: {:?}",
        result.content
    );
    result
        .structured_content
        .context("expected structured output")
}

async fn workflow(discover: bool) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("project.sqlite3");
    let client = connect(&database, discover).await?;
    assert_workflow_instructions(&client)?;

    let tools = client.list_all_tools().await?;
    let mut names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "create_source",
            "create_task",
            "get_source",
            "get_task_context",
            "list_decisions",
            "list_sources",
            "list_tasks",
            "record_decision",
            "record_source_action",
            "update_source",
            "update_task",
        ]
    );
    for tool in &tools {
        assert!(
            tool.output_schema.is_some(),
            "missing output schema: {}",
            tool.name
        );
        assert!(!serde_json::to_string(&*tool.input_schema)?.contains("project_id"));
    }
    let update_schema = &tools
        .iter()
        .find(|tool| tool.name == "update_source")
        .unwrap()
        .input_schema;
    let required = update_schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("source_id")));
    assert!(required.contains(&json!("expected_revision")));
    assert!(!required.contains(&json!("action")));
    assert!(
        update_schema["properties"]["action"]
            .to_string()
            .contains("null")
    );

    let full_content = format!(
        "# Project conventions\n{}\nEND",
        "完整 context\n".repeat(4000)
    );
    let inline = call(
        &client,
        "create_source",
        json!({
            "name": "Working conventions", "description": "Read for every task",
            "content": full_content, "context_policy": "always"
        }),
    )
    .await?;
    let external = call(&client, "create_source", json!({
        "name": "Team channel", "description": "Recent project decisions and blockers",
        "locator": "slack://workspace/project-channel",
        "action": {"instruction": "Read project-relevant messages since checkpoint", "trigger": {"kind": "interval", "every_minutes": 1440}}
    })).await?;
    assert_eq!(external["action_due"], true);

    let decision = call(
        &client,
        "record_decision",
        json!({
            "statement": "One project per SQLite database/server process",
            "rationale": "Keep the first MVP local and simple"
        }),
    )
    .await?;
    assert!(decision["created_at"].as_str().unwrap().ends_with('Z'));
    let decisions = call(&client, "list_decisions", json!({})).await?;
    assert_eq!(decisions["decisions"].as_array().unwrap().len(), 1);

    let verification = call(
        &client,
        "create_task",
        json!({
            "title": "Verify implementation", "problem_or_goal": "Prove the feature works",
            "design": "Implement and run the end-to-end scenario against the final change",
            "acceptance_criteria": ["Record code revision, command, and outcome"]
        }),
    )
    .await?;
    let task = call(
        &client,
        "create_task",
        json!({
            "title": "Implement feature", "problem_or_goal": "Build a working feature",
            "design": "Use conventions and channel context; write a result document",
            "acceptance_criteria": ["Behavior matches the design", "Verification task done"],
            "context_source_ids": [external["id"]], "completion_task_ids": [verification["id"]]
        }),
    )
    .await?;
    let tasks = call(&client, "list_tasks", json!({"ready_to_start": true})).await?;
    assert_eq!(tasks["tasks"].as_array().unwrap().len(), 2);
    let context = call(&client, "get_task_context", json!({"task_id": task["id"]})).await?;
    assert_eq!(context["current_decisions"][0]["id"], decision["id"]);
    let sources = context["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(
        sources
            .iter()
            .find(|source| source["id"] == inline["id"])
            .unwrap()["content"],
        full_content
    );
    assert_eq!(
        context["dependency_state"]["unmet_completion_task_ids"],
        json!([verification["id"]])
    );

    let running = call(
        &client,
        "update_task",
        json!({
            "task_id": task["id"], "expected_revision": task["revision"], "status": "in_progress"
        }),
    )
    .await?;
    let rejected = raw(&client, "update_task", json!({
        "task_id": task["id"], "expected_revision": running["revision"], "status": "done", "result": "Implemented"
    })).await?;
    assert_eq!(rejected.is_error, Some(true));

    let observed_at = (Utc::now() - TimeDelta::seconds(1)).to_rfc3339();
    let observed = call(&client, "record_source_action", json!({
        "source_id": external["id"], "expected_revision": external["revision"],
        "result": {"outcome": "success", "observed_at": observed_at, "summary": "Team agreed on a local MVP.", "checkpoint": "message-123", "references": ["slack://workspace/project-channel/message-123"]}
    })).await?;
    assert_eq!(observed["action_due"], false);
    let edited = call(&client, "update_source", json!({
        "source_id": external["id"], "expected_revision": observed["revision"], "description": "Project team context"
    })).await?;
    assert_eq!(edited["snapshot"], observed["snapshot"]);
    let expanded = call(&client, "get_source", json!({"source_id": external["id"]})).await?;
    assert_eq!(expanded["revision"], edited["revision"]);
    let descriptors = call(&client, "list_sources", json!({"task_id": task["id"]})).await?;
    assert_eq!(descriptors["sources"].as_array().unwrap().len(), 1);
    assert!(descriptors["sources"][0].get("content").is_none());

    let output = call(&client, "create_source", json!({
        "name": "Verification report", "description": "Result and evidence for implemented feature", "content": "Revision demo-123; command cargo test; all passed."
    })).await?;
    call(&client, "update_task", json!({
        "task_id": verification["id"], "expected_revision": verification["revision"], "status": "done",
        "result": "Tested demo-123 with cargo test; all passed (test fixture evidence)", "output_source_ids": [output["id"]]
    })).await?;
    let completed = call(&client, "update_task", json!({
        "task_id": task["id"], "expected_revision": running["revision"], "status": "done",
        "result": "Implemented and verified by the completed verification task", "output_source_ids": [output["id"]]
    })).await?;
    assert_eq!(completed["status"], "done");
    let conflict = raw(&client, "update_task", json!({
        "task_id": task["id"], "expected_revision": running["revision"], "progress": "stale write"
    })).await?;
    assert_eq!(conflict.is_error, Some(true));
    client.cancel().await?;

    // A new process sees the same project without any project identifier.
    let restarted = connect(&database, discover).await?;
    assert_workflow_instructions(&restarted)?;
    let persisted = call(
        &restarted,
        "get_task_context",
        json!({"task_id": task["id"]}),
    )
    .await?;
    assert_eq!(persisted["task"]["status"], "done");
    assert_eq!(persisted["task"]["revision"], completed["revision"]);
    assert_eq!(persisted["outputs"][0]["id"], output["id"]);
    assert_eq!(persisted["sources"].as_array().unwrap().len(), 2);
    restarted.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn mcp_stdio_discovery_lifecycle() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(45), workflow(true)).await?
}

#[tokio::test]
async fn mcp_stdio_initialize_lifecycle() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(45), workflow(false)).await?
}
