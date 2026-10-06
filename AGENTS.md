# Agent guidance for planr

planr is a small planning layer for agent-built products: tasks as a Markdown graph in a planning repository, a dependency-free Node CLI, a board (standalone HTML and bb plugin) and one Agent Skill. Building happens elsewhere (for example `split-orchestrator`). planr says what is next; it never builds, reviews or verifies.

Read `docs/contract.md` (data and CLI contract) before changing code, and `bb-plugin/README.md` before changing the plugin.

## Hard limits (crossing these needs a maintainer's explicit OK)

- No database, no locks/leases/heartbeats/atomic pick, no daemon, no MCP or HTTP server. Exception: `planr board --serve`, a loopback-only file server with live reload.
- No per-task gates, evidence system, auto-created fix/review items, or custom benchmark harness.
- No new task statuses. Statuses: open, in-progress, review, blocked, decision, done, dropped.
- Node ≥ 20, no runtime dependencies, no network access.

Stop and ask a maintainer when a change would cross one of these limits, introduces a new persistence mechanism, or makes a dispatcher do more than start threads.

## Size orientation (not a gate)

Rough sizes: engine (`skills/planr/scripts/`) around 800 lines, skill around 60, tests around 150 (graph rules and frontmatter rewriting only, no regression test per bug fix). Format honestly: one statement per line, ≤ 110 characters, never compress code to hit a number. Going over is fine; mention the numbers in the handoff and do not stop work or ask about it.

## Conventions

- One owner per rule: readiness, unlocks and validation live only in `skills/planr/scripts/lib/graph.mjs`; the board reads derived fields and never recomputes them.
- The planning repository is the source of truth. The engine and CLI never write outside it and never commit, push, merge or deploy. Exception: the bb plugin's host entry commits and pushes human board actions (approve goal, close goal, task checked) from its own worktree, as described in `bb-plugin/README.md`.
- bb plugin (`bb-plugin/`): live tests only against an isolated local bb instance, never a bb server used for real work. Installing a development build there needs a maintainer's OK.
- Tests run with `node --test`. Exercise real planning data only in a throwaway clone, never in a live planning repo. Fixtures stay synthetic.
- Keep `artifacts/` untracked; it holds local agent run records.
- Internal plans, decisions and release notes stay outside this repository.
- English for code, docs and UI labels. Task titles keep the planning repo's language.

## Release
After maintainer approval, run `node scripts/bump-version.mjs X.Y.Z` and add its CHANGELOG section.
Remove `private` only in that approved release commit, then tag `vX.Y.Z` in `instructa/planr`.
CI publishes through `release.yml`; prerelease tags use npm `next`. planr ships through npm only.
Plugin build needs the bb CLI and is omitted from CI; verify it on an isolated local bb instance.
Static `site/` (planr.so) from `docs/`: `cd site && npm ci && npm run build`; deploy needs maintainer OK.
`scripts/bump-version.mjs` syncs plugin manifests and both marketplace npm pins with the release version.
