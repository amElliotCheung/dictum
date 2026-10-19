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
human. Mutable task and source updates require the latest `expected_revision`.
See [AGENTS.md](AGENTS.md) for all fifteen tools and their contracts, and
[WORKFLOW.md](WORKFLOW.md) for the embedded agent workflow.

## Open the dashboard

Run this alongside the MCP client, using the same absolute database path:

```sh
DICTUM_DATABASE=/absolute/path/to/shared/dictum.sqlite3 \
  target/release/dictum-mcp dashboard --port 3000
```

Open <http://127.0.0.1:3000>, select a project, and choose a task to see its brief,
sources, dependencies, and execution history. The dashboard polls for updates.
It only reads records; create and update them through MCP. The listener binds to
`127.0.0.1`; `--port 0` selects a free port and prints the URL to stderr.

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

Existing v2/v3 databases require explicit migration to a new destination; startup
does not silently upgrade them. Run `dictum-mcp migrate --help` for usage.

## License

[MIT](LICENSE).
