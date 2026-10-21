//! Real-process dashboard tests: MCP seeds/mutates SQLite, HTTP reads it.
mod support;

use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, ChildStderr, Command, Output, Stdio},
    time::{Duration, Instant},
};
use support::Project;
use tempfile::TempDir;

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

struct Dashboard {
    child: Child,
    _stderr: BufReader<ChildStderr>,
    base: String,
    port: u16,
    database: String,
}

impl Dashboard {
    fn start(database: &Path, port: u16) -> Self {
        Self::start_with_env(database, port, &[])
    }

    fn start_with_env(database: &Path, port: u16, environment: &[(&str, &str)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dictum-mcp"));
        command
            .arg("dashboard")
            .arg("--port")
            .arg(port.to_string())
            .env("DICTUM_DATABASE", database)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        for (name, value) in environment {
            command.env(name, value);
        }
        let mut child = command.spawn().expect("start dashboard process");
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        let mut base = None;
        let mut using = None;
        while base.is_none() {
            let mut line = String::new();
            let read = stderr.read_line(&mut line).unwrap();
            assert!(read > 0, "dashboard exited before reporting its URL");
            let line = line.trim();
            if let Some(value) = line.strip_prefix("dictum-mcp dashboard: using ") {
                using = Some(value.to_string());
            }
            if let Some(value) = line.strip_prefix("dictum-mcp dashboard: listening on ") {
                base = Some(value.to_string());
            }
        }
        let base = base.unwrap();
        let port = base
            .rsplit_once(':')
            .expect("URL includes a port")
            .1
            .parse()
            .expect("URL port is numeric");
        Self {
            child,
            _stderr: stderr,
            base,
            port,
            database: using.expect("database path was reported"),
        }
    }

    fn get(&self, path: &str) -> HttpResponse {
        self.request("GET", path, None, &[])
    }

    fn get_with_headers(&self, path: &str, headers: &[(&str, &str)]) -> HttpResponse {
        self.request("GET", path, None, headers)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        host: Option<&str>,
        headers: &[(&str, &str)],
    ) -> HttpResponse {
        let host = host
            .map(str::to_string)
            .unwrap_or_else(|| format!("127.0.0.1:{}", self.port));
        http_request(self.port, method, path, &host, headers, &[])
    }

    fn request_with_body(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> HttpResponse {
        let host = format!("127.0.0.1:{}", self.port);
        http_request(self.port, method, path, &host, headers, body)
    }

    fn post_json(
        &self,
        path: &str,
        value: &Value,
        origin: Option<&str>,
        token: Option<&str>,
    ) -> HttpResponse {
        self.post_raw(
            path,
            "application/json",
            &serde_json::to_vec(value).unwrap(),
            origin,
            token,
        )
    }

    fn post_raw(
        &self,
        path: &str,
        content_type: &str,
        body: &[u8],
        origin: Option<&str>,
        token: Option<&str>,
    ) -> HttpResponse {
        let mut headers = vec![("Content-Type", content_type)];
        if let Some(origin) = origin {
            headers.push(("Origin", origin));
        }
        if let Some(token) = token {
            headers.push(("X-Dictum-CSRF", token));
        }
        self.request_with_body("POST", path, &headers, body)
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn run_token(&self) -> (String, i64) {
        let config = self.get("/api/run-token").json();
        (
            config["token"].as_str().unwrap().to_string(),
            config["timeout_seconds"].as_i64().unwrap(),
        )
    }
}

impl Drop for Dashboard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn http_request(
    port: u16,
    method: &str,
    target: &str,
    host: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> HttpResponse {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut request =
        format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    stream.write_all(request.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let (head, body) = text
        .split_once("\r\n\r\n")
        .expect("response has a header terminator");
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .expect("status line")
        .split(' ')
        .nth(1)
        .expect("status code")
        .parse()
        .unwrap();
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(':').expect("header has a colon");
            (name.to_ascii_lowercase(), value.trim().to_string())
        })
        .collect();
    HttpResponse {
        status,
        headers,
        body: body.to_string(),
    }
}

fn run_dashboard(database: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dictum-mcp"))
        .arg("dashboard")
        .args(arguments)
        .env("DICTUM_DATABASE", database)
        .output()
        .expect("run dashboard command")
}

fn call(client: &Project, name: &str, input: Value) -> Value {
    client.call(name, input).unwrap()
}

fn create_task(client: &Project, project_id: &str, title: &str, kind: &str, extra: Value) -> Value {
    let mut input = json!({
        "project_id": project_id,
        "title": title,
        "kind": kind,
        "problem_or_goal": format!("Deliver {title}."),
        "design": "Implement the stated behavior and verify it end to end.",
        "acceptance_criteria": ["The behavior is covered by a passing integration test."]
    });
    if let (Some(base), Some(extra)) = (input.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    call(client, "create_task", input)
}

fn update_task(client: &Project, task: &Value, mut patch: Value) -> Value {
    patch["project_id"] = task["project_id"].clone();
    patch["task_id"] = task["id"].clone();
    patch["expected_revision"] = task["revision"].clone();
    call(client, "update_task", patch)
}

fn wait_for_port_free(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("port {port} was not released");
}

fn dashboard_database(directory: &TempDir) -> PathBuf {
    directory.path().join("dashboard.sqlite3")
}

fn write_agent_script(directory: &Path, name: &str, body: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

fn codex_fixture() -> &'static str {
    r#"#!/bin/sh
if [ "${1:-}" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
dir="${DICTUM_FIXTURE_DIR:?}"
name="$(basename "$PWD")"
printf '%s\n' "$@" > "$dir/$name-argv.txt"
env > "$dir/$name-env.txt"
echo ok > fixture-output.txt
echo '{"type":"item.completed","item":{"type":"agent_message","text":"Fixture agent started"}}'
case "$name" in
  fail)
    echo "permission denied: fixture cannot proceed" >&2
    exit 7
    ;;
  sleep|cancel|restart|timeout)
    sleep 300 &
    child=$!
    printf '%s\n%s\n' "$$" "$child" > "$dir/$name-pids.txt"
    echo '{"type":"item.completed","item":{"type":"command_execution","command":"sleep 300"}}'
    wait "$child"
    exit 0
    ;;
esac
echo '{"type":"item.completed","item":{"type":"command_execution","command":"echo checked"}}'
sleep 0.4
exit 0
"#
}

fn claude_fixture() -> &'static str {
    r#"#!/bin/sh
if [ "${1:-}" = "--version" ]; then
  echo "2.1.304 (Claude Code)"
  exit 0
fi
dir="${DICTUM_FIXTURE_DIR:?}"
name="$(basename "$PWD")"
printf '%s\n' "$@" > "$dir/$name-argv.txt"
env > "$dir/$name-env.txt"
echo ok > fixture-output.txt
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Fixture Claude agent started"},{"type":"tool_use","name":"Bash"}]}}'
echo '{"type":"result","subtype":"success","result":"Fixture finished"}'
sleep 0.4
exit 0
"#
}

fn wait_for_run(dashboard: &Dashboard, path: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let detail = dashboard.get(path).json();
        if detail["run"]["status"].as_str() != Some("running") {
            return detail;
        }
        assert!(
            Instant::now() < deadline,
            "run did not finish before the deadline: {detail}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_file(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(value) = std::fs::read_to_string(path)
            && !value.is_empty()
        {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "fixture file {} was not written",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn process_running(pid: i64) -> bool {
    let status = Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    status.success()
}

fn wait_for_process_exit(pid: i64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while process_running(pid) {
        assert!(
            Instant::now() < deadline,
            "process {pid} was not terminated"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn dashboard_serves_board_statuses_isolation_and_listener_lifecycle() {
    let directory = tempfile::tempdir().unwrap();
    let database = dashboard_database(&directory);
    let client = Project::open(&database).unwrap();
    let project_a = client.project_id.clone();
    let project_b: Value = client
        .raw_call(
            "create_project",
            json!({
                "name": "Test project",
                "description": "A second project whose display name duplicates the first"
            }),
        )
        .unwrap();
    let project_b_id = project_b["id"].as_str().unwrap().to_string();
    assert_ne!(project_a, project_b_id);

    let todo = create_task(&client, &project_a, "Todo task", "one_time", json!({}));
    let in_progress = create_task(&client, &project_a, "Active task", "one_time", json!({}));
    let _in_progress = update_task(&client, &in_progress, json!({"status": "in_progress"}));
    let blocked = create_task(&client, &project_a, "Blocked task", "one_time", json!({}));
    update_task(&client, &blocked, json!({"status": "blocked"}));
    let done = create_task(&client, &project_a, "Done task", "one_time", json!({}));
    let done_result = call(
        &client,
        "record_task_result",
        json!({
            "project_id": project_a,
            "task_id": done["id"],
            "expected_revision": done["revision"],
            "outcome": "success",
            "evidence": "Verified in the dashboard fixture."
        }),
    );
    let mut done_with_result = done.clone();
    done_with_result["revision"] = done_result["task_revision"].clone();
    update_task(&client, &done_with_result, json!({"status": "done"}));
    let _active = create_task(
        &client,
        &project_a,
        "Nightly sweep",
        "recurring",
        json!({"trigger_event": "nightly"}),
    );
    let paused = create_task(
        &client,
        &project_a,
        "Paused sweep",
        "recurring",
        json!({"trigger_event": "weekly"}),
    );
    update_task(&client, &paused, json!({"status": "paused"}));
    let gate = create_task(&client, &project_a, "Gate task", "one_time", json!({}));
    let gated_start = create_task(
        &client,
        &project_a,
        "Start-gated task",
        "one_time",
        json!({"prerequisite_task_ids": [gate["id"]]}),
    );
    let gated_done = create_task(
        &client,
        &project_a,
        "Done-gated task",
        "one_time",
        json!({"completion_task_ids": [gate["id"]]}),
    );
    let other_task = create_task(
        &client,
        &project_b_id,
        "Other project task",
        "one_time",
        json!({}),
    );

    let dashboard = Dashboard::start(&database, 0);
    assert!(dashboard.base.starts_with("http://127.0.0.1:"));
    assert_ne!(dashboard.port, 0);
    assert_eq!(
        std::fs::canonicalize(&dashboard.database).unwrap(),
        database.canonicalize().unwrap()
    );

    // Project discovery keeps duplicate names as distinct descriptors.
    let projects = dashboard.get("/api/projects").json();
    let projects = projects["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 2);
    assert!(
        projects
            .iter()
            .all(|project| project["name"] == "Test project")
    );
    assert!(projects.iter().any(|project| project["id"] == project_a));
    assert!(projects.iter().any(|project| project["id"] == project_b_id));

    // The board exposes every task, ascending by ID, across all lifecycles.
    let tasks = dashboard
        .get(&format!("/api/projects/{project_a}/tasks"))
        .json();
    let tasks = tasks["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 9);
    let ids: Vec<i64> = tasks
        .iter()
        .map(|task| task["id"].as_i64().unwrap())
        .collect();
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    let statuses: Vec<&str> = tasks
        .iter()
        .map(|task| task["status"].as_str().unwrap())
        .collect();
    for status in ["todo", "in_progress", "blocked", "done", "active", "paused"] {
        assert!(statuses.contains(&status), "missing status {status}");
    }
    let start_gated = tasks
        .iter()
        .find(|task| task["id"] == gated_start["id"])
        .unwrap();
    assert_eq!(
        start_gated["dependency_state"]["unmet_prerequisite_task_ids"],
        json!([gate["id"]])
    );
    assert_eq!(start_gated["dependency_state"]["ready_to_start"], false);
    let done_gated = tasks
        .iter()
        .find(|task| task["id"] == gated_done["id"])
        .unwrap();
    assert_eq!(
        done_gated["dependency_state"]["unmet_completion_task_ids"],
        json!([gate["id"]])
    );
    assert_eq!(todo["status"], "todo");

    let other_tasks = dashboard
        .get(&format!("/api/projects/{project_b_id}/tasks"))
        .json();
    let other_tasks = other_tasks["tasks"].as_array().unwrap();
    assert_eq!(other_tasks.len(), 1);
    assert_eq!(other_tasks[0]["id"], other_task["id"]);

    // Cross-project and unknown reads are indistinguishable 404s.
    assert_eq!(
        dashboard
            .get(&format!(
                "/api/projects/{project_b_id}/tasks/{}",
                todo["id"]
            ))
            .status,
        404
    );
    assert_eq!(
        dashboard
            .get(&format!(
                "/api/projects/{project_a}/tasks/{}",
                other_task["id"]
            ))
            .status,
        404
    );
    assert_eq!(
        dashboard
            .get("/api/projects/00000000-0000-7000-8000-000000000000/tasks")
            .status,
        404
    );
    assert_eq!(
        dashboard
            .get(&format!("/api/projects/{project_a}/tasks/not-a-number"))
            .status,
        400
    );
    assert_eq!(
        dashboard
            .get(&format!("/api/projects/{project_a}/sources/0"))
            .status,
        400
    );
    let error = dashboard
        .get(&format!("/api/projects/{project_a}/tasks/not-a-number"))
        .json();
    assert!(error["error"].as_str().unwrap().contains("task id"));

    // Static assets are self-contained and carry the restrictive policy.
    let index = dashboard.get("/");
    assert_eq!(index.status, 200);
    assert!(index.body.contains("<title>Dictum</title>"));
    assert!(index.body.contains("/app.js"));
    assert!(index.body.contains("/app.css"));
    assert!(!index.body.contains("http://"));
    assert!(!index.body.contains("https://"));
    assert!(
        index
            .header("content-security-policy")
            .unwrap()
            .contains("script-src 'self'")
    );
    assert!(index.header("access-control-allow-origin").is_none());
    let script = dashboard.get("/app.js");
    assert_eq!(script.status, 200);
    assert_eq!(
        script.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert!(!script.body.is_empty());
    let style = dashboard.get("/app.css");
    assert_eq!(style.status, 200);
    assert_eq!(
        style.header("content-type"),
        Some("text/css; charset=utf-8")
    );
    assert!(!style.body.contains("http://"));
    let favicon = dashboard.get("/favicon.svg");
    assert_eq!(favicon.status, 200);
    assert_eq!(favicon.header("content-type"), Some("image/svg+xml"));

    let head = dashboard.request("HEAD", "/", None, &[]);
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    let expected_length = index.body.len().to_string();
    assert_eq!(
        head.header("content-length"),
        Some(expected_length.as_str())
    );

    // Method, route, Host and Origin boundaries.
    let post = dashboard.request("POST", "/api/projects", None, &[]);
    assert_eq!(post.status, 405);
    assert!(post.header("allow").unwrap().contains("GET"));
    assert_eq!(dashboard.get("/api/does-not-exist").status, 404);
    assert_eq!(
        dashboard
            .request("GET", "/", Some("evil.example"), &[])
            .status,
        403
    );
    assert_eq!(
        dashboard
            .get_with_headers("/api/projects", &[("Origin", "http://evil.example")])
            .status,
        403
    );
    let local_origin = format!("http://localhost:{}", dashboard.port);
    assert_eq!(
        dashboard
            .get_with_headers("/api/projects", &[("Origin", &local_origin)])
            .status,
        200
    );

    // Occupied-port failure is clear and nonzero.
    let port = dashboard.port.to_string();
    let occupied = run_dashboard(&database, &["--port", &port]);
    assert!(!occupied.status.success());
    assert!(String::from_utf8_lossy(&occupied.stderr).contains("failed to bind dashboard"));

    // Killing the dashboard releases its listener; data persists across restart.
    drop(dashboard);
    wait_for_port_free(port.parse().unwrap());
    let restarted = Dashboard::start(&database, port.parse().unwrap());
    assert_eq!(
        restarted
            .get(&format!("/api/projects/{project_a}/tasks"))
            .json()["tasks"]
            .as_array()
            .unwrap()
            .len(),
        9
    );
}

#[test]
fn dashboard_read_api_is_bounded_read_only_and_reflects_concurrent_mcp_writes() {
    let directory = tempfile::tempdir().unwrap();
    let database = dashboard_database(&directory);
    let client = Project::open(&database).unwrap();
    let project_id = client.project_id.clone();
    let observed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let adversarial = "# Adversarial context\n\n<script>window.__xss = 1</script>\n\n[bad](javascript:alert(1))\n\n![remote](https://evil.example/tracker.png)\n\n**Readable fallback** with `inline code`.\n\n```\nliteral <b>markup</b>\n```";

    let inline_source = call(
        &client,
        "create_source",
        json!({
            "project_id": project_id,
            "name": "Inline adversarial source",
            "description": "Stored Markdown that must stay inert in the UI",
            "content": adversarial,
            "context_policy": "always",
            "action": {
                "instruction": "Review the document without fetching anything remotely.",
                "trigger": {"kind": "manual"}
            }
        }),
    );
    let inline_source = call(
        &client,
        "record_source_action",
        json!({
            "project_id": project_id,
            "source_id": inline_source["id"],
            "expected_revision": inline_source["revision"],
            "result": {
                "outcome": "success",
                "observed_at": observed_at,
                "summary": "The document is current.",
                "checkpoint": "checkpoint-1",
                "references": ["https://example.com/reference"]
            }
        }),
    );
    let external_source = call(
        &client,
        "create_source",
        json!({
            "project_id": project_id,
            "name": "External source",
            "description": "A locator that must never be fetched by the dashboard",
            "locator": "https://example.com/not-fetched",
            "action": {
                "instruction": "Re-read the external document.",
                "trigger": {"kind": "interval", "every_minutes": 1440}
            }
        }),
    );
    let external_source = call(
        &client,
        "record_source_action",
        json!({
            "project_id": project_id,
            "source_id": external_source["id"],
            "expected_revision": external_source["revision"],
            "result": {
                "outcome": "success",
                "observed_at": observed_at,
                "summary": "External state is current.",
                "checkpoint": null,
                "references": []
            }
        }),
    );
    let output_source = call(
        &client,
        "create_source",
        json!({
            "project_id": project_id,
            "name": "Implementation evidence",
            "description": "Output source used by execution history",
            "content": "# Evidence\n\nDetails remain mutable."
        }),
    );
    call(
        &client,
        "record_decision",
        json!({
            "project_id": project_id,
            "statement": "Dashboard reads share the MCP store without mutating it.",
            "rationale": "Both transports use the same application boundary."
        }),
    );

    let task = create_task(
        &client,
        &project_id,
        "History task",
        "one_time",
        json!({
            "context_source_ids": [inline_source["id"]]
        }),
    );
    let task = update_task(&client, &task, json!({"progress": "Prepared for history"}));
    let mut revision = task["revision"].as_i64().unwrap();
    let mut latest_result_id = 0;
    for index in 0..25 {
        let result = call(
            &client,
            "record_task_result",
            json!({
                "project_id": project_id,
                "task_id": task["id"],
                "expected_revision": revision,
                "outcome": "success",
                "evidence": format!("Recorded evidence for run {index}"),
                "output_source_ids": [output_source["id"]]
            }),
        );
        revision = result["task_revision"].as_i64().unwrap();
        latest_result_id = result["id"].as_i64().unwrap();
    }
    let other_task = create_task(&client, &project_id, "Cursor task", "one_time", json!({}));
    let other_result = call(
        &client,
        "record_task_result",
        json!({
            "project_id": project_id,
            "task_id": other_task["id"],
            "expected_revision": other_task["revision"],
            "outcome": "success",
            "evidence": "A result belonging to another task."
        }),
    );

    let dashboard = Dashboard::start(&database, 0);
    let context_path = format!("/api/projects/{project_id}/tasks/{}", task["id"]);
    let results_path = format!("{context_path}/results");

    let context = dashboard.get(&context_path).json();
    assert_eq!(context["task"]["id"], task["id"]);
    assert_eq!(context["task"]["revision"], revision);
    assert_eq!(context["current_decisions"].as_array().unwrap().len(), 1);
    let embedded = context["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["id"] == inline_source["id"])
        .unwrap();
    assert_eq!(embedded["content"], adversarial);
    assert_eq!(embedded["snapshot"]["checkpoint"], "checkpoint-1");
    assert_eq!(context["latest_result"]["id"], latest_result_id);
    assert_eq!(context["latest_successful_result"]["id"], latest_result_id);

    let first_page = dashboard
        .get(&format!("{results_path}?page_size=20"))
        .json();
    assert_eq!(first_page["results"].as_array().unwrap().len(), 20);
    let cursor = first_page["next_before_id"].as_i64().unwrap();
    assert_eq!(cursor, first_page["results"][19]["id"].as_i64().unwrap());
    let second_page = dashboard
        .get(&format!("{results_path}?page_size=20&before_id={cursor}"))
        .json();
    assert_eq!(second_page["results"].as_array().unwrap().len(), 5);
    assert!(second_page["next_before_id"].is_null());
    assert_eq!(
        first_page["results"][0]["output_source_ids"],
        json!([output_source["id"]])
    );
    assert_eq!(
        dashboard
            .get(&format!(
                "{results_path}?page_size=20&before_id={}",
                other_result["id"]
            ))
            .status,
        404
    );
    assert_eq!(
        dashboard.get(&format!("{results_path}?page_size=0")).status,
        400
    );
    assert_eq!(
        dashboard
            .get(&format!("{results_path}?page_size=nope"))
            .status,
        400
    );
    assert_eq!(
        dashboard.get(&format!("{results_path}?before_id=0")).status,
        400
    );

    let external = dashboard
        .get(&format!(
            "/api/projects/{project_id}/sources/{}",
            external_source["id"]
        ))
        .json();
    assert_eq!(external["locator"], "https://example.com/not-fetched");
    assert_eq!(
        external["snapshot"]["summary"],
        "External state is current."
    );
    assert_eq!(external["action"]["trigger"]["every_minutes"], 1440);

    // Repeat reads, then compare MCP state before and after.
    let context_before = call(&client, "get_task_context", json!({"task_id": task["id"]}));
    let source_before = call(
        &client,
        "get_source",
        json!({"source_id": inline_source["id"]}),
    );
    let history_before = call(
        &client,
        "list_task_results",
        json!({"task_id": task["id"], "page_size": 100}),
    );
    let tasks_before = call(&client, "list_tasks", json!({}));
    for _ in 0..3 {
        assert_eq!(dashboard.get("/api/projects").status, 200);
        assert_eq!(
            dashboard
                .get(&format!("/api/projects/{project_id}/tasks"))
                .status,
            200
        );
        assert_eq!(dashboard.get(&context_path).status, 200);
        assert_eq!(dashboard.get(&results_path).status, 200);
        assert_eq!(
            dashboard
                .get(&format!(
                    "/api/projects/{project_id}/sources/{}",
                    inline_source["id"]
                ))
                .status,
            200
        );
    }
    let context_after = call(&client, "get_task_context", json!({"task_id": task["id"]}));
    let source_after = call(
        &client,
        "get_source",
        json!({"source_id": inline_source["id"]}),
    );
    let history_after = call(
        &client,
        "list_task_results",
        json!({"task_id": task["id"], "page_size": 100}),
    );
    let tasks_after = call(&client, "list_tasks", json!({}));
    assert_eq!(
        context_before["task"]["revision"],
        context_after["task"]["revision"]
    );
    assert_eq!(
        context_before["task"]["updated_at"],
        context_after["task"]["updated_at"]
    );
    assert_eq!(
        context_before["latest_result"]["id"],
        context_after["latest_result"]["id"]
    );
    assert_eq!(source_before["revision"], source_after["revision"]);
    assert_eq!(source_before["updated_at"], source_after["updated_at"]);
    assert_eq!(source_before["snapshot"], source_after["snapshot"]);
    assert_eq!(
        source_before["latest_attempt"],
        source_after["latest_attempt"]
    );
    assert_eq!(source_before["action_due"], source_after["action_due"]);
    assert_eq!(
        source_before["next_action_at"],
        source_after["next_action_at"]
    );
    assert_eq!(history_before["results"], history_after["results"]);
    assert_eq!(tasks_before["tasks"], tasks_after["tasks"]);

    // A concurrent MCP write becomes visible to the next HTTP read.
    let latest = context_after["task"].clone();
    let updated = update_task(
        &client,
        &latest,
        json!({"progress": "Updated by a concurrent MCP process"}),
    );
    let tasks = dashboard
        .get(&format!("/api/projects/{project_id}/tasks"))
        .json();
    let visible = tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == task["id"])
        .unwrap();
    assert_eq!(visible["progress"], "Updated by a concurrent MCP process");
    assert_eq!(visible["revision"], updated["revision"]);
    let context = dashboard.get(&context_path).json();
    assert_eq!(context["task"]["revision"], updated["revision"]);
    let created = create_task(
        &client,
        &project_id,
        "Fresh MCP task",
        "one_time",
        json!({}),
    );
    let tasks = dashboard
        .get(&format!("/api/projects/{project_id}/tasks"))
        .json();
    assert!(
        tasks["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["id"] == created["id"])
    );

    // Restart on the same port; SQLite remains the only source of truth.
    let port = dashboard.port;
    drop(dashboard);
    wait_for_port_free(port);
    let restarted = Dashboard::start(&database, port);
    let tasks = restarted
        .get(&format!("/api/projects/{project_id}/tasks"))
        .json();
    assert_eq!(
        tasks["tasks"].as_array().unwrap().len(),
        3,
        "fixture task, cursor task and fresh MCP task"
    );
}

#[test]
fn dashboard_result_feedback_survives_history_changes_and_explicit_resolution() {
    let directory = tempfile::tempdir().unwrap();
    let database = dashboard_database(&directory);
    let client = Project::open(&database).unwrap();
    let project_id = client.project_id.clone();
    let foreign_project = call(
        &client,
        "create_project",
        json!({"name": "Other project", "description": "Feedback must stay project scoped."}),
    );
    let output = call(
        &client,
        "create_source",
        json!({
            "name": "Original implementation",
            "description": "The output rejected by its reviewer.",
            "content": "# Original output\n\nThe implementation omits restart persistence."
        }),
    );
    let task = create_task(
        &client,
        &project_id,
        "Review and revise implementation",
        "one_time",
        json!({}),
    );
    let original = call(
        &client,
        "record_task_result",
        json!({
            "task_id": task["id"],
            "expected_revision": task["revision"],
            "outcome": "success",
            "evidence": "The original implementation passed its initial checks.",
            "output_source_ids": [output["id"]]
        }),
    );
    let subsequent = call(
        &client,
        "record_task_result",
        json!({
            "task_id": task["id"],
            "expected_revision": original["task_revision"],
            "outcome": "success",
            "evidence": "A later run also reported success before human review."
        }),
    );
    let mut done = task.clone();
    done["revision"] = subsequent["task_revision"].clone();
    let done = update_task(&client, &done, json!({"status": "done"}));

    let dashboard = Dashboard::start(&database, 0);
    let context_path = format!("/api/projects/{project_id}/tasks/{}", task["id"]);
    let results_path = format!("{context_path}/results");
    let before_review = dashboard.get(&context_path).json();
    assert_eq!(before_review["task"]["status"], "done");
    assert_eq!(before_review["outstanding_feedback"], json!([]));

    // The existing dashboard process sees feedback written by a concurrent MCP client.
    let feedback = call(
        &client,
        "record_task_feedback",
        json!({
            "task_id": task["id"],
            "result_id": original["id"],
            "expected_revision": done["revision"],
            "disposition": "changes_requested",
            "body": "## Changes required\n\nThe output loses state after restart. Persist the result and verify a fresh process can read it."
        }),
    );
    let reviewed = dashboard.get(&context_path).json();
    assert_eq!(reviewed["task"]["status"], "todo");
    assert_eq!(reviewed["task"]["revision"], feedback["task_revision"]);
    assert_eq!(reviewed["outstanding_feedback"][0]["feedback"], feedback);
    assert_eq!(
        reviewed["outstanding_feedback"][0]["result"]["id"],
        original["id"]
    );
    assert_eq!(
        reviewed["outstanding_feedback"][0]["result"]["output_source_ids"],
        json!([output["id"]])
    );
    assert_eq!(feedback["project_id"], project_id);
    assert_eq!(feedback["task_id"], task["id"]);
    assert!(
        feedback["recorded_at"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(feedback["addressed_by_result_id"].is_null());

    let revised = update_task(
        &client,
        &reviewed["task"],
        json!({"design": "Persist outputs and verify them after restarting the process."}),
    );
    let mut revision = revised["revision"].clone();
    let mut newest_failure_id = Value::Null;
    for index in 0..21 {
        let failure = call(
            &client,
            "record_task_result",
            json!({
                "task_id": task["id"],
                "expected_revision": revision,
                "outcome": "failure",
                "evidence": format!("Revision attempt {index}: restart persistence still fails.")
            }),
        );
        revision = failure["task_revision"].clone();
        newest_failure_id = failure["id"].clone();
    }
    let outstanding = dashboard.get(&context_path).json();
    assert_eq!(outstanding["task"]["design"], revised["design"]);
    assert_eq!(outstanding["latest_result"]["id"], newest_failure_id);
    assert_eq!(
        outstanding["latest_successful_result"]["id"],
        subsequent["id"]
    );
    assert_eq!(
        outstanding["outstanding_feedback"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(outstanding["outstanding_feedback"][0]["feedback"], feedback);
    assert_eq!(
        outstanding["outstanding_feedback"][0]["result"]["evidence"],
        original["evidence"]
    );
    let first_page = dashboard.get(&results_path).json();
    assert_eq!(first_page["results"].as_array().unwrap().len(), 20);
    assert!(
        first_page["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|result| result["id"] != original["id"])
    );
    let older_path = format!(
        "{results_path}?before_id={}",
        first_page["next_before_id"].as_i64().unwrap()
    );
    let older = dashboard.get(&older_path).json();
    let original_with_feedback = older["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["id"] == original["id"])
        .unwrap();
    assert_eq!(original_with_feedback["feedback"], json!([feedback]));

    let foreign_context = format!(
        "/api/projects/{}/tasks/{}",
        foreign_project["id"].as_str().unwrap(),
        task["id"]
    );
    assert_eq!(dashboard.get(&foreign_context).status, 404);
    assert_eq!(
        dashboard.get(&format!("{foreign_context}/results")).status,
        404
    );
    let state_before_reads = call(&client, "get_task_context", json!({"task_id": task["id"]}));
    let history_before_reads = call(
        &client,
        "list_task_results",
        json!({"task_id": task["id"], "page_size": 100}),
    );
    for _ in 0..3 {
        assert_eq!(dashboard.get(&context_path).json(), outstanding);
        assert_eq!(dashboard.get(&older_path).json(), older);
        assert_eq!(
            dashboard.request("POST", &context_path, None, &[]).status,
            405
        );
        assert_eq!(
            dashboard.request("POST", &results_path, None, &[]).status,
            405
        );
    }
    assert_eq!(
        call(&client, "get_task_context", json!({"task_id": task["id"]})),
        state_before_reads
    );
    assert_eq!(
        call(
            &client,
            "list_task_results",
            json!({"task_id": task["id"], "page_size": 100})
        ),
        history_before_reads
    );

    // Only the explicitly identified successful follow-up resolves the request.
    let addressed = call(
        &client,
        "record_task_result",
        json!({
            "task_id": task["id"],
            "expected_revision": revision,
            "outcome": "success",
            "evidence": "Restarted both processes and verified the stored output persists.",
            "addressed_feedback_ids": [feedback["id"]]
        }),
    );
    let resolved = dashboard.get(&context_path).json();
    assert_eq!(resolved["outstanding_feedback"], json!([]));
    assert_eq!(resolved["latest_result"]["id"], addressed["id"]);
    assert_eq!(
        resolved["latest_result"]["addressed_feedback_ids"],
        json!([feedback["id"]])
    );
    assert_eq!(resolved["task"]["status"], "todo");
    let resolved_history = dashboard.get(&older_path).json();
    let resolved_original = resolved_history["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["id"] == original["id"])
        .unwrap();
    assert_eq!(
        resolved_original["feedback"][0]["addressed_by_result_id"],
        addressed["id"]
    );

    drop(client);
    let port = dashboard.port;
    drop(dashboard);
    wait_for_port_free(port);
    let restarted_client = Project::open(&database).unwrap();
    let restarted = Dashboard::start(&database, port);
    assert_eq!(restarted.get(&context_path).json(), resolved);
    assert_eq!(restarted.get(&older_path).json(), resolved_history);
    assert_eq!(
        call(
            &restarted_client,
            "get_task_context",
            json!({"project_id": project_id, "task_id": task["id"]})
        ),
        resolved
    );
}

#[test]
fn dashboard_exposes_project_decisions_and_sources_with_scope_and_persistence() {
    let directory = tempfile::tempdir().unwrap();
    let database = dashboard_database(&directory);
    let client = Project::open(&database).unwrap();
    let project_a = client.project_id.clone();

    let inline = call(
        &client,
        "create_source",
        json!({
            "name": "Linked design note",
            "description": "A context source attached to a task",
            "content": "# Design\n\nKeep decisions and sources below the task board.",
            "context_policy": "always"
        }),
    );
    let external = call(
        &client,
        "create_source",
        json!({
            "name": "External specification",
            "description": "An unlinked locator with a manual action",
            "locator": "https://example.com/specification",
            "action": {
                "instruction": "Review the external specification.",
                "trigger": {"kind": "manual"}
            }
        }),
    );
    create_task(
        &client,
        &project_a,
        "Linked task",
        "one_time",
        json!({"context_source_ids": [inline["id"]]}),
    );
    let first_decision = call(
        &client,
        "record_decision",
        json!({
            "statement": "Keep tasks above project reference sections.",
            "rationale": "Tasks are the dashboard's primary content."
        }),
    );
    let current_decision = call(
        &client,
        "record_decision",
        json!({
            "statement": "Show decisions and sources below the task board.",
            "rationale": "They remain available without opening a task.",
            "supersedes_id": first_decision["id"]
        }),
    );

    let other_project: Value = client
        .raw_call(
            "create_project",
            json!({
                "name": "Isolation project",
                "description": "Records must not leak across project scopes"
            }),
        )
        .unwrap();
    let other_project = other_project["id"].as_str().unwrap().to_string();
    let other_source: Value = client
        .raw_call(
            "create_source",
            json!({
                "project_id": other_project,
                "name": "Other project source",
                "description": "Belongs only to the isolation project",
                "content": "Other project content"
            }),
        )
        .unwrap();
    let other_decision: Value = client
        .raw_call(
            "record_decision",
            json!({
                "project_id": other_project,
                "statement": "Other project decision",
                "rationale": "Belongs only to the isolation project"
            }),
        )
        .unwrap();

    let dashboard = Dashboard::start(&database, 0);
    let decisions_path = format!("/api/projects/{project_a}/decisions");
    let sources_path = format!("/api/projects/{project_a}/sources");

    let decisions = dashboard.get(&decisions_path).json();
    let decisions = decisions["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 1, "only current decisions are returned");
    assert_eq!(decisions[0]["id"], current_decision["id"]);
    assert_eq!(decisions[0]["supersedes_id"], first_decision["id"]);
    assert_eq!(decisions[0]["is_current"], true);

    let sources = dashboard.get(&sources_path).json();
    let sources = sources["sources"].as_array().unwrap();
    assert_eq!(
        sources.len(),
        2,
        "linked and unlinked project sources are listed"
    );
    for source in sources {
        assert!(
            source.get("content").is_none() && source.get("locator").is_none(),
            "descriptors stay compact"
        );
    }
    let inline_descriptor = sources
        .iter()
        .find(|source| source["id"] == inline["id"])
        .unwrap();
    assert_eq!(inline_descriptor["representation"], "inline");
    assert_eq!(inline_descriptor["context_policy"], "always");
    assert_eq!(inline_descriptor["has_action"], false);
    let external_descriptor = sources
        .iter()
        .find(|source| source["id"] == external["id"])
        .unwrap();
    assert_eq!(external_descriptor["representation"], "external");
    assert_eq!(external_descriptor["context_policy"], "when_relevant");
    assert_eq!(external_descriptor["has_action"], true);
    assert_eq!(external_descriptor["action_due"], false);
    assert!(external_descriptor["next_action_at"].is_null());

    let other_decisions = dashboard
        .get(&format!("/api/projects/{other_project}/decisions"))
        .json();
    let other_decisions = other_decisions["decisions"].as_array().unwrap();
    assert_eq!(other_decisions.len(), 1);
    assert_eq!(other_decisions[0]["id"], other_decision["id"]);
    let other_sources = dashboard
        .get(&format!("/api/projects/{other_project}/sources"))
        .json();
    let other_sources = other_sources["sources"].as_array().unwrap();
    assert_eq!(other_sources.len(), 1);
    assert_eq!(other_sources[0]["id"], other_source["id"]);
    assert_eq!(
        dashboard
            .get("/api/projects/00000000-0000-7000-8000-000000000000/decisions")
            .status,
        404
    );
    assert_eq!(
        dashboard
            .get("/api/projects/00000000-0000-7000-8000-000000000000/sources")
            .status,
        404
    );
    assert_eq!(
        dashboard.request("POST", &decisions_path, None, &[]).status,
        405
    );
    assert_eq!(
        dashboard.request("POST", &sources_path, None, &[]).status,
        405
    );

    // Repeated reads do not advance decisions or source action state.
    let decisions_before = call(
        &client,
        "list_decisions",
        json!({"include_superseded": true}),
    );
    let sources_before = call(&client, "list_sources", json!({}));
    for _ in 0..3 {
        assert_eq!(dashboard.get(&decisions_path).status, 200);
        assert_eq!(dashboard.get(&sources_path).status, 200);
        assert_eq!(
            dashboard
                .get(&format!(
                    "/api/projects/{project_a}/sources/{}",
                    external["id"]
                ))
                .status,
            200
        );
    }
    let decisions_after = call(
        &client,
        "list_decisions",
        json!({"include_superseded": true}),
    );
    let sources_after = call(&client, "list_sources", json!({}));
    assert_eq!(decisions_before, decisions_after);
    assert_eq!(sources_before, sources_after);

    let port = dashboard.port;
    drop(dashboard);
    wait_for_port_free(port);
    let restarted = Dashboard::start(&database, port);
    assert_eq!(
        restarted.get(&decisions_path).json()["decisions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        restarted.get(&sources_path).json()["sources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn dashboard_runs_one_shot_agents_with_duplicate_and_cwd_protection() {
    let directory = tempfile::tempdir().unwrap();
    let database = dashboard_database(&directory);
    let client = Project::open(&database).unwrap();
    let project_id = client.project_id.clone();
    let target = create_task(&client, &project_id, "Run target", "one_time", json!({}));
    let other = create_task(
        &client,
        &project_id,
        "Other run target",
        "one_time",
        json!({}),
    );
    let claude_task = create_task(
        &client,
        &project_id,
        "Claude run target",
        "one_time",
        json!({}),
    );

    let fixtures = directory.path().join("fixtures");
    std::fs::create_dir_all(&fixtures).unwrap();
    let codex = write_agent_script(&fixtures, "codex-agent", codex_fixture());
    let claude = write_agent_script(&fixtures, "claude-agent", claude_fixture());
    let success_dir = directory.path().join("success");
    let other_dir = directory.path().join("other");
    let claude_dir = directory.path().join("claude");
    for path in [&success_dir, &other_dir, &claude_dir] {
        std::fs::create_dir_all(path).unwrap();
    }
    let environment = [
        ("DICTUM_FIXTURE_DIR", fixtures.to_str().unwrap()),
        ("DICTUM_CODEX_BIN", codex.to_str().unwrap()),
        ("DICTUM_CLAUDE_BIN", claude.to_str().unwrap()),
    ];
    let dashboard = Dashboard::start_with_env(&database, 0, &environment);
    let (token, timeout) = dashboard.run_token();
    assert_eq!(timeout, 1_800);
    let origin = dashboard.origin();

    let target_runs = format!("/api/projects/{project_id}/tasks/{}/runs", target["id"]);
    let other_runs = format!("/api/projects/{project_id}/tasks/{}/runs", other["id"]);
    let claude_runs = format!(
        "/api/projects/{project_id}/tasks/{}/runs",
        claude_task["id"]
    );

    // Run history is readable before any attempt exists.
    let get_launch = dashboard.get(&target_runs);
    assert_eq!(get_launch.status, 200);
    assert_eq!(get_launch.json()["runs"], json!([]));
    let unsupported = dashboard.request("PUT", &target_runs, None, &[]);
    assert_eq!(unsupported.status, 405);

    // Mutation authenticity: Origin, JSON content type and the per-service token.
    let launch_body = json!({
        "agent": "codex",
        "working_directory": success_dir.to_str().unwrap()
    });
    assert_eq!(
        dashboard
            .post_json(&target_runs, &launch_body, None, Some(&token))
            .status,
        403
    );
    assert_eq!(
        dashboard
            .post_json(
                &target_runs,
                &launch_body,
                Some("http://evil.example"),
                Some(&token)
            )
            .status,
        403
    );
    assert_eq!(
        dashboard
            .post_json(&target_runs, &launch_body, Some(&origin), None)
            .status,
        403
    );
    assert_eq!(
        dashboard
            .post_json(&target_runs, &launch_body, Some(&origin), Some("wrong"))
            .status,
        403
    );
    assert_eq!(
        dashboard
            .post_raw(
                &target_runs,
                "text/plain",
                &serde_json::to_vec(&launch_body).unwrap(),
                Some(&origin),
                Some(&token)
            )
            .status,
        415
    );
    assert_eq!(
        dashboard
            .post_json(
                &target_runs,
                &json!({"agent": "codex", "working_directory": "relative/path"}),
                Some(&origin),
                Some(&token)
            )
            .status,
        400
    );
    assert_eq!(
        dashboard
            .post_json(
                &target_runs,
                &json!({
                    "agent": "codex",
                    "working_directory": success_dir.to_str().unwrap(),
                    "command": "rm -rf /"
                }),
                Some(&origin),
                Some(&token)
            )
            .status,
        400
    );

    let foreign_project: Value = client
        .raw_call(
            "create_project",
            json!({"name": "Foreign", "description": "Run scope must not cross projects."}),
        )
        .unwrap();
    let foreign_runs = format!(
        "/api/projects/{}/tasks/{}/runs",
        foreign_project["id"].as_str().unwrap(),
        target["id"]
    );
    assert_eq!(
        dashboard
            .post_json(&foreign_runs, &launch_body, Some(&origin), Some(&token))
            .status,
        404
    );
    assert_eq!(dashboard.get(&foreign_runs).status, 404);

    let launched = dashboard.post_json(&target_runs, &launch_body, Some(&origin), Some(&token));
    assert_eq!(launched.status, 201, "{}", launched.body);
    let run = launched.json();
    let run_id = run["id"].as_i64().unwrap();
    assert_eq!(run["project_id"], project_id);
    assert_eq!(run["task_id"], target["id"]);
    assert_eq!(run["status"], "running");
    assert_eq!(run["agent"], "codex");
    assert!(run["agent_version"].as_str().unwrap().contains("0.145.0"));
    assert_eq!(
        run["working_directory"],
        success_dir.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(run["input_task_revision"], target["revision"]);

    // Duplicate launches are rejected for the task and for the working directory.
    let duplicate = dashboard.post_json(&target_runs, &launch_body, Some(&origin), Some(&token));
    assert_eq!(duplicate.status, 409);
    assert!(duplicate.body.contains("active run"));
    let cwd_conflict = dashboard.post_json(&other_runs, &launch_body, Some(&origin), Some(&token));
    assert_eq!(cwd_conflict.status, 409);
    assert!(cwd_conflict.body.contains("working directory"));

    let other_launch = dashboard.post_json(
        &other_runs,
        &json!({"agent": "codex", "working_directory": other_dir.to_str().unwrap()}),
        Some(&origin),
        Some(&token),
    );
    assert_eq!(other_launch.status, 201, "{}", other_launch.body);
    let other_run = other_launch.json();

    let claude_launch = dashboard.post_json(
        &claude_runs,
        &json!({"agent": "claude", "working_directory": claude_dir.to_str().unwrap()}),
        Some(&origin),
        Some(&token),
    );
    assert_eq!(claude_launch.status, 201, "{}", claude_launch.body);
    let claude_run = claude_launch.json();

    let detail_path = format!("{target_runs}/{run_id}");
    let detail = wait_for_run(&dashboard, &detail_path);
    assert_eq!(detail["run"]["status"], "succeeded");
    assert_eq!(detail["run"]["exit_code"], 0);
    assert!(detail["run"]["ended_at"].is_string());
    assert!(
        detail["progress"]
            .as_str()
            .unwrap()
            .contains("Fixture agent started"),
        "unexpected progress: {}",
        detail["progress"]
    );
    assert!(
        detail["progress"]
            .as_str()
            .unwrap()
            .contains("echo checked")
    );
    assert!(detail["log"].as_str().unwrap().contains("item.completed"));
    assert_eq!(detail["results"], json!([]));

    // The fixture received the exact task prompt, run identity and MCP environment.
    let argv = wait_for_file(&fixtures.join("success-argv.txt"));
    assert!(argv.contains(&format!("Dictum one-shot run {run_id}")));
    assert!(argv.contains("workspace-write"));
    assert!(argv.contains("--ignore-user-config"));
    assert!(argv.contains("mcp_servers.dictum.env.DICTUM_RUN_ID"));
    assert!(argv.contains(&format!(
        "mcp_servers.dictum.command=\"{}\"",
        env!("CARGO_BIN_EXE_dictum-mcp")
    )));
    let captured_env = wait_for_file(&fixtures.join("success-env.txt"));
    assert!(captured_env.contains(&format!("DICTUM_RUN_ID={run_id}")));
    assert!(captured_env.contains("DICTUM_DATABASE="));
    assert!(success_dir.join("fixture-output.txt").exists());

    let claude_detail = wait_for_run(&dashboard, &format!("{claude_runs}/{}", claude_run["id"]));
    assert_eq!(claude_detail["run"]["status"], "succeeded");
    assert!(
        claude_detail["progress"]
            .as_str()
            .unwrap()
            .contains("Fixture Claude agent started")
    );
    let claude_argv = wait_for_file(&fixtures.join("claude-argv.txt"));
    assert!(claude_argv.contains("--permission-mode"));
    assert!(claude_argv.contains("acceptEdits"));
    assert!(claude_argv.contains("--permission-prompts"));
    assert!(claude_argv.contains("--strict-mcp-config"));
    assert!(claude_argv.contains("--allowedTools"));
    assert!(claude_argv.contains("mcp__dictum"));
    assert!(claude_argv.contains("claude-md-and-agents-md"));
    let claude_env = wait_for_file(&fixtures.join("claude-env.txt"));
    assert!(claude_env.contains(&format!(
        "DICTUM_RUN_ID={}",
        claude_run["id"].as_i64().unwrap()
    )));

    // A successful process does not change lifecycle state.
    let context = call(
        &client,
        "get_task_context",
        json!({"task_id": target["id"]}),
    );
    assert_eq!(context["task"]["status"], "todo");

    // A Dictum MCP process spawned for the attempt tags its result with the run.
    let run_id_text = run_id.to_string();
    let run_client = Project::open_with_env(&database, &[("DICTUM_RUN_ID", &run_id_text)]).unwrap();
    let other_context = call(&client, "get_task_context", json!({"task_id": other["id"]}));
    let wrong_task = run_client
        .raw_call::<Value>(
            "record_task_result",
            json!({
                "project_id": project_id,
                "task_id": other["id"],
                "expected_revision": other_context["task"]["revision"],
                "outcome": "success",
                "evidence": "This must not be attributed to another task."
            }),
        )
        .unwrap_err()
        .to_string();
    assert!(
        wrong_task.contains("does not belong"),
        "unexpected error: {wrong_task}"
    );
    let linked = run_client
        .call::<Value>(
            "record_task_result",
            json!({
                "project_id": project_id,
                "task_id": target["id"],
                "expected_revision": context["task"]["revision"],
                "outcome": "success",
                "evidence": "Fixture result linked to its launching run."
            }),
        )
        .unwrap();
    let linked_detail = dashboard.get(&detail_path).json();
    assert_eq!(linked_detail["results"][0]["id"], linked["id"]);
    assert_eq!(
        linked_detail["results"][0]["evidence"],
        "Fixture result linked to its launching run."
    );
    let rejected = run_client
        .raw_call::<Value>(
            "update_task",
            json!({
                "project_id": project_id,
                "task_id": target["id"],
                "expected_revision": linked["task_revision"],
                "status": "done"
            }),
        )
        .unwrap_err()
        .to_string();
    assert!(
        rejected.contains("scoped to dashboard run"),
        "unexpected error: {rejected}"
    );

    let _ = wait_for_run(&dashboard, &format!("{other_runs}/{}", other_run["id"]));

    // Only one live dashboard service may own runs for a database.
    let second = run_dashboard(&database, &["--port", "0"]);
    assert!(!second.status.success());
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("another dashboard service"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    // Restarting the dashboard rotates the request token and preserves run history.
    let port = dashboard.port;
    drop(dashboard);
    wait_for_port_free(port);
    let restarted = Dashboard::start_with_env(&database, port, &environment);
    let (new_token, _) = restarted.run_token();
    assert_ne!(new_token, token);
    let persisted = restarted.get(&detail_path).json();
    assert_eq!(persisted["run"]["status"], "succeeded");
    assert_eq!(persisted["results"][0]["id"], linked["id"]);
    assert_eq!(
        restarted.get(&target_runs).json()["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn dashboard_run_failures_cancel_timeout_restart_and_repeat_attempts_are_durable() {
    let directory = tempfile::tempdir().unwrap();
    let database = dashboard_database(&directory);
    let client = Project::open(&database).unwrap();
    let project_id = client.project_id.clone();

    let fail_task = create_task(
        &client,
        &project_id,
        "Failing attempt",
        "one_time",
        json!({}),
    );
    let cancel_task = create_task(
        &client,
        &project_id,
        "Cancelled attempt",
        "one_time",
        json!({}),
    );
    let timeout_task = create_task(
        &client,
        &project_id,
        "Timeout attempt",
        "one_time",
        json!({}),
    );
    let restart_task = create_task(
        &client,
        &project_id,
        "Restart attempt",
        "one_time",
        json!({}),
    );
    let gate = create_task(&client, &project_id, "Gate", "one_time", json!({}));
    let gated = create_task(
        &client,
        &project_id,
        "Start-gated attempt",
        "one_time",
        json!({"prerequisite_task_ids": [gate["id"]]}),
    );
    let done = create_task(
        &client,
        &project_id,
        "Completed attempt",
        "one_time",
        json!({}),
    );
    let done_result = call(
        &client,
        "record_task_result",
        json!({
            "task_id": done["id"],
            "expected_revision": done["revision"],
            "outcome": "success",
            "evidence": "Completed before the run eligibility check."
        }),
    );
    let mut done_task = done.clone();
    done_task["revision"] = done_result["task_revision"].clone();
    update_task(&client, &done_task, json!({"status": "done"}));
    let recurring = create_task(
        &client,
        &project_id,
        "Recurring attempt",
        "recurring",
        json!({}),
    );
    let paused = create_task(
        &client,
        &project_id,
        "Paused recurring attempt",
        "recurring",
        json!({}),
    );
    update_task(&client, &paused, json!({"status": "paused"}));

    let fixtures = directory.path().join("fixtures");
    std::fs::create_dir_all(&fixtures).unwrap();
    let codex = write_agent_script(&fixtures, "codex-agent", codex_fixture());
    let claude = write_agent_script(&fixtures, "claude-agent", claude_fixture());
    let directories = [
        "fail",
        "cancel",
        "timeout",
        "restart",
        "recurring",
        "success",
    ];
    for name in directories {
        std::fs::create_dir_all(directory.path().join(name)).unwrap();
    }
    let environment = [
        ("DICTUM_FIXTURE_DIR", fixtures.to_str().unwrap()),
        ("DICTUM_CODEX_BIN", codex.to_str().unwrap()),
        ("DICTUM_CLAUDE_BIN", claude.to_str().unwrap()),
        ("DICTUM_RUN_TIMEOUT_SECONDS", "2"),
    ];
    let dashboard = Dashboard::start_with_env(&database, 0, &environment);
    let (token, timeout) = dashboard.run_token();
    assert_eq!(timeout, 2);
    let origin = dashboard.origin();

    let launch = |task: &Value, directory_name: &str| {
        let path = format!("/api/projects/{project_id}/tasks/{}/runs", task["id"]);
        let body = json!({
            "agent": "codex",
            "working_directory": directory.path().join(directory_name).to_str().unwrap()
        });
        let response = dashboard.post_json(&path, &body, Some(&origin), Some(&token));
        (path, response)
    };

    let (gated_path, gated_response) = launch(&gated, "success");
    assert_eq!(gated_response.status, 400);
    assert!(gated_response.body.contains("before-start"));
    assert_eq!(dashboard.get(&gated_path).json()["runs"], json!([]));
    let (_, paused_response) = launch(&paused, "success");
    assert_eq!(paused_response.status, 400);
    assert!(paused_response.body.contains("only active recurring"));
    let (_, done_response) = launch(&done_task, "success");
    assert_eq!(done_response.status, 400);
    assert!(done_response.body.contains("done one-time task"));

    let (recurring_path, recurring_response) = launch(&recurring, "recurring");
    assert_eq!(
        recurring_response.status, 201,
        "{}",
        recurring_response.body
    );
    let recurring_run = recurring_response.json();
    let recurring_detail = wait_for_run(
        &dashboard,
        &format!("{recurring_path}/{}", recurring_run["id"]),
    );
    assert_eq!(recurring_detail["run"]["status"], "succeeded");
    let recurring_context = call(
        &client,
        "get_task_context",
        json!({"task_id": recurring["id"]}),
    );
    assert_eq!(recurring_context["task"]["status"], "active");

    // Failure reports the exit status and stderr without inventing a task result.
    let (fail_path, fail_response) = launch(&fail_task, "fail");
    assert_eq!(fail_response.status, 201);
    let fail_run = fail_response.json();
    let fail_detail = wait_for_run(&dashboard, &format!("{fail_path}/{}", fail_run["id"]));
    assert_eq!(fail_detail["run"]["status"], "failed");
    assert_eq!(fail_detail["run"]["exit_code"], 7);
    assert!(
        fail_detail["progress"]
            .as_str()
            .unwrap()
            .contains("permission denied"),
        "unexpected progress: {}",
        fail_detail["progress"]
    );
    assert_eq!(fail_detail["results"], json!([]));

    // The user resolves the reason, updates the durable brief, and explicitly retries.
    let fail_context = call(
        &client,
        "get_task_context",
        json!({"task_id": fail_task["id"]}),
    );
    let updated = update_task(
        &client,
        &fail_context["task"],
        json!({"progress": "Retry after reading the permission failure."}),
    );
    let (_, repeat_response) = launch(&fail_task, "fail");
    assert_eq!(repeat_response.status, 201);
    let repeat_run = repeat_response.json();
    assert_eq!(repeat_run["input_task_revision"], updated["revision"]);
    let repeat_detail = wait_for_run(&dashboard, &format!("{fail_path}/{}", repeat_run["id"]));
    assert_eq!(repeat_detail["run"]["status"], "failed");
    assert_eq!(
        dashboard.get(&fail_path).json()["runs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Cancellation terminates the agent and its descendant process group.
    let (cancel_path, cancel_response) = launch(&cancel_task, "cancel");
    assert_eq!(cancel_response.status, 201);
    let cancel_run = cancel_response.json();
    let cancel_pids = wait_for_file(&fixtures.join("cancel-pids.txt"));
    let cancel_pids: Vec<i64> = cancel_pids
        .lines()
        .map(|line| line.trim().parse().unwrap())
        .collect();
    let cancel_response = dashboard.post_json(
        &format!("{cancel_path}/{}/cancel", cancel_run["id"]),
        &json!({}),
        Some(&origin),
        Some(&token),
    );
    assert_eq!(cancel_response.status, 200, "{}", cancel_response.body);
    let cancel_detail = wait_for_run(&dashboard, &format!("{cancel_path}/{}", cancel_run["id"]));
    assert_eq!(cancel_detail["run"]["status"], "cancelled");
    assert_eq!(cancel_detail["run"]["stop_reason"], "cancelled by user");
    assert!(cancel_detail["run"]["cancel_requested_at"].is_string());
    for pid in cancel_pids {
        wait_for_process_exit(pid);
    }

    // A bounded execution time terminates the same process group.
    let (timeout_path, timeout_response) = launch(&timeout_task, "timeout");
    assert_eq!(timeout_response.status, 201);
    let timeout_run = timeout_response.json();
    let timeout_pids = wait_for_file(&fixtures.join("timeout-pids.txt"));
    let timeout_pids: Vec<i64> = timeout_pids
        .lines()
        .map(|line| line.trim().parse().unwrap())
        .collect();
    let timeout_detail = wait_for_run(&dashboard, &format!("{timeout_path}/{}", timeout_run["id"]));
    assert_eq!(timeout_detail["run"]["status"], "timed_out");
    assert!(
        timeout_detail["run"]["stop_reason"]
            .as_str()
            .unwrap()
            .contains("2 second run limit")
    );
    for pid in timeout_pids {
        wait_for_process_exit(pid);
    }

    // A hard service restart leaves durable interrupted state and cleans up the
    // marked agent process group when the new service starts.
    let (restart_path, restart_response) = launch(&restart_task, "restart");
    assert_eq!(restart_response.status, 201);
    let restart_run = restart_response.json();
    let restart_pids = wait_for_file(&fixtures.join("restart-pids.txt"));
    let restart_pids: Vec<i64> = restart_pids
        .lines()
        .map(|line| line.trim().parse().unwrap())
        .collect();
    let port = dashboard.port;
    drop(dashboard);
    wait_for_port_free(port);
    let restarted = Dashboard::start_with_env(&database, port, &environment);
    let restart_detail = wait_for_run(&restarted, &format!("{restart_path}/{}", restart_run["id"]));
    assert_eq!(restart_detail["run"]["status"], "interrupted");
    assert!(
        restart_detail["run"]["stop_reason"]
            .as_str()
            .unwrap()
            .contains("restarted")
    );
    for pid in restart_pids {
        wait_for_process_exit(pid);
    }
    assert_eq!(
        restarted.get(&restart_path).json()["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn dashboard_rejects_unavailable_or_old_agents_without_creating_a_run() {
    let directory = tempfile::tempdir().unwrap();
    let database = dashboard_database(&directory);
    let client = Project::open(&database).unwrap();
    let project_id = client.project_id.clone();
    let task = create_task(&client, &project_id, "Run gate", "one_time", json!({}));
    let workdir = directory.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    let missing = directory.path().join("missing-agent");
    let old = write_agent_script(
        directory.path(),
        "old-agent",
        "#!/bin/sh\nif [ \"${1:-}\" = \"--version\" ]; then echo 'codex-cli 0.1.0'; exit 0; fi\nexit 0\n",
    );
    let runs_path = format!("/api/projects/{project_id}/tasks/{}/runs", task["id"]);
    let body = json!({"agent": "codex", "working_directory": workdir.to_str().unwrap()});

    for (agent, expected) in [
        (missing.to_str().unwrap(), "cannot run"),
        (old.to_str().unwrap(), "too old"),
    ] {
        let environment = [
            ("DICTUM_CODEX_BIN", agent),
            ("DICTUM_CLAUDE_BIN", missing.to_str().unwrap()),
        ];
        let dashboard = Dashboard::start_with_env(&database, 0, &environment);
        let (token, _) = dashboard.run_token();
        let origin = dashboard.origin();
        let response = dashboard.post_json(&runs_path, &body, Some(&origin), Some(&token));
        assert_eq!(response.status, 400, "{}", response.body);
        assert!(
            response.body.contains(expected),
            "expected {expected:?} in {}",
            response.body
        );
        assert_eq!(dashboard.get(&runs_path).json()["runs"], json!([]));
        let port = dashboard.port;
        drop(dashboard);
        wait_for_port_free(port);
    }
}

#[test]
fn dashboard_rejects_invalid_cli_ports_and_reports_usage() {
    let directory = tempfile::tempdir().unwrap();
    let database = dashboard_database(&directory);
    for arguments in [
        vec!["--port", "65536"],
        vec!["--port", "not-a-port"],
        vec!["--port"],
        vec!["--unknown"],
    ] {
        let output = run_dashboard(&database, &arguments);
        assert!(
            !output.status.success(),
            "expected failure for {arguments:?}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("--port") || stderr.contains("unknown dashboard option"),
            "unclear error for {arguments:?}: {stderr}"
        );
    }
    let help = run_dashboard(&database, &["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: dictum-mcp"));
}
