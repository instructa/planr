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
- trace item, log add, and log read (including three-stage route observations)
- built-in policy/binding catalog listing plus safe-pack/custom composition status
- declarative registry verification with canonical evaluation/safe-binding gates, preview-first immutable import, and manifest-anchored integrity/signature/freshness-checked offline cache listing
- policy preset preview/apply by path or built-in id with repository-only target validation and deterministic provenance lock
- deterministic offline preset simulation plus explicit opt-in live-host execution with Planr-controlled challenge workspaces, strict task artifacts read and hashed by Planr, candidate/task outcome oracles, failed-live-attempt `unverified`/incomplete lifecycle semantics, production policy-capability checks, estimated arbitrary-process claims, optional Ed25519-verified run/suite/time/task/challenge-bound telemetry that alone can promote effective route and usage evidence to trusted/recommendation-eligible, observed process latency, transition/correction/violation counts, result hashes, and deterministic lifecycle thresholds
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

- Codex: `.planr/integrations/codex-mcp.toml` plus `.codex/agents/` roles
- Claude Code: `.mcp.json` plus `.claude/agents/` roles
- Cursor: `.cursor/mcp.json` plus `.cursor/agents/` roles and `.cursor/skills/` skill copies

The Cursor dry-run additionally prints a `cursor://anysphere.cursor-deeplink/mcp/install` link whose embedded config (`planr mcp`, no `--db`) is safe at user scope because each workspace resolves its own database. Planr does not edit global client configuration without a separate explicit operator action; the deeplink requires the operator to click it and confirm inside Cursor.

`planr install <client> --no-mcp` is the plugin-style variant: it writes the subagent roles (and, for Cursor, the skills) but no MCP configuration at all, for setups that use skills and agents over the CLI only.
