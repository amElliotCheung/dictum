//! Read-only HTTP dashboard: a sibling transport over the same Store as MCP.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

use crate::{
    model::*,
    store::{Store, StoreError, StoreResult},
};

const INDEX: &str = include_str!("../web/index.html");
const CSS: &str = include_str!("../web/dist/app.css");
const JS: &str = include_str!("../web/dist/app.js");
const FAVICON: &str = include_str!("../web/favicon.svg");

const MAX_HEADER_BYTES: usize = 16 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; object-src 'none'";

/// Serve the embedded dashboard on the IPv4 loopback interface.
pub async fn serve(store: Store, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("failed to bind dashboard listener on 127.0.0.1:{port}"))?;
    let address = listener
        .local_addr()
        .context("failed to read dashboard listener address")?;
    eprintln!("dictum-mcp dashboard: using {}", store.path().display());
    eprintln!(
        "dictum-mcp dashboard: listening on http://127.0.0.1:{}",
        address.port()
    );

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("dashboard listener failed")?;
                let store = store.clone();
                let port = address.port();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(store, stream, port).await {
                        eprintln!("dictum-mcp dashboard: request failed: {error}");
                    }
                });
            }
            () = shutdown_signal() => break,
        }
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                () = async { let _ = tokio::signal::ctrl_c().await; } => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Get,
    Head,
    Other,
}

struct Request {
    method: Method,
    target: String,
    headers: Vec<(String, String)>,
}

async fn handle_connection(store: Store, mut stream: TcpStream, port: u16) -> std::io::Result<()> {
    let request = match timeout(READ_TIMEOUT, read_request(&mut stream)).await {
        Ok(Ok(request)) => request,
        Ok(Err(())) => {
            return write_response(&mut stream, Response::error(400, "bad request"), false).await;
        }
        Err(_) => {
            return write_response(&mut stream, Response::error(408, "request timeout"), false)
                .await;
        }
    };
    let head_only = request.method == Method::Head;
    let response = dispatch(&store, &request, port).await;
    write_response(&mut stream, response, head_only).await
}

async fn dispatch(store: &Store, request: &Request, port: u16) -> Response {
    if request.method == Method::Other {
        return Response::method_not_allowed();
    }
    if !allowed_host(&request.headers, port) {
        return Response::error(403, "host is not allowed");
    }
    if !allowed_origin(&request.headers, port) {
        return Response::error(403, "origin is not allowed");
    }

    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((request.target.as_str(), ""));
    if !path.starts_with('/') {
        return Response::error(400, "invalid request target");
    }
    let segments = match decode_segments(path) {
        Ok(segments) => segments,
        Err(()) => return Response::error(400, "invalid path"),
    };
    let segments: Vec<&str> = segments.iter().map(String::as_str).collect();

    match segments.as_slice() {
        [] => Response::static_asset(200, "text/html; charset=utf-8", INDEX),
        ["app.css"] => Response::static_asset(200, "text/css; charset=utf-8", CSS),
        ["app.js"] => Response::static_asset(200, "text/javascript; charset=utf-8", JS),
        ["favicon.svg"] => Response::static_asset(200, "image/svg+xml", FAVICON),
        ["api", "projects"] => {
            api_response(store, |store| {
                Ok(ProjectList {
                    projects: store.list_projects(ListProjects { query: None })?,
                })
            })
            .await
        }
        ["api", "projects", project_id, "tasks"] => {
            let project_id = (*project_id).to_string();
            api_response(store, move |store| {
                Ok(TaskList {
                    tasks: store.list_tasks(ListTasks {
                        project_id,
                        ..ListTasks::default()
                    })?,
                })
            })
            .await
        }
        ["api", "projects", project_id, "tasks", task_id] => {
            let project_id = (*project_id).to_string();
            let task_id = match parse_id("task id", task_id) {
                Ok(task_id) => task_id,
                Err(response) => return response,
            };
            api_response(store, move |store| {
                store.get_task_context(&project_id, task_id)
            })
            .await
        }
        ["api", "projects", project_id, "tasks", task_id, "results"] => {
            let project_id = (*project_id).to_string();
            let task_id = match parse_id("task id", task_id) {
                Ok(task_id) => task_id,
                Err(response) => return response,
            };
            let page_size = match parse_query_i64(query, "page_size") {
                Ok(value) => value,
                Err(response) => return response,
            };
            let before_id = match parse_query_i64(query, "before_id") {
                Ok(value) => value,
                Err(response) => return response,
            };
            api_response(store, move |store| {
                store.list_task_results(ListTaskResults {
                    project_id,
                    task_id,
                    page_size,
                    before_id,
                })
            })
            .await
        }
        ["api", "projects", project_id, "sources", source_id] => {
            let project_id = (*project_id).to_string();
            let source_id = match parse_id("source id", source_id) {
                Ok(source_id) => source_id,
                Err(response) => return response,
            };
            api_response(store, move |store| store.get_source(&project_id, source_id)).await
        }
        _ => Response::error(404, "not found"),
    }
}

