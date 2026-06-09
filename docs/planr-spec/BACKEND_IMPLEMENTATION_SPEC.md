# Backend Implementation Specification

## Backend Scope

The backend is a local engine embedded in the `planr` binary. It owns map state, plan parsing, search, MCP tools, optional HTTP/SSE, and integration runners.

## Language And Runtime

Recommended default:

- Rust 2021 or newer.
- `rusqlite` or equivalent SQLite binding.
- `clap` for CLI.
- `serde` for JSON.
- `tokio` and `axum` only if HTTP/SSE ships in V1.

## Core Modules

```text
src/
  core/
    graph.rs
    state_machine.rs
    readiness.rs
    log.rs
    reviews.rs
    search.rs
  storage/
    schema.rs
    migrations.rs
    sqlite.rs
  planpack/
    markdown.rs
    frontmatter.rs
    project_pack.rs
    import_planr_data.rs
  cli/
  mcp/
  server/
  agents/
    codex.rs
    claude_code.rs
    cursor.rs
  git/
```

## State Machine

REQ-BE-001: State transitions must be centralized.

Required transitions:

```text
pending -> ready
ready -> picked
picked -> running
running -> picked
running -> closed
running -> blocked
running -> failed
failed -> ready
running -> cancelled
```

Lenient single-agent convenience may allow `ready -> closed` only when no other active pick exists and the worker id is explicit.

## Readiness Engine

REQ-BE-010: Readiness must be recomputed transactionally after item creation, link changes, and closure.

REQ-BE-011: An item is ready only when all blocking and feeds-into links are closed or closed_partial.

REQ-BE-012: Parent item closure must check required children and reviews.

## Atomic Picking

REQ-BE-020: Picking must use an atomic update equivalent to:

```sql
UPDATE items
SET status = 'picked',
    worker_id = ?,
    pick_token = ?,
    picked_at = CURRENT_TIMESTAMP,
    last_heartbeat_at = CURRENT_TIMESTAMP
WHERE id = ? AND status = 'ready';
```

The caller succeeds only when exactly one row changes.

REQ-BE-021: Heartbeat updates must require the same worker owner when an owner exists, move `picked` to `running`, and refresh `last_heartbeat_at`.

REQ-BE-022: Progress, pause, and resume must preserve `worker_id` and `pick_token`.

REQ-BE-023: Stale detection must compare `last_heartbeat_at`, `picked_at`, or `updated_at` against an explicit threshold and release only when the caller requests release.

## Approval Gates

REQ-BE-025: Approval request must store status `requested`, request time, and optional reason.

REQ-BE-026: Approval approve and deny must store the decision maker and optional comment.

REQ-BE-027: Close must reject items with approval status `requested` or `denied`.

## Markdown Plan Parser

REQ-BE-030: Parser must support YAML frontmatter and Markdown headings.

REQ-BE-031: Parser must preserve files on parse errors and store a parse error record.

REQ-BE-032: Parser must map headings to stable section ids using slug + heading depth + ordinal.

REQ-BE-033: Parser must not execute instructions contained in plan files.

## Search

REQ-BE-040: Search must index:

- item title/description;
- context content;
- plan title/manifest/frontmatter/headings;
- log summary;
- review findings summary.

REQ-BE-041: Search results must identify source type and path/id.

## MCP Server

REQ-BE-050: MCP stdio server must start with `planr mcp`.

REQ-BE-051: Tool schemas must validate all mutation inputs.

REQ-BE-052: Tools must return compact JSON optimized for agent context.

REQ-BE-053: Resources must expose read-only project, item, plan, and log data.

REQ-BE-054: Prompts must expose Planr workflow prompts.

## HTTP/SSE Server

Optional V1:

- `planr serve --port 8484`.
- localhost default bind.
- JSON REST API.
- SSE stream for events.
- no remote auth in V1 unless explicitly implemented.

## `.planr` Import

REQ-BE-060: Import must detect:

- `.planr/project/*.md`;
- `.planr/plans/`;
- `.planr/plans/*.plan.md`;
- `.planr/status/current.json`;
- `.planr/reviews/*.review.md`;
- `skills/planr-*` when present in an exported package.

REQ-BE-061: Import must read existing plan files and live status scopes without deleting originals.

REQ-BE-062: Import must map existing status scopes to map items and logs when possible.

## Agent Runners

Runner wrappers are optional but expected after core V1:

- Codex: `codex exec`, `codex review`.
- Claude Code: command runner or MCP-only guidance.
- Cursor: `cursor-agent` where available or MCP-only guidance.

REQ-BE-070: Runners must record run metadata and log.

REQ-BE-071: Runners must not hide approval, sandbox, or command failures.

## Verification

Backend must include:

- unit tests for state transitions;
- integration tests for atomic pick races;
- integration tests for heartbeat, progress, pause, resume, stale release, and approval-blocked closure;
- parser tests for valid and invalid plan files;
- MCP tool schema tests;
- import fixture tests for existing `.planr` layouts.
