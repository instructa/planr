# Changelog

All notable changes to Planr are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.1.19] - 2026-06-11

The symmetry pack, from the fifth Codex dogfood run: every flag an agent reasonably infers from an existing write-side or scope-side flag now exists on the read side.

### Added

- `context list --tag <tag>`: notes are recoverable by the tag they were stored with — `planr context list --tag goal-contract` fetches the goal contract directly instead of scanning all notes. Closes the write/read asymmetry with `context add --tag`.
- `map show --plan <plan-id>`: the map narrowed to one plan's items, the links among them, and plan-scoped counts — plan-scoped goal runs on shared boards see their contract's slice. Unknown plan ids error instead of silently showing the whole board (same rule as `pick --plan`). Also on MCP `planr_map_show` (`plan`) and HTTP `GET .../map?plan=`.
- `plan audit` with `holds: false` now carries `next`: the exact command for the first actionable gap — build the map, pick the ready review or work item (plan-scoped), resolve the blocking approval, inspect stalled leases, or log the missing verification. The last output that ended in a clause list instead of an action.

### Changed

- Skills and docs recover the goal contract with `planr context list --tag goal-contract` (planr-goal, planr-loop, GOALS).
- Provisioned agent roles pin cost tiers: the worker and reviewer role files set a cheaper model/effort tier than the loop driver, since the pick packet bounds their scope. Documented in GOALS "Cost Tiering" (Codex TOMLs, Claude Code subagents).

## [1.1.18] - 2026-06-11

Kills the last structural guess from the fourth Codex dogfood run: map granularity is now a checked contract, not something the agent discovers after `map build`.

### Fixed

- `artifact add` no longer stamps every path artifact as `text/plain`: without `--mime`, the type is inferred from the file extension (png, jpg, svg, pdf, json, md, html, mp4, …), so screenshots and recordings carry honest mime types in the audit trail — across CLI, MCP `planr_artifact_add`, and HTTP `POST /v1/artifacts`. Inline `--content` still defaults to `text/plain`.

### Added

- `plan check` flags an unexpanded scaffold task list: when the plan still carries only the placeholder task (or none at all), the structured warning names the file and the granularity contract — one `### TASK-00n:` heading (or `- [ ]` line) per verifiable slice, typically 4-8, in execution order. The coarse-map guess dies before `map build` instead of after it.
- `map build`'s single-coarse-item hint now states the repair options with granularity guidance (expand the task list and rebuild, or break down per slice derived from the acceptance criteria) instead of a bare breakdown pointer.

### Changed

- Skills: goal prep expands the plan's task list before `map build` — one verifiable slice per task, derived from the acceptance criteria (planr-goal).
- Skills: workers put the decisive output line into the `done` summary ("12 tests passed"), because reviewers see recorded command strings, not the maker's terminal (planr-work).
- Skills: in single-agent hosts the review bar rises — gates only on the riskiest slices (core implementation, final live verification), the rest closes with plain `done` (planr-loop).

## [1.1.17] - 2026-06-11

Attribution can never fall through a crack: fixes from the third manual Codex dogfood run, where `done --review` on a never-picked item left the target `ready` and the review `unattributed`.

### Fixed

- `done` on a ready item that was never picked now adopts it: the lease (worker id, pick token, timestamps) is written retroactively before logging, so the completion always records a maker, the `in_review` transition is never skipped silently, and `review_mode` can no longer degrade to `unattributed` through this path. Inspired by plandb's lenient-complete-with-backfill, extended to carry identity, not just timestamps.
- `review request` on a settled item (`closed`, `closed_partial`, `cancelled`, `failed`) fails with `invalid_transition` and a follow-up suggestion instead of creating a gate on finished work. Pre-attaching a review gate to pending/blocked work stays legal.

### Added

- `done --review` output names the target's resulting status ("… is in_review") and the `next` command is plan-scoped when the item belongs to a plan (`planr pick --plan <id> --work-type review --json`), so the reviewer command is copy-pastable without resolving the plan id.
- `review close` explains an `unattributed` mode inline: the target carried no recorded lease — instead of stamping the word without context.

### Changed

