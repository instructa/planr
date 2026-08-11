# Planr Architecture

Planr V1 is a single Rust binary with explicit module ownership. The crate stays small enough that a Cargo workspace would add more process overhead than value today, and there is only one deployable: the `planr` CLI. The source tree is split by ownership boundary inside that crate instead of using a premature workspace. Shared mutations that must behave identically across CLI, MCP, and HTTP live in one place (`src/app/application.rs`) so the three surfaces cannot drift.

## Repository Layout

- `src/`: the Rust CLI (module ownership below).
- `tests/e2e.rs`: real CLI, MCP, HTTP, import, review-gate, run-log, and concurrent-pick tests.
- `plugins/planr/`: the installable plugin payload — all ten skills, independent Claude/Cursor worker and reviewer role assets, and the per-host plugin manifests. Planr does not ship model-pinned Codex roles.
- `.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`: marketplace manifests pointing Codex and Claude Code at `plugins/planr`.
- `docs/`: user and contributor guides; `docs/contracts/` holds the frozen product contracts that gate CI.
- `.planr/plans/product/planr/`: the production specification package for Planr V1, stored as a normal Planr product plan package. It is the only committed `.planr` path; all other runtime state stays local.
- `examples/real-world-flow.md`: executable real-world operator flow.
- `scripts/`: installer and release packaging scripts.
- `npm/`: the npm wrapper package.

## Module Ownership

