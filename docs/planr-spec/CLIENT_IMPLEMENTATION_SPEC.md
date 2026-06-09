# Client Implementation Specification

## Client Surfaces

- CLI: primary V1 interface.
- MCP client: primary agent integration.
- Optional TUI/dashboard: local visual inspection.
- Install helpers: client-specific setup for Codex, Claude Code, Cursor.

## CLI Requirements

- REQ-CLI-001: Every command that mutates state must support `--json`.
- REQ-CLI-002: Human output must include next actions without hiding failure causes.
- REQ-CLI-003: Commands must be composable in shell scripts.
- REQ-CLI-004: `--db` or equivalent must allow alternate database paths.
- REQ-CLI-005: Planr must derive a stable worker id automatically from client/session context where available.
- REQ-CLI-006: `planr prompt` must print client-ready CLI, MCP, or HTTP instructions and report that global config was not edited.
- REQ-CLI-007: Recovery, import, cancellation, and replan commands must support preview-first workflows before destructive mutations.

## Command Groups

```text
planr project ...
planr plan ...
planr map ...
planr item ...
planr pick
planr log ...
planr review ...
planr recover ...
planr prompt cli|mcp|http
planr close
planr context ...
planr search
planr doctor
planr mcp
planr serve
```

## MCP Client Requirements

- REQ-CLIENT-010: MCP tools must have stable names and JSON schemas.
- REQ-CLIENT-011: MCP prompts must expose plan/work/review/map/summary workflows.
- REQ-CLIENT-012: MCP resources must be read-only.
- REQ-CLIENT-013: Mutation tools must return compact log summaries and next-action hints.

## Codex Integration

V1 must provide:

- `planr install codex` or `planr doctor --client codex`.
- MCP registration instructions compatible with Codex CLI.
- Optional AGENTS.md snippet.
- Optional `planr codex run` post-V1 wrapper for `codex exec`.

Acceptance:

- REQ-CLIENT-020: A Codex user can see Planr MCP registration command and verify it with `codex mcp list`.
- REQ-CLIENT-021: Codex prompts must not assume Codex is the only agent in the project.

## Claude Code Integration

V1 must provide:

- `.mcp.json` project-scoped config example.
- `claude mcp add` command example where available.
- Prompt/skill instructions that preserve Planr graph SSOT.

Acceptance:

- REQ-CLIENT-030: Claude Code install output must explain project vs user scope.
- REQ-CLIENT-031: Claude Code prompt package must include plan/work/review/map/summary workflows.

## Cursor Integration

V1 must provide:

- `.cursor/mcp.json` project config example.
- Global `~/.cursor/mcp.json` example.
- Cursor Agent usage notes.

Acceptance:

- REQ-CLIENT-040: Cursor install output must distinguish stdio, SSE, and streamable HTTP options when relevant.
- REQ-CLIENT-041: Cursor prompts must avoid relying on Codex-only skill behavior.

## Optional TUI/Dashboard

If implemented:

- read-only by default;
- optional mutation actions behind confirmation;
- graph and list views;
- live event updates;
- local-only server.

The implemented local browser review workspace is served at `/review` with data from `/v1/review-workspace`.

## Offline Behavior

All V1 client flows must work without internet once the binary is installed.

## Error Handling

Errors must include:

- machine-readable code;
- plain-language message;
- affected object id/path;
- suggested next command when safe.

## Client Tests

- CLI golden output tests.
- JSON schema output tests.
- MCP tool discovery tests.
- Config-generation fixture tests for Codex, Claude Code, and Cursor.
- Prompt output tests for CLI, MCP, HTTP, and per-client wording.
- Browser workspace smoke tests against localhost.
