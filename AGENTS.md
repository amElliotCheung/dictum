# Dictum

`AGENTS.md` is part of this repository's source code and should evolve with the package. Update it when the project design, MCP tool contracts, or file layout changes meaningfully. In particular, consider updating the file index when adding, moving, or removing a file. Routine work does not require editing this document.

## 1. Project overview

Dictum is a local project-management MCP server for coding agents. Its goal is to let any working agent understand a project, execute a well-defined task, and resume previous work using durable records instead of the original conversation. The agent performs research, external tool calls, implementation, and verification; Dictum manages context, relationships, progress, and accepted decisions.

The MVP uses Rust, `rmcp`, stdio, and one SQLite database containing multiple projects per server process. Multiple local clients must point to the same database to share project state. `DICTUM_DATABASE` selects its path; the default is `.dictum/dictum.sqlite3` relative to the process working directory. Stdout is reserved for MCP; diagnostics go to stderr.

The binary also serves a read-only localhost dashboard, `dictum-mcp dashboard --port 3000`, as a sibling transport over the same Store and database. The React 19 + TanStack Query UI uses shadcn/ui-style components and Tailwind CSS v4. Its production assets are embedded in the binary; normal use needs no frontend dev server, CDN, external fonts, or remote services. The dashboard binds only `127.0.0.1`, exposes GET-only JSON reads, never mutates records or executes source actions, rejects foreign Host and Origin values, and serves a restrictive content policy. MCP and dashboard processes can run concurrently and restart independently.

`WORKFLOW.md` is the canonical, compact agent workflow (at most 2,000 characters). It is embedded in the binary and delivered through MCP server instructions, not a separate tool or required skill installation. Put task-authoring and authorization rules first. The human and agent coauthor the actual task content, not a separate approval brief; preserve agreed intent, constraints, required approach, and acceptance criteria. Once authorized, one coordinator can derive inspectable tasks, delegate bounded work, obtain separate reviews, and record evidence. These are agent guidance, not server-enforced approval or orchestration. Clients control instruction inclusion, retention, and subagent inheritance; actual CLI behavior needs separate validation.

The model has eight tables: `projects`, `tasks`, `sources`, `decisions`, `task_sources`, `task_dependencies`, `task_results`, and `task_result_sources`.

- **Projects:** a name, description, immutable server-assigned UUIDv7 ID, and UTC `created_at`. Names need not be unique. Every task, source, decision, relationship, dependency and supersession belongs to exactly one project. Require an existing `project_id` on all task/source/decision tools and reject cross-project references atomically. Discover with `list_projects` before `list_tasks(project_id, ready_to_start=true)` and `get_task_context(project_id, task_id)`; resolve ambiguity or no match with the human rather than implicitly creating or selecting a project. Scope does not grant execution authorization or claim work.