- `src/main.rs`: process composition root. Owns top-level module wiring, process startup, database opening, error printing, and dispatch into `App`.
- `src/cli.rs`: CLI contract boundary. Owns `clap` command definitions, option parsing types, value enums, and command DTOs used by app dispatch.
- `src/app/mod.rs`: application composition boundary. Owns the `App` runtime state, top-level dispatch, shared app-local row helpers, and app submodule wiring.
- `src/app/commands.rs`: general CLI use-case orchestration. Owns project, plan, map, item, link, pick, approval, log, close, review, context, search, doctor, and shared install command handlers.
- `src/app/grok.rs`: Grok-specific install orchestration. Owns safe project TOML reconciliation, repository workflow writes, and the explicit no-hooks result.
- `src/app/prompts.rs`: CLI, MCP, and HTTP prompt output. Host routing prompt composition remains in `src/app/agents.rs`.
- `src/app/flow.rs`: compound workflow boundary. Owns evidence log writing (with heartbeat folding), canonical FeatureRun settlement input normalization, the close transition core, the pick work packet, and the `done` command shared by CLI, HTTP, and MCP surfaces.
- `src/app/git_review.rs`: Git and PR review evidence boundary. Owns worktree detection, scoped changed-file provenance, PR URL context, and dirty-worktree safety projections.
- `src/app/mcp.rs`: MCP stdio boundary. Owns MCP protocol request routing, tool calls, resource reads, and prompt responses.
- `src/app/packages.rs`: package import/export boundary. Owns reusable JSON templates, preview-before-import, durable ReviewGate package projections, and local-first encrypted bundle metadata.
- `src/app/http.rs`: localhost HTTP/SSE boundary. Owns HTTP request parsing, routes, SSE stream output, and HTTP response mapping.
- `src/app/repository.rs`: application data access helpers. Owns Planr query/update helpers over projects, plans, graph items, links, runs, logs, artifacts, events, approvals, search, and map projections.
- `src/app/lease.rs`: worker lease ownership. Owns the single pick query (`PickFilter`: exclude, work type, plan scope), worker ownership checks, runtime heartbeat/progress/pause state, and stale-pick detection.
- `src/app/review.rs`: ReviewGate application logic. Owns review annotations, feedback ingestion, scoped evidence, durable attempt completion, finding resolution, and gate lookup.
- `src/app/recovery.rs`: recovery automation logic. Owns item retry policy configuration, task conditions, stale/timed-out sweeps, retry scheduling, and recovery result projections.
- `src/app/execution_run.rs`: canonical FeatureRun and ReviewGate application boundary. Owns run phases, outcome batching and settlement, typed escalation, role leases, durable findings/re-review, and final product review projections.
- `src/app/execution_state.rs`: canonical `planr.execution_state.v2` read boundary. It projects one run-scoped FeatureRun, active batch, role owner, persisted budget amounts/provenance/digest/deadline, ReviewGate, attempts, findings, stable reason code, and next action for CLI, MCP, HTTP, pick, trace, status, recovery, package, and audit consumers.
- `src/app/surfaces.rs`: non-CLI runtime surfaces. Owns trace, scrub, artifact, event, debug, export, and import command handlers.
- `src/app/inspection.rs`: local inspection helpers. Owns debug bundles, context/link snapshots, pick context, secret scans, export value assembly, run recording, search results, and Planr-directory import parsing.
- `src/app/audit.rs`: goal contract audit boundary. Owns the clause-by-clause `plan audit` verdict (items settled, required independent material reviews, exactly one current independent final product review, approvals clear, canonical Evidence coverage) and its human rendering. Claim-only verification logs remain isolated to frozen pre-Evidence compatibility.
- `src/app/application.rs`: shared surface-mutation boundary. Owns the approval request/approve/deny, context, log, artifact, and close mutations reused verbatim by CLI, MCP, and HTTP handlers so the three surfaces cannot drift.
- `src/app/repository/`: focused data-access submodules (`item.rs`, `plan.rs`, `project.rs`, `link.rs`, `context.rs`, `evidence.rs`, `execution_run.rs`, `search.rs`) split out of `src/app/repository.rs` by entity ownership. Execution-state reads select ReviewGates strictly by run id; project/plan-wide history is never projected as the current run.
- `src/model.rs`: JSON-facing data transfer types and typed vocabulary. Owns serializable Planr DTOs plus the `ItemStatus`, `WorkType`, `LinkKind`, and `ApprovalStatus` enums with their parsing and display behavior, used by CLI JSON, MCP, HTTP, storage rows, and tests.
- `src/storage/mod.rs`: SQLite connection boundary. Owns default database path, connection setup, pragma configuration, and storage submodule exports.
- `src/storage/schema.rs`: SQLite schema boundary. Owns DDL, additive schema upgrade helpers, and schema version recording.
- `src/storage/execution_run_schema.rs`: the sole persisted compatibility boundary for FeatureRun rollout. It upgrades already-stored review/fix history into durable terminal history, but no application, packet, agent asset, or documentation path produces the superseded live shape.
- `src/storage/rows.rs`: SQLite row mapping boundary. Owns row-to-DTO and row-to-JSON mapping functions.
- `src/planpack.rs`: Markdown package generation and parsing. Owns project context templates, product/build plan templates, plan metadata parsing, hashes, search body extraction, and task extraction.
- `src/agents.rs`: agent profile registry core. Owns `.planr/agents.toml` parsing, registry validation warnings, and the pure advisory `resolve_route` precedence logic (override > work_type > plan > default); no storage or host concerns.
- `src/usage_policy.rs`: provider-neutral Usage Policy v1 core. Owns strict `.planr/policy.toml` parsing, policy and task-contract vocabulary, materiality classification, budget/concurrency validation, and the pure five-way transition resolver; it contains no provider ids, host dispatch, or execution-permission behavior.
- `src/execution_policy.rs`: execution admission core. Owns per-role filesystem, network, tool/MCP, structured command, environment, hook, secret, and approval grants; permission-diff previews; bounded task-contract admission; fail-closed command grammar; and isolated write-scope concurrency. It never selects models or mutates graph state. `src/app/policy.rs` binds that pure decision to the current SQLite lease owner and pick token, and only treats an admission from that exact lease generation as authoritative.
- `src/route_audit.rs`: provider-neutral run-observation contract. Owns strict requested/resolved/effective route stages, model/effort/fork enforcement confidence, transition provenance, policy/binding versions, and per-dimension metering confidence. It rejects requested-only values in the effective stage rather than inferring host execution.
- `src/app/agents.rs`: routing application boundary. Owns the `agents` and `item route` command handlers, the shared `*_value` JSON shapes reused by MCP, per-item route facts assembly, and registry-aware role content selection for installs.
- `src/app/agents_init.rs`: registry bootstrap boundary. Owns `planr agents init` — the static cost-tiering scaffold and, per the agent-pool plan, the flag-spec builder and interactive wizard.
- `src/integrations.rs`: shared agent-client descriptor boundary. Owns MCP install metadata, tool schemas, resources, and text response wrapping.
- `src/integrations/grok.rs`: document-preserving Grok project MCP configuration and conflict policy.
- `src/rolefiles.rs`: static host workflow roles and Cursor skill payloads. It does not select or pin models; externally generated routing artifacts stay outside Planr ownership.
- `src/util.rs`: small CLI-boundary utilities. Owns ids, timestamps, path helpers, output formatting, and safe file writes.

