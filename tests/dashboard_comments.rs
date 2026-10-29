//! Comment writes and hierarchy discovery through real HTTP and MCP processes.
mod support;

use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use support::Project;

struct Dashboard {
    child: Child,
    port: u16,
    _stderr: BufReader<std::process::ChildStderr>,
}

impl Dashboard {
    fn start(database: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dictum-mcp"))
            .args(["dashboard", "--port", "0"])
            .env("DICTUM_DATABASE", database)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = BufReader::new(child.stderr.take().unwrap());
        let mut dashboard = Self {
            child,
            port: 0,
            _stderr: stderr,
        };
        loop {
            let mut line = String::new();
            assert!(
                dashboard._stderr.read_line(&mut line).unwrap() > 0,
                "dashboard exited"
            );
            if let Some(url) = line
                .trim()
                .strip_prefix("dictum-mcp dashboard: listening on ")
            {
                dashboard.port = url.rsplit_once(':').unwrap().1.parse().unwrap();
                break;
            }
        }
        dashboard
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn get(&self, path: &str) -> (u16, Value) {
        request(self.port, "GET", path, &[], "")
    }

    fn token(&self) -> String {
        let (status, body) = self.get("/api/request-token");
        assert_eq!(status, 200);
        body["token"].as_str().unwrap().to_owned()
    }

    fn post(&self, path: &str, token: &str, body: Value) -> (u16, Value) {
        request(
            self.port,
            "POST",
            path,
            &[
                ("Origin", &self.origin()),
                ("Content-Type", "application/json"),
                ("X-Dictum-CSRF", token),
            ],
            &body.to_string(),
        )
    }
}

impl Drop for Dashboard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut wire =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    for (name, value) in headers {
        wire.push_str(&format!("{name}: {value}\r\n"));
    }
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-length"))
    {
        wire.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    wire.push_str("\r\n");
    wire.push_str(body);
    stream.write_all(wire.as_bytes()).unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(body).unwrap())
}

fn call(client: &Project, tool: &str, input: Value) -> Value {
    client.call(tool, input).unwrap()
}

fn task(client: &Project, extra: Value) -> Value {
    let mut input = json!({
        "title": "Comment target",
        "problem_or_goal": "Keep discussion attached to the version actually read.",
        "design": "Append immutable comments through the real dashboard.",
        "acceptance_criteria": ["Version references survive task edits and restarts."]
    });
    input
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    call(client, "create_task", input)
}

