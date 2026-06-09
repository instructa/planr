# Technical Architecture

## Architecture Goals

- REQ-ARCH-001: Keep item state, picks, worker runtime state, approval gates, links, log, reviews, and events in one local SQLite source of truth.
- REQ-ARCH-002: Keep rich product and build plan context in repo-local Markdown files that remain useful without Planr installed.
- REQ-ARCH-003: Support CLI, MCP, and optional HTTP/SSE as lenses over the same core engine.
- REQ-ARCH-004: Make Codex, Claude Code, Cursor, and generic MCP clients first-class integration targets.
- REQ-ARCH-005: Avoid provider-specific logic in the core graph engine.

## System Context Diagram

```text
Developer
  -> planr CLI
     -> Core engine
        -> SQLite graph database
        -> .planr Markdown project
        -> Git repository

Agent clients
  -> MCP stdio / MCP HTTP
     -> Core engine
        -> SQLite graph database
        -> .planr Markdown project

Optional dashboard
  -> Local HTTP/SSE server
     -> Core engine
```

## Component Boundaries

- `core`: map graph operations, state machine, item readiness, worker runtime state, approval gates, log, reviews, contexts, search.
- `storage`: SQLite schema, migrations, transactions, FTS indexes.
- `planpack`: `.planr` project pack, plan parsing, Markdown frontmatter, review artifacts.
- `cli`: user commands and deterministic output.
- `mcp`: tools, resources, prompts, capability negotiation.
- `server`: optional local REST/SSE API.
- `agents`: integration helpers for Codex, Claude Code, Cursor, and generic clients.
- `git`: worktree, branch, diff, and changed-file log.
- `recovery`: stale-pick detection, timeout handling, retry policy, and manual condition reporting.
- `review_workspace`: local browser review HTML and workspace JSON projection.
- `packages`: reusable JSON export/import packages with plan snapshots, logs, contexts, and review artifacts.

## Client Architecture

- CLI is the primary client and must be scriptable.
- TUI/dashboard are optional clients over the same API.
- MCP clients must receive concise, structured responses optimized for LLM context.
- All client layers must call core services, not mutate database or Markdown directly.

## Backend Architecture

V1 is a local backend packaged into the CLI binary:

- SQLite database at `.planr/planr.sqlite` by default.
- WAL mode enabled for concurrent readers and serialized writers.
- Local service optional for dashboards and long-running agent orchestration.
- Local review workspace is served from the same localhost HTTP boundary as the REST/SSE API.
- No cloud backend in V1.

## Data Architecture

- Map state, worker heartbeats, progress, stale ownership data, and approval gates: SQLite.
- Rich product plans, build plans, and project context: `.planr/*.md`.
- Live status summaries: SQLite.
- Search: SQLite FTS over items, contexts, plan metadata/frontmatter/headings, logs, and review summaries.
- Large artifacts: paths and metadata by default; inline content only for small explicitly provided artifacts.

## AI Architecture

Planr does not call model providers by default. It guides external agents through:

- MCP tools for map, plan, log, and review operations.
- MCP prompts for `plan`, `work`, `review`, `map`, and `summary` workflows.
- Client-specific install snippets for Codex, Claude Code, and Cursor.
- Optional runner wrappers for local Codex/Claude/Cursor CLIs when explicitly configured.

## Auth And Identity

- V1 local mode uses OS user access and file permissions.
- Worker identity is an explicit `worker_id` string, not an auth boundary.
- HTTP server binds to localhost by default.
- Remote HTTP mode is post-V1 and must require authentication.

## Integrations

- Codex: CLI instructions, MCP registration, optional `codex exec` runner, optional `codex review` integration.
- Claude Code: `.mcp.json` or CLI-based MCP registration guidance.
- Cursor: `.cursor/mcp.json` project config and global config guidance.
- Generic MCP: stdio first; streamable HTTP optional.
- Git: worktree isolation and scoped diff log.

## Security Architecture

- No shell hooks by default.
- Any command runner must show command, cwd, environment policy, and worker id.
- Secrets must not be stored in database, plans, logs, or analytics.
- MCP tools that mutate state must be separated from read-only resources/prompts.
- Destructive graph operations require preview or explicit flags.

## Privacy Architecture

- Local-only by default.
- No provider telemetry by default.
- No content logging by default.
- Export and delete commands must remove Planr database state and `.planr` artifacts when requested.

## Observability

- Local structured event log in SQLite.
- Optional JSONL debug log with content scrubbing.
- `planr doctor` for installation, database, MCP, and client integration checks.
- `planr trace item <id>` for item lifecycle debugging.

## Deployment Environments

- Local development: source checkout and debug binary.
- Local production: installed binary via package manager or release asset.
- CI: headless CLI mode with explicit db/path.
- Hosted/team: out of scope for V1.

## Failure Modes And Fallback Behavior

- Database locked: retry bounded writes, then return actionable diagnostic.
- Corrupt Markdown plan: preserve file, mark parse status degraded, keep map usable.
- Missing MCP client support: print manual CLI instructions.
- Agent run interrupted: keep item picked/running with heartbeat timeout and release/re-pick command.
- Recovery sweep interrupted: preview is non-mutating; apply mutates only listed recoverable work and records events.
- Review fails: create fix item chain instead of closing the parent.
- Package import cancelled: preview leaves database and `.planr` files unchanged.

## Scalability Assumptions

- V1 target: hundreds of projects, tens of thousands of items, thousands of plan files per machine.
- SQLite and FTS are sufficient for V1.
- Remote multi-user concurrency is post-V1.

## Open Technical Decisions

- OD-ARCH-001: Whether to implement worktree management in core V1 or as an extension.
- OD-ARCH-002: Whether dashboard uses server-rendered HTML, TUI, or a small SPA.
