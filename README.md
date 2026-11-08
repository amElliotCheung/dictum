# Dictum

Dictum is a local project-management MCP server for coding agents. It keeps tasks,
reusable sources, accepted decisions, and versioned task comments in SQLite so work
can resume without the original conversation. Project topics organize records for
discovery. A bundled dashboard displays the same records and lets you post
comments and execution reports.

All project, topic, task, source, decision, and comment IDs are server-generated
UUIDv7 strings. References and pagination cursors use the same canonical lowercase,
hyphenated format; numeric IDs and other UUID versions are rejected. A shared Rust
generator preserves ascending append order across clients, restarts and clock
regressions. SQLite does not provide a built-in UUIDv7 function.

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
     "topics": ["visitor greetings"],
     "content": "# Requirements\n\nShow a useful greeting."
   }
   ```

   Sources accept exactly one of Markdown `content` or an external `locator`.
   A locator registers a reference; the server does not fetch it automatically.

3. Call `create_task`, replacing `SOURCE_ID` with the returned source ID:

   ```json
   {
     "project_id": "PROJECT_ID",
     "title": "Add a greeting",
     "topics": ["visitor greetings"],
     "priority": "high",
     "problem_or_goal": "Visitors need a useful greeting.",
     "design": "Render the greeting described in the linked requirements.",
     "acceptance_criteria": ["The greeting appears in the UI."],
     "context_source_ids": ["SOURCE_ID"]
   }
   ```

4. Call `get_context`, replacing `TASK_ID` with the returned task ID:

   ```json
   {"project_id": "PROJECT_ID", "task_id": "TASK_ID"}
   ```

   This returns the full project, task brief, linked context sources, project
   sources with `context_policy: "always"`, current decisions, and dependency state. Inline
   sources are returned in full. Use `get_source` with `project_id` and `source_id`
   to read an individual source. Keep always-included sources small and link only
   context needed for the task. `list_sources` returns descriptors; listing a
   source does not load its content into task context.

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
[WORKFLOW.md](WORKFLOW.md) for the embedded agent workflow. Its brevity principle
applies to all guidance and record fields, including Source descriptions and
action instructions: retain only what later humans or agents need to understand,
decide, act or verify.

## Read project or task context

Call `get_context` with only `project_id` to orient yourself before selecting or
creating a task:

```json
{"project_id": "PROJECT_ID"}
```

The result contains the full `project` with its `topics` directory, ordered
`promoted_decision_ids` and resolved `promoted_decisions` summaries, all `always` Sources, and all current
decisions in ascending ID order with their full statement, rationale, and alternatives. `task`,
`parent_task`, and `dependency_state` are null; `recent_comments` is empty and
both `latest_execution` and `latest_successful_execution` are null.

Add `task_id` to include the full task and parent brief, dependency state,
deduplicated task-linked plus always Sources, and bounded comments and execution
evidence. Both modes read one consistent database snapshot. Inline Sources remain
full and untruncated; context decisions are complete and unpaginated. Use
`get_context` in place of the former `get_task_context` tool.

## Organize records with topics

Tasks, Sources, and decisions can have zero or more project topics. Optionally
pass a `topics` array of names to `create_task`, `create_source`, and
`record_decision`; omission or `[]` creates an unclassified record. These writes
atomically reuse or create supplied topics; no separate topic tool or setup step
is needed. Topic names are local to a project.
Whitespace and case are normalized for exact reuse, while the first collapsed
display spelling is preserved. Repeated names in one request are deduplicated.

Use topics when they help discovery. Reuse an existing concept from
`project.topics` when available. Prefer specific names such as
`user authentication` over `authentication`; avoid creating synonymous topics. Projects typically need tens of topics, with
no enforced count limit or topic hierarchy. All project responses include the
complete directory, including currently unused topics, as `{id, project_id,
name}` objects. Task records, task summaries, Sources, Source descriptors, and
decisions return the same objects in their own `topics` arrays.

Use an exact topic name to discover matching records with `list_tasks`,
`list_sources`, or `list_decisions`:

```json
{"project_id": "PROJECT_ID", "topic": "user authentication"}
```

Topic matching uses the same normalization as writes and combines with other
filters before pagination. Unknown names return no matches; blank names fail.
Lookup never creates a topic. Membership does not automatically include other
records in context: keep explicit task context links and fetch needed Sources.

`update_task` and `update_source` accept a replacement `topics` array under the
record's latest `expected_revision`. Omission preserves membership and `[]` clears
it. Null arrays, non-string elements, and blank names are rejected on all writes.
Topic edits advance the task or Source revision, while preserving Source action
freshness. Topics are not inherited
from parent tasks or linked Sources. Decisions and their topic memberships are
immutable, and replacements use their own optional topics, defaulting to empty
without inheriting the predecessor's topics. Creating topics does not change the project's description revision or timestamps.

## Browse sources and decisions

`list_sources` returns compact descriptors and `list_decisions` returns full
decision records. Both accept `page_size` (default 20, range 1–100) and optional
`before_id`, and return newest IDs first with `next_before_id` for an older page:

```json
{"project_id": "PROJECT_ID", "page_size": 5}
```

Pass a non-null `next_before_id` as `before_id` on the next call, retaining the
same filters. `before_id` must be a UUIDv7 ID of an existing Source or decision,
respectively, in the same project. Cursor validation is independent of filters;
the cursor record need not match them. `next_before_id` is null when no older
matching record remains, even if the page is full.

Source filters include `task_id`, `representation`, `action_due`, `query`, and `topic`;
task filtering covers explicit context links. Decision listings default to
current choices; `include_superseded: true` includes their history, and `topic`
filters either view. Later appends
have higher IDs and do not appear on older pages. Edits, action freshness, and
decision supersession can change data or filter membership between requests.
Project discovery through `list_projects` remains unpaginated.

## Update a project and promote foundational decisions

Call `list_projects` to identify the project and read its latest `revision`, then
call `update_project` with that revision and optional replacements:

```json
{
  "project_id": "PROJECT_ID",
  "expected_revision": 1,
  "description": "The project's current purpose and scope.",
  "promoted_decision_ids": ["DECISION_ID"]
}
```

The tool returns the updated project, increments `revision`, and sets server UTC
`updated_at`. New projects start at revision 1 with matching `created_at` and
`updated_at`. Each accepted update increments revision, including identical values
or an update with both optional fields omitted. Omitted fields preserve their
values; descriptions must be nonblank when supplied. `promoted_decision_ids`
replaces the ordered references, `[]` clears them, and null values and unknown
fields are rejected. References must identify decisions in this project;
duplicate references to the same decision chain retain the first supplied ID and
position. Description and references are validated and written atomically.
The project ID, name and creation time are preserved. On a revision conflict,
reread with `list_projects` and reassess the edit before retrying.

Promote a small set of foundational choices, such as core product boundaries or
storage choices. Keep statements in decisions; the project stores references.
Every project response includes `promoted_decision_ids`; only `get_context`
expands `project.promoted_decisions` into `{reference_id, id, statement}` summaries.
If promoted A is replaced by B, its stored reference remains A while the summary
returns `reference_id: A`, `id: B` and B's current statement. Resolution follows
the entire chain in the same context snapshot, without rewriting references or
changing project revision or timestamps. Full rationale remains available in
`current_decisions`, which still includes every current decision.

Promotion changes prominence; all current decisions retain their meaning and
execution authorization remains separate. The dashboard shows promoted summaries
near the project description, independent of topic filters and task selection,
and refreshes them with project context.

## Keep environment facts available

Use a concise inline Source with `context_policy: "always"` for environment facts
needed across tasks, such as the production AWS account, region, and DynamoDB
table names. Include the environment scope and exact resource identifiers, plus
a verification reference or date where useful. Update the Source when reality
changes. Facts describe current state; decisions record accepted choices and
their rationale. An accepted decision alone does not establish that its intended
state has been deployed.

Keep the project description focused on purpose and scope. Every context
read includes always Sources in full, so reserve them for essential facts and
keep longer supporting material in separate Sources fetched when relevant.

## Record an accepted decision

Follow the [workflow decision rule](WORKFLOW.md) when evidence suggests a better
direction or the human's session instructions conflict with a stored choice.

Call `record_decision` with the accepted statement and rationale. Optional
`alternatives_rejected` preserves rejected options as Markdown:

```json
{
  "project_id": "PROJECT_ID",
  "statement": "Keep the greeting in the page header.",
  "rationale": "Visitors see it immediately on every page.",
  "topics": ["visitor greetings"],
  "alternatives_rejected": "- A modal greeting would interrupt navigation."
}
```

Omitting `alternatives_rejected` or supplying null stores null; empty text is
allowed. Decisions are immutable. Record a replacement with `supersedes_id` when
an accepted choice changes; each decision retains its own alternatives.
`list_decisions` and the current decisions in `get_context` return this field.
The dashboard shows an **Alternatives rejected** section when the field is
non-null, in both project and task-context decision cards.

Every returned decision includes a server-derived `current_decision_id`: its own
ID when current, or the ID at the current end of its supersession chain when
historical. For example, if decision A chooses DynamoDB, B replaces it with
PostgreSQL, and C replaces B with DynamoDB again, a subsequent history read
returns C's UUIDv7 as `current_decision_id` on all three records. Their statements,
rationales and timestamps remain unchanged. The field is computed at read time,
including when the current decision is outside the requested history page; it
does not add a database column or require a schema migration.

Normal context and decision discovery include only current choices. Use
`list_decisions` with `include_superseded: true` for history, and follow each
record's `current_decision_id` for current intent. Replacing a historical record
fails and reports its current decision ID; reread the current choice before
retrying. A reversal must create a new decision with a complete current statement
and rationale, including the reasons that apply now. Supersession follows explicit
links; the server does not infer that independently created decisions address
the same question.

## Subtasks

Create a direct subtask by supplying `parent_task_id` to `create_task`. Parent and
child must belong to the same project and have the same kind: one-time with
one-time, recurring with recurring. Only one level is supported. Recurring
subtasks must explicitly set `kind: "recurring"`; the default remains `one_time`.

Put shared goals, constraints, and approach in the parent, and specific work and
acceptance criteria in the child. The two briefs complement each other without
repeating shared content. `get_context` with a subtask ID returns it and its complete
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
  "task_id": "TASK_ID",
  "task_revision": 1,
  "body": "Implemented the greeting in commit abc1234; desktop and mobile browser checks passed.",
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

Every comment, with or without `execution_result`, must help later humans or
agents understand, review or resume the task. Use 1–3 short sentences or bullets
with new findings, corrections, blockers, next steps or evidence. An execution
report identifies what was done or checked, the implementation or artifact tested,
and its outcome; include remaining limitations
when relevant. `task_revision` identifies the task brief, not a code revision.
Fresh verification can justify a `no_change` report even when no files changed.
Always satisfy explicit reporting requirements and the evidence required for
completion. Use output Sources for longer durable artifacts when needed; a
concise note does not need its own Source.

Every comment permanently retains its target `task_revision`, including when a
new report targets an older version. Targets must be between 1 and the current
revision. Bodies are nonblank Markdown, at most 4,000 characters. Optional
`reply_to_comment_id` references another comment on the same task, even across
versions. There are no author fields, review dispositions, or resolution links.
Rework is discussion plus an explicit task update, without automatic reopening.

Task context includes ten recent comments, `latest_execution`, and
`latest_successful_execution`; matching IDs identify the same entry. Start with
this bounded context, then fetch older history only to resolve a specific gap.
Use `list_task_comments` with a small page such as `page_size: 5` (default 20,
maximum 100 per page) for newest-first history,
`next_before_id` as `before_id` to continue, and optional `has_execution_result` to
filter. Stop when the needed context is found; avoid routine full-history scans.
`get_task_comment` expands a particular `comment_id`, and `get_source` expands
only the outputs needed for the task. Check new comments independently of task
revision changes, using `list_task_comments` rather than reloading full task
context solely to poll comments. Comments never require `expected_revision`;
their `task_revision` identifies what they discuss.

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
and Recurring task tabs. Select a project topic to filter tasks and the project
Decisions and Sources sections together. Choose a task to see its brief, dependencies, and
versioned conversation. Promoted decisions remain visible near the project
description across topic filters. Expand a subtask row to load its details; collapsed
children do not fetch full context. Project decisions and sources appear below
the task board, newest first, with **Load more** for older pages. Expand a source
to read inline Markdown or external metadata.
The dashboard polls task records and comments independently. Post comments,
replies, and optional execution reports from task details; each entry displays its
target version and whether that version is current. Drafts retain the viewed
version across refreshes and errors; use the explicit retarget action to choose
the latest version. Task/project/source authoring remains on MCP. The listener binds to `127.0.0.1`; `--port 0` selects a free port
and prints the URL to stderr. Comment POSTs require an allowed Host and Origin,
JSON within the 64 KiB request limit, and a per-dashboard request token. The
browser obtains that token from `/api/request-token`; it changes on restart.
Other writes are rejected.

HTTP clients can read project-only context at `GET /api/projects/:id/context`
and task context at `GET /api/projects/:id/tasks/:task_id`. Project `/tasks`,
`/decisions`, and `/sources` routes accept a `topic` name query parameter. The
`/decisions` and `/sources` routes also accept `page_size` and `before_id` query
parameters and return `next_before_id`, following the MCP pagination contract.

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

Only the current schema (v16) is supported. Startup rejects older databases
without modifying them; preserve the file and use a fresh path such as
`.dictum/dictum-v16.sqlite3`. Re-create needed records through MCP rather than
migrating the older database.

## License

[MIT](LICENSE).
