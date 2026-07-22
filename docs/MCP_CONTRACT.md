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
- eval suite check, run, show, compare, gate, invalidate, rescore, and evidence refs
- trace item, log add, and log read (including three-stage route observations)
- provider-neutral agent registry reads and route overrides
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

## Eval Evidence Contract

`planr_eval_evidence_ref` mirrors `planr eval evidence-ref`: it attaches an eval run or comparison id to an existing log, review item, or artifact. The returned eval envelope records `closure_authority: false`; eval verdicts are audit evidence only and never close, approve, or otherwise mutate map item status.

## Install Contract

`planr install <client> --dry-run` prints the complete client-owned MCP, role, skill, and hook-reconciliation paths for Codex, Claude Code, and Cursor without writing them. Non-dry install writes only repository-local files, with this ownership contract:

- Codex: the CLI writes `.planr/integrations/codex-mcp.toml` and `.codex/hooks.json`; the plugin owns all ten workflow skills; neither path writes Planr project roles or project skills
- Claude Code: the CLI writes `.mcp.json`, standalone `.claude/agents/` roles, and `.claude/settings.json` hooks, but no project skills; the plugin owns all ten workflow skills and its plugin agents
- Cursor: the CLI writes `.cursor/mcp.json`, both `.cursor/agents/` roles, all ten `.cursor/skills/` skill copies, and `.cursor/hooks.json`

The Cursor dry-run additionally prints a `cursor://anysphere.cursor-deeplink/mcp/install` link whose embedded config (`planr mcp`, no `--db`) is safe at user scope because each workspace resolves its own database. Planr does not edit global client configuration without a separate explicit operator action; the deeplink requires the operator to click it and confirm inside Cursor.

`--no-mcp` skips only the project MCP artifact: Codex reconciles hooks only; Claude Code writes standalone roles and hooks but no project skills; Cursor writes roles, all ten skills, and hooks. `--no-hooks` is the independent hook opt-out and can be combined with `--no-mcp`.