- Skills: the goal contract's "all reviews closed" clause audits review items that exist — plain-`done` items satisfy the contract without a review gate, so skipping low-signal reviews never blocks `plan audit` (planr-goal, planr-loop).
- Skills: single-quote `--files` values containing `$` (route files like `watch.$videoId.tsx`) so the shell does not expand them (planr-work).

## [1.1.16] - 2026-06-11

Filter-aware picks and a breakdown contract, from the second manual Codex dogfood run (the guess-killer validation run).

### Added

- `item breakdown` has an explicit title contract: repeat `--into` once per child, or pass one value with newline- or comma-separated titles — both parse identically (CLI and MCP `planr_item_breakdown`). The output now lists every created child with id and status, the `blocks` chain links, the parked parent, and the next command, instead of a bare count. MCP breakdown now chains children and parks the parent exactly like the CLI (it previously created flat, unchained children).
- A null pick caused by filters explains itself: when ready work exists but `--work-type`, `--plan`, or the own-review exclusion rejected all of it, `reason` is `ready_items_excluded_by_filter` and the response carries `excluded` (each ready item with its mismatch cause) and `repair` (the exact pick commands that would lease that work) — across CLI, MCP, and HTTP. Replaces the contradictory `no_ready_item_in_plan`/`no_ready_item_of_work_type` answers that reported `ready: 1` alongside "no item".
- `done` without `--next` sets `next` to the exact follow-up command (`planr pick --work-type review --json` after a review request, `planr pick --json` after a close), so every settlement output ends in an action.

### Changed

- Skills: one agent instance keeps one `PLANR_WORKER_ID` — never export a second identity inside the same instance to make a review look `independent`; an honest `single_agent` stamp beats a fake `independent` one (planr-review, planr-loop).
- Skills: request reviews where they carry signal — implementation slices and user-facing work finish with `done --review`; trivial inspection/baseline items close with plain `done`, evidence still required (planr-loop).
- Install docs list npm (`npm install -g planr`) as a package-manager path alongside Homebrew, and the Homebrew section no longer reads as pre-publication.

## [1.1.15] - 2026-06-11

### Fixed

- npm publish failed sigstore provenance validation because `package.json` had no `repository` field; npm requires `repository.url` to match the provenance source repository.

## [1.1.14] - 2026-06-11

Release engineering: deterministic version bumps, CI secret scanning, and npm as a real install channel.

### Added

- `scripts/release.sh <x.y.z> "summary"`: the only supported release path. Syncs the version into `Cargo.toml`, `package.json`, and both plugin manifests, requires a committed changelog section, runs `cargo test`, `npm pack --dry-run`, and the local leak gate, then commits, tags, and pushes in one step.
- Release workflow tag gate now verifies `package.json`, both plugin manifests, and the `CHANGELOG.md` section against the tag, not just `Cargo.toml`.
- CI secret scanning in `security.yml`: TruffleHog (verified results, full history) and Trivy (secret + misconfig), both pinned by commit SHA.
- npm is a real install channel: the release workflow's `npm-publish` job bundles all four platform binaries (checksum-verified against the release `SHA256SUMS`) into `npm/native/` and publishes via npm Trusted Publishing (OIDC, no token secret). Gated on the `NPM_PUBLISH_ENABLED` repository variable. The wrapper resolves `PLANR_NATIVE_BIN`, then the bundled platform binary, then local cargo builds; no postinstall, no install-time downloads.

### Changed

- `packageManager` pinned to pnpm 11 (current stable, integrity-pinned). No `devEngines` block: npm enforces it and would refuse the `npm pack` release gate.

## [1.1.13] - 2026-06-10

Guess-killer pack from the first fully manual Codex dogfood run (YT clone): every place the agent had to guess now answers itself.

### Added

- `planr plan audit <plan-id>` (CLI, MCP `planr_plan_audit`): one-call contract verdict over a plan's map scope. Evaluates `items_settled`, `reviews_complete`, `approvals_clear`, and `verification_logged` clause by clause with evidence, includes the stored goal contract, and answers `holds: true/false`. Replaces the hand-assembled final audit in goal loops.
- `done`, `close`, and `review close` report what the settlement `unlocked` — every item that became ready, with id, title, and work type — in JSON and human output (also on MCP `planr_close_item` and HTTP `POST /v1/items/{id}/close`).
- `done`/`close` echo the item's `post_condition` at completion time and emit a `hint` when downstream items depend on an item that settled without `--cmd`/`--tests` evidence.
- `review_mode` is derived automatically on `review close`: the closing reviewer identity is compared against the target's lease holder and recorded as `single_agent`, `independent`, or `unattributed` on the close response, review log, artifact, and event. The maker/checker ceremony note is gone.
- `log add --kind verification` is the canonical shape for live-verify evidence; `plan audit` checks for it when a goal contract exists.

