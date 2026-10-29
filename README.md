# Dictum

Dictum is a local project-management MCP server for coding agents. It keeps tasks,
reusable sources, accepted decisions, and versioned task comments in SQLite so work
can resume without the original conversation. A bundled dashboard displays the
same records and lets you post comments and execution reports.

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
     "priority": "high",
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

Tasks accept `priority: "low"`, `"medium"`, or `"high"`; omitting it at creation
defaults to `"medium"` for both one-time and recurring tasks. `update_task` can
change priority with the current `expected_revision`; omitting priority preserves
it. `list_tasks` accepts an exact priority filter and returns high, medium, then
low priority tasks, with ascending task IDs within each priority. Task context
and summaries include priority. The dashboard shows a labeled color badge on
each task card, keeps this order within status columns, and displays priority in
task details. Priority does not change lifecycle or dependency gates.

Task creation records the brief; execution still needs authorization from the
human. Append execution evidence and output Source links with
`record_task_comment.execution_result`. A one-time task can be marked `done` only
after all dependency gates are satisfied and at least one successful execution
comment exists. Comments never change lifecycle; update the brief or status
directly with `update_task` when work changes.
Mutable project, task and source updates require the latest `expected_revision`. See
[AGENTS.md](AGENTS.md) for all seventeen tools and their contracts, and
[WORKFLOW.md](WORKFLOW.md) for the embedded agent workflow.

## Update a project description

Call `list_projects` to identify the project and read its latest `revision`, then
call `update_project` with that revision and the required nonblank replacement:

```json
{
  "project_id": "PROJECT_ID",
  "expected_revision": 1,
  "description": "The project's current purpose and scope."
}
```

The tool returns the updated project, increments `revision`, and sets server UTC
`updated_at`. New projects start at revision 1 with matching `created_at` and
`updated_at`. Each accepted update increments revision, including identical text.
The project ID, name, creation time and related records are preserved. Null or
blank descriptions and unknown fields are rejected. On a revision conflict,
reread with `list_projects` and reassess the edit before retrying. The
dashboard displays the new description on its next project refresh (every 30
seconds).

## Record an accepted decision

Call `record_decision` with the accepted statement and rationale. Optional
`alternatives_rejected` preserves rejected options as Markdown:

```json
{
  "project_id": "PROJECT_ID",
  "statement": "Keep the greeting in the page header.",
  "rationale": "Visitors see it immediately on every page.",
  "alternatives_rejected": "- A modal greeting would interrupt navigation."
}
```

Omitting `alternatives_rejected` or supplying null stores null; empty text is
allowed. Decisions are immutable. Record a replacement with `supersedes_id` when
an accepted choice changes; each decision retains its own alternatives.
`list_decisions` and the current decisions in `get_task_context` return this field.
The dashboard shows an **Alternatives rejected** section when the field is
non-null, in both project and task-context decision cards.

## Subtasks

Create a direct subtask by supplying `parent_task_id` to `create_task`. Parent and
child must belong to the same project and have the same kind: one-time with
one-time, recurring with recurring. Only one level is supported. Recurring
subtasks must explicitly set `kind: "recurring"`; the default remains `one_time`.

Put shared goals, constraints, and approach in the parent, and specific work and
acceptance criteria in the child. The two briefs complement each other without
repeating shared content. `get_task_context` returns the subtask and its complete
`parent_task` together, without recursively expanding parent sources or history.

In `update_task`, an omitted parent preserves it, null detaches it, and an ID
reparents the task. A task with children cannot itself become a subtask. In
`list_tasks`, omitted `parent_task_id` lists all tasks, null lists roots, and an ID
lists direct children. Hierarchy organizes work; add explicit dependencies when
children must finish before a parent can finish.

## Versioned comments and execution reports

A task starts at `revision: 1`. Every accepted `update_task` advances that version,
including status, progress, priority, and relationship edits. Comment appends leave
the task revision and update time unchanged.

Call `record_task_comment` with the task version you actually read:

```json
{
  "project_id": "PROJECT_ID",
  "task_id": 1,
  "task_revision": 1,
  "body": "Implemented the greeting; the mobile browser check passed.",
  "execution_result": {
    "outcome": "success",
    "output_source_ids": []
  }
}
```

Omit `execution_result` for ordinary discussion. Its outcomes are `success`,
`no_change`, and `failure`; the comment body supplies evidence. Both task kinds
may record already-performed work, including while recurring work is paused.
Success/no-change requires satisfied start and completion dependencies. A failure
can report unmet gates. Output IDs must belong to the same project; source
contents remain mutable rather than becoming snapshots.

Every comment permanently retains its target `task_revision`, including when a
new report targets an older version. Targets must be between 1 and the current
revision. Bodies are nonblank Markdown, at most 4,000 characters. Optional
`reply_to_comment_id` references another comment on the same task, even across
versions. There are no author fields, review dispositions, or resolution links.
Rework is discussion plus an explicit task update, without automatic reopening.

Task context includes ten recent comments, `latest_execution`, and
`latest_successful_execution`; matching IDs identify the same entry. Use
`list_task_comments` for newest-first history (default 20, maximum 100 per page),
`next_before_id` as `before_id` to continue, and optional `has_execution_result` to
filter. `get_task_comment` expands a particular `comment_id`. Comments never
require `expected_revision`; their `task_revision` identifies what they discuss.

Versions identify task records, not retained historical brief bodies or frozen
parent/source context. Historical success still satisfies the execution-evidence
gate; agents and humans decide whether it applies to the final work.

## Open the dashboard

Run this alongside the MCP client, using the same absolute database path:

```sh
DICTUM_DATABASE=/absolute/path/to/shared/dictum.sqlite3 \
  target/release/dictum-mcp dashboard --port 3000
```

Open <http://127.0.0.1:3000>, select a project, and switch between the One-time
and Recurring task tabs. Choose a task to see its brief, dependencies, and
versioned conversation. Expand a subtask row to load its details; collapsed
children do not fetch full context. Project decisions and all project sources appear below the task board; expand a source to read inline Markdown or external metadata.
The dashboard polls task records and comments independently. Post comments,
replies, and optional execution reports from task details; each entry displays its
target version and whether that version is current. Drafts retain the viewed
version across refreshes and errors; use the explicit retarget action to choose
the latest version. Task/project/source authoring remains on MCP. The listener binds to `127.0.0.1`; `--port 0` selects a free port
and prints the URL to stderr. Comment POSTs require an allowed Host and Origin,
JSON within the 64 KiB request limit, and a per-dashboard request token. The
browser obtains that token from `/api/request-token`; it changes on restart.
Other writes are rejected.

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

Only the current schema (v13) is supported. Startup rejects older databases
without modifying them; preserve the file and use a fresh path such as
`.dictum/dictum-v13.sqlite3`. Re-create needed records through MCP rather than
migrating the older database.

## License

[MIT](LICENSE).
