# Dictum

`AGENTS.md` is part of this repository's source code and should evolve with the package. Update it when the project design, MCP tool contracts, or file layout changes meaningfully. In particular, consider updating the file index when adding, moving, or removing a file. Routine work does not require editing this document.

## 1. Project overview

Dictum is a local project-management MCP server for coding agents. Its goal is to let any working agent understand a project, execute a well-defined task, and resume previous work using durable records instead of the original conversation. The agent performs research, external tool calls, implementation, and verification; Dictum manages context, relationships, progress, and accepted decisions.

The MVP uses Rust, `rmcp`, stdio, and one SQLite database containing multiple projects per server process. Multiple local clients must point to the same database to share project state. `DICTUM_DATABASE` selects its path; the default is `.dictum/dictum.sqlite3` relative to the process working directory. Stdout is reserved for MCP; diagnostics go to stderr.

`WORKFLOW.md` is the canonical, compact agent workflow (at most 2,000 characters). It is embedded in the binary and delivered through MCP server instructions, not a separate tool or required skill installation. Put task-authoring and authorization rules first. The human and agent coauthor the actual task content, not a separate approval brief; preserve agreed intent, constraints, required approach, and acceptance criteria. Once authorized, one coordinator can derive inspectable tasks, delegate bounded work, obtain separate reviews, and record evidence. These are agent guidance, not server-enforced approval or orchestration. Clients control instruction inclusion, retention, and subagent inheritance; actual CLI behavior needs separate validation.

The agreed model has six tables: `projects`, `tasks`, `sources`, `decisions`, `task_sources`, and `task_dependencies`.

- **Projects:** a name, description, immutable server-assigned UUIDv7 ID, and UTC `created_at`. Names need not be unique. Every task, source, decision, relationship, dependency and supersession belongs to exactly one project. Require an existing `project_id` on all task/source/decision tools and reject cross-project references atomically. Discover with `list_projects` before `list_tasks(project_id, ready_to_start=true)` and `get_task_context(project_id, task_id)`; resolve ambiguity or no match with the human rather than implicitly creating or selecting a project. Scope does not grant execution authorization or claim work.

- **Tasks:** a title, required Markdown sections for problem/goal and design, acceptance criteria, and optional rejected alternatives. Keep this execution brief separate from status, progress, and the actual result/verification evidence. A fresh agent should be able to work from the brief and linked sources. Statuses are `todo`, `in_progress`, `blocked`, and `done`.
- **Task relationships:** sources are linked many-to-many with role `context` or `output`; a source may have both roles. Dependencies have kind `before_start` or `before_done`, each requiring the referenced task to be done. Reject self-links and cycles across both kinds. Starting work and satisfying completion gates are separate; completing dependencies never automatically marks a task done. A test task's result must identify what was tested and the outcome; the working agent determines whether that evidence applies to the final implementation.
- **Sources:** a name, a short description explaining contents and relevance, and exactly one of inline `content` or an external `locator`. `context_policy` is `always` or `when_relevant`. An optional `action` contains an `instruction` and a `manual` or `interval` trigger. Context inclusion and action scheduling are independent.
- **Source state:** retain one successful `snapshot` and one `latest_attempt` on the source, without a check-history table. A snapshot contains `observed_at`, a short current `summary`, optional opaque `checkpoint`, and supporting `references`. Target one to three sentences and enforce at most 1,000 characters for summaries. Successful no-change results advance freshness while retaining the brief; failures retain the entire last successful snapshot. Only completing the configured action advances its freshness/checkpoint; a targeted lookup for one task does not. Changing the content, locator, or action invalidates state produced under the old definition.
- **Context retrieval:** return full, untruncated inline documents. External sources return their locator, action, snapshot, latest attempt, and computed due information. Task context includes only that project's current decisions and the deduplicated union of context-linked sources and `always` sources; output links remain visible as task results. A fresh brief may still require deeper reading through the agent's external tools. Reading context never records an action as completed.
- **Decisions:** accepted project-wide choices, distinct from sources. Store statement, rationale, and server-assigned UTC `created_at`. Preserve an earlier choice by creating a replacement with `supersedes_id`; do not overwrite decision history. Current decisions have no successor, and a current decision can have only one replacement.

Use transactions, foreign keys, and optimistic revision checks for mutable task/source records. Derive readiness and freshness instead of maintaining duplicate flags. Keep the MVP free of hosting, built-in connectors, background scheduling, agent launching, generic workflow engines, and separate handoff/artifact models. A document produced by a task becomes an ordinary Source linked with role `output`.

## 2. Tools supported

Optional update fields preserve their existing values when omitted. Explicit null clears nullable fields (task alternatives/progress/result, source content/locator/action, and no-change checkpoint). Supplied relationship arrays replace that relationship set atomically; an empty array clears it. Mutable updates require `expected_revision` from the latest read.

