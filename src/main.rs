use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use dictum_mcp::{server::DictumServer, store::Store};
use rmcp::{ServiceExt, transport::stdio};

fn database_path() -> PathBuf {
    env::var_os("DICTUM_DATABASE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".dictum/dictum.sqlite3"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let database_path = database_path();
    let store = Store::open(&database_path)
        .with_context(|| format!("failed to open database at {}", database_path.display()))?;

    eprintln!("dictum-mcp: using {}", database_path.display());

    let service = DictumServer::new(store)
        .serve(stdio())
        .await
        .context("failed to start MCP stdio service")?;

    service
        .waiting()
        .await
        .context("MCP stdio service stopped unexpectedly")?;

    Ok(())
}