### Changed

- `map build` chains created items in plan order with `blocks` links and lists every created item, link, and the next command — no more flat unordered maps and no post-build `map show` round-trip.
- `plan check` warnings are structured (`{"file", "section", "message", "fix"}`); the human output names the exact file to edit and the re-run command.
- `invalid_transition` errors carry the exact repair command for the current state: which review to close, which approval to resolve, that blockers must settle first, or that a settled item needs a follow-up instead.
- Skills: `planr-loop`/`planr-status` use `plan audit` as the stop condition, `planr-goal` teaches direct plan-file repair, `planr-work` teaches verification logs and transient-failure hygiene, `planr-verify-web` adds the system-Chrome-over-CDP fallback tier, `planr-review` drops the single-agent ceremony note.

## [1.1.12] - 2026-06-10

Plan-scoped picks, from the first live `/goal` run with Codex.

### Added

- `pick --plan <plan-id>` (CLI, MCP `planr_pick_item`, HTTP `POST /v1/pick`) restricts the lease to one plan's items, so plan-scoped goal runs never pick work outside their contract when several plans share the board. An unknown plan id is an error, never a silent unscoped pick. A null pick in plan scope reports `reason: "no_ready_item_in_plan"`.

### Changed

- All pick surfaces lease through one query contract (`PickFilter`: exclude, work type, plan scope) owned by the new `src/app/lease.rs` module; the `next_pick_value`/`next_pick_value_excluding` wrapper functions are gone.

## [1.1.11] - 2026-06-10

Cosmetic batch from the v1.1.10 dogfood run.

### Added

- `PLANR_WORKER_ID` environment override: agents export an explicit identity (e.g. `maker-1`, `checker-1`) once per session and every pick, log, heartbeat, and ownership check attributes to it instead of `client:host:user`. Takes precedence over `PLANR_SESSION_ID`.
- `close_target` is available through MCP `planr_review_close` and HTTP `POST /v1/reviews/{id}/close` — full parity with `review close --close-target`.

### Fixed

- JSON errors carry the specific machine-readable code: closing a settled review reports `{"error": {"code": "already_closed"}}` instead of `internal_error`.
- The review artifact written by `review close --close-target` snapshots the target after its transition, so the evidence shows the final status (`closed`) instead of the stale `in_review`.
- Item ids no longer contain `--` when the 32-char slug truncation lands on a hyphen.
- `plan split` no longer duplicates the source title in the build plan title, slug, and filename when the slice already repeats it.

### Changed

- Log list fields (`files`, `commands`, `tests`, `review_findings`) always serialize as `[]` instead of `null` — one stable shape across `log list`, `log show`, the pick packet, and traces.
- `deeper_reads` hints in the pick packet consistently include `--json`.

## [1.1.10] - 2026-06-10

Fix pack from the v1.1.9 dogfood run.

### Added

- `review close --reviewer <id>` records the checker's identity on the review log summary, the review artifact (`Reviewer:` line and metadata), and the `review_closed` event; defaults to the worker id. Maker and checker stay distinguishable in the audit trail.
- `pick --work-type <type>` (CLI, MCP `planr_pick_item`, HTTP `POST /v1/pick`) restricts the lease to one work type, so checker agents pick only `review` items and makers only work items.
- A null pick is never blind: `{"item": null}` now carries a `reason` (`empty_map`, `all_settled`, `nothing_ready`, `no_ready_item_of_work_type`, `ready_items_not_pickable`) and the `remaining` snapshot — across CLI, MCP, and HTTP.

### Fixed

- `review close` on an already-settled review now fails with `already_closed` instead of exiting 0 and silently duplicating review logs, the target's auto-completion log, and the artifact — duplicates polluted handoff evidence for downstream items.
- `close_effect` on a review item now previews the `--close-target` cascade: it lists the work that closing the review (and with it the reviewed item) would unlock, instead of claiming nothing unlocks right before the close promotes the next item.