## FeatureRun Execution Boundary

Every live worker decision is a typed work packet:

- `kind: "outcome"` is ordinary maker work; `mode: "finding_repair"` returns named findings to the same responsible maker and ReviewGate without creating a fix item.
- `kind: "review_gate"` is independently leased checker work. Attempts and findings remain children of that gate, never graph items.
- `kind: "verification"` is a fresh verifier lease over a frozen canonical source digest. Product source is read-only; trusted Evidence can commit only through the source-checked Evidence transaction.
- `kind: "hold"` is an admission or capability stop. A driver must report its classification and next action; it cannot reinterpret the hold as permission to replace the maker, weaken verification, or open an ad hoc review.

All four packets embed the same run-scoped `planr.execution_state.v2` projection. Host skills, generated roles, installed copies, and Stop hooks render that state but own no lifecycle or budget policy. Ordinary maker settlement uses plain `planr done`; only an allowed protected-risk interrupt uses structured escalation. Review findings are logged and resolved on the existing gate. Evidence runs once after stable source freeze and selectively reruns only invalidated obligations after a product repair.

The one exception is persisted database input in `src/storage/execution_run_schema.rs`. Existing user databases may contain historical review/fix graph rows, so the schema upgrade consumes them once into terminal history. That read boundary does not justify aliases, fallback commands, dual DTOs, or live legacy producers elsewhere.

## Planr 2.0 Immutable Budget Boundary

The Planr 2.0 budget authority is one immutable `planr.feature_run_budget_contract.v2` created atomically with each FeatureRun. Bounded contracts contain a persisted UTC run-start anchor, complete wall-seconds/tool-call/token totals, exact maker/verification/review/repair/release allocations, per-dimension metering requirements, and a canonical digest. Unbounded mode is an explicit contract with no numeric limits or reserves. Every admitted bounded task carries positive maxima for all three dimensions and an overflow-checked absolute UTC deadline.

Ownership is singular and dependencies are one-way:

- `src/usage_policy.rs` owns the provider-neutral contract types, validation, provenance, phase protection, snapshots, and pure checked arithmetic.
- `src/execution_policy.rs` owns pure admission over typed concurrency plus persisted budget snapshots.
- `src/app/execution_run.rs` resolves authored policy once and binds the immutable contract to run creation.
- `src/app/feature_run_evidence.rs` owns transactional reservation, append-only observation, reconciliation, phase release, and hold sequencing.
- `src/app/execution_state.rs` owns the sole `planr.execution_state.v2` projection reused by CLI, MCP, HTTP, work packets, trace, status, generated roles, and skills.
- Storage owns insert-only persistence and integrity mechanics; host adapters only enforce supplied maxima/deadlines and report observations with provenance.

Runtime decisions never reload `.planr/policy.toml`, fabricate allowances, infer trusted usage from projections, synthesize missing active-run state, or expose a compatibility budget DTO. Missing, invalid, corrupt, or unenforceable budget state is a typed hold before dispatch.

An incompatible active FeatureRun is retired only through the plan-scoped application lifecycle in `src/app/execution_run.rs`. Pure eligibility and policy-cancel state changes live in `src/execution_run.rs`; `src/app/repository/execution_run.rs` ends the batch and leases, updates the run with optimistic concurrency, and records provenance in one transaction. CLI, MCP, and HTTP only transport the closed `incompatible-budget` reason and reuse the canonical result. Restart never writes a budget contract or successor run; a later ordinary pick remains the sole atomic run-plus-contract creation path.

