# Changelog

All notable changes to Planr are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.7.2] - 2026-07-25

### Changed

- Made the release workspace inventory deterministic: only the Planr root and documentation application participate in the tracked pnpm workspace, while ignored local experiments cannot change the lockfile.
- Made external release-eval fixtures self-contained by resolving their paths from the external suite directory and binding candidate binaries and databases to explicit absolute paths.
- Required every published architecture, including macOS x86_64 under Rosetta, to execute and report the exact tagged version before its release asset can upload.

### Security

- Refreshed immutable GitHub Action pins to their reviewed Node 24 releases and added a repository-owned workflow contract check that rejects unpinned or unreviewed actions.
- Kept provider keys and private evaluation content out of CI: model-backed release evidence remains a candidate-bound local maintainer gate, while CI exercises only deterministic synthetic cases.

## [1.7.1] - 2026-07-25

### Changed

- Clarified that Planr's complete evaluation workflow is CLI-first and does not require MCP; MCP remains an optional mirrored integration surface.
- Slimmed the goal, loop, and task-graph skills while preserving their planning, evidence, review, recovery, host-dispatch, and eval-authorization contracts in focused references.
- Updated the optional external routing handoff and public documentation for the exact verified Switchloom v0.3.2 package without moving lifecycle ownership into Planr Core.
- Separated maintainer-only lean-skill suites, baselines, model/effort runs, and results from the public repository while retaining Planr's provider-neutral eval CLI, generic lifecycle example, and deterministic release-gate contract.

### Added

- Added a local-only fail-closed release eval receipt gate that binds the current candidate and an explicitly supplied external suite/database to Planr-validated effective route evidence and a freshly recomputed comparison before any commit, tag, or push.
- Added a self-contained, synthetic CI test for the release-evidence mechanism; it performs no provider call and requires no private suite, result database, receipt, or API key.

### Security

- Kept model/provider credentials, raw prompts, raw completions, mutable eval actions, and long-lived npm tokens out of CI; release evaluation remains a maintainer-local gate and npm publication remains OIDC-based.
- Updated the documentation runtime to Next.js 16.2.11 and refreshed its locked dependency graph for the verified vulnerability-remediation baseline.

## [1.7.0] - 2026-07-22

### Added

- Added durable evaluation suites, runs, comparisons, and replayable evidence across CLI and MCP, including correctness, quality, performance, and regression gates.
- Added cost-per-verified-success reporting and effort recommendations so model choices are evaluated by successful task outcomes instead of token prices alone.
- Added resumable live-evaluation dogfood contracts with invalidation and rescoring support, preserving provenance when inputs, graders, or pricing assumptions change.

### Security

- Added repository leak, dependency, workflow, privacy, and forbidden-staged-file gates with BetterLeaks, Trivy, and hardened GitHub Actions permissions.
- Sanitized temporary oracle output and cleanup paths so evaluation logs do not retain secrets or personally identifiable workspace data.

### Fixed

- Stabilized the live-evaluation timing fixture and pruned ignored workspace state from dependency lock evidence.

## [1.6.0] - 2026-07-21

### Added

- Added a human-only boxed graph observer with `planr map show --view diagram`. Its condensed nodes keep status, id, and title to at most two content lines, while `--full` restores complete node details.
- Added `planr map watch` for live, change-driven graph supervision from a second terminal, with plan scoping, configurable polling, optional screen clearing, and `--until-settled` support.

### Changed

- Added an accessible terminal palette for map states and routes, with automatic TTY detection plus `--no-color` and `NO_COLOR` opt-outs.
- Clarified dependency progress in human output: satisfied `blocks` edges render as `blocks✓` in the agent-adjacent tree and neutral `then` routes in the diagram, while unresolved dependencies remain red `blocks` routes.
- Documented diagram and watch output as human supervision surfaces. Coding agents continue to use the compact default tree, `planr map show --json`, or the event-stream API.

### Compatibility

- Kept map status, link kinds, JSON output, readiness computation, and persistence unchanged; the new labels and colors are presentation-only.

## [1.5.2] - 2026-07-20

### Changed

