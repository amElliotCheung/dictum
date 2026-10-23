//! One-shot, manually launched local agent attempts.
//!
//! The dashboard process is the single run supervisor for a database. It spawns
//! temporary Codex/Claude Code subprocesses, captures bounded output, and keeps
//! durable run records separate from append-only task results.

use std::{
    collections::{BTreeSet, HashMap},
    env,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
    sync::{Mutex, mpsc, watch},
    task::JoinHandle,
    time::{Instant, sleep, timeout},
};
use uuid::Uuid;

use crate::{
    model::*,
    store::{CreateRun, Store, StoreError, StoreResult},
};

const DEFAULT_TIMEOUT_SECONDS: u64 = 30 * 60;
const LOG_LIMIT: usize = 256 * 1024;
const LOG_SLACK: usize = 64 * 1024;
const PROGRESS_LINE_LIMIT: usize = 600;
const PROGRESS_LINES: usize = 40;
const LINE_LIMIT: usize = 64 * 1024;
const TERMINATE_GRACE: Duration = Duration::from_secs(3);
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const MIN_CODEX_VERSION: (u64, u64, u64) = (0, 100, 0);
// `pluginConfigs` under cc-plugin-agents-md@builtin requires this version.
const MIN_CLAUDE_VERSION: (u64, u64, u64) = (2, 1, 285);
const PROMPT_MARKER: &str = "Dictum one-shot run";

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("run worker failed: {0}")]
    Worker(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopKind {
    Cancel,
    Shutdown,
}

#[derive(Debug, Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

#[derive(Debug)]
struct OutputEvent {
    stream: StreamKind,
    line: String,
}

pub struct RunManager {
    store: Store,
    instance_id: String,
    token: String,
    timeout: Duration,
    codex_bin: String,
    claude_bin: String,
    codex_model: Option<String>,
    claude_model: Option<String>,
    dictum_binary: PathBuf,
    database_identity: PathBuf,
    active: Mutex<HashMap<i64, watch::Sender<StopKind>>>,
    lifecycle: Mutex<()>,
    stopping: AtomicBool,
}

impl RunManager {
    /// Claim single-service ownership, clean up attempts left by a previous
    /// service process, and prepare a run token for same-origin mutations.
    pub async fn start(store: Store) -> Result<Self, RunError> {
        let database_identity = std::fs::canonicalize(store.path()).map_err(|error| {
            RunError::Validation(format!("cannot resolve the run database: {error}"))
        })?;
        let instance_id = Uuid::new_v4().to_string();
        let claim_store = store.clone();
        let claim_instance = instance_id.clone();
        tokio::task::spawn_blocking(move || {
            claim_store.claim_service(&claim_instance, std::process::id())
        })
        .await
        .map_err(|error| RunError::Worker(error.to_string()))??;
        let manager = Self {
            store,
            instance_id,
            token: Uuid::new_v4().to_string(),
            timeout: configured_timeout(),
            codex_bin: env::var("DICTUM_CODEX_BIN").unwrap_or_else(|_| "codex".to_string()),
            claude_bin: env::var("DICTUM_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string()),
            codex_model: env::var("DICTUM_CODEX_MODEL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            claude_model: env::var("DICTUM_CLAUDE_MODEL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            dictum_binary: env::current_exe().map_err(|error| {
                RunError::Validation(format!("cannot locate dictum-mcp: {error}"))
            })?,
            database_identity,
            active: Mutex::new(HashMap::new()),
            lifecycle: Mutex::new(()),
            stopping: AtomicBool::new(false),
        };
        if let Err(error) = manager.reconcile_stale_runs().await {
            let _ = manager.store.release_service(&manager.instance_id);
            return Err(error);
        }
        Ok(manager)
    }

    pub fn config(&self) -> RunConfig {
        RunConfig {
            token: self.token.clone(),
            timeout_seconds: self.timeout.as_secs() as i64,
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub async fn list(&self, project_id: &str, task_id: i64) -> Result<RunList, RunError> {
        let project_id = project_id.to_string();
        store_call(&self.store, move |store| {
            store.list_runs(&project_id, task_id, None)
        })
        .await
    }

    pub async fn detail(
        &self,
        project_id: &str,
        task_id: i64,
        run_id: i64,
        include_log: bool,
    ) -> Result<RunDetail, RunError> {
        let project_id = project_id.to_string();
        store_call(&self.store, move |store| {
            store.get_run(&project_id, task_id, run_id, include_log)
        })
        .await
    }

    pub async fn launch(
        self: &Arc<Self>,
        project_id: &str,
        task_id: i64,
        input: LaunchRun,
    ) -> Result<RunSummary, RunError> {
        let _lifecycle = self.lifecycle.lock().await;
        if self.stopping.load(Ordering::SeqCst) {
            return Err(RunError::Conflict(
                "the dashboard service is shutting down; start it again before launching a run"
                    .to_string(),
            ));
        }
        let working_directory = resolve_working_directory(&input.working_directory)?;
        let (executable, version) = resolve_agent(
            match input.agent {
                RunAgent::Codex => &self.codex_bin,
                RunAgent::Claude => &self.claude_bin,
            },
            input.agent,
        )
        .await?;
        let project = project_id.to_string();
        let create = CreateRun {
            project_id: project.clone(),
            task_id,
            agent: input.agent,
            agent_version: version,
            agent_executable: executable,
            working_directory,
        };
        let run = store_call(&self.store, move |store| store.create_run(create)).await?;
        let prompt = build_prompt(&self.database_identity, &run);
        let (stop_tx, stop_rx) = watch::channel(StopKind::Cancel);
        {
            let mut active = self.active.lock().await;
            active.insert(run.id, stop_tx);
        }
        let manager = Arc::clone(self);
        let spawned_run = run.clone();
        tokio::spawn(async move {
            supervise(manager, spawned_run, prompt, stop_rx).await;
        });
        if self.stopping.load(Ordering::SeqCst) {
            let active = self.active.lock().await;
            if let Some(sender) = active.get(&run.id) {
                let _ = sender.send(StopKind::Shutdown);
            }
        }
        Ok(run)
    }

    pub async fn cancel(
        &self,
        project_id: &str,
        task_id: i64,
        run_id: i64,
    ) -> Result<RunSummary, RunError> {
        let project = project_id.to_string();
        let run = store_call(&self.store, move |store| {
            store.request_run_cancel(&project, task_id, run_id)
        })
        .await?;
        if run.status == RunStatus::Running {
            let sender = self.active.lock().await.get(&run_id).cloned();
            match sender {
                Some(sender) => {
                    let _ = sender.send(StopKind::Cancel);
                }
                None => {
                    // The supervisor may have finished between the cancel write
                    // and the in-memory lookup. Report the durable state instead
                    // of a misleading conflict for an already-finished attempt.
                    let project = project_id.to_string();
                    let current = store_call(&self.store, move |store| {
                        store.get_run(&project, task_id, run_id, false)
                    })
                    .await?;
                    if current.run.status == RunStatus::Running {
                        return Err(RunError::Conflict(
                            "this run is not supervised by the current dashboard service"
                                .to_string(),
                        ));
                    }
                    return Ok(current.run);
                }
            }
        }
        Ok(run)
    }

    /// Stop supervisors, kill their process groups, and release service ownership.
    pub async fn shutdown(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        self.stopping.store(true, Ordering::SeqCst);
        {
            let active = self.active.lock().await;
            for sender in active.values() {
                let _ = sender.send(StopKind::Shutdown);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            if self.active.lock().await.is_empty() {
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        // A wedged supervisor must not leave a claimed service lock behind.
        if let Ok(runs) = self.store.running_runs() {
            let markers = runs
                .iter()
                .map(|run| {
                    (
                        run.pid,
                        run_marker(
                            &self.database_identity,
                            &run.project_id,
                            run.task_id,
                            run.id,
                            &run.started_at,
                        ),
                    )
                })
                .collect();
            cleanup_runs(markers).await;
        }
        let _ = self
            .store
            .interrupt_running_runs("dashboard service stopped before the run finished");
        let _ = self.store.release_service(&self.instance_id);
    }

    async fn reconcile_stale_runs(&self) -> Result<(), RunError> {
        let runs = store_call(&self.store, |store| store.running_runs()).await?;
        let markers = runs
            .iter()
            .map(|run| {
                (
                    run.pid,
                    run_marker(
                        &self.database_identity,
                        &run.project_id,
                        run.task_id,
                        run.id,
                        &run.started_at,
                    ),
                )
            })
            .collect();
        cleanup_runs(markers).await;
        let marked = store_call(&self.store, |store| {
            store.interrupt_running_runs("dashboard service restarted before the run finished")
        })
        .await?;
        if marked > 0 {
            eprintln!("dictum-mcp dashboard: marked {marked} unfinished run(s) as interrupted");
        }
        Ok(())
    }
}

async fn supervise(
    manager: Arc<RunManager>,
    run: RunSummary,
    prompt: String,
    mut stop: watch::Receiver<StopKind>,
) {
    let (status, exit_code, stop_reason, mut output) =
        execute(&manager, &run, &prompt, &mut stop).await;
    output.trim_to_limit();
    let truncated = output.log_truncated;
    let progress = output.progress_text();
    let log = output.log;
    let run_id = run.id;
    let mut finalized = false;
    for attempt in 1..=3 {
        let log = log.clone();
        let progress = progress.clone();
        let stop_reason = stop_reason.clone();
        match store_call(&manager.store, move |store| {
            store.finish_run(
                run_id,
                status,
                exit_code,
                stop_reason.as_deref(),
                &log,
                truncated,
                &progress,
            )
        })
        .await
        {
            Ok(_) => {
                finalized = true;
                break;
            }
            Err(error) => {
                eprintln!(
                    "dictum-mcp dashboard: could not finalize run {run_id} (attempt {attempt}/3): {error}"
                );
                sleep(Duration::from_millis(200)).await;
            }
        }
    }
    if !finalized {
        eprintln!(
            "dictum-mcp dashboard: run {run_id} remains marked running after finalization failed; a restart will reconcile it"
        );
    }
    manager.active.lock().await.remove(&run.id);
}

async fn execute(
    manager: &RunManager,
    run: &RunSummary,
    prompt: &str,
    stop: &mut watch::Receiver<StopKind>,
) -> (RunStatus, Option<i32>, Option<String>, Output) {
    let mut output = Output::new(run.agent);
    let mut command = match build_command(manager, run, prompt) {
        Ok(command) => command,
        Err(error) => {
            return (RunStatus::Failed, None, Some(error.to_string()), output);
        }
    };
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return (
                RunStatus::Failed,
                None,
                Some(format!("could not start {}: {error}", run.agent_executable)),
                output,
            );
        }
    };
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let mut wait = tokio::spawn(async move { child.wait().await });
    if let Some(pid) = pid {
        let run_id = run.id;
        if let Err(error) =
            store_call(&manager.store, move |store| store.set_run_pid(run_id, pid)).await
        {
            let _ = terminate_waited(Some(pid), &mut wait).await;
            return (
                RunStatus::Failed,
                None,
                Some(format!("could not record the run process: {error}")),
                output,
            );
        }
    }
    let (tx, mut rx) = mpsc::channel(128);
    if let Some(stdout) = stdout {
        tokio::spawn(read_lines(stdout, StreamKind::Stdout, tx.clone()));
    }
    if let Some(stderr) = stderr {
        tokio::spawn(read_lines(stderr, StreamKind::Stderr, tx.clone()));
    }
    drop(tx);
    let deadline = tokio::time::sleep(manager.timeout);
    tokio::pin!(deadline);
    let mut stop_kind = None;
    let mut timed_out = false;
    let mut exit: Option<ExitStatus> = None;
    let mut readers_done = false;
    let mut persist = tokio::time::interval(Duration::from_secs(1));
    persist.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_ok() {
                    stop_kind = Some(*stop.borrow());
                }
                break;
            }
            () = &mut deadline => {
                timed_out = true;
                break;
            }
            _ = persist.tick() => {
                if output.dirty && persist_output(manager, run.id, &mut output).await {
                    output.dirty = false;
                }
            }
            event = rx.recv(), if !readers_done => {
                match event {
                    Some(event) => output.push(event.stream, &event.line),
                    None => readers_done = true,
                }
            }
            result = &mut wait => {
                exit = join_exit(result);
                break;
            }
        }
    }

    if stop_kind.is_some() || timed_out {
        exit = terminate_waited(pid, &mut wait).await;
    } else if let Some(pid) = pid {
        // The process exited; ensure surviving descendants do not linger.
        signal_group(pid, libc::SIGKILL);
    }

    let drain_deadline = Instant::now() + Duration::from_millis(750);
    while !readers_done && Instant::now() < drain_deadline {
        match timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Some(event)) => output.push(event.stream, &event.line),
            Ok(None) => readers_done = true,
            Err(_) => {}
        }
    }

    let status = match (stop_kind, timed_out) {
        (Some(StopKind::Cancel), _) => RunStatus::Cancelled,
        (Some(StopKind::Shutdown), _) => RunStatus::Interrupted,
        (_, true) => RunStatus::TimedOut,
        (None, false) => match exit {
            Some(exit) if exit.success() => RunStatus::Succeeded,
            _ => RunStatus::Failed,
        },
    };
    let exit_code = exit.and_then(|status| status.code());
    let stop_reason = match status {
        RunStatus::Cancelled => Some("cancelled by user".to_string()),
        RunStatus::Interrupted => Some("dashboard service stopped".to_string()),
        RunStatus::TimedOut => Some(format!(
            "exceeded the {} second run limit",
            manager.timeout.as_secs()
        )),
        RunStatus::Failed => Some(match exit {
            Some(status) => format!("agent process exited with {status}"),
            None => "agent process did not return an exit status".to_string(),
        }),
        _ => None,
    };
    (status, exit_code, stop_reason, output)
}

async fn persist_output(manager: &RunManager, run_id: i64, output: &mut Output) -> bool {
    let progress = output.progress_text();
    let (delta, replace_log) = if output.trimmed_since_persist {
        (
            String::new(),
            Some((output.log.clone(), output.log_truncated)),
        )
    } else {
        let delta = output
            .log
            .get(output.persisted_len..)
            .unwrap_or("")
            .to_string();
        if delta.is_empty() && progress == output.persisted_progress {
            return true;
        }
        (delta, None)
    };
    let stored_progress = progress.clone();
    let saved = store_call(&manager.store, move |store| {
        store.append_run_output(
            run_id,
            &delta,
            replace_log
                .as_ref()
                .map(|(log, truncated)| (log.as_str(), *truncated)),
            &stored_progress,
        )
    })
    .await
    .is_ok();
    if saved {
        output.persisted_len = output.log.len();
        output.persisted_progress = progress;
        output.trimmed_since_persist = false;
    }
    saved
}

fn join_exit(
    result: Result<std::io::Result<ExitStatus>, tokio::task::JoinError>,
) -> Option<ExitStatus> {
    match result {
        Ok(Ok(status)) => Some(status),
        _ => None,
    }
}

async fn terminate_waited(
    pid: Option<u32>,
    wait: &mut JoinHandle<std::io::Result<ExitStatus>>,
) -> Option<ExitStatus> {
    if let Some(pid) = pid {
        signal_group(pid, libc::SIGTERM);
    }
    let mut status = None;
    if let Ok(Ok(result)) = timeout(TERMINATE_GRACE, &mut *wait).await {
        status = result.ok();
    }
    // Even when the leader exits on SIGTERM, descendants may ignore it.
    if let Some(pid) = pid {
        signal_group(pid, libc::SIGKILL);
    }
    if status.is_none()
        && let Ok(Ok(result)) = timeout(TERMINATE_GRACE, &mut *wait).await
    {
        status = result.ok();
    }
    status
}

fn signal_group(pid: u32, signal: i32) {
    crate::process::signal_group(i64::from(pid), signal);
}

async fn cleanup_runs(runs: Vec<(Option<i64>, String)>) {
    if runs.is_empty() {
        return;
    }
    let _ = tokio::task::spawn_blocking(move || cleanup_marked_runs(&runs)).await;
}

/// Best-effort cleanup for attempts left running by a previous service. One
/// process-table snapshot is matched against the exact database/run markers;
/// the recorded PID is only trusted when its command still carries its marker.
fn cleanup_marked_runs(runs: &[(Option<i64>, String)]) {
    let output = std::process::Command::new("ps")
        .args(["-ww", "-axo", "pid=,pgid=,command="])
        .output();
    let Ok(output) = output else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let rows: Vec<(i64, i64, String)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let mut parts = line.splitn(3, |character: char| character.is_whitespace());
            let pid = parts.next()?.parse::<i64>().ok()?;
            let pgid = parts.next()?.parse::<i64>().ok()?;
            let command = parts.next()?.trim_start().to_string();
            Some((pid, pgid, command))
        })
        .collect();
    let mut groups = BTreeSet::new();
    for (recorded_pid, marker) in runs {
        if let Some(pid) = recorded_pid
            && crate::process::alive(*pid)
            && let Some((_, group, _)) = rows
                .iter()
                .find(|(row_pid, _, command)| row_pid == pid && command.contains(marker))
        {
            groups.insert(*group);
        }
        for (_, group, command) in &rows {
            if command.contains(marker) {
                groups.insert(*group);
            }
        }
    }
    let own_group = crate::process::current_group();
    let groups: Vec<i64> = groups
        .into_iter()
        .filter(|group| *group > 0 && *group != own_group)
        .collect();
    if groups.is_empty() {
        return;
    }
    for group in &groups {
        crate::process::signal_group(*group, libc::SIGTERM);
    }
    std::thread::sleep(Duration::from_millis(250));
    for group in &groups {
        if crate::process::group_alive(*group) {
            crate::process::signal_group(*group, libc::SIGKILL);
        }
    }
}