async fn api_response<T, F>(store: &Store, operation: F) -> Response
where
    T: Serialize + Send + 'static,
    F: FnOnce(Store) -> StoreResult<T> + Send + 'static,
{
    match query(store, operation).await {
        Ok(value) => Response::json(&value),
        Err(error) => error.into_response(),
    }
}

async fn query<T, F>(store: &Store, operation: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(Store) -> StoreResult<T> + Send + 'static,
{
    let store = store.clone();
    match tokio::task::spawn_blocking(move || operation(store)).await {
        Ok(result) => result.map_err(ApiError::Store),
        Err(error) => Err(ApiError::Worker(error.to_string())),
    }
}

enum ApiError {
    Store(StoreError),
    Worker(String),
}

impl ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Store(error) => match &error {
                StoreError::Validation(message) => Response::error(400, message),
                StoreError::ProjectNotFound(_) | StoreError::NotFound { .. } => {
                    Response::error(404, &error.to_string())
                }
                StoreError::RevisionConflict { .. } => Response::error(409, &error.to_string()),
                StoreError::Database(_)
                | StoreError::Io(_)
                | StoreError::Json(_)
                | StoreError::UnsupportedSchema { .. } => {
                    eprintln!("dictum-mcp dashboard: internal error: {error}");
                    Response::error(500, "internal server error")
                }
            },
            Self::Worker(message) => {
                eprintln!("dictum-mcp dashboard: database worker failed: {message}");
                Response::error(500, "internal server error")
            }
        }
    }
}

struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    allow: bool,
}

impl Response {
    fn static_asset(status: u16, content_type: &'static str, body: &'static str) -> Self {
        Self {
            status,
            content_type,
            body: body.as_bytes().to_vec(),
            allow: false,
        }
    }

    fn json<T: Serialize>(value: &T) -> Self {
        match serde_json::to_vec(value) {
            Ok(body) => Self {
                status: 200,
                content_type: "application/json; charset=utf-8",
                body,
                allow: false,
            },
            Err(error) => {
                eprintln!("dictum-mcp dashboard: serialization failed: {error}");
                Self::internal_error()
            }
        }
    }

    fn error(status: u16, message: &str) -> Self {
        let body = serde_json::to_vec(&serde_json::json!({ "error": message }))
            .unwrap_or_else(|_| b"{\"error\":\"internal server error\"}".to_vec());
        Self {
            status,
            content_type: "application/json; charset=utf-8",
            body,
            allow: false,
        }
    }

    fn internal_error() -> Self {
        Self {
            status: 500,
            content_type: "application/json; charset=utf-8",
            body: b"{\"error\":\"internal server error\"}".to_vec(),
            allow: false,
        }
    }

    fn method_not_allowed() -> Self {
        Self {
            allow: true,
            ..Self::error(405, "method not allowed")
        }
    }
}

async fn write_response(
    stream: &mut TcpStream,
    response: Response,
    head_only: bool,
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nContent-Security-Policy: {}\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len(),
        CSP
    );
    if response.allow {
        head.push_str("Allow: GET, HEAD\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    if !head_only {
        stream.write_all(&response.body).await?;
    }
    stream.shutdown().await
}

