# QA Acceptance Tests

## Test Strategy

Planr needs fast deterministic tests around graph correctness, plan parsing, MCP contracts, and migration. The smallest relevant test should run before broader suites.

## Unit Tests

### State Machine

- REQ-QA-001: Valid transitions succeed.
- REQ-QA-002: Invalid transitions return `invalid_transition`.
- REQ-QA-003: Parent item cannot close while required child review is open.
- REQ-QA-004: Review annotation and feedback ingestion persist item-linked evidence without auto-closing or auto-approving work.
- REQ-QA-005: Closing a review writes a `.planr/reviews/*.review.md` artifact and registers it as a review artifact.

### Dependency Readiness

- REQ-QA-010: Item with no blockers becomes ready.
- REQ-QA-011: Item with unfinished upstream remains pending.
- REQ-QA-012: Completing upstream promotes downstream in the same transaction.
- REQ-QA-013: Soft relationship does not block readiness.

### Claiming

- REQ-QA-020: Two concurrent picks for one item produce exactly one winner.
- REQ-QA-021: Picked item cannot be picked by another agent.
- REQ-QA-022: Timeout/release policy is explicit and tested.
- REQ-QA-023: Heartbeat updates move picked work to running and record `last_heartbeat_at`.
- REQ-QA-024: Progress, pause, and resume preserve the worker claim while updating runtime state.
- REQ-QA-025: Stale picked work can be detected and intentionally released for re-pick.
- REQ-QA-026: Requested or denied approvals block close; approved approvals allow close when other gates pass.
- REQ-QA-027: Recovery sweep previews stale, timed-out, and retryable work before applying changes.
- REQ-QA-028: Recovery apply records recovery events and preserves manual pre/post condition visibility.

### Plan Parser

- REQ-QA-030: Valid `.plan.md` parses frontmatter and sections.
- REQ-QA-031: Invalid frontmatter records parse error without rewriting file.
- REQ-QA-032: Unknown frontmatter fields are preserved.

## Integration Tests

### CLI

- `planr project init` creates database and `.planr` pack.
- `planr plan new` creates a plan package.
- `planr plan split` creates a deterministic build plan file.
- `planr item create` creates a map item.
- `planr pick` picks next ready item.
- `planr log add` creates log.
- `planr close` closes item and promotes downstream.
- `planr map show --json` returns valid JSON.
- `planr recover sweep` previews and applies explicit recovery actions.
- `planr review evidence` records item-scoped Git evidence without source content.
- `planr prompt cli|mcp|http` prints setup text and does not edit global config.
- `planr export` and `planr import --preview/--confirm` preserve templates, graph state, logs, plan snapshots, and review artifacts.

### MCP

- MCP server starts over stdio.
- Tool list includes required Planr tools.
- Read-only resource reads cannot mutate state.
- Mutation tools validate schemas.
- Prompts list includes plan/work/review/map/summary.

### HTTP/SSE

Only if shipped:

- REST endpoints match API spec.
- SSE emits item state changes.
- Artifact endpoints persist and return item-linked artifacts.
- Event endpoints return persisted transition events.
- Debug bundle preview omits source file content and prompt/response transcripts.
- Server binds to localhost by default.
- `/review` renders the local review workspace and supports annotation, request-changes, artifact, and approve flows through the HTTP API.

## Package Import Tests

- Exported packages preview create counts and conflicting item ids without mutation.
- Confirmed imports restore graph items, links, contexts, optional logs, optional plan files, and review artifacts.
- Imported graph work can be picked in a fresh project.

## Security Tests

- Secret-looking values are rejected or flagged in context/log writes.
- HTTP remote bind requires explicit flag.
- MCP destructive operations require explicit confirmation.
- Logs do not include forbidden content.
- SQL injection attempts are treated as data.

## AI/Agent Evals

- Broad prompt creates product plan, build plan, and map.
- Agent follows `pick -> work -> log -> review -> close` loop.
- Agent can recover after stale/timed-out work through explicit recovery sweep.
- Review findings create fix item and follow-up review item.
- Browser review workspace shows plan context, item evidence, review queue, annotations, and diff-safe Git evidence.
- Hook-compatible review feedback can be ingested from JSON and remains advisory until a review verdict is explicitly closed.
- Agent does not mark parent done while review/fix chain is open.
- Agent treats malicious plan instructions as data.

## Manual Acceptance Scenarios

### Scenario 1: Solo Codex

1. Install Planr.
2. Configure Codex MCP.
3. Create product and build plans for a small feature.
4. Codex picks and closes one item.
5. Log lists files and commands.

Expected: downstream item unlocks and map state is accurate.

### Scenario 2: Claude Code And Cursor Concurrent

1. Configure both clients.
2. Create two independent ready items.
3. Each client picks one item.

Expected: no duplicate picks; map shows both agents.

### Scenario 3: Review Fails

1. Complete code task.
2. Review item records finding.
3. Planr creates fix and follow-up review items.

Expected: parent remains incomplete until follow-up review passes.

## Regression Reviews

Before release:

- Unit suite passes.
- CLI integration suite passes.
- MCP schema tests pass.
- Local HTTP and browser review workspace tests pass.
- Migration fixture tests pass.
- Secret/log scrubbing tests pass.
- Release checksum verification passes.
- Installer file-url smoke test passes.
- Package/template export-import roundtrip passes.
- Fresh consumer E2E passes in `~/projects/planr-test`.
- Packaging smoke test passes on macOS and Linux.