async fn read_lines<R>(reader: R, stream: StreamKind, tx: mpsc::Sender<OutputEvent>)
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut line = Vec::with_capacity(1024);
    let mut truncated = false;
    loop {
        let chunk = match reader.fill_buf().await {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = tx
                    .send(OutputEvent {
                        stream,
                        line: format!("[output read error: {error}]"),
                    })
                    .await;
                break;
            }
        };
        if chunk.is_empty() {
            if !line.is_empty() {
                send_line(&tx, stream, &line, truncated).await;
            }
            break;
        }
        if let Some(position) = chunk.iter().position(|byte| *byte == b'\n') {
            append_bounded(&mut line, &chunk[..position], &mut truncated);
            let consumed = position + 1;
            reader.consume(consumed);
            if !send_line(&tx, stream, &line, truncated).await {
                break;
            }
            line.clear();
            truncated = false;
        } else {
            let consumed = chunk.len();
            append_bounded(&mut line, chunk, &mut truncated);
            reader.consume(consumed);
        }
    }
}

fn append_bounded(line: &mut Vec<u8>, bytes: &[u8], truncated: &mut bool) {
    // Keep one extra byte so a line of exactly LINE_LIMIT bytes plus CRLF is
    // not spuriously marked truncated before the CR is stripped.
    let capacity = LINE_LIMIT.saturating_add(1);
    let remaining = capacity.saturating_sub(line.len());
    if bytes.len() > remaining {
        line.extend_from_slice(&bytes[..remaining]);
        *truncated = true;
    } else {
        line.extend_from_slice(bytes);
    }
}