#[test]
fn dashboard_comments_keep_versions_outputs_replies_and_history_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("comments.sqlite3");
    let client = Project::open(&database).unwrap();
    let parent = task(&client, json!({}));
    let child = task(&client, json!({"parent_task_id": parent["id"]}));
    let output = call(
        &client,
        "create_source",
        json!({
            "name": "Browser proof", "description": "Evidence produced by this execution", "content": "# Passed"
        }),
    );
    let dashboard = Dashboard::start(&database);
    let base = format!("/api/projects/{}/tasks", client.project_id);
    let path = format!("{base}/{}/comments", parent["id"]);
    let token = dashboard.token();

    let (status, headers) = dashboard.get(&format!("{base}?parent_task_id={}", parent["id"]));
    assert_eq!(status, 200);
    assert_eq!(headers["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(headers["tasks"][0]["id"], child["id"]);
    assert!(headers["tasks"][0].get("parent_task_id").is_none());
    assert_eq!(
        dashboard.get(&format!("{base}?parent_task_id=null")).1["tasks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        dashboard.get(&format!("{base}/{}", child["id"])).1["parent_task"],
        parent
    );

    let (status, note) = dashboard.post(
        &path,
        &token,
        json!({"task_revision": 1, "body": "Original version discussion"}),
    );
    assert_eq!(status, 201);
    assert!(note["execution_result"].is_null());
    assert_eq!(
        client
            .get_task_context(parent["id"].as_i64().unwrap())
            .unwrap()
            .task
            .revision,
        1
    );
    let edited = call(
        &client,
        "update_task",
        json!({
            "task_id": parent["id"], "expected_revision": 1, "title": "Updated task"
        }),
    );
    let (status, execution) = dashboard.post(&path, &token, json!({
        "task_revision": 1, "body": "Delayed report for the original version: browser check passed.",
        "execution_result": {"outcome": "success", "output_source_ids": [output["id"]]}
    }));
    assert_eq!(status, 201);
    assert_eq!(execution["task_revision"], 1);
    let (status, reply) = dashboard.post(&path, &token, json!({
        "task_revision": 2, "body": "This reply concerns the updated brief.", "reply_to_comment_id": note["id"]
    }));
    assert_eq!(status, 201);
    assert_eq!(
        dashboard.get(&format!("{path}/{}", execution["id"])).1,
        execution
    );
    let page = dashboard.get(&format!("{path}?page_size=1")).1;
    assert_eq!(page["comments"][0], reply);
    let later = call(
        &client,
        "record_task_comment",
        json!({
            "task_id": parent["id"], "task_revision": 1, "body": "Another historical note"
        }),
    );
    let older = dashboard
        .get(&format!(
            "{path}?page_size=1&before_id={}",
            page["next_before_id"]
        ))
        .1;
    assert_eq!(older["comments"][0], execution);
    assert_eq!(
        dashboard
            .get(&format!("{path}?has_execution_result=true"))
            .1["comments"],
        json!([execution])
    );
    assert_eq!(
        dashboard
            .get(&format!("{path}?has_execution_result=false"))
            .1["comments"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        dashboard.get(&format!("{base}/{}", parent["id"])).1["task"],
        edited
    );
    assert_eq!(
        dashboard.get(&format!("{base}/{}/results", parent["id"])).0,
        404
    );

    let done = call(
        &client,
        "update_task",
        json!({
            "task_id": parent["id"], "expected_revision": 2, "status": "done"
        }),
    );
    assert_eq!(done["revision"], 3);
    assert_eq!(dashboard.post(&path, &token, json!({"task_revision": 3, "body": "Rework discussion has no automatic lifecycle effect."})).0, 201);
    assert_eq!(
        dashboard.get(&format!("{base}/{}", parent["id"])).1["task"],
        done
    );
    drop(dashboard);

    let restarted = Dashboard::start(&database);
    let next_token = restarted.token();
    assert_ne!(next_token, token);
    assert_eq!(
        restarted
            .post(
                &path,
                &token,
                json!({"task_revision": 1, "body": "Old service token"})
            )
            .0,
        403
    );
    assert_eq!(restarted.get(&format!("{path}/{}", later["id"])).1, later);
    assert_eq!(
        restarted
            .post(
                &path,
                &next_token,
                json!({"task_revision": 1, "body": "Valid delayed discussion after restart"})
            )
            .0,
        201
    );
}

#[test]
fn dashboard_comment_writes_reject_invalid_scopes_origins_tokens_and_bodies_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("security.sqlite3");
    let client = Project::open(&database).unwrap();
    let target = task(&client, json!({}));
    let other_task = task(&client, json!({}));
    let foreign_reply = call(
        &client,
        "record_task_comment",
        json!({
            "task_id": other_task["id"], "task_revision": 1, "body": "Another task's discussion"
        }),
    );
    let foreign = call(
        &client,
        "create_project",
        json!({"name": "Other", "description": "Isolated"}),
    );
    let foreign_source = call(
        &client,
        "create_source",
        json!({
            "project_id": foreign["id"], "name": "Foreign", "description": "Not an output here", "content": "Private"
        }),
    );
    let dashboard = Dashboard::start(&database);
    let path = format!(
        "/api/projects/{}/tasks/{}/comments",
        client.project_id, target["id"]
    );
    let token = dashboard.token();
    let origin = dashboard.origin();
    let valid = json!({"task_revision": 1, "body": "Uncommitted"}).to_string();
    for (headers, expected) in [
        (
            vec![
                ("Content-Type", "application/json"),
                ("X-Dictum-CSRF", token.as_str()),
            ],
            403,
        ),
        (
            vec![
                ("Origin", "http://evil.example"),
                ("Content-Type", "application/json"),
                ("X-Dictum-CSRF", token.as_str()),
            ],
            403,
        ),
        (
            vec![
                ("Origin", origin.as_str()),
                ("Origin", origin.as_str()),
                ("Content-Type", "application/json"),
                ("X-Dictum-CSRF", token.as_str()),
            ],
            403,
        ),
        (
            vec![
                ("Origin", origin.as_str()),
                ("Content-Type", "application/json"),
            ],
            403,
        ),
        (
            vec![
                ("Origin", origin.as_str()),
                ("Content-Type", "application/json"),
                ("X-Dictum-CSRF", "incorrect"),
            ],
            403,
        ),
        (
            vec![
                ("Origin", origin.as_str()),
                ("Content-Type", "text/plain"),
                ("X-Dictum-CSRF", token.as_str()),
            ],
            415,
        ),
    ] {
        assert_eq!(
            request(dashboard.port, "POST", &path, &headers, &valid).0,
            expected
        );
    }
    for input in [
        json!({"task_revision": 0, "body": "Invalid target"}),
        json!({"task_revision": 2, "body": "Future target"}),
        json!({"task_revision": 1, "body": " "}),
        json!({"task_revision": 1, "body": "x".repeat(4001)}),
        json!({"task_revision": 1, "body": "No author fields", "author_kind": "human"}),
        json!({"task_revision": 1, "body": "No reviews", "disposition": "changes_requested"}),
        json!({"task_revision": 1, "body": "Cannot override URL", "task_id": other_task["id"]}),
        json!({"task_revision": 1, "body": "Wrong reply", "reply_to_comment_id": foreign_reply["id"]}),
        json!({"task_revision": 1, "body": "Wrong output", "execution_result": {"outcome": "success", "output_source_ids": [foreign_source["id"]]}}),
    ] {
        assert!(matches!(dashboard.post(&path, &token, input).0, 400 | 404));
    }
    assert_eq!(
        request(
            dashboard.port,
            "POST",
            &path,
            &[("Content-Length", "65537")],
            ""
        )
        .0,
        400
    );
    assert_eq!(
        request(
            dashboard.port,
            "POST",
            &path,
            &[("Content-Length", "0"), ("Content-Length", "0")],
            ""
        )
        .0,
        400
    );
    assert_eq!(
        request(
            dashboard.port,
            "POST",
            &path,
            &[("Transfer-Encoding", "chunked")],
            ""
        )
        .0,
        400
    );
    assert_eq!(dashboard.get(&path).1["comments"], json!([]));
    let current = client
        .get_task_context(target["id"].as_i64().unwrap())
        .unwrap();
    assert_eq!(current.task.revision, 1);
    assert_eq!(
        current.task.updated_at,
        target["updated_at"].as_str().unwrap()
    );
    assert_eq!(
        dashboard
            .post(
                &path,
                &token,
                json!({"task_revision": 1, "body": "🦀".repeat(4000)})
            )
            .0,
        201
    );
}
