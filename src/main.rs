use std::{env, ffi::OsString, path::PathBuf};

use anyhow::{Context, Result, bail};
use dictum_mcp::{dashboard, server::DictumServer, store::Store};
use rmcp::{ServiceExt, transport::stdio};

const USAGE: &str = "Usage: dictum-mcp\n       dictum-mcp dashboard [--port PORT]\n\nWith no arguments, serve MCP over stdio using DICTUM_DATABASE.\nDashboard serves the localhost UI and owns one-shot agent runs on 127.0.0.1 (default port 3000; 0 selects a free port) using the same database.\nOnly the current schema is supported; older databases are rejected without modification.";

fn database_path() -> PathBuf {
    env::var_os("DICTUM_DATABASE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".dictum/dictum.sqlite3"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.is_empty() {
        return run_mcp().await;
    }
    match arguments[0].to_str() {
        Some("dashboard") => run_dashboard(&arguments[1..]).await,
        Some("--help" | "-h") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => bail!("unknown command; {USAGE}"),
    }
}

async fn run_mcp() -> Result<()> {
    let database_path = database_path();
    let store = Store::open(&database_path)
        .with_context(|| format!("failed to open database at {}", database_path.display()))?;

    eprintln!("dictum-mcp: using {}", database_path.display());

    let run_id = env::var("DICTUM_RUN_ID")
        .ok()
        .and_then(|value| value.parse::<i64>().ok());
    let service = DictumServer::with_run_id(store, run_id)
        .serve(stdio())
        .await
        .context("failed to start MCP stdio service")?;

    service
        .waiting()
        .await
        .context("MCP stdio service stopped unexpectedly")?;

    Ok(())
}

async fn run_dashboard(arguments: &[OsString]) -> Result<()> {
    let mut port = None;
    let mut arguments = arguments.iter();
    while let Some(flag) = arguments.next() {
        if matches!(flag.to_str(), Some("--help" | "-h")) {
            println!("{USAGE}");
            return Ok(());
        }
        if flag.to_str() != Some("--port") {
            bail!(
                "unknown dashboard option {}; {USAGE}",
                flag.to_string_lossy()
            );
        }
        if port.is_some() {
            bail!("dashboard --port was supplied more than once");
        }
        let value = arguments
            .next()
            .context("dashboard --port requires a value")?;
        let value = value.to_str().context("dashboard port must be UTF-8")?;
        port = Some(match value.parse::<u32>() {
            Ok(port) if port <= u32::from(u16::MAX) => port as u16,
            _ => bail!("invalid --port {value}: expected an integer between 0 and 65535"),
        });
    }
    let port = port.unwrap_or(3000);
    let database_path = database_path();
    let store = Store::open(&database_path)
        .with_context(|| format!("failed to open database at {}", database_path.display()))?;
    dashboard::serve(store, port).await
}
