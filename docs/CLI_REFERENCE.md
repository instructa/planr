# CLI Reference

Generated from `planr --help` shape for V1.

```text
planr project init [--client codex|claude|cursor|all] [--force] [name]
planr project show [--json]
planr project list [--json]
planr plan new "App idea" [--platform web] [--ai] [--backend]
planr plan refine <plan-id> [--note "..."]
planr plan split <plan-id> --slice "MVP backend"
planr plan check <plan-id>
planr plan show <plan-id>
planr plan archive <plan-id>
planr map show
planr map build --from <plan-id>
planr map lane --critical
planr map pressure
planr map status
planr map preview --close <item-id>
planr map unlocks <item-id>
planr map lookahead [--from <item-id>] [--limit 10]
planr item create "title" --description "..." [--after <item-id>] [--timeout-seconds N] [--max-retries N] [--retry-backoff fixed|exponential] [--retry-delay-ms N] [--pre "..."] [--post "..."]
planr item breakdown <item-id> --into "A, B, C"
planr item insert "title" --description "..." --after <item-id> [--before <item-id>] [--preview|--confirm]
planr item amend <item-id> --note "..." [--tag amendment]
planr item replan <parent-id> --into "A, B, C" [--preview|--confirm]
planr link add <from-item> <to-item> --type blocks
planr pick
planr pick release <item-id> [--force]
planr pick heartbeat [item-id]
planr pick progress <item-id> --percent 0..100 [--note "..."]
planr pick pause <item-id> [--note "..."]
planr pick resume <item-id>
planr pick stale [--older-than-seconds 900] [--release]
planr recover sweep [--older-than-seconds 900] [--apply]
planr approval request <item-id> [--reason "..."]
planr approval approve <item-id> --by "name" [--comment "..."]
planr approval deny <item-id> --by "name" [--comment "..."]
planr approval list [--open]
planr artifact add "name"|--name "name" [--item <item-id>] [--kind evidence] [--path file] [--content "..."] [--mime text/plain]
planr artifact show <artifact-id>
planr artifact list [--item <item-id>]
planr event list [--item <item-id>] [--limit 50]
planr debug bundle [--item <item-id>] --preview
planr log add --item <item-id> --summary "..." [--files a --files b | --files a,b] [--cmd "..."]
planr review request <item-id>
planr review annotate <item-id> --message "..." [--severity info|warning|blocking] [--file path] [--line N] [--author "..."]
planr review ingest <item-id> (--from feedback.json|--stdin)
planr review artifact <review-item-id> [--out .planr/reviews/custom.review.md]
planr review evidence <item-id> [--pr-url https://...]
planr review close <review-item-id> --verdict complete|not-complete|unclear [--close-target]
planr close [item-id] --summary "..." [--next]
planr done [item-id] --summary "..." [--files a --files b] [--cmd "..."] [--tests "..."] [--review] [--next]
planr context add "text" [--item <item-id>] [--tag discovery]
planr search "query"
planr doctor [--client codex|claude|cursor|all]
planr install codex|claude|cursor [--dry-run]
planr prompt cli|mcp|http [--client codex|claude|cursor|all]
planr mcp
planr serve --port 7526
planr import <file> [--preview] [--confirm]
planr export --out planr.json [--include-plans] [--include-logs] [--template-name "..."] [--tag tag]
```

Global flags: `--db <path>`, `--json`, `--no-color`.

## JSON Envelope Convention

With `--json`, responses follow one convention so agents never guess where data lives:

- Errors: `{"error": {"code": "...", "message": "..."}}` with a non-zero exit code.
- The affected single item is always available under the top-level `item` key (`pick`, `close`, `item create/update/cancel`, `pick release`, `item breakdown`, approval and runtime commands). Action-specific keys like `closed`, `cancelled`, or `released` carry the id and stay for context.
- Collections use plural keys: `items`, `plans`, `logs`, `reviews`, `artifacts`, `approvals`, `events`.
- Other single objects use their semantic key: `plan`, `log`, `review`, `artifact`, `context`.
- Optional guidance appears under `hint` or `next` when a follow-up command is the expected move.

`plan check` validates path, YAML frontmatter, and that required sections have content: build plans need `## Scope Decision`, `## Verification`, and `## Acceptance Criteria` filled; product plans need `## Problem`, `## Requirements`, and `## Success Criteria` filled in `PRODUCT_SPEC.md`. Empty scaffolds fail with named warnings.

`review ingest` accepts hook-compatible JSON and records it as feedback only. It never closes review work and never approves an item by itself.

```json
{
  "reviewer": "local-reviewer",
  "verdict": "not-complete",
  "findings": ["Add the missing failing-path test"],
  "annotations": [
    {
      "message": "The close path needs regression coverage",
      "severity": "blocking",
      "file": "tests/e2e.rs",
      "line": 42
    }
  ]
}
```

`review request` (and `done --review`) moves a picked or running target to `in_review`: work is finished, evidence is logged, the item waits on its gate. The owner keeps the pick and can still log evidence; `in_review` items are never handed out by `pick`.

`review close` writes `.planr/reviews/<review-item-id>.review.md` and registers it as a review artifact. A `not-complete` or `unclear` verdict creates fix and follow-up review work; the follow-up review gates the same target item, so the chain keeps working with `--close-target`. With `--close-target` (complete verdicts only) the reviewed item is closed in the same command, provided it already has a completion log. `review close` responses include the same `remaining` progress snapshot as `done` and `close`.

`trace item` on a review item inlines the target item and its evidence logs under `target`, so a reviewer's first trace already contains what is being audited. The human (non-JSON) mode renders the packet: status, owner, links, logs.

`done` is the compound worker command: it writes a completion log, then requests a review (`--review`) or closes the item, and optionally picks the next ready item (`--next`). It chains the same single-owner operations as `log add`, `review request`, `close`, and `pick` — identical evidence, fewer commands. `done` and `close` responses include a `remaining` progress snapshot (`counts`, `settled`, `total`) so loop agents can evaluate their stop condition without an extra `map status` call.

`pick --json` returns the full work packet: the item, recall `context`, and a `trace` object (links, logs, runtime, recovery, conditions, approval), so no separate `trace item` call is needed. Evidence written via `log add` or `done` by the pick owner refreshes the runtime heartbeat automatically.

`review evidence` reports Git worktree status scoped to files named by item logs or artifacts. Dirty files without item provenance are listed as unrelated and are not treated as agent-owned evidence. `--pr-url` records an item-scoped PR reference before returning the evidence package.

`recover sweep` previews by default. With `--apply`, timed-out picked work that has a retry budget (`max_retries > 0`) is marked `failed` with an `item_timed_out` event; stale work and timeouts without a retry budget are released back to `ready`. Failed work re-enters `ready` once its retry delay has elapsed (`retry_delay_ms`, doubled per retry under `exponential` backoff) until the budget is exhausted. Every transition records a recovery event. Item pre/post conditions are visible in pick context, trace output, and close previews; post conditions are reported as manual verification gates instead of being guessed automatically.

`serve` exposes the local review workspace at `/review` and its JSON projection at `/v1/review-workspace`.

`prompt` prints ready-to-use agent instructions without editing global config. Use `prompt cli` for shell agents, `prompt mcp` for MCP setup text, and `prompt http` for localhost automation/review workspace usage.

`export` writes a reusable Planr JSON package with package requirements metadata, graph state, contexts, optional logs, optional plan file snapshots, and review artifact snapshots. `import` previews JSON packages by default and mutates only with `--confirm`.