- **Tasks:** a title, required Markdown sections for problem/goal and design, acceptance criteria, and optional rejected alternatives. Keep this execution brief separate from status, progress, and result/verification evidence. Immutable `kind` defaults to `one_time`, with `todo`, `in_progress`, `blocked`, and `done` statuses. `recurring` tasks reuse the same ID, brief and context sources and use `active` (default) or `paused`. Active describes eligibility for externally authorized work, not a running invocation or installed schedule. Only recurring tasks accept an optional nonblank `trigger_event` label; it is external event metadata, not a subscription or wakeup guarantee. `ready_to_start` remains a one-time todo query; discover recurrence explicitly by kind/status.
- **Execution results:** append-only `task_results` entries retain every success, no_change or failure and per-entry output Source IDs in `task_result_sources`. Each entry stores nonblank reported evidence (at most 4,000 characters), server UTC `recorded_at` (not execution observation time), and the resulting task revision. Append and same-project output associations are atomic under `expected_revision`. Both kinds may append; no append changes lifecycle, legacy result text or current output links. A paused task can record already-performed work without reactivation. Recurring callers cannot replace/clear task-level result or outputs with `update_task`. One-time compatibility retains those updates and explicit done behavior. No-change counts as successful; failures preserve earlier successes. Source identities remain associated, but their contents retain the mutable-source contract. No automatic deletion, per-run task, exactly-once execution or claim mechanism exists.
- **Task relationships:** sources are linked many-to-many with role `context` or `output`; a source may have both roles. Dependencies have kind `before_start` or `before_done`, each requiring the referenced task to be done. Targets must be one_time tasks, because recurring tasks never reach done. Reject self-links and cycles across both kinds. Recurring tasks may depend on ordinary tasks. Appending success/no_change requires both start and completion gates satisfied for either kind; failure reports are allowed with unmet gates. Starting ordinary work and satisfying completion gates remain separate; dependencies never automatically complete work. A test result must identify what was tested and the outcome; the agent determines whether evidence applies to the final implementation.
- **Sources:** a name, a short description explaining contents and relevance, and exactly one of inline `content` or an external `locator`. `context_policy` is `always` or `when_relevant`. An optional `action` contains an `instruction` and a `manual` or `interval` trigger. Context inclusion and action scheduling are independent.
- **Source state:** retain one successful `snapshot` and one `latest_attempt` on the source, without a check-history table. A snapshot contains `observed_at`, a short current `summary`, optional opaque `checkpoint`, and supporting `references`. Target one to three sentences and enforce at most 1,000 characters for summaries. Successful no-change results advance freshness while retaining the brief; failures retain the entire last successful snapshot. Only completing the configured action advances its freshness/checkpoint; a targeted lookup for one task does not. Changing the content, locator, or action invalidates state produced under the old definition.
- **Context retrieval:** return full, untruncated inline documents. External sources return their locator, action, snapshot, latest attempt, and computed due information. Task context includes that project's current decisions and deduplicated context-linked plus `always` sources. Current one-time output descriptors stay separate. History is bounded to `latest_result` and `latest_successful_result`; matching IDs identify the same entry. Historical output IDs appear on those entries, without loading all prior entries or output descriptors. Retrieve older full entries via `list_task_results`, newest IDs first, default page size 20 (1..100). `next_before_id` continues with older IDs; cursors must exist in the same task/project. Concurrent appends do not alter subsequent pages. Expand source content with `get_source`. Reading never records execution or advances freshness.
- **Decisions:** accepted project-wide choices, distinct from sources. Store statement, rationale, and server-assigned UTC `created_at`. Preserve an earlier choice by creating a replacement with `supersedes_id`; do not overwrite decision history. Current decisions have no successor, and a current decision can have only one replacement.

Use transactions, foreign keys, and optimistic revision checks for mutable task/source records. Derive readiness and freshness instead of maintaining duplicate flags. Keep the MVP free of hosting, built-in connectors, background scheduling, agent launching, generic workflow engines, and separate handoff/artifact models. A document produced by a task becomes an ordinary Source linked with role `output`.

## 2. Tools supported

Optional update fields preserve their existing values when omitted. Explicit null clears nullable fields (task alternatives/progress/result/trigger_event, source content/locator/action, and no-change checkpoint). Supplied relationship arrays replace that set atomically; an empty array clears it. One-time output_source_ids null retains the existing compatibility no-op; recurring tasks reject that field whenever supplied. Mutable updates require `expected_revision` from the latest read.