fn trim_partial_utf8(bytes: &[u8]) -> &[u8] {
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes,
        Err(error) if error.error_len().is_none() && error.valid_up_to() > 0 => {
            &bytes[..error.valid_up_to()]
        }
        Err(_) => bytes,
    }
}

/// Lines are always delivered lossily, so non-UTF-8 tool output cannot stop the
/// reader or close the child's pipe. Truncation happens after CR stripping and
/// at a UTF-8 boundary.
async fn send_line(
    tx: &mpsc::Sender<OutputEvent>,
    stream: StreamKind,
    bytes: &[u8],
    truncated: bool,
) -> bool {
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    let bytes = if truncated {
        trim_partial_utf8(bytes)
    } else {
        bytes
    };
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if truncated || text.len() > LINE_LIMIT {
        let mut cut = LINE_LIMIT.min(text.len());
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str(" …[line truncated]");
    }
    tx.send(OutputEvent { stream, line: text }).await.is_ok()
}

fn build_command(
    manager: &RunManager,
    run: &RunSummary,
    prompt: &str,
) -> Result<Command, RunError> {
    let mut command = Command::new(&run.agent_executable);
    command
        .current_dir(&run.working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    let database = manager.store.path().to_string_lossy().to_string();
    // Run identity and database access belong only to the explicit MCP
    // configuration below. Leaving any DICTUM_* variable on the agent
    // environment would taint an unrelated Dictum process it starts.
    for (name, _) in env::vars_os() {
        if name.to_string_lossy().starts_with("DICTUM_") {
            command.env_remove(name);
        }
    }
    match run.agent {
        RunAgent::Codex => {
            command
                .arg("exec")
                .arg("--json")
                .arg("--sandbox")
                .arg("workspace-write")
                .arg("--skip-git-repo-check")
                .arg("--ephemeral")
                // Keep the attempt scope deterministic: only the explicit
                // run-scoped Dictum MCP configuration is added below.
                .arg("--ignore-user-config")
                .arg("--cd")
                .arg(&run.working_directory)
                .arg("-c")
                .arg("project_doc_max_bytes=262144")
                .arg("-c")
                .arg(format!(
                    "mcp_servers.dictum.command={}",
                    toml_string(&manager.dictum_binary.to_string_lossy())
                ))
                .arg("-c")
                .arg(format!(
                    "mcp_servers.dictum.env.DICTUM_DATABASE={}",
                    toml_string(&database)
                ))
                .arg("-c")
                .arg(format!(
                    "mcp_servers.dictum.env.DICTUM_RUN_ID={}",
                    toml_string(&run.id.to_string())
                ));
            if let Some(model) = &manager.codex_model {
                command.arg("-m").arg(model);
            }
            command.arg(prompt);
        }
        RunAgent::Claude => {
            let mcp = json!({
                "mcpServers": {
                    "dictum": {
                        "command": manager.dictum_binary.to_string_lossy(),
                        "args": [],
                        "env": {
                            "DICTUM_DATABASE": database,
                            "DICTUM_RUN_ID": run.id.to_string()
                        }
                    }
                }
            });
            let settings = json!({
                "pluginConfigs": {
                    "cc-plugin-agents-md@builtin": {
                        "options": {"instructionFiles": "claude-md-and-agents-md"}
                    }
                }
            });
            command
                .arg("-p")
                .arg(prompt)
                .arg("--output-format")
                .arg("stream-json")
                .arg("--verbose")
                .arg("--permission-mode")
                .arg("acceptEdits")
                .arg("--permission-prompts")
                .arg("none")
                // Headless MCP tools are refused unless pre-approved. The
                // run-scoped server still rejects every write except its result.
                .arg("--allowedTools")
                .arg("mcp__dictum")
                .arg("--no-session-persistence")
                .arg("--strict-mcp-config")
                .arg("--mcp-config")
                .arg(serde_json::to_string(&mcp).map_err(|error| {
                    RunError::Validation(format!("cannot encode MCP configuration: {error}"))
                })?)
                .arg("--settings")
                .arg(serde_json::to_string(&settings).map_err(|error| {
                    RunError::Validation(format!("cannot encode runtime settings: {error}"))
                })?);
            if let Some(model) = &manager.claude_model {
                command.arg("--model").arg(model);
            }
        }
    }
    Ok(command)
}

fn toml_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

/// Encode all persisted identity fields, including the database location, so
/// recovery never confuses database-local IDs or copied database files. Hex
/// keeps the marker intact in `ps` output even for paths with whitespace.
fn run_marker(
    database: &Path,
    project_id: &str,
    task_id: i64,
    run_id: i64,
    started_at: &str,
) -> String {
    let identity = json!([
        database.as_os_str().as_encoded_bytes(),
        project_id,
        task_id,
        run_id,
        started_at,
    ])
    .to_string();
    let encoded: String = identity.bytes().map(|byte| format!("{byte:02x}")).collect();
    format!("[dictum-run-v1:{encoded}]")
}

fn build_prompt(database: &Path, run: &RunSummary) -> String {
    let marker = run_marker(
        database,
        &run.project_id,
        run.task_id,
        run.id,
        &run.started_at,
    );
    format!(
        "{marker}\n{PROMPT_MARKER} {run_id} for project {project_id}, task #{task_id}.\n\n\
         Before acting, call the Dictum MCP tool get_task_context with project_id \"{project_id}\" and task_id {task_id}. \
         Follow the complete brief, current decisions, context sources, dependency state, and outstanding feedback. \
         The authorized scope is exactly that task; do not modify unrelated project records.\n\n\
         Work only in {directory}. This is one bounded attempt: research, edit, and test as needed, then stop. \
         Do not change the task status or mark the task done. Before finishing, record exactly one task result through Dictum MCP \
         with outcome success, no_change, or failure and concise truthful evidence. If required information or permissions are missing, \
         stop and state the exact question or reason in your final message; do not wait for input, guess, broaden permissions, or retry automatically.",
        run_id = run.id,
        project_id = run.project_id,
        task_id = run.task_id,
        directory = run.working_directory,
    )
}

fn resolve_working_directory(raw: &str) -> Result<String, RunError> {
    if raw.trim().is_empty() {
        return Err(RunError::Validation(
            "working_directory must be a nonblank absolute path".to_string(),
        ));
    }
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(RunError::Validation(
            "working_directory must be an absolute path".to_string(),
        ));
    }
    canonical_utf8(path, "working_directory", "directory")
}

