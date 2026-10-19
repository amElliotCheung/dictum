//! Real-process dashboard tests: MCP seeds/mutates SQLite, HTTP reads it.
mod support;

use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
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
        let mut child = Command::new(env!("CARGO_BIN_EXE_dictum-mcp"))
            .arg("dashboard")
            .arg("--port")
            .arg(port.to_string())
            .env("DICTUM_DATABASE", database)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start dashboard process");
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
        http_request(self.port, method, path, &host, headers)
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
    request.push_str("Content-Length: 0\r\n\r\n");
    stream.write_all(request.as_bytes()).unwrap();
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
    update_task(
        &client,
        &done,
        json!({"status": "done", "result": "Verified in the dashboard fixture."}),
    );
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
    let task = update_task(
        &client,
        &task,
        json!({
            "output_source_ids": [output_source["id"]],
            "progress": "Prepared for history"
        }),
    );
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
    assert_eq!(
        context["task"]["output_source_ids"],
        json!([output_source["id"]])
    );
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