async fn read_request(stream: &mut TcpStream) -> Result<Request, ()> {
    let mut data = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        if let Some(end) = find_header_end(&data) {
            data.truncate(end + 4);
            return parse_request(&data);
        }
        if data.len() >= MAX_HEADER_BYTES {
            return Err(());
        }
        match stream.read(&mut buffer).await {
            Ok(0) => return Err(()),
            Ok(count) => data.extend_from_slice(&buffer[..count]),
            Err(_) => return Err(()),
        }
    }
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_request(bytes: &[u8]) -> Result<Request, ()> {
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    let header_block = text.strip_suffix("\r\n\r\n").ok_or(())?;
    let mut lines = header_block.split("\r\n");
    let request_line = lines.next().ok_or(())?;
    let mut parts = request_line.split(' ');
    let method = match parts.next().ok_or(())? {
        "GET" => Method::Get,
        "HEAD" => Method::Head,
        _ => Method::Other,
    };
    let target = parts.next().ok_or(())?.to_string();
    let version = parts.next().ok_or(())?;
    if parts.next().is_some() || !version.starts_with("HTTP/1.") {
        return Err(());
    }
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(())?;
        if name.is_empty() || name.bytes().any(|byte| byte <= b' ' || byte == b':') {
            return Err(());
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
    }
    Ok(Request {
        method,
        target,
        headers,
    })
}

fn allowed_host(headers: &[(String, String)], port: u16) -> bool {
    let hosts: Vec<&str> = headers
        .iter()
        .filter(|(name, _)| name == "host")
        .map(|(_, value)| value.trim())
        .collect();
    if hosts.len() != 1 {
        return false;
    }
    let host = hosts[0].to_ascii_lowercase();
    let mut allowed = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if port == 80 {
        allowed.push("127.0.0.1".to_string());
        allowed.push("localhost".to_string());
    }
    allowed.iter().any(|candidate| candidate == &host)
}

fn allowed_origin(headers: &[(String, String)], port: u16) -> bool {
    let origins: Vec<&str> = headers
        .iter()
        .filter(|(name, _)| name == "origin")
        .map(|(_, value)| value.trim())
        .collect();
    if origins.is_empty() {
        return true;
    }
    if origins.len() != 1 {
        return false;
    }
    let origin = origins[0].to_ascii_lowercase();
    let mut allowed = vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ];
    if port == 80 {
        allowed.push("http://127.0.0.1".to_string());
        allowed.push("http://localhost".to_string());
    }
    allowed.iter().any(|candidate| candidate == &origin)
}

fn parse_id(name: &str, value: &str) -> Result<i64, Response> {
    match value.parse::<i64>() {
        Ok(id) if id > 0 => Ok(id),
        _ => Err(Response::error(
            400,
            &format!("{name} must be a positive integer"),
        )),
    }
}

fn parse_query_i64(query: &str, name: &str) -> Result<Option<i64>, Response> {
    match query_value(query, name) {
        Ok(None) => Ok(None),
        Ok(Some(value)) => value
            .parse::<i64>()
            .map(Some)
            .map_err(|_| Response::error(400, &format!("{name} must be an integer"))),
        Err(()) => Err(Response::error(400, "invalid query")),
    }
}

fn query_value(query: &str, name: &str) -> Result<Option<String>, ()> {
    let mut found = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(key, true)? == name {
            found = Some(percent_decode(value, true)?);
        }
    }
    Ok(found)
}

fn decode_segments(path: &str) -> Result<Vec<String>, ()> {
    let rest = path.strip_prefix('/').ok_or(())?;
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    rest.split('/')
        .map(|segment| percent_decode(segment, false))
        .collect()
}

fn percent_decode(value: &str, plus_as_space: bool) -> Result<String, ()> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(());
                }
                let high = hex(bytes[index + 1]).ok_or(())?;
                let low = hex(bytes[index + 2]).ok_or(())?;
                output.push(high * 16 + low);
                index += 3;
            }
            b'+' if plus_as_space => {
                output.push(b' ');
                index += 1;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(output).map_err(|_| ())
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "Error",
    }
}