- Completed the routing-ownership hard cut: Planr Core now remains only the provider-neutral consumer of repository declarations, route resolution, route evidence, and route-audit metadata. It does not own provider policy, host bindings, generated role files, routing-bundle application, catalog publication, or lifecycle cleanup.
- Kept Switchloom optional and external. The verified handoff fixture is Switchloom v0.2.1, which may manage repository-local routing artifacts outside Planr; Planr does not install, invoke, compile, download, apply, or uninstall Switchloom output.
- Tightened public release wording around the standalone path: Planr works with no routing declaration, and requested-only routing metadata is never treated as proof that a model, effort, role, or fallback actually ran.
- Audited README and routing docs so current guidance names provider-neutral declarations and evidence as Planr's boundary, while preserving historical changelog entries as historical release notes.

## [1.5.1] - 2026-07-18

### Fixed

- Locked the client provisioning contract in CLI help, generated reference, docs, and E2E tests: Codex installs project MCP/hooks but no project roles or skills; Claude Code installs project MCP/standalone roles/hooks while its plugin owns skills; Cursor installs project MCP/roles/skills/hooks. `--no-mcp` skips only MCP, and `--no-hooks` independently skips hooks.
- Removed the current routing-bundle application surface from Planr docs, generated references, tests, and release wording. Planr now documents only provider-neutral repository declarations and route evidence; optional routing lifecycle is external and repository-local, with Switchloom v0.2.0 as an external tool rather than something Planr invokes, installs, or uninstalls.

## [1.5.0] - 2026-07-17

Routing policy became an optional package instead of a responsibility compiled into Planr Core. This section is historical: the routing-bundle application boundary described here was removed in 1.5.1.

### Changed

- Hard-cut opinionated model routing out of Planr Core. At the time, Core owned the provider-neutral registry, route resolution and evidence, plus strict RoutingBundle v1 inspect/preview/apply. The independently buildable `planr-routing` workspace package owned named policies, exact models and effort, Codex/Claude Code/Cursor/mixed-host bindings, generated roles and skills, evaluation, signing, registry data, and catalog publication.
- Removed the legacy preset CLI, MCP tools, Rust modules, root policy fixtures, and root website ownership without aliases or compatibility layers. The historical replacement flow compiled a bundle with `planr-routing` and applied it through `planr routing bundle`; no command edited user configuration.
- Catalog entries remain deterministic, experimental, and unrecommended. Offline or caller-asserted evidence cannot promote them. Detached signatures and signed bundles require an independently supplied trusted signer and Ed25519 public key.
- Hardened repository application against parent/child artifact collisions and rollback residue. Global Codex, Claude Code, Cursor, shell, keychain, credential, and XDG sentinels remain unchanged across bundle application.

## [1.4.0] - 2026-07-16

Verified presets turn model routing from a hand-authored host configuration into an inspectable, policy-driven workflow. This release adds safe built-in policy/binding pairs, reproducible evaluation and signed registry evidence, and a public catalog deployed from repository-owned infrastructure.

### Added