| Tool | Parameters and purpose |
| --- | --- |
| `create_project` | Required `name` and `description`. Create a project with a server-assigned UUIDv7 ID and creation time; names need not be unique. |
| `list_projects` | Optional `query` over names/descriptions. Discover projects before selecting an explicit scope. |
| `create_task` | Required `title`, `problem_or_goal`, `design`, `acceptance_criteria`; optional `kind` (default one_time), recurring-only `trigger_event`, `alternatives_rejected`, `context_source_ids`, `prerequisite_task_ids`, `completion_task_ids`. Create the brief and relationships together. |
| `list_tasks` | Optional `kind`, `status`, exact `trigger_event`, `ready_to_start`, and `query` filters. Discover tasks and unmet dependencies; recurring tasks are never ready_to_start. |
| `get_task_context` | `task_id`. Read the complete task brief, progress/result, dependency state, current decisions, and expanded context sources. |
| `update_task` | `task_id`, `expected_revision`, and optional brief, kind-specific `status`, `progress`, recurring `trigger_event`, context/dependency IDs. One-time tasks also accept legacy `result`/output IDs and explicit completion; recurring tasks reject either field. Kind is immutable. |
| `record_task_result` | `task_id`, `expected_revision`, `outcome` (success/no_change/failure), nonblank `evidence` (max 4,000 characters), optional `output_source_ids`. Append evidence and associations atomically; increment revision without changing lifecycle. |
| `list_task_results` | `task_id`, optional `page_size` (default 20, 1..100) and `before_id`. Return newest-first `results` and `next_before_id` for stable older-history pagination. |
| `create_source` | `name`, `description`, exactly one of `content`/`locator`; optional `context_policy` and `action`. Register reusable project information; external registration does not create the external resource. |
| `list_sources` | Optional `task_id`, `representation` (`inline`/`external`), `action_due`, and `query` filters. Discover compact source descriptors; task filtering uses explicit links. |
| `get_source` | `source_id`. Expand a source to its complete inline content or external reference and current action state. |
| `update_source` | `source_id`, `expected_revision`, and optional definition changes. Correct content, location, description, context policy, or action. |
| `record_source_action` | `source_id`, `expected_revision`, and `result`: success with observation time/brief/checkpoint/references, no-change with observation time and optional checkpoint, or failure with an error. Atomically retain the latest outcome and successful snapshot. |
| `record_decision` | `statement`, `rationale`, optional `supersedes_id`. Record an accepted choice with its creation time, optionally replacing a current choice. |
| `list_decisions` | Optional `include_superseded`; default false. Read current decisions or their retained history. |

All thirteen task/source/decision tools require `project_id`; the two project tools do not. Task, task-summary, task-result, source, source-descriptor and decision results include `project_id`. Project lists return `projects`, each with `id`, `name`, `description`, and `created_at`. Unknown project IDs and records outside the supplied project are errors. Project scoping is data isolation, not authorization or a claim. Inline content and task sections are Markdown. Source action intervals use positive `every_minutes`; no successful snapshot means immediately due. Manual actions have no automatic due time. Tool descriptions and results distinguish reported verification from independently executed tests. Task filtering in `list_sources` covers current context/output links; retrieve historical output associations via `list_task_results`.

## 3. File layout

This index describes files that currently exist. Update it as implementation files are added or responsibilities move.

