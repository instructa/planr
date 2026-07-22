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
planr plan audit <plan-id>
planr plan show <plan-id>
planr plan archive <plan-id>
planr map show [--plan <plan-id>] [--view tree|diagram] [--full]
planr map watch [--plan <plan-id>] [--view tree|diagram] [--full] [--interval-ms 1000] [--no-clear] [--until-settled]
planr map build --from <plan-id>
planr map lane --critical
planr map pressure
planr map status
planr map preview --close <item-id>
planr map unlocks <item-id>
planr map lookahead [--from <item-id>] [--limit 10]
planr item create "title" --description "..." [--after <item-id>] [--timeout-seconds N] [--max-retries N] [--retry-backoff fixed|exponential] [--retry-delay-ms N] [--pre "..."] [--post "..."]
planr item breakdown <item-id> --into "A" --into "B" [--into "C"]
planr item insert "title" --description "..." --after <item-id> [--before <item-id>] [--preview|--confirm]
planr item amend <item-id> --note "..." [--tag amendment]
planr item replan <parent-id> --into "A, B, C" [--preview|--confirm]
planr item update <item-id> [--title "..."] [--description "..."] [--work-type <type>]
# plan task lists can declare work types inline: `### TASK-001 (backend): ...` / `- [ ] (frontend) ...` seed map build directly
planr item route <item-id> [--set <profile>|--clear]
planr link add <from-item> <to-item> --type blocks
planr prime [--hook-json]
planr pick [--work-type <type>] [--plan <plan-id>] [--peek]
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
# --kind is free-form vocabulary; common values: evidence, screenshot, video, recording, report, review
planr artifact show <artifact-id>
planr artifact list [--item <item-id>]
planr event list [--item <item-id>] [--limit 50]
planr debug bundle [--item <item-id>] --preview
planr trace item <item-id>
planr log add --item <item-id> --summary "..." [--files a --files b | --files a,b] [--cmd "..."] [--kind completion|progress|verification] [--profile <id>] [--route-audit <observation.json>]
# machine consumers: --cmd writes the `commands` field in log JSON; --tests writes `tests`
planr review request <item-id>
planr review annotate <item-id> --message "..." [--severity info|warning|blocking] [--file path] [--line N] [--author "..."]
planr review ingest <item-id> (--from feedback.json|--stdin)
planr review artifact <review-item-id> [--out .planr/reviews/custom.review.md]
planr review evidence <item-id> [--pr-url https://...]
planr review close <review-item-id> --verdict complete|not-complete|unclear [--close-target]
planr eval suite-check --input suite.json
planr eval run --input run.json
planr eval show suite|run|comparison|invalidation <id>
planr eval compare <baseline-run-id> <candidate-run-id> [--policy-digest default]
planr eval gate <comparison-id>
planr eval invalidate run|comparison <target-id> --reason "..." [--reason-code code] [--replacement-hint "..."]
planr eval rescore <run-id> [--id <new-run-id>]
planr eval evidence-ref run|comparison <target-id> log|review|artifact <attachment-id> --item <item-id>
planr close [item-id] --summary "..." [--next]
planr done [item-id] --summary "..." [--files a --files b] [--cmd "..."] [--tests "..."] [--review] [--next] [--profile <id>]
planr context add "text" [--item <item-id>] [--tag discovery]
planr context list [--item <item-id>] [--tag <tag>]
planr search "query"
planr agents init [--force]
planr agents init --profile|--skill|--route|--default-route|--interactive
planr agents list [--json]
planr agents check
planr doctor [--client codex|claude|cursor|all]
planr install codex|claude|cursor [--dry-run] [--no-mcp] [--force] [--no-hooks]
planr prompt cli|mcp|http [--client codex|claude|cursor|all]
planr prompt routing [--client codex|claude|cursor|all]
planr mcp
planr serve --port 7526
planr import <file> [--preview] [--confirm]
planr export --out planr.json [--include-plans] [--include-logs] [--template-name "..."] [--tag tag]
```

Global flags: `--db <path>`, `--json`, `--no-color`. They are valid in both positions: `planr --json pick` and `planr pick --json` behave identically. Human map output uses ANSI status colors only when stdout is an interactive terminal. `--no-color`, `NO_COLOR` (even when empty), `TERM=dumb`, redirected stdout, and JSON disable color; `PLANR_FORCE_COLOR=1` is available for compatible terminal wrappers and tests, but never overrides an explicit opt-out.

## JSON Envelope Convention

With `--json`, responses follow one convention so agents never guess where data lives:

- Errors: `{"error": {"code": "...", "message": "..."}}` with a non-zero exit code. Codes include `not_found`, `invalid_transition`, `already_closed`, `bad_request`, `locked`, `parse_error`, and `internal_error`. `invalid_transition` messages carry the exact repair command for the current state (e.g. which review to close, which approval to resolve, or that blockers must settle first) — the error is the instruction.
- List fields on logs (`files`, `commands`, `tests`, `review_findings`) are always arrays — `[]` when empty, never `null`.
- The affected single item is always available under the top-level `item` key (`pick`, `close`, `item create/update/cancel`, `pick release`, `item breakdown`, approval and runtime commands). Action-specific keys like `closed`, `cancelled`, or `released` carry the id and stay for context.
- Collections use plural keys: `items`, `plans`, `logs`, `reviews`, `artifacts`, `approvals`, `events`.
- Other single objects use their semantic key: `plan`, `log`, `review`, `artifact`, `context`.
- Optional guidance appears under `hint` or `next` when a follow-up command is the expected move.

`plan check` validates path, YAML frontmatter, and that required sections have content: build plans need `## Scope Decision`, `## Verification`, and `## Acceptance Criteria` filled; product plans need `## Problem`, `## Requirements`, and `## Success Criteria` filled in `PRODUCT_SPEC.md`. It also flags a task list that still contains only the scaffold placeholder (or no work specs at all) — `map build` would turn that into a single coarse item, so the fix names the granularity contract: one `### TASK-00n:` heading (or `- [ ]` line) per verifiable slice, typically 4-8, in execution order. Each warning is structured — `{"file", "section", "message", "fix"}` — and names the exact file to edit plus the re-run command, so a failed check is a repair instruction, not a riddle.

`plan audit <plan-id>` is the one-call contract verdict for a plan's map scope. JSON shape for machine consumers: top-level `holds` (bool) and `clauses[]` with `clause` (name) and `pass` (bool) — e.g. `planr plan audit <id> --json | jq '.holds, (.clauses[] | {clause, pass})'`. It evaluates four clauses with evidence: `items_settled` (open items listed), `reviews_complete` (open review items listed), `approvals_clear` (requested/denied approvals listed), and `verification_logged` (logs with `--kind verification` on scope items). The stored goal contract (`planr context --tag goal-contract` mentioning the plan id) is included; the verification clause is binding only when such a contract exists. `holds: true` means the contract is satisfied — loop agents use this as their stop condition instead of stitching the verdict together from `map status`, `log list`, and `approval list`. When the contract is open, `next` carries the exact next command derived from the first actionable gap: build the map, pick the ready review or work item (plan-scoped), resolve the blocking approval, inspect stalled leases, or log the missing verification. Also available as MCP `planr_plan_audit`.

`map show` defaults to the compact `tree` view used by agents and existing terminal workflows; coding agents should prefer `map show --json` when they need structured state. The tree keeps the canonical edge vocabulary visible: active dependencies render as red `blocks`, while an edge satisfied by a `closed` or `closed_partial` upstream renders as dim `blocks✓`. `map show --view diagram` is exclusively a human-supervision view and coding agents must not invoke it. It provides fixed-width boxes with at most two content lines in `ICON ITEM-ID → TITLE` form, translating satisfied `blocks` edges to neutral `then` while active or cancelled blockers remain `blocks`; `hands_to`, joins, disconnected components, and cycle warnings remain explicit. Humans can add `--full` for verbose nodes with status words, complete wrapped titles, active workers, critical-lane markers, and downstream pressure. `--full` is rejected with tree view. This is a read-only presentation over the same canonical map projection; JSON/MCP/HTTP always retain the stored `kind: "blocks"` and are identical across diagram detail choices.

`map show --plan <plan-id>` narrows either human view to one plan's items and the links among them, with plan-scoped counts — plan-scoped goal runs on shared boards audit their slice, not the whole board. An unknown plan id is an error, never a silent unscoped view. Also available on MCP `planr_map_show` (`plan`) and HTTP `GET /v1/projects/{id}/map?plan=<plan-id>`.

`map watch` is the read-only live companion for a second human terminal. Coding agents must not invoke it; they should use `map show --json` snapshots or `planr serve` plus `/v1/events/stream`. Watch defaults to the condensed `--view diagram`, polls every 1000 ms, compares the canonical scoped projection, and clears/redraws an interactive terminal only when graph state changes. `--full`, `--view tree`, `--plan <plan-id>`, `--interval-ms <n>` (minimum 100), `--no-clear`, and `--until-settled` control presentation and lifetime; Ctrl-C uses normal process interruption. JSON mode remains rejected because watch is intentionally human-only.

`context list --tag <tag>` filters notes by the tag they were stored with (`context add --tag`), so e.g. the goal contract is recoverable with `planr context list --tag goal-contract` instead of scanning all notes.

`map build` chains the created items in plan order with `blocks` links — build plan steps are ordered, so the map inherits that order instead of leaving everything flat. The output lists every created item with its status, the created links, and the next command; adjust order with `planr link add` before picking if execution order differs from document order.

`item breakdown` follows the same contract: each `--into` flag carries one child title (a single value may also pack newline- or comma-separated titles), children are chained with `blocks` links in the given order, and the parent parks as a blocked gate until they settle. The output lists every child with id and status, the chain links, and the next command — across CLI and MCP `planr_item_breakdown`.

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

`review request` (and `done --review`) moves a picked or running target to `in_review`: work is finished, evidence is logged, the item waits on its gate. The owner keeps the pick and can still log evidence; `in_review` items are never handed out by `pick`. Reviews are gates, so pre-attaching one to a pending or blocked item is legal (the target keeps its status and the gate holds the close later); requesting a review on a settled item fails with `invalid_transition`. `done` on a ready item that was never picked adopts it first — the lease is written retroactively under the current worker, so the completion always carries a maker identity and the `in_review` transition is never skipped. The `done --review` response names the target's resulting status and a plan-scoped reviewer pick command (`next: planr pick --plan <id> --work-type review --json`).

`review close` writes `.planr/reviews/<review-item-id>.review.md` and registers it as a review artifact. A `not-complete` or `unclear` verdict creates fix and follow-up review work; the follow-up review gates the same target item, so the chain keeps working with `--close-target`. With `--close-target` (complete verdicts only) the reviewed item is closed in the same command, provided it already has a completion log; the artifact is rendered after the target transition, so it snapshots the final target status. `--close-target` is also available through MCP `planr_review_close` and HTTP `POST /v1/reviews/{id}/close` (`"close_target": true`). `review close` responses include the same `remaining` progress snapshot as `done` and `close`. `--reviewer <id>` records the checker's identity on the review log, artifact, and event (defaults to the worker id), keeping maker and checker distinguishable in the audit trail. Closing an already-settled review fails with error code `already_closed` instead of silently duplicating evidence logs. The maker/checker split is derived, not declared: `review_mode` compares the closing reviewer identity against the target item's lease holder and reports `single_agent`, `independent`, or `unattributed` in the response, review log, artifact, and event — no ceremony note required. An `unattributed` close explains itself in the output: it means the target has no recorded lease (work was never picked or its lease was released), not that the reviewer's identity was missing.

`eval` exposes the same stored evaluation services to CLI and MCP. Every eval JSON response is one document with the frozen envelope `ok`, `command`, `object`, `warnings`, `reasons`, and `error`; MCP tools return the same envelope inside the text content, including `isError` tool results. `suite-check` reports command `eval.suite.check`, stores or verifies an immutable suite snapshot, `run` starts a run from a JSON envelope and can record cases/samples plus a terminal status, `compare` persists a verdict from two stored runs, and `gate` exits non-zero for `regressed` or `insufficient_evidence` while still printing the verdict, blocker, effects, uncertainty, coverage/freshness gates, and deeper-read commands. Eval exit codes are fixed: `0` for accepted command outcomes, `1` for regression or promotion-gate failure, `2` for insufficient evidence, `3` for invalid or unsafe input, and `4` for infrastructure errors such as missing stored runs. Error objects include `code`, `message`, `reasons`, and `field`. `invalidate` records invalidation evidence for a run or comparison, and `rescore` starts a new run linked back to the source run. `evidence-ref` attaches a run or comparison to an existing log, review item, or artifact for audit/package portability; it is never closure authority and never changes map item status. MCP mirrors these operations as `planr_eval_suite_check`, `planr_eval_run`, `planr_eval_show`, `planr_eval_compare`, `planr_eval_gate`, `planr_eval_invalidate`, `planr_eval_rescore`, and `planr_eval_evidence_ref`. A repository-owned smoke suite lives at `examples/eval/planr-lifecycle-smoke.suite.json`; recalculate fixture digests before using it outside repo-local examples.

`trace item` on a review item inlines the target item and its evidence logs under `target`, so a reviewer's first trace already contains what is being audited. The human (non-JSON) mode renders the packet: status, owner, links, logs. When the item has a declared route, a run profile, or a route observation, the trace gains a `routing` section — the declared route next to every run's actual client/profile and, when supplied, its independent requested → resolved → effective model/effort/context-fork stages, transition reason, policy/binding provenance, and per-dimension metering confidence. Unknown effective host values remain `null`/`unavailable`; requested values are never copied forward as proof. MCP `planr_trace_item` returns the identical JSON shape.

`doctor` also reports the agent registry in all states without ever failing: absent (informational hint), degraded (parse error with line context), or loaded with profile/route counts, validation warnings, and per-artifact drift — a rendered role file whose content no longer matches what the current registry would render is flagged `drifted` with a `planr install <client> --force` hint, while files without the generated-from header are the user's (`manual`) and never flagged.

`done` is the compound worker command: it writes a completion log, then requests a review (`--review`) or closes the item, and optionally picks the next ready item (`--next`). Without `--next`, the response's `next` field carries the exact follow-up command instead (`planr pick --work-type review --json` after a review request, `planr pick --json` after a close), so every settlement output ends in an action. It chains the same single-owner operations as `log add`, `review request`, `close`, and `pick` — identical evidence, fewer commands. `done`, `close`, and `review close` report what the settlement `unlocked` (id, title, work type of every item that became ready), `done` and `close` echo the item's `post_condition` at completion time, and a `hint` asks for `--cmd`/`--tests` evidence when downstream items depend on an item that closed without any. `done`, `close`, `review close`, and the pick packet include a `remaining` progress snapshot so loop agents can evaluate their stop condition without an extra `map status` call. `remaining.counts` always carries the full status vocabulary (`pending`, `ready`, `picked`, `running`, `in_review`, `blocked`, `failed`, `cancelled`, `closed`, `closed_partial`) with explicit zeros — a missing count never has to be inferred.

`prime` prints the compact state block host hooks inject at session start and after context compaction (project, map counts, held items with log status, goal contract, registry presence, next command); `--hook-json` emits the Claude Code SessionStart envelope. In a repo without a Planr database it exits silently and creates nothing — hooks must never break a session. Installs wire it into the host's hook system by default (`--no-hooks` skips; existing hook files are merged additively, never overwritten). Full guide: [Host Hooks](HOOKS.md).

`pick --json` returns one flat work packet in which every fact appears exactly once: `item`, `links`, `logs`, `runtime`, `recovery`, `conditions`, `approval`, recall context (`contexts`, `relevant_contexts`, `upstream_handoffs`, `review_history`, `source_links`, `possible_file_conflicts`), `close_effect`, `privacy`, `deeper_reads`, and `remaining`. Worker identity lives in `item.worker_id` and `runtime.worker_id`. Empty collections and inactive gates are omitted — a missing key means "empty". No separate `trace item` call is needed. Evidence written via `log add` or `done` by the pick owner refreshes the runtime heartbeat automatically. The same packet shape is returned by MCP `planr_pick_item`, HTTP `POST /v1/pick`, and `done --next`. `pick --peek` (MCP: `"peek": true`) returns the same packet for the next pickable item *without* leasing it — no lease, heartbeat, or pick event is written, and the packet carries `"peek": true` with the item still `ready`. Built for dispatching drivers: read the routing block, dispatch the worker, and let the worker take the lease under its own identity — replacing the pick → `pick release --force` → re-pick sequence.

`pick --work-type <type>` restricts the lease to one work type, so checker agents pick only `review` items and makers only work items. `pick --plan <plan-id>` restricts the lease to one plan's items, so plan-scoped goal runs never pick work outside their contract even when other plans share the board; an unknown plan id is an error, never a silent unscoped pick. Both filters are available on MCP `planr_pick_item` and HTTP `POST /v1/pick` (`work_type`, `plan`). A null pick is never blind: `{"item": null}` carries a `reason` (`empty_map`, `all_settled`, `nothing_ready`, `ready_items_excluded_by_filter`) and the `remaining` snapshot. When ready work exists but the active filters rejected all of it, `excluded` lists each ready item with the cause (`work_type` mismatch, outside the `--plan` scope, or just requested by this worker) and `repair` carries the exact pick commands that would lease that work — across CLI, MCP, and HTTP. On a review item, `close_effect` previews the full `--close-target` cascade: it lists the work that closing the review (and with it the reviewed item) would unlock.

`policy show` and `policy check` inspect the provider-neutral Usage Policy v1 file (`.planr/policy.toml`). A missing policy is an explicit successful state that preserves existing advisory routing; a malformed or unsafe policy fails `check` with parser/field diagnostics and leaves enforcement unavailable. Usage Policy v1 fixes delegation depth at one and keeps retry, availability fallback, quality escalation, quota downgrade, and safety stop as distinct transition contracts. Its separate `execution` section declares bounded per-role filesystem, network, tool/MCP, structured command, environment, hook, secret-reference, and approval grants. `policy admit <request.json>` requires the picked item's current `item_id` and `pick_token`, authorizes the current worker as lease owner, evaluates a bounded task contract before delegation, previews any permission addition, records the decision and lease generation, rejects unclassified commands and destructive or out-of-scope work, permits overlapping readers, and admits concurrent writers only for disjoint scopes isolated in distinct worktrees. Released or recovered leases never inherit historical admitted scopes. MCP `planr_policy_show`, `planr_policy_check`, and `planr_policy_admit`, plus HTTP `POST /v1/policy/admit`, call the same admission service and return the same decision shape.

`agents init` writes a provider-neutral `.planr/agents.toml` scaffold or compiles explicit profile and route flags. `agents list` and `agents check` inspect it, while pick packets carry the resolved opaque profile, host, model, effort, tier, skill, route chain, and matched selector. Full guide: [Model Routing](MODEL_ROUTING.md).

`log add` and `done` accept `--profile <id>` — the registry profile the run actually executed on (`PLANR_PROFILE` env is the fallback, so rendered role files can export it). They also accept `--route-audit <observation.json>` for the strict three-stage observation contract. A route audit records each stage's model, effort, and context-fork value with enforcement state (`verified`, `requested_only`, `estimated`, or `unavailable`), an evidence-source enum, typed transition plus reason, policy/binding ids and versions, and wall-time/tool/token/credit values with independent confidence. It is stored in run metadata, projected into its durable log and trace, and emits local route-stage/transition events. The profile remains backward compatible: when it differs from the item's declared route, Planr emits advisory `route_mismatch_observed`. Runs without commands/tests record neither a run nor a route observation. MCP `planr_log_add` and HTTP item log accept the same optional `route_observation` object.

`artifact add` infers the mime type from the file extension when `--path` is given without `--mime` (PNG screenshots land as `image/png`, not `text/plain`); inline `--content` defaults to `text/plain`. The same inference applies on MCP `planr_artifact_add` and HTTP `POST /v1/artifacts`.

`review evidence` reports Git worktree status scoped to files named by item logs or artifacts. Dirty files without item provenance are listed as unrelated and are not treated as agent-owned evidence. `--pr-url` records an item-scoped PR reference before returning the evidence package.

`recover sweep` previews by default. With `--apply`, timed-out picked work that has a retry budget (`max_retries > 0`) is marked `failed` with an `item_timed_out` event; stale work and timeouts without a retry budget are released back to `ready`. Failed work re-enters `ready` once its retry delay has elapsed (`retry_delay_ms`, doubled per retry under `exponential` backoff) until the budget is exhausted. Every transition records a recovery event. Item pre/post conditions are visible in pick context, trace output, and close previews; post conditions are reported as manual verification gates instead of being guessed automatically.

`serve` exposes the local review workspace at `/review` and its JSON projection at `/v1/review-workspace`.

`prompt` prints ready-to-use agent instructions without editing global config. Use `prompt cli` for shell agents, `prompt mcp` for MCP setup text, and `prompt http` for localhost automation/review workspace usage. `prompt routing` emits a paste-ready model-prioritization block for the driver session: the route table from `.planr/agents.toml` (every route, profile, and fallback), native Codex `agent_type` plus explicit fork guidance referring to the generated repository role TOMLs, the Claude `CLAUDE_CODE_SUBAGENT_MODEL` env preemption, Cursor's silent plan/policy/Max-Mode overrides, and process-dispatch snippets (`pi`, `opencode run`) only for hosts without role files. `--json` carries the same content structured, and a missing or unreadable registry still prints the host guidance with a pointer instead of failing.

`export` writes a reusable Planr JSON package with package requirements metadata, graph state, contexts, optional logs, optional plan file snapshots, review artifact snapshots, and — when present — the agent registry (`.planr/agents.toml`) as a raw snapshot. `import` previews JSON packages by default and mutates only with `--confirm`; the preview names the registry with its exact action (`create`, `identical`, or `conflict`), and an existing registry that differs is never overwritten — the conflict is reported with a hint to remove the local file first. Packages without a registry import unchanged.