- Usage Policy v1 and execution-policy enforcement: provider-neutral budgets, role capabilities, transitions, write boundaries, requested/resolved/effective route evidence, and correction or violation audit records are shared across CLI, MCP, and HTTP execution paths.
- Preset composition through `planr agents preset list` and `planr agents preset apply`: four embedded policies, five host bindings, and 20 declared safe pairs can be resolved by id, previewed as deterministic repository-local artifact diffs, and confirmed without touching user or global configuration. Explicit file inputs remain supported but are clearly marked custom rather than inheriting safe status by name.
- Reproducible preset evaluation through `planr agents preset evaluate`: versioned challenge tasks, Planr-read artifact hashes, task-bound outcome oracles, lifecycle thresholds, optional live-host execution, and independently pinned Ed25519 telemetry receipts. Recommendations require complete, current, trusted route and usage evidence; offline estimates and incomplete runs cannot recommend.
- Optional signed preset registry commands (`verify`, preview-first `import`, and offline `list`) with immutable manifest-hash-addressed caching, lifecycle and compatibility checks, separately provisioned maintainer trust, and re-verification of cached content. Active projects and previously imported packs continue working when the registry is unavailable.
- Public [Planr Preset Catalog](https://planr-test-catalog.office-35d.workers.dev/) generated from the canonical verifier and evaluation report. Repository-owned Alchemy/Cloudflare tooling builds an allowlisted static publication, deploys an isolated `test` stage, and ships restrictive response headers without storing private signing keys in the site or deployment environment.
- Historical guides covered preset composition, evaluation, and the registry, plus CLI, MCP-contract, architecture, and deployment documentation.

### Security

- Preset application and registry publication fail closed on incompatible hosts, unsafe permissions, traversal or symlink escapes, overwrite conflicts, executable or binary content, secret-like values, invalid signatures, revoked or stale evidence, checksum mismatches, and untrusted telemetry. Diagnostics redact sensitive values, and confirmed writes are restricted to allowlisted paths inside the repository.

## [1.3.0] - 2026-07-14

Native host hooks: loop state that survives sessions becomes mechanism instead of discipline. Hardened through a three-round independent GPT-5.5 review (verdict: complete) and two dogfood runs — a full routed loop on a Cloud Agent VM and a local hook-execution run — before this stable release. Includes everything previewed in 1.3.0-alpha.1.

### Added

- Native host hooks, installed by default: `planr install codex|claude|cursor` now wires `planr prime` into the host's hook system — Cursor `.cursor/hooks.json` (`sessionStart` with the `--cursor-json` envelope), Claude Code `.claude/settings.json` (`SessionStart` with matcher `startup|resume|compact`, using the `--hook-json` envelope), Codex `.codex/hooks.json` (`SessionStart`, with the one-time `/hooks` trust note in the install output). Every new session — including post-compaction session starts — gets map state injected automatically, so loop recovery becomes mechanism instead of discipline. Only session-start events are wired: they are where hosts actually inject hook output as context (pre/post-compaction events cannot). `--no-hooks` opts out; existing hook files are merged additively (foreign entries preserved, planr entries never duplicated, unparseable files left untouched with a note); every hook command fails open (`|| true`, 10s timeout) so a missing or broken planr never blocks a session.
- `planr prime [--hook-json|--cursor-json]`: one compact, bounded, deterministic state block (project, map counts, up to five held items with completion-log status, goal contract truncated char-safe, registry presence, next command). In a repo without a Planr database it exits silently and creates nothing. `--hook-json` emits the Claude Code SessionStart envelope; `--cursor-json` emits Cursor's `additional_context` shape.
- Evidence guard (Cursor `subagentStop`), identity-scoped: a subagent that stops while *its own* pick (via `PLANR_WORKER_ID`/`PLANR_SESSION_ID`) has no completion log gets one advisory follow-up naming the item and the two ways out (`planr done` or `planr pick release`); without an explicit identity it stays silent instead of steering agents toward foreign items. JSON built by jq (never string interpolation), always exit 0, shellcheck-clean.
- docs/HOOKS.md: what gets installed per host, the fail-open and additive-merge rules, the Codex trust model, and how to remove the hooks.

### Changed

- `planr map status` human output shows the actual summary (settled/total, per-status counts, up to five items per bucket) instead of a bare "map status calculated" — the fourth dogfood run pointed out that `prime` sends users exactly there.
- `planr prime`'s `next:` hint after a fully settled map points at the contract verdict (`planr plan audit <plan-id> --json`, plan id parsed from the stored goal contract) instead of another status read.
- `planr install <client>` prints `hooks: unchanged (already current)` when the hook reconciliation was a no-op, so re-installs confirm themselves.
- `planr doctor` probes `cursor-agent` and `agent` in addition to `cursor` before reporting the Cursor CLI as not installed.
- Docs: artifact `--kind` vocabulary documented (screenshot, video, recording, ...), planr-verify-web shows a video-artifact example, and CLI_REFERENCE carries a one-line jq example for the `plan audit` JSON shape (`holds`, `clauses[].pass`).

## [1.3.0-alpha.1] - 2026-07-07

Preview release of 1.3.0. All changes are listed under [1.3.0] above; this tag shipped the hooks feature set before the fourth dogfood run's polish landed.

## [1.2.0] - 2026-07-06

Per-task model routing becomes a declared contract instead of prose, and Cursor becomes a first-class client — one command installs everything the plugin would carry. The whole feature set was hardened through three live dogfood runs (a real web app built end to end through the routed pool) plus an independent GPT-5.5 review before this stable release. Includes everything previewed in 1.2.0-alpha.1.

### Added

- `planr pick --peek` (MCP `planr_pick_item` with `"peek": true`): the full work packet for the next pickable item — routing block included — without writing a lease, heartbeat, or pick event. Dispatching drivers read, dispatch, and leave the lease to the worker's own identity; the third dogfood run needed a pick → `pick release --force` → re-pick dance per item for this, three calls that are now one.
- Work-type annotations in plan task lists: `### TASK-001 (backend): ...` headings and `- [ ] (frontend) ...` checklist items seed `map build` items with the annotated work type, so routing binds at build time with zero retags — the third dogfood run needed a manual `item update --work-type` round because everything seeded as `code`. Annotations are single identifier-like tokens (prose parentheticals never match); unannotated tasks keep `code`; the planning skills declare use cases in the task list and fall back to retagging for existing maps.
- `planr item cancel --reason`: the why travels in a new `item_cancelled` graph event (recorded on every confirmed cancel), so cancellations are auditable instead of living only in chat history (dogfood run 3).
- `planr agents routing`: discoverability alias for `planr prompt routing` — the place people guess the dispatch block lives.
- `agents check` warns (advisory, exit unchanged) when a profile pins a skill with no `SKILL.md` under the project or home skill directories (`.cursor`/`.claude`/`.agents`/`.codex`); the third dogfood run shipped a worker silently missing its pinned skill.
- Agent profile registry: `.planr/agents.toml` declares named profiles (host client, model, effort, cost tier, capabilities) and advisory routes from work selectors to profiles with fallback chains — "code goes to codex/gpt-5.5 xhigh, falls back to cursor/fable-5; review stays on the driver tier" is one declared file instead of hand-maintained prose in three host dialects. Planr never dispatches models; hosts stay the authority.
- `planr pick --json` now carries a `routing` block (profile, client, model, effort, cost tier, fallbacks, matched selector) resolved per item with precedence `work_type` > `plan` > default route, so a driver dispatches the right worker model from the pick packet alone — including the fallback order when the primary hits a rate limit. Omitted entirely when no registry resolves; deleting the registry restores pre-feature packets byte-identically.
- `planr agents list` and `planr agents check`: registry inspection with advisory warnings (unknown profile references, empty or duplicate selectors, review work routed to a budget tier, secret-like values). `check` exits non-zero only on parse failure — a missing registry is a state, not an error, and a malformed one degrades picking to no-routing instead of blocking it.
- Use-case pools with skill pairing: profiles accept an optional `skill` field naming the skill the profile dispatches with (e.g. `frontend-design`), carried into pick-packet routing blocks, `item route`, `agents list`, and the `prompt routing` table — combined with free-form work types (`--work-type frontend` routes via `match = { work_type = "frontend" }`), the registry declares a small agent pool per use case: Fable+Opus for frontend/design, Fable+GPT-5.5 for backend, each paired with its skill. Passthrough vocabulary like model ids; profiles without a skill omit the key, keeping existing routing blocks byte-identical.
- `planr agents init [--force]`: writes a commented starter registry so adoption never begins from a blank file — the cost-tiering defaults as working TOML (premium driver that keeps the verdicts, standard steerable implementer, budget helper for token-hungry side work; code/fix routes with a driver fallback, review pinned premium, a default route) with inline comments teaching the tiering model. Guaranteed to parse with zero `agents check` warnings; an existing registry is never overwritten without `--force`. The output names the file and the two follow-ups (`agents check`, `install <client> --force`).
- Generated registries (scaffold, flag builder, wizard) teach the client-honesty rule where clients get declared: a comment states that a `codex` profile is only honest when the driver really spawns a Codex process, and that runs record the observed host with deviations flagged in `trace item`.
- Configurable `agents init`: individual pools generate from repeatable spec flags — `--profile <id>=<client>/<model>[@<effort>][#<tier>]`, `--skill <profile>=<skill>`, `--route <work_type>=<profile>[,<fallback>...]`, `--default-route` — with fail-closed validation (unknown profile references and malformed specs error naming the grammar before anything is written; consistent specs are guaranteed a zero-warning registry). Model ids keep embedded slashes, so opencode-style `provider/model-id` values work. `--interactive` walks the same questions as clack-style guided prompts (driver, use cases, per-use-case pairing, default route, optional role-file installs at the end); it is a thin shell over the same builder the flags feed, requires a real terminal (agents get a clean error pointing at the flag grammar instead of a hung prompt), and conflicts with spec flags at parse time. The `agents_registry_initialized` event now records the `mode` (scaffold, flags, or wizard).
- Per-item route overrides: `planr item route <id>` shows the resolved route and whether an override or policy won, `--set <profile>` pins one item to a registry profile (override beats every policy route; the pick packet reports `"matched_selector": "override"`), `--clear` unpins. `--set` rejects profile ids a loaded registry does not declare but warns-and-stores when the registry is missing or malformed, so offline edits stay possible; a pin whose profile later leaves the registry falls back to policy with a repair hint, never an error. Both mutations are graph events (`route_overridden`, `route_override_cleared`), so re-routing decisions are auditable via `planr event list`.
- MCP routing tools with the exact CLI JSON shapes: `planr_agents_list` and `planr_item_route` (read), `planr_item_route_set` and `planr_item_route_clear` (mutate).
- Rendered worker role files carry their own audit instruction: the renderer knows the concrete profile it pins, so the worker body now says "pass `--profile <id>` on every `planr done` and `planr log add`" — profile reporting follows from the role definition instead of worker memory (in Codex TOML the note lands inside `developer_instructions`, where the model actually reads it).
- Registry-rendered host role files: when `.planr/agents.toml` exists, `planr install codex|claude|cursor` renders the worker and reviewer subagent role files with model pins from it — the `work_type=code` route pins the worker, `work_type=review` the reviewer, in each host's strict vocabulary (Codex `model` + `model_reasoning_effort` with `developer_instructions` always present, Claude `model:` + `effort:` frontmatter, Cursor `model:` only). A role only pins profiles whose `client` matches the install target (fallback-chain scanned), so a Cursor-profile review route never writes a Cursor model into a Codex TOML. Rendered files carry a `# generated from .planr/agents.toml` header; without a registry the static files are written byte-identically to previous releases.
- `planr install <client> --force`: explicit re-render/overwrite of provisioned role and skill files after registry edits. Without it, existing files are still never touched (provision-once unchanged).
- `planr prompt routing [--client codex|claude|cursor|all]`: a paste-ready model-prioritization block for driver sessions — the route table (every route, profile, fallback), per-host dispatch guidance naming the traps that silently defeat pins (Codex `fork_turns: "none"` + session-restart-after-re-render, Claude's `CLAUDE_CODE_SUBAGENT_MODEL` env preemption, Cursor's plan/admin/Max-Mode overrides), and `codex exec`/`pi`/`opencode run` process-dispatch snippets pre-filled from the code route. `--json` carries the same content structured; a missing or unreadable registry prints the guidance with a pointer instead of failing.
- Observed-client auditing: runs record the host they observably executed under (`observed_client`), detected from environment variables the hosts set themselves (`CODEX_SANDBOX`/`CODEX_SESSION_ID`, `CLAUDECODE`, `CURSOR_AGENT`/`CURSOR_INVOKED_AS`) — observed rather than self-declared, no flags. When a loaded registry's declared route names a different client, Planr emits one advisory `client_mismatch_observed` event, and `trace item` shows the observed host per run with a client-mismatch marker. This closes the audit blindspot where profile self-report masks a client-level deviation (e.g. Cursor subagents standing in for a declared Codex profile — exactly what a live acceptance audit found); it also audits runs whose worker never reported a profile. Unknown host means nothing is stored and nothing compared.
- Run-profile auditing: `planr log add` and `planr done` accept `--profile <id>` (fallback: the `PLANR_PROFILE` env var, which rendered role files can export) and store it on the recorded run. When the reported profile differs from the item's declared route, Planr emits one advisory `route_mismatch_observed` event with the declared and actual ids and the run id — every host has a silent override path, so declared-vs-actual is the only trustworthy signal. No profile, no run, or no registry means no comparison and no event; mismatches never block logging, reviews, or closes. Also accepted by MCP `planr_log_add` and `POST .../log` as optional `profile`.
- `planr trace item` gains a `routing` section when the item has a declared route or a profiled run: the declared route (profile + matched selector) next to every run's actual client/profile with an advisory `mismatch` marker and count. Items without either keep their exact pre-routing trace shape.
- `planr doctor` reports the agent registry in every state without ever failing: absent (informational), degraded (parse error with line context), loaded (profile/route counts, validation warnings), plus per-artifact drift — rendered role files whose generated-from content no longer matches the current registry are flagged with a `planr install <client> --force` hint; header-less files are the user's (`manual`) and never flagged.
- Package export/import carries the agent registry: `export` snapshots `.planr/agents.toml` raw, `import` previews the exact action (`create`, `identical`, `conflict`) and never overwrites a differing local registry — the conflict is reported with a remove-and-re-import hint. Pre-registry packages import unchanged.
- `planr install cursor` is now the one-command Cursor setup: besides `.cursor/mcp.json` it provisions the `planr-worker` and `planr-reviewer` subagents (`.cursor/agents/*.md`, Cursor frontmatter with `model: inherit` cost-tier note) and copies all ten Planr skills to `.cursor/skills/`, so `/planr`, `/planr-loop`, `/planr-worker`, and `/planr-reviewer` work without waiting on the marketplace listing. Existing files are never overwritten.
- One-click user-level MCP install: `planr install cursor --dry-run` (and the non-dry output) prints a `cursor://anysphere.cursor-deeplink/mcp/install` link. The embedded config is `planr mcp` without `--db`, so each workspace resolves its own `.planr` database — safe at user scope.
- `planr install <client> --no-mcp`: client-specific CLI-first setup without MCP. Codex writes no roles or project skills, Claude Code writes standalone roles but no project skills, and Cursor writes roles plus skills; default hooks remain unless `--no-hooks` is also passed. `--no-mcp --dry-run` lists the client-owned files and hook intent.
- `planr project init --client all` now provisions Cursor subagent roles alongside Codex and Claude Code; `--client cursor` provisions them for Cursor alone.
- The Cursor plugin manifest registers both subagents and its version is guarded by the release script and the version drift test (it had drifted to 1.1.12).
- `docs/CURSOR.md` documents multitasking with Cursor's built-in features: maker/checker subagent dispatch, parallel and background subagents over pick leases, cloud agent caveats, and the shared-absolute-`--db` rule for parallel agents in git worktrees.

### Fixed

- Alpha release channel: `scripts/release.sh` accepts semver pre-release versions (`1.2.0-alpha.1`; `-alpha.N`/`-beta.N`/`-rc.N` only), and the release workflow marks such tags as GitHub prereleases (the curl installer's `latest` stays stable; testers pin via `PLANR_VERSION`), publishes npm under the `alpha` dist-tag (`npm install -g planr@alpha`), and never moves the Homebrew tap. Documented in RELEASE.md.
- Concurrent first connections no longer race the WAL conversion with a zero busy timeout: `busy_timeout` is now set before `journal_mode = WAL` when opening the database, so parallel workers whose very first `planr pick` hits a fresh or contended database wait out the lock instead of failing (or stalling) on it. A parallel first-pick storm (8 processes × 4 rounds under a hard watchdog) is now a regression test, and TROUBLESHOOTING.md documents the remaining look-alike: host tool harnesses that stop draining stdout block the child on a full pipe, which resolves on retry and is outside Planr's control.
- Hardening from an independent GPT-5.5 review of the trust slice: a blank `--reviewer ""` (or whitespace/empty identity env vars) no longer counts as an explicit identity — it stamps `single_agent`, and an empty `PLANR_WORKER_ID` no longer masks a set `PLANR_SESSION_ID`; `agents init` spec ids (profiles, route work types) are restricted to TOML-bare-key-safe characters and values reject control characters, fail-closed with the rule in the message; generated registries are parse-checked before any write as defense in depth; and `install` falls back to the static role file when a hand-written registry carries render-unsafe values (quoted TOML keys with newlines or `"""` would otherwise corrupt the rendered artifact).
- `independent` review stamps must be earned, not lucked into: `review close` stamps `independent` only when the reviewer identity was explicitly set (`--reviewer` or `PLANR_WORKER_ID`); a review closed under the anonymous fallback identity stamps `single_agent` even when the identity strings happen to differ from the maker's (a live loop run produced exactly this accidental `independent`). Explicit-identity flows are unchanged.
- `item create --work-type` is no longer a hidden flag: it is the way items opt into use-case routing, so it now appears in `--help` with the routing context (found while dogfooding the routing walkthrough).
- Links no longer fail silently on unknown item ids: `link add`, `item create --after`, and every other link write now error with the offending id instead of writing nothing (previously a truncated id was ignored without signal — also a dogfood find).
- `item create --after <bad-id>` is atomic: the id is validated before the item persists, so the error leaves nothing behind and a retry cannot duplicate the item (dogfood find from testing the loud-link fix).
- The `item cancel` refusal names the repair path (`--preview` first, then `--confirm`) and the item id instead of a bare "refusing to cancel".
- `item update` now records an `item_updated` graph event with the changed fields — a work-type retag changes routing, and the audit log previously had no trace of it (found during a loop-run acceptance audit).
- The provisioned reviewer role files instruct passing `--reviewer <id>` explicitly on `review close`: shell exports do not survive between subagent tool calls, and a review closed under the default identity can stamp `independent` on luck alone.

### Changed

- `planr event list` human output lists one event per line (timestamp, type, item, compact payload) instead of a bare count — grep pipelines over events work now; `--json` is unchanged.
- `route_mismatch_observed` and `client_mismatch_observed` payloads carry the originating `log_kind`, so audit consumers can discount the legitimate case of a driver adding a verification log to a routed item.
- `trace item` drops the legacy identity-derived client bracket on runs that carry an observed host (`run ... [human] profile x on cursor` read contradictory).
- MODEL_ROUTING documents the verification-log mismatch case and the exact-host-slug rule for single-host pools; CLI_REFERENCE documents the `--cmd` flag → `commands` JSON field mapping; the planr-work skill requires verification log commands to be copy-paste replayable.
- Route-aware tagging without user involvement: `planr item update` gains `--work-type`, and the planning skills (planr-plan, planr-task-graph, planr-goal) now instruct agents to read the registry's route selectors (`agents list --json`) after `map build` and retag items to the matching use case (`frontend`, `backend`, ...) themselves — a human never has to know work types exist for routing to bind.
- The worker and loop skills are routing-aware: `planr-work` instructs workers to report the profile they actually ran on (`--profile`/`PLANR_PROFILE`) as part of the evidence, and `planr-loop` instructs drivers to dispatch on the pick packet's `routing` block and walk the `fallbacks` chain on rate limits.
- `docs/MODEL_ROUTING.md` describes the shipped end state (quick start via `agents init`, overrides, rendering, prompt routing, run audit) and adds a five-host matrix (Cursor, Claude Code, Codex CLI, opencode, Pi) with each host's native mechanism and its silent-override traps. `docs/GOALS.md` Cost Tiering is corrected to July 2026 host behavior: the Codex spawn regressions (#26868/#26363) are fixed in v0.138+ and replaced by the live `fork_turns` and session-start registry-staleness traps (#26408); the Claude section covers v2.1.196 `inherit` semantics, the signal-free `CLAUDE_CODE_SUBAGENT_MODEL` clamp (#57718), and silent `availableModels` fallbacks; Cursor's admin/plan/Max-Mode override path is a named trap.
- Shared surface mutations (approval request/approve/deny, context create, log add, artifact add, item close) now live once in `src/app/application.rs`; CLI, MCP, and HTTP call the same helpers, so MCP evidence logs now record runs and refresh the pick heartbeat exactly like CLI logs.
- Item status, work type, link kind, and approval status are typed enums in `src/model.rs`; invalid vocabulary is rejected at the boundary instead of stored as strings (`link remove --type` now errors on unknown kinds).
- Toolchain: edition 2024 and MSRV 1.85 (was 1.80), with an explicit clippy lint policy in `Cargo.toml`.

## [1.2.0-alpha.1] - 2026-07-05

Preview release of 1.2.0. All changes are listed under [1.2.0] above; this tag shipped the routing feature set before the third dogfood run's findings landed.

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

- `planr project init` provisions standalone Claude Code loop roles; Codex workflow skills are plugin-owned and `planr install codex` writes project MCP/hooks, not project agents.

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

[Unreleased]: https://github.com/instructa/planr/compare/v1.7.2...HEAD
[1.7.2]: https://github.com/instructa/planr/compare/v1.7.1...v1.7.2
[1.7.1]: https://github.com/instructa/planr/compare/v1.7.0...v1.7.1
[1.7.0]: https://github.com/instructa/planr/compare/v1.6.0...v1.7.0
[1.6.0]: https://github.com/instructa/planr/compare/v1.5.2...v1.6.0
[1.5.2]: https://github.com/instructa/planr/compare/v1.5.1...v1.5.2
[1.5.1]: https://github.com/instructa/planr/compare/v1.5.0...v1.5.1
[1.5.0]: https://github.com/instructa/planr/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/instructa/planr/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/instructa/planr/compare/v1.3.0-alpha.1...v1.3.0
[1.3.0-alpha.1]: https://github.com/instructa/planr/compare/v1.2.0...v1.3.0-alpha.1
[1.2.0]: https://github.com/instructa/planr/compare/v1.2.0-alpha.1...v1.2.0
[1.2.0-alpha.1]: https://github.com/instructa/planr/compare/v1.1.19...v1.2.0-alpha.1
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