### Changed

- `map show` and `map status` report the same explicit-zero status counts as the `remaining` snapshot (full 10-status vocabulary), plus `settled` and `total` — one counts shape across all surfaces.
- The pick packet no longer carries a third top-level `worker_id` copy; worker identity lives in `item.worker_id` and `runtime.worker_id`.
- Handoff and recall summaries truncate at a word boundary with a `[truncated]` marker instead of cutting tokens in half.

## [1.1.9] - 2026-06-10

Polish pack from the v1.1.8 dogfood run.

### Changed

- The pick output is now one flat work packet: the nested `context` and `trace` envelopes are gone, every fact (item, links, logs, runtime, recovery, conditions, recall context, `close_effect`, `privacy`, `deeper_reads`) appears exactly once, and empty collections or inactive gates are omitted — a missing key means "empty". The same shape is returned by `planr pick`, `done --next`, MCP `planr_pick_item`, and HTTP `POST /v1/pick`.
- `remaining.counts` always carries the full status vocabulary (`pending`, `ready`, `picked`, `running`, `in_review`, `blocked`, `failed`, `cancelled`, `closed`, `closed_partial`) with explicit zeros, so consumers never infer missing statuses.
- The pick packet includes the `remaining` board-progress snapshot, matching `done`, `close`, and `review close`.
- Docs clarify that global flags (`--json`, `--db`, `--no-color`) are valid before and after the subcommand.

## [1.1.8] - 2026-06-10

Friction findings from the v1.1.7 comparison dogfood run.

### Added

- `in_review` status: `done --review` / `review request` moves a picked or running item to `in_review` (ownership kept), so "work finished, waiting on the gate" is visible instead of masquerading as `running`. `in_review` items accept owner evidence and heartbeats, are excluded from new picks and stale sweeps, and `map status` reports them in their own bucket.
- `trace item` on a review item inlines the target item and its completion logs under `target` — a reviewer's first trace already contains what is being audited.
- `trace item` human mode renders the packet (status, owner, links, logs) instead of printing only "trace complete".
- `review close` responses include the `remaining` board-progress snapshot, like `done` and `close`.

### Changed

- Follow-up reviews created by a `not-complete`/`unclear` verdict now gate the same target item (`reviews` link), so `review close --close-target` keeps working across the fix chain and the target stays `in_review` until the chain settles.
- Skills teach `--tests` for test evidence (test runs in `--tests`, build/serve commands in `--cmd`).

## [1.1.7] - 2026-06-10

### Added

- Long-running goal workflow: new `planr-goal` prep skill compiles a broad goal into a checked plan, a linked map, and a durable goal contract (`planr context --tag goal-contract`), then prints the starter command for the host's loop driver (Codex/Claude Code `/goal`, automations, or manual re-dispatch). Documented end-to-end in `docs/GOALS.md`.
- `done` and `close` responses report board progress: a `remaining` snapshot (`counts`, `settled`, `total`) in JSON and `[1/2 settled · 0 ready]` in text — loop agents evaluate their stop condition without an extra `map status` call.

### Changed

- `planr-loop` is now framed as the iteration protocol under an external orchestrator (`/goal`, automation, or human re-dispatch); the loop contract is stored in Planr context and re-read each iteration instead of relying on chat memory.
- `planr-status` gained a goal-contract check: read the stored contract and report `contract holds` / `contract open` with the exact unmet clauses.
- Skills overview and spec command surface teach the short worker path: `pick --json` -> `done --summary ... --review --next` -> `review close --close-target`.

## [1.1.6] - 2026-06-10

Overhead cut: 8 -> 3 commands per item.

### Added

- `planr done` — compound worker command: completion log plus review request (`--review`) or direct close, and `--next` to pick the following item, in one call.
- `review close --verdict complete --close-target` closes the review's target item along with the review (only when a completion log exists).

### Changed

- `pick --json` returns the full trace work packet (links, logs, runtime, recovery, conditions, approval) — no separate `trace item` call needed.
- `log add` and `done` refresh the pick heartbeat automatically — no separate `pick heartbeat` for evidence-producing work.
- `--next` never hands a worker its own freshly created review, preserving maker/checker separation.

## [1.1.5] - 2026-06-10

Friction fixes from the dogfood run.

