# Analytics And Observability Specification

## Observability Goals

- Help users debug item state, picks, MCP setup, and agent runs.
- Avoid collecting source code, prompts, responses, secrets, or private plan content.
- Make local diagnostics exportable for bug reports after user review.

## Local Event Log

Events stored in SQLite:

- project_created
- item_created
- item_ready
- item_picked
- item_heartbeat
- item_progress
- item_paused
- item_resumed
- item_started
- item_closed
- item_blocked
- item_failed
- dependency_added
- plan_parsed
- plan_parse_failed
- log_created
- artifact_created
- review_annotation_added
- review_feedback_ingested
- review_artifact_written
- review_requested
- review_closed
- context_created
- import_completed
- import_parsed
- export_written
- mcp_tool_called
- doctor_check_completed

## Metrics

Local aggregate metrics:

- items by status;
- ready queue size;
- running item count;
- blocked item count;
- average pick-to-close duration;
- failed run count;
- review finding count;
- MCP call count by tool name;
- database schema version.

## Logs

Default logs:

- command name;
- exit status;
- duration;
- error code;
- item id/project id;
- client type.

Forbidden logs:

- full source code;
- plan body content by default;
- prompts/responses;
- command output unless explicitly attached by user;
- secrets or environment values.

Debug bundle preview must include counts, ids, paths, event metadata, and logs, but must not inline source file content or prompt/response transcripts. Inline artifact content is only present when the user explicitly attached small content as an artifact.

## Doctor Diagnostics

`planr doctor` must check:

- binary version;
- database open/schema-upgrade status;
- `.planr` pack presence;
- Git repo status;
- Codex availability and MCP config hint;
- Claude Code availability/config hint;
- Cursor config hint;
- MCP stdio server startup;
- optional HTTP server startup;
- permission issues.

## Alerts

V1 has no remote alerts. CLI/TUI should visibly show:

- stale running items without heartbeat;
- database lock issues;
- parse errors;
- failed agent runs;
- blocked critical path.

## Cost Monitoring

Planr does not call providers by default. Optional runner wrappers may record provider/model metadata and token/cost estimates only when exposed by the client and only as metadata.

## Debug Bundle

`planr debug bundle` should create a local archive containing:

- version;
- schema version;
- redacted config;
- event metadata;
- doctor output;
- selected item/log metadata.

It must exclude plan bodies, source files, prompts, responses, and secrets by default.

## Acceptance Criteria

- REQ-ANA-001: A user can diagnose why no items are ready.
- REQ-ANA-002: A user can diagnose why an MCP client cannot see Planr tools.
- REQ-ANA-003: Debug export is redacted by default.
- REQ-ANA-004: No content analytics are emitted in V1.