| Tool | Parameters and purpose |
| --- | --- |
| `create_project` | Required `name` and `description`. Create a project with a server-assigned UUIDv7 ID and creation time; names need not be unique. |
| `list_projects` | Optional `query` over names/descriptions. Discover projects before selecting an explicit scope. |
| `create_task` | Required `title`, `problem_or_goal`, `design`, `acceptance_criteria`; optional `alternatives_rejected`, `context_source_ids`, `prerequisite_task_ids`, `completion_task_ids`. Create the brief and its relationships together. |
| `list_tasks` | Optional `status`, `ready_to_start`, and `query` filters. Discover tasks and their unmet start/completion dependencies. |
| `get_task_context` | `task_id`. Read the complete task brief, progress/result, dependency state, current decisions, and expanded context sources. |
| `update_task` | `task_id`, `expected_revision`, and optional brief, `status`, `progress`, `result`, context/output source IDs, or dependency IDs. Save progress, change links, or explicitly complete work. |
| `create_source` | `name`, `description`, exactly one of `content`/`locator`; optional `context_policy` and `action`. Register reusable project information; external registration does not create the external resource. |
| `list_sources` | Optional `task_id`, `representation` (`inline`/`external`), `action_due`, and `query` filters. Discover compact source descriptors; task filtering uses explicit links. |
| `get_source` | `source_id`. Expand a source to its complete inline content or external reference and current action state. |
| `update_source` | `source_id`, `expected_revision`, and optional definition changes. Correct content, location, description, context policy, or action. |
| `record_source_action` | `source_id`, `expected_revision`, and `result`: success with observation time/brief/checkpoint/references, no-change with observation time and optional checkpoint, or failure with an error. Atomically retain the latest outcome and successful snapshot. |
| `record_decision` | `statement`, `rationale`, optional `supersedes_id`. Record an accepted choice with its creation time, optionally replacing a current choice. |
| `list_decisions` | Optional `include_superseded`; default false. Read current decisions or their retained history. |

All eleven task/source/decision tools require `project_id`; the two project tools do not. Task, task-summary, source, source-descriptor and decision results include `project_id`. Project lists return `projects`, each with `id`, `name`, `description`, and `created_at`. Unknown project IDs and records outside the supplied project are errors. Project scoping is data isolation, not an authorization or claim system. Inline source content and task sections are Markdown. Source action intervals use positive `every_minutes`; an interval action without a successful snapshot is due immediately. Manual actions have no automatic due time. Tool descriptions and structured results must distinguish reported verification from independently executed tests.

## 3. File layout

This index describes files that currently exist. Update it as implementation files are added or responsibilities move.

| File | Responsibility |
| --- | --- |
| `AGENTS.md` | Maintained project overview, MCP contract, and agent navigation index. |
| `WORKFLOW.md` | Short, host-neutral task-authoring and execution guidance embedded as MCP server instructions. |
| `Cargo.toml` | Rust package, library/binary targets, dependencies, and development dependencies. |
| `Cargo.lock` | Locked dependency versions for reproducible builds. |
| `.gitignore` | Excludes build output and local SQLite data files from version control. |
| `src/main.rs` | Binary entry point: resolve the database path, run explicit migration CLI commands, or launch the MCP stdio process. |
| `src/lib.rs` | Library module exports. |
| `src/migration.rs` | Explicit read-only import of supported v2 data into a checked, new v3 database without replacing either file. |
| `src/model.rs` | Typed MCP inputs/results, JSON schemas, and nullable-update semantics. |
| `src/server.rs` | Server metadata and embedded workflow instructions; thirteen MCP tool handlers, descriptions, annotations, and structured results. |
| `src/store.rs` | SQLite transactions, project isolation, validation, revisions, dependency gates, context assembly, freshness, and UUIDv7 generation. |
| `src/schema.sql` | Six-table SQLite schema v3, project-scoped foreign keys, constraints, and indexes. |
| `tests/mcp_stdio.rs` | End-to-end instruction delivery, tool discovery, task workflow, and restart tests for both MCP lifecycles, launched outside the repository. |
| `tests/project_workflows.rs` | End-to-end context, dependency, action-state, decision, persistence, and concurrent-client scenarios. |
| `tests/multi_project.rs` | Real-process discovery, UUIDv7 persistence, mandatory project scopes, isolation, interleaved clients, and database constraint checks. |
| `tests/migration.rs` | Real CLI migration, full legacy value preservation, restart, unsupported-input protection, and recoverable failure tests. |
| `tests/fixtures/schema_v2.sql` | Frozen supported single-project schema for legacy migration fixtures. |
| `tests/support/mod.rs` | Test client that launches the real binary and invokes tools over MCP stdio. |

Build with `cargo build --locked`; launch `target/debug/dictum-mcp` through an MCP client with `DICTUM_DATABASE` set to an absolute shared database path. Use `cargo fmt --check`, `cargo test --locked`, and `cargo clippy --locked --all-targets -- -D warnings` to verify changes. The server rejects earlier schemas without modifying them. Explicitly migrate a v2 single-project database with `dictum-mcp migrate --source OLD --destination NEW --name NAME --description DESC`. The destination must be new; migration creates one UUIDv7 project and preserves existing record IDs, relationships, revisions, timestamps and source state while leaving the source unchanged. Older unsupported prototype databases remain rejected; preserve them and choose a new path.

## 4. Rules

- End-to-end tests only. No unit tests. Exercise the real server process through MCP stdio, including its SQLite persistence; do not test internal functions or the store API directly. Database setup/inspection is allowed for test fixtures and persistence assertions.