A compatible budget-held FeatureRun resumes only through `planr run resolve-budget-hold --plan <id>`, owned by `src/app/feature_run_evidence.rs`. The application transaction revalidates the immutable contract, persisted snapshot, active reservation deadlines, exact held phase, canonical role owner, and lease generation before restoring that phase. Incompatible contracts require restart; capability holds, corrupt state, expired deadlines, missing reservations, unrepaired ceilings, and owner mismatches remain held. MCP and HTTP transport the same typed application result without policy or arithmetic.

## Eval Contract V1/V1.1 Ownership

The frozen product contract lives in `docs/contracts/EVAL_CONTRACT_V1.md` until implementation promotes it into code. Its V1.1 efficiency-evidence amendment is additive and keeps the same owner split:

- repository EvalSuite manifests and fixtures are authored inputs;
- the suite loader owns validation, safe path resolution, normalization, canonical ordering, and digests;
- SQLite owns immutable suite snapshots, runs, case results, samples, attempt lineage, metering-basis provenance, comparisons, invalidations, rescoring provenance, and eval evidence references;
- the comparison engine owns verdicts, reason codes, matched effective-treatment compatibility, and derived failure-inclusive efficiency metrics;
- existing map/log/review/approval state remains authoritative for work closure, so eval evidence may support a review but must never close or reopen items by itself.

## Boundary Rules

- Command parsing belongs in `src/cli.rs`; process startup belongs in `src/main.rs`; command execution belongs under `src/app/`.
- `src/main.rs` must stay small enough to be only a composition root. It must not own product use cases.
- `src/app/mod.rs` must stay small enough to wire runtime state and dispatch. It must not absorb app submodule behavior.
- SQLite schema belongs in `src/storage/schema.rs`; row mapping belongs in `src/storage/rows.rs`; app data access helpers belong in `src/app/repository.rs` and its submodules.
- Mutations shared by more than one surface (CLI, MCP, HTTP) belong in `src/app/application.rs`; surface handlers must call the shared helper instead of repeating SQL.
- Markdown templates belong in `planpack.rs`; command handlers should request generated file sets instead of embedding large template bodies.
- Agent install metadata and MCP schema descriptors belong in `src/integrations.rs`; client-specific strings should not drift across command handlers and docs.
- General graph DTO and vocabulary-enum changes belong in `src/model.rs`; FeatureRun/ExecutionBatch/ReviewGate status projection changes belong in `src/app/execution_state.rs`. JSON response shapes reuse those owners before adding ad hoc maps.
- Item status, work type, link kind, and approval status values are typed enums; new states must be added to the enum, not smuggled in as strings.
- Utility code must stay narrow. If a helper starts owning product behavior, move it to the owning module instead of growing `util.rs`.
- Do not add catch-all `common`, `shared`, or broad utility modules. New modules must name a durable ownership boundary.

## Single-Crate Decision

Planr remains a single crate for V1 because:

- there is one deployable binary and no separate service or reusable library boundary;
- the current behavior contract is tighter when CLI, MCP, HTTP, storage, and docs ship together;
- module-level ownership gives the needed architecture separation without duplicating Cargo settings or release packaging;
- npm, release, and external consumer tests assume one native binary named `planr`.

A Cargo workspace was tried and reverted: it produced anemic crates whose only job was being a layer, plus re-export shims in the binary. A workspace should be introduced only after a concrete deployable, reuse, compilation, or team ownership boundary exists and package/release scripts are updated in the same change.

## Future Extract Points

If Planr grows past the V1 binary shape, the first clean extraction path is:

- `planr-core`: `model.rs`, graph invariants, plan package contracts, and pure use-case types.
- `planr-storage`: `src/storage/*`, storage repositories, schema upgrades, and import/export packages.
- `planr-cli`: `src/cli.rs`, human output, and install helpers.
- `planr-server`: `src/app/http.rs`, `src/app/mcp.rs`, and runtime server adapters.

Do not extract those crates until a real reuse, compile-time, or ownership boundary exists. Every extraction must leave a real owner with runtime code, tests, and one-way dependencies.
