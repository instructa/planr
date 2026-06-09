# MCP Contract

Planr exposes a local stdio MCP server with a stable V1 contract for coding-agent clients.

## Server

```bash
planr --db .planr/planr.sqlite mcp
```

The server supports:

- `tools/list`
- `tools/call`
- `resources/list`
- `resources/read`
- `prompts/list`
- `prompts/get`

## Machine-Checkable Fixture

The canonical fixture is:

```text
docs/fixtures/mcp-contract.json
```

Tests compare this fixture against live MCP stdio responses, install dry-run output, and CLI reference coverage.

## Tool Contract

Every tool declares a real JSON Schema: typed `properties`, explicit `required` fields, and `additionalProperties = false`. The only exception is `planr_review_ingest`, which keeps `additionalProperties = true` so arbitrary hook payload shapes can be ingested. Unknown tools return an `isError` MCP result containing a JSON error with code `not_found`.

Required groups:

- project and map reads
- plan creation, refinement, split, check, and link
- map build, preview, unlocks, lookahead, and pressure-oriented reads
- item create, breakdown, insert, amend, and replan
- pick, heartbeat, progress, pause, resume, stale inspection, and recovery sweep
- approval request, approve, deny, and list
- artifact add, list, and show
- event list and debug bundle preview
- log add and read
- review annotate, ingest, artifact, evidence, and close
- item close, context create, and search

`planr_recover_sweep` mirrors `planr recover sweep`: it previews by default and only mutates state when `apply` is true. It returns stale picked work, timed-out work, retryable failed work, exhausted failures, and applied release/retry counts.

## Review Contract

Review feedback ingestion is advisory:

- `planr_review_annotate` stores item-linked annotation context.
- `planr_review_ingest` stores hook-compatible feedback and never auto-closes or auto-approves work.
- `planr_review_artifact` writes a privacy-minimized review artifact.
- `planr_review_evidence` returns Git/PR evidence scoped to files named by item logs or artifacts, and treats unrelated dirty files as non-owned.
- `planr_review_close` records the final verdict, writes a review artifact, and creates fix/follow-up review work when the verdict is not clean.

HTTP mirrors the same rule: `GET /v1/reviews/:id/artifact` is read-only; `POST /v1/reviews/:id/artifact` writes an artifact explicitly.

## Install Contract

`planr install <client> --dry-run` prints project-scoped configuration for Codex, Claude Code, and Cursor. Non-dry install writes only repository-local files:

- Codex: `.planr/integrations/codex-mcp.toml`
- Claude Code: `.mcp.json`
- Cursor: `.cursor/mcp.json`

Planr does not edit global client configuration without a separate explicit operator action.