| File | Responsibility |
| --- | --- |
| `AGENTS.md` | Maintained project overview, MCP contract, and agent navigation index. |
| `WORKFLOW.md` | Short, host-neutral task-authoring and execution guidance embedded as MCP server instructions. |
| `Cargo.toml` | Rust package, library/binary targets, dependencies, and development dependencies. |
| `Cargo.lock` | Locked dependency versions for reproducible builds. |
| `.gitignore` | Excludes build output and local SQLite data files from version control. |
| `src/main.rs` | Binary entry point: resolve the database path, run explicit migration CLI commands, launch the MCP stdio process, or serve the read-only localhost dashboard. |
| `src/lib.rs` | Library module exports. |
| `src/dashboard.rs` | Read-only HTTP adapter: loopback listener, Host/Origin checks, GET routes over Store reads, structured errors, security headers, and embedded frontend assets. |
| `src/migration.rs` | Explicit read-only v2/v3 import into a checked, new v4 database without replacing either file. |
| `src/model.rs` | Typed MCP inputs/results, JSON schemas, and nullable-update semantics. |
| `src/server.rs` | Server metadata and embedded workflow instructions; fifteen MCP tool handlers, descriptions, annotations, and structured results. |
| `src/store.rs` | SQLite transactions, project isolation, validation, revisions, dependency gates, context assembly, freshness, and UUIDv7 generation. |
| `src/schema.sql` | Eight-table SQLite schema v4, kind-specific lifecycle, project-scoped history/foreign keys, constraints and indexes. |
| `tests/mcp_stdio.rs` | End-to-end instruction delivery, tool discovery, task workflow, and restart tests for both MCP lifecycles, launched outside the repository. |
| `tests/project_workflows.rs` | End-to-end context, dependency, action-state, decision, persistence, and concurrent-client scenarios. |
| `tests/multi_project.rs` | Real-process discovery, UUIDv7 persistence, mandatory project scopes, isolation, interleaved clients, and database constraint checks. |
| `tests/migration.rs` | Real CLI migration, full legacy value preservation, restart, unsupported-input protection, and recoverable failure tests. |
| `tests/recurring_tasks.rs` | Real MCP recurring lifecycle, append-only history, bounded context, pagination, revision, scope and dependency scenarios. |
| `tests/dashboard.rs` | Real-process dashboard scenarios: statuses, project isolation, URL/API semantics, pagination, read-without-mutation, concurrent MCP writes, Host/Origin policy, listener lifecycle and restart persistence. |
| `tests/fixtures/schema_v2.sql` | Frozen supported single-project schema for legacy migration fixtures. |
| `tests/fixtures/schema_v3.sql` | Frozen supported multi-project schema for explicit v3 upgrade fixtures. |
| `tests/support/mod.rs` | Test client that launches the real binary and invokes tools over MCP stdio. |
| `web/package.json` | React 19, TanStack Query, shadcn/ui-style component dependencies, and Vite build scripts. |
| `web/package-lock.json` | Locked frontend dependency graph for reproducible builds. |
| `web/vite.config.ts` | Production build that emits one embedded `app.js` and `app.css` bundle. |
| `web/tsconfig.json` | Strict TypeScript configuration for the frontend and Vite build. |
| `web/index.html` | Dashboard HTML shell with same-origin asset references and an inline-script-free policy surface. |
| `web/favicon.svg` | Embedded dashboard favicon. |
| `web/src/main.tsx` | React entry point with `QueryClientProvider`. |
| `web/src/App.tsx` | URL selection, project header, board queries, polling and detail-panel orchestration. |
| `web/src/components/Board.tsx` | Lifecycle groups, status columns, counts, search state and accessible task cards. |
| `web/src/components/TaskDetail.tsx` | Full brief, dependencies, decisions, sources, outputs, bounded/live history and explicit source expansion; expanded outputs poll and retain cached documents with a retry notice on refresh failure. |
| `web/src/components/ProjectChooser.tsx` | Explicit project chooser that distinguishes duplicate names by full ID. |
| `web/src/components/ui/button.tsx`, `card.tsx`, `badge.tsx`, `input.tsx` | shadcn/ui-style primitives used by the dashboard. |
| `web/src/lib/api.ts`, `types.ts`, `format.ts`, `utils.ts` | Typed read API, TanStack Query keys, formatting helpers and class merging. |
| `web/src/lib/markdown.tsx` | Safe Markdown-to-React renderer: escaped text, http(s)-only links, no raw HTML or image fetching. |
| `web/src/index.css` | Tailwind CSS v4 theme, base styles and Markdown presentation. |
| `web/dist/app.js`, `web/dist/app.css` | Committed production bundle embedded into the Rust binary at compile time. |

Build with `cargo build --locked`; launch `target/debug/dictum-mcp` through an MCP client with `DICTUM_DATABASE` set to an absolute shared path. Launch the dashboard with `DICTUM_DATABASE=/abs/path target/debug/dictum-mcp dashboard --port 3000` (port `0` selects a free port); it reports the actual URL and database path on stderr. The committed `web/dist` bundle lets `cargo build --locked` work without Node; after changing frontend sources, run `(cd web && npm ci && npm run build)` and commit the refreshed bundle. Verify with `cargo fmt --check`, `cargo test --locked`, and `cargo clippy --locked --all-targets -- -D warnings`. Startup rejects older schemas without modifying them. Explicit v3 upgrade: `dictum-mcp migrate --source OLD --destination NEW` (no naming flags). Direct v2 import: add `--name NAME --description DESC` to create one UUIDv7 project. Both require a new destination, preserve all old IDs, values, revisions, timestamps, source state and relationships, and leave the source unchanged. V3 preserves project UUIDs/metadata. Existing tasks become one_time; legacy result text is retained without fabricated history. Unsupported prototypes remain rejected; preserve them and choose a new path.

## 4. Rules

- End-to-end tests only. No unit tests. Exercise the real processes through MCP stdio and, for dashboard changes, the real HTTP listener against the same SQLite database, including persistence; do not test internal functions or the store API directly. Database setup/inspection is allowed for test fixtures and persistence assertions. UI behavior needs a real browser check in addition to HTTP assertions.