fn canonical_utf8(path: &Path, label: &str, kind: &str) -> Result<String, RunError> {
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        RunError::Validation(format!(
            "{label} {} cannot be resolved: {error}",
            path.display()
        ))
    })?;
    let matches = match kind {
        "directory" => canonical.is_dir(),
        "file" => canonical.is_file(),
        _ => false,
    };
    if !matches {
        return Err(RunError::Validation(format!(
            "{label} {} is not a {kind}",
            canonical.display()
        )));
    }
    canonical
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| RunError::Validation(format!("{label} must be valid UTF-8")))
}

async fn resolve_agent(bin: &str, agent: RunAgent) -> Result<(String, String), RunError> {
    if bin.trim().is_empty() {
        return Err(RunError::Validation(
            "agent executable is empty".to_string(),
        ));
    }
    let bin = resolve_executable(bin)?;
    let mut command = Command::new(&bin);
    command.arg("--version").kill_on_drop(true);
    let output = timeout(VERSION_TIMEOUT, command.output())
        .await
    .map_err(|_| {
        RunError::Validation(format!(
            "{bin} --version timed out; check the configured agent executable"
        ))
    })?
    .map_err(|error| {
        RunError::Validation(format!(
            "cannot run {bin} --version: {error}; install the agent or configure DICTUM_CODEX_BIN/DICTUM_CLAUDE_BIN"
        ))
    })?;
    if !output.status.success() {
        return Err(RunError::Validation(format!(
            "{bin} --version failed: {}",
            first_line(&String::from_utf8_lossy(&output.stderr))
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let version = parse_version(&text).ok_or_else(|| {
        RunError::Validation(format!(
            "{bin} --version did not report a parseable version: {}",
            first_line(&text)
        ))
    })?;
    let minimum = match agent {
        RunAgent::Codex => MIN_CODEX_VERSION,
        RunAgent::Claude => MIN_CLAUDE_VERSION,
    };
    if version < minimum {
        return Err(RunError::Validation(format!(
            "{} {}.{}.{} is too old; this run profile requires at least {}.{}.{} for reliable instruction loading",
            agent_name(agent),
            version.0,
            version.1,
            version.2,
            minimum.0,
            minimum.1,
            minimum.2
        )));
    }
    let reported = first_line(&text);
    let reported = if reported.is_empty() {
        format!("{}.{}.{}", version.0, version.1, version.2)
    } else {
        reported.chars().take(200).collect()
    };
    Ok((bin, reported))
}

/// A bare name is resolved through PATH at both version check and spawn time.
/// A slash-containing relative path must be resolved against the dashboard's
/// working directory; otherwise the agent would run a different file after
/// `current_dir` changes.
fn resolve_executable(bin: &str) -> Result<String, RunError> {
    let path = Path::new(bin);
    if path.is_absolute() {
        return Ok(bin.to_string());
    }
    if bin.contains('/') || bin.contains('\\') {
        return canonical_utf8(path, "agent executable", "file");
    }
    Ok(bin.to_string())
}

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    for token in text.split(|character: char| !(character.is_ascii_digit() || character == '.')) {
        let mut parts = token.split('.');
        let (Some(major), Some(minor), Some(patch)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (Ok(major), Ok(minor), Ok(patch)) = (
            major.parse::<u64>(),
            minor.parse::<u64>(),
            patch.parse::<u64>(),
        ) else {
            continue;
        };
        if parts.next().is_none() {
            return Some((major, minor, patch));
        }
    }
    None
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_string()
}

fn agent_name(agent: RunAgent) -> &'static str {
    match agent {
        RunAgent::Codex => "Codex",
        RunAgent::Claude => "Claude Code",
    }
}

fn configured_timeout() -> Duration {
    let seconds = env::var("DICTUM_RUN_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    Duration::from_secs(seconds)
}

async fn store_call<T, F>(store: &Store, operation: F) -> Result<T, RunError>
where
    T: Send + 'static,
    F: FnOnce(Store) -> StoreResult<T> + Send + 'static,
{
    let store = store.clone();
    match tokio::task::spawn_blocking(move || operation(store)).await {
        Ok(result) => result.map_err(RunError::Store),
        Err(error) => Err(RunError::Worker(error.to_string())),
    }
}

struct Output {
    agent: RunAgent,
    log: String,
    log_truncated: bool,
    progress: Vec<String>,
    dirty: bool,
    persisted_len: usize,
    persisted_progress: String,
    trimmed_since_persist: bool,
}

impl Output {
    fn new(agent: RunAgent) -> Self {
        Self {
            agent,
            log: String::new(),
            log_truncated: false,
            progress: Vec::new(),
            dirty: false,
            persisted_len: 0,
            persisted_progress: String::new(),
            trimmed_since_persist: false,
        }
    }

    fn push(&mut self, stream: StreamKind, line: &str) {
        self.dirty = true;
        match stream {
            StreamKind::Stdout => {
                self.log.push_str(line);
                self.log.push('\n');
                match serde_json::from_str::<Value>(line) {
                    Ok(value) => {
                        if let Some(progress) = progress_from_value(self.agent, &value) {
                            self.push_progress(&progress);
                        }
                    }
                    Err(_) => self.push_progress(line),
                }
            }
            StreamKind::Stderr => {
                self.log.push_str("[stderr] ");
                self.log.push_str(line);
                self.log.push('\n');
                if !line.trim().is_empty() {
                    self.push_progress(&format!("stderr: {line}"));
                }
            }
        }
        if self.log.len() > LOG_LIMIT + LOG_SLACK {
            self.trim_log(LOG_LIMIT);
        }
    }

    fn trim_to_limit(&mut self) {
        if self.log.len() > LOG_LIMIT {
            self.trim_log(LOG_LIMIT);
        }
    }

    fn trim_log(&mut self, limit: usize) {
        let mut cut = self.log.len() - limit;
        while cut < self.log.len() && !self.log.is_char_boundary(cut) {
            cut += 1;
        }
        self.log.drain(..cut);
        self.log_truncated = true;
        self.trimmed_since_persist = true;
    }

    fn push_progress(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let mut text: String = line.chars().take(PROGRESS_LINE_LIMIT).collect();
        if line.chars().count() > PROGRESS_LINE_LIMIT {
            text.push('…');
        }
        self.progress.push(text);
        if self.progress.len() > PROGRESS_LINES {
            self.progress.remove(0);
        }
    }

    fn progress_text(&self) -> String {
        self.progress.join("\n")
    }
}

fn progress_from_value(agent: RunAgent, value: &Value) -> Option<String> {
    match agent {
        RunAgent::Claude => claude_progress(value),
        RunAgent::Codex => codex_progress(value),
    }
}

fn claude_progress(value: &Value) -> Option<String> {
    match value.get("type")?.as_str()? {
        "assistant" => {
            let mut parts = Vec::new();
            for item in value.get("message")?.get("content")?.as_array()? {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = item.get("text").and_then(Value::as_str) {
                            let text = text.trim();
                            if !text.is_empty() {
                                parts.push(text.to_string());
                            }
                        }
                    }
                    Some("tool_use") => {
                        let name = item.get("name").and_then(Value::as_str).unwrap_or("tool");
                        parts.push(format!("Using {name}"));
                    }
                    _ => {}
                }
            }
            (!parts.is_empty()).then(|| parts.join(" · "))
        }
        "result" => value
            .get("result")
            .and_then(Value::as_str)
            .map(|text| format!("Result: {}", text.trim())),
        _ => None,
    }
}

fn codex_progress(value: &Value) -> Option<String> {
    let kind = value.get("type")?.as_str()?;
    if kind == "item.completed" || kind == "item.started" {
        let item = value.get("item")?;
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        if let Some(command) = item.get("command").and_then(Value::as_str) {
            return Some(format!("$ {command}"));
        }
        if let Some(message) = item.get("message").and_then(Value::as_str) {
            return Some(format!("Error: {message}"));
        }
        if item.get("type").and_then(Value::as_str) == Some("file_change") {
            return Some("File change applied".to_string());
        }
        if item.get("type").and_then(Value::as_str) == Some("reasoning") {
            return None;
        }
        return item
            .get("type")
            .and_then(Value::as_str)
            .map(|kind| format!("{kind} completed"));
    }
    if kind == "turn.failed" || kind == "error" {
        return value
            .get("error")
            .and_then(|error| {
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .or_else(|| error.as_str())
            })
            .or_else(|| value.get("message").and_then(Value::as_str))
            .map(|message| format!("Error: {message}"));
    }
    None
}
