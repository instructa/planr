# Changelog

All notable changes to Planr are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/instructa/planr/compare/v1.1.9...HEAD
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
