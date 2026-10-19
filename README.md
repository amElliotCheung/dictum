# Dictum

Dictum is a local project-management MCP server for coding agents. It keeps tasks,
reusable sources, accepted decisions, and execution results in SQLite so work can
resume without the original conversation. A bundled, read-only dashboard displays
the same records in your browser.

## Build and connect

Requires Rust 1.88 or newer and a native C compiler for bundled SQLite.

```sh
git clone https://github.com/amElliotCheung/dictum.git
cd dictum
cargo build --locked --release
```

Configure an MCP client to launch the binary over stdio. For clients that use an
`mcpServers` JSON configuration, replace both example paths with absolute paths:

```json
{
  "mcpServers": {
    "dictum": {
      "command": "/absolute/path/to/dictum/target/release/dictum-mcp",
      "env": {
        "DICTUM_DATABASE": "/absolute/path/to/shared/dictum.sqlite3"
      }
    }
  }
}
```

Point every client and the dashboard at the same database to share state. Without
`DICTUM_DATABASE`, the server uses `.dictum/dictum.sqlite3` relative to its working
directory. It creates the database and parent directories when needed. MCP uses
stdout; diagnostics use stderr.

## Create a task with a source

Ask your connected agent to call these MCP tools in order. The objects below are
tool arguments, not shell commands. Substitute the IDs returned by earlier calls.

1. Call `list_projects` with `{}` and select the intended project. For a new
   workspace, explicitly call `create_project`:

   ```json
   {"name": "Example", "description": "An example project"}
   ```

2. Call `create_source` with the returned project ID:

   ```json
   {
     "project_id": "PROJECT_ID",
     "name": "Requirements",
     "description": "Requirements for the example task",
     "content": "# Requirements\n\nShow a useful greeting."
   }
   ```

   Sources accept exactly one of Markdown `content` or an external `locator`.
   A locator registers a reference; the server does not fetch it automatically.

3. Call `create_task`, replacing `1` with the returned source ID:

   ```json
   {
     "project_id": "PROJECT_ID",
     "title": "Add a greeting",
     "problem_or_goal": "Visitors need a useful greeting.",
     "design": "Render the greeting described in the linked requirements.",
     "acceptance_criteria": ["The greeting appears in the UI."],
     "context_source_ids": [1]
   }
   ```

4. Call `get_task_context`, replacing `1` with the returned task ID:

   ```json
   {"project_id": "PROJECT_ID", "task_id": 1}
   ```

   This returns the full task brief, linked context sources, project sources with
   `context_policy: "always"`, current decisions, and dependency state. Inline
   sources are returned in full. Use `get_source` with `project_id` and `source_id`
   to read an individual source.

Task creation records the brief; execution still needs authorization from the
human. Execution evidence and output Source links are appended with
`record_task_result`. A one-time task can be marked `done` only after all
dependency gates are satisfied, at least one successful result exists, and no
outstanding change requests remain.
Mutable task and source updates require the latest `expected_revision`. See
[AGENTS.md](AGENTS.md) for all sixteen tools and their contracts, and
[WORKFLOW.md](WORKFLOW.md) for the embedded agent workflow.

## Review a result and request rework

Call `record_task_feedback` with the current task revision and the specific result
being reviewed:

```json
{
  "project_id": "PROJECT_ID",
  "task_id": 1,
  "result_id": 2,
  "expected_revision": 4,
  "disposition": "changes_requested",
  "body": "The greeting disappears on mobile. Keep it visible at 320px and verify that layout."
}
```

Omit `disposition` for a nonblocking `comment`. Feedback is append-only and stays
attached to its original result. A change request reopens a completed one-time
task to `todo`; recurring tasks retain their active/paused status. Edit the brief
separately with `update_task` if the agreed requirements change.

The next agent receives every outstanding feedback entry and its original result
in `get_task_context.outstanding_feedback`, even after task edits or newer attempts.
After performing the rework, append a `success` or `no_change` execution with
`addressed_feedback_ids: [FEEDBACK_ID]` and evidence describing the fix and checks.
This records reported remediation, not human acceptance. New attempts never
implicitly clear feedback; failures cannot address it. Result history retains the
review and the addressing result ID. The dashboard displays this same feedback
and history; writes remain MCP-only.

## Open the dashboard

Run this alongside the MCP client, using the same absolute database path:

```sh
DICTUM_DATABASE=/absolute/path/to/shared/dictum.sqlite3 \
  target/release/dictum-mcp dashboard --port 3000
```

Open <http://127.0.0.1:3000>, select a project, and switch between the One-time
and Recurring task tabs. Choose a task to see its brief, dependencies, and
execution history, including outstanding feedback and reported fixes. Project
decisions and all project sources appear below the task board; expand a source to read inline Markdown or external metadata.
The dashboard polls for updates. It only reads records; create and update them
through MCP. The listener binds to `127.0.0.1`; `--port 0` selects a free port
and prints the URL to stderr.

The committed frontend bundle is embedded in the binary. Normal builds and use
need no Node.js, frontend server, external fonts, or remote services.

## Development

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Tests launch the actual MCP binary and HTTP listener with temporary databases.
Dashboard changes also need a real browser check.

After changing frontend sources, use Node.js 22.12 or newer and rebuild the
committed assets before rebuilding Rust:

```sh
cd web
npm ci
npm run build
```

Only the current schema (v6) is supported. Startup rejects older databases
without modifying them; preserve the file and start a new database.

## License

[MIT](LICENSE).