### Changed

- `log add --files` is repeatable: `--files a --files b` or comma-separated `a,b`.
- `artifact add` accepts the name positionally or via `--name`, with a clear error message.
- Consistent JSON envelope: the affected item is always available under the top-level `item` key.
- `plan check` is strict: empty required sections fail instead of passing green.
- `map build` is idempotent: re-runs create no duplicates, and building from plans with 0/1 items prints a hint.

## [1.1.4] - 2026-06-10

### Added

- Parent gates roll up automatically: closing the last open child settles the parent.

### Changed

- Worker identity is stable across pick, log, and close operations.
- README rewritten to an agent-first narrative.

## [1.1.3] - 2026-06-10

### Added

- `planr project init` and `planr install codex` provision the loop subagent role files (`.codex/agents/*.toml`, `.claude/agents/*.md`) automatically — no manual copying.

## [1.1.2] - 2026-06-10

### Changed

- Plugin payload moved to `plugins/planr/` so Codex can install the plugin from the marketplace manifest.

## [1.1.1] - 2026-06-10

### Added

- Documented plugin install paths across README and the client integration docs.

### Changed

- Established distinct product identity; documented both project journeys (new project from an idea, feature/refactor/fix on an existing project).

## [1.1.0] - 2026-06-10

### Added

- Packaged the repository as an official Codex, Claude Code, and Cursor plugin (skills plus `planr-worker`/`planr-reviewer` subagent roles).
- `planr` router skill: one entry point that dispatches to the right stage skill from live map state.
- `planr-loop` skill: autonomous closing loop — work, live verification, independent review, fix items — until the map is clean or the iteration budget runs out.
- `planr-verify-web` capability skill for live browser verification.

### Fixed

- Plan frontmatter integrity and review-chain readiness issues found while dogfooding.

## [1.0.0] - 2026-06-09

Initial Planr product release.

### Added

- Core product flow: idea -> product plan -> build plan -> map -> pick -> log -> review/evidence -> recovery/package -> close.
- Map graph as the authoritative state for item status, links, picks, reviews, approvals, and completion.
- MCP server (`planr mcp`) with real per-tool JSON Schemas; local HTTP/SSE server (`planr serve`) with correct status codes, CORS, and threaded serving.
- Recovery retry lifecycle: timeouts mark picked work failed, backoff drives retries, stale picks release back to ready.
- `planr scrub --confirm` redacts stored secrets.
- Tag-driven release pipeline with multi-target builds (darwin/linux, arm64/x86_64) and Homebrew tap automation.
- Skill workflow documentation for Codex, Claude Code, Cursor, and MCP-only clients.

[Unreleased]: https://github.com/instructa/planr/compare/v1.1.16...HEAD
[1.1.16]: https://github.com/instructa/planr/compare/v1.1.15...v1.1.16
[1.1.15]: https://github.com/instructa/planr/compare/v1.1.14...v1.1.15
[1.1.14]: https://github.com/instructa/planr/compare/v1.1.13...v1.1.14
[1.1.13]: https://github.com/instructa/planr/compare/v1.1.12...v1.1.13
[1.1.12]: https://github.com/instructa/planr/compare/v1.1.11...v1.1.12
[1.1.11]: https://github.com/instructa/planr/compare/v1.1.10...v1.1.11
[1.1.10]: https://github.com/instructa/planr/compare/v1.1.9...v1.1.10
[1.1.9]: https://github.com/instructa/planr/compare/v1.1.8...v1.1.9
[1.1.8]: https://github.com/instructa/planr/compare/v1.1.7...v1.1.8
[1.1.7]: https://github.com/instructa/planr/compare/v1.1.6...v1.1.7
[1.1.6]: https://github.com/instructa/planr/compare/v1.1.5...v1.1.6
[1.1.5]: https://github.com/instructa/planr/compare/v1.1.4...v1.1.5
[1.1.4]: https://github.com/instructa/planr/compare/v1.1.3...v1.1.4
[1.1.3]: https://github.com/instructa/planr/compare/v1.1.2...v1.1.3
[1.1.2]: https://github.com/instructa/planr/compare/v1.1.1...v1.1.2
[1.1.1]: https://github.com/instructa/planr/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/instructa/planr/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/instructa/planr/releases/tag/v1.0.0
