use std::{env, ffi::OsString, path::PathBuf};

use anyhow::{Context, Result, bail};
use dictum_mcp::{
    dashboard,
    migration::{MigrationOptions, migrate},
    server::DictumServer,
    store::Store,
};
use rmcp::{ServiceExt, transport::stdio};

const USAGE: &str = "Usage: dictum-mcp\n       dictum-mcp dashboard [--port PORT]\n       dictum-mcp migrate --source OLD --destination NEW [--name NAME --description DESCRIPTION]\n\nWith no arguments, serve MCP over stdio using DICTUM_DATABASE.\nDashboard serves the read-only localhost UI on 127.0.0.1 (default port 3000; 0 selects a free port) using the same database.\nMigration copies a supported v2 or v3 database into a new v4 database; it never replaces either file.\nVersion 2 requires --name and --description for its new project. Version 3 preserves every project and rejects those options.";

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
    if arguments[0].to_str() == Some("dashboard") {
        return run_dashboard(&arguments[1..]).await;
    }
    run_command(arguments)
}

async fn run_mcp() -> Result<()> {
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

fn run_command(arguments: Vec<OsString>) -> Result<()> {
    if arguments.len() == 1 && matches!(arguments[0].to_str(), Some("--help" | "-h")) {
        println!("{USAGE}");
        return Ok(());
    }
    if arguments[0].to_str() != Some("migrate") {
        bail!("unknown command; {USAGE}");
    }
    let mut source = None;
    let mut destination = None;
    let mut name = None;
    let mut description = None;
    let mut arguments = arguments.into_iter().skip(1);
    while let Some(flag) = arguments.next() {
        if matches!(flag.to_str(), Some("--help" | "-h")) {
            println!("{USAGE}");
            return Ok(());
        }
        let flag = flag.to_str().context("migration option must be UTF-8")?;
        if !matches!(
            flag,
            "--source" | "--destination" | "--name" | "--description"
        ) {
            bail!("unknown migration option {flag}; {USAGE}");
        }
        let value = arguments
            .next()
            .with_context(|| format!("missing value for {flag}"))?;
        let duplicate = match flag {
            "--source" => source.replace(PathBuf::from(value)).is_some(),
            "--destination" => destination.replace(PathBuf::from(value)).is_some(),
            "--name" => name
                .replace(
                    value
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("project name must be UTF-8"))?,
                )
                .is_some(),
            "--description" => description
                .replace(
                    value
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("project description must be UTF-8"))?,
                )
                .is_some(),
            _ => unreachable!(),
        };
        if duplicate {
            bail!("migration option {flag} was supplied more than once");
        }
    }
    let report = migrate(MigrationOptions {
        source: source.context("migration requires --source")?,
        destination: destination.context("migration requires --destination")?,
        name,
        description,
    })?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
