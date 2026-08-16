# Tasks

These tasks are implementation-ready work items for coding agents. They intentionally avoid storypoints and timelines.

## Core FeatureRun Recovery

### TASK-CORE-FREEZE-001: Add Typed Stale Source-Freeze Restart

Keep the central FeatureRun lifecycle/application service as the single policy owner. Diagnose only an active stale source freeze with no verification item, terminally retire the unusable run without mutating its freeze, atomically route stranded code outcomes to ready, and expose the same typed restart through pick, CLI, and MCP. Successor run creation and current-source freezing stay on canonical pick/readiness paths. Add no database repair, fallback, compatibility path, nullable special-case wrapper, legacy handling, or adapter/UI owner.

Acceptance: the focused existing central restart application test passes in BUILD; one public lifecycle invariant is admitted separately in HARDEN.

## Alpha 6 Binding Evidence Hard Cut

### TASK-ALPHA6-EVIDENCE-001: Make Policy And Obligations One Fail-Closed Authority

Keep typed build-plan frontmatter as criterion identity authority and `app/proof` as the sole completeness owner; reject zero, partial, duplicate, and undeclared binding sets before implementation, readiness, coverage settlement, and final review; persist a capability hold when verifier readiness fails; remove claim-log, partial-row, and empty-lineage compatibility authority; retain explicit migration as the only obligation writer.

Acceptance: `cargo check --lib` passes, followed by one focused lifecycle invariant in the separate hardening phase.

## V1.1 Differentiation

These tasks implement `V1_1_DIFFERENTIATION_CONTRACT.md`. They are Planr-owned product work and must not introduce public references to local comparison products.

### TASK-V11-001: Establish V1.1 Gap Contract

Goal:
Create the executable V1.1 contract before coding.

Context:
Supports every REQ-V11-* requirement in `V1_1_DIFFERENTIATION_CONTRACT.md`.

Requirements:
- Define graph intelligence, pick recall, recovery, durable ReviewGates, scoped Git review, distribution, templates, docs, and E2E acceptance criteria.
- Use Planr vocabulary only.
- Keep the contract specific enough that coding agents can work each slice without hidden context.

Files or areas likely involved:
- `docs/contracts/V1_1_DIFFERENTIATION_CONTRACT.md`.
- `.planr/plans/product/planr/TASKS.md`.
- `.planr/plans/product/planr/README.md`.

Acceptance criteria:
- Contract exists with stable requirement ids.
- Public docs contain no local comparison-product names.
- Downstream implementation tasks can map directly to requirement ids.

Tests:
- Forbidden-reference scrub over README, docs, src, tests, examples, scripts, skills, `.github`, package metadata, and `.planr/project`.

Dependencies:
- V1 final validation gate.

Do not do:
- Do not implement feature code in this contract task.

### TASK-V11-002: Implement Graph Intelligence

Goal:
Make Planr map analysis mathematically useful for agents and operators.

Context:
Supports REQ-V11-GRAPH-001 through REQ-V11-GRAPH-006.

Requirements:
- Compute critical path across blocking edges.
- Compute transitive pressure and direct blocker counts.
- Diagnose cycles before readiness and path output mislead agents.
- Share one effect engine across close, cancel, insert, replan, and dependency previews.
- Expose consistent results in CLI, MCP, HTTP, and JSON.

Files or areas likely involved:
- `src/app/repository.rs`.
- `src/app/commands.rs`.
- `src/app/http.rs`.
- `src/app/mcp.rs`.
- `tests/e2e.rs`.

Acceptance criteria:
- Branched and deep graph fixtures prove longest-path behavior.
- Transitive bottleneck fixtures prove downstream impact ranking.
- Cycle fixtures produce actionable diagnostics.

Tests:
- Focused unit tests for graph algorithms.
- CLI/MCP/HTTP E2E coverage for critical path, pressure, lookahead, and previews.

Dependencies:
- TASK-V11-001.

Do not do:
- Do not treat priority sorting as critical-path analysis.

### TASK-V11-003: Implement Automatic Pick Recall

Goal:
Make every pick return the context an agent needs to start safely.

Context:
Supports REQ-V11-RECALL-001 through REQ-V11-RECALL-005.

Requirements:
- Rank relevant contexts, plans, logs, and review summaries.
- Include upstream handoffs, linked plan references, ReviewGate history, blockers, unlocks, possible file conflicts, and deeper-read commands.
- Keep responses size-bounded and privacy-safe.
- Share behavior across CLI, MCP, and HTTP pick routes.

Files or areas likely involved:
- `src/app/inspection.rs`.
- `src/app/repository.rs`.
- `src/app/commands.rs`.
- `src/app/http.rs`.
- `src/app/mcp.rs`.
- `tests/e2e.rs`.

Acceptance criteria:
- Relevant historical decisions appear in pick output without a separate search.
- Irrelevant contexts rank below relevant ones.
- Prompt transcripts, source contents, secrets, and large artifacts are omitted by default.

Tests:
- Ranking fixture tests.
- Pick output E2E tests across CLI, MCP, and HTTP.

Dependencies:
- TASK-V11-001.

Do not do:
- Do not dump all project context into every pick.

### TASK-V11-004: Implement Recovery Automation

Goal:
Make interruption recovery deterministic and inspectable.

Context:
Supports REQ-V11-RECOVERY-001 through REQ-V11-RECOVERY-006.

Requirements:
- Add sweeper command/API with dry-run or preview-first behavior.
- Add timeout and retry/backoff fields and transitions.
- Reclaim stale picks only through explicit recovery behavior.
- Surface preconditions and postconditions in pick, trace, review, and close preview.
- Report all closure blockers in one gate report.

Files or areas likely involved:
- `src/storage/schema.rs`.
- `src/model.rs`.
- `src/app/repository.rs`.
- `src/app/commands.rs`.
- `src/app/http.rs`.
- `src/app/mcp.rs`.
- `tests/e2e.rs`.

Acceptance criteria:
- Interrupted work can be diagnosed, released, and safely re-picked.
- Retryable failure returns to ready only after documented policy allows it.
- Unsatisfied conditions block or warn exactly as documented.

Tests:
- Timeout and stale-pick tests.
- Retry/backoff tests.
- Close-preview condition tests.

Dependencies:
- TASK-V11-001.

Do not do:
- Do not silently mutate or reclaim active work by default.

### TASK-V11-005: Build Durable FeatureRun Review Gates

Goal:
Give reviewers one canonical durable review workflow across every surface.

Context:
Supports REQ-V11-REVIEW-001 through REQ-V11-REVIEW-005.

Requirements:
- Create or reuse ReviewGates only through canonical FeatureRun settlement and final product review services.
- Record independent leases, immutable attempts, verdicts, and durable findings.
- Resolve named findings on the same gate and return it to pending re-review.
- Project final ReviewGate state consistently through CLI, MCP, HTTP, audit, status, package, and trace.

Files or areas likely involved:
- `src/app/http.rs`.
- `src/app/review.rs`.
- `src/app/execution_run.rs`.
- `docs/`.
- `tests/e2e.rs`.

Acceptance criteria:
- A material outcome produces one independently leasable ReviewGate.
- Findings persist until explicitly resolved, then the same gate is re-reviewed.
- Final review acceptance satisfies plan audit without review/fix map items.

Tests:
- Cross-surface FeatureRun settlement tests.
- ReviewGate finding/re-review tests.
- Final product review projection tests.

Dependencies:
- TASK-V11-001.

Do not do:
- Do not require a cloud account or hosted paste service.

### TASK-V11-006: Implement Scoped Git And Review Evidence

Goal:
Ground reviews in exact local source-change evidence.

Context:
Supports REQ-V11-GIT-001 through REQ-V11-GIT-005.

Requirements:
- Detect worktree, branch, commit, dirty state, and changed files.
- Distinguish item evidence from unrelated dirty files.
- Add file/line capable review evidence without storing full source contents by default.
- Document local-only PR behavior or implement verified optional PR URL support.

Files or areas likely involved:
- `src/app/review.rs`.
- `src/app/inspection.rs`.
- `src/app/surfaces.rs`.
- `tests/e2e.rs`.
- `docs/OPERATING_MODEL.md`.

Acceptance criteria:
- Unrelated dirty files do not become agent-owned evidence.
- ReviewGate evidence lists considered and excluded files.
- File and line annotations survive export/import.

Tests:
- Git fixture tests.
- Dirty-worktree review tests.

Dependencies:
- TASK-V11-001.

Do not do:
- Do not store full private source content by default.

### TASK-V11-007: Harden Distribution And Client Setup

Goal:
Make Planr installable and configurable as a polished user product.

Context:
Supports REQ-V11-DIST-001 through REQ-V11-DIST-006.

Requirements:
- Keep release curl install as the primary user path.
- Add Homebrew-ready path or explicit tap-publication condition.
- Keep Cargo/source as maintainer workflow.
- Add or document client setup for Codex, Claude Code, Cursor, explicitly opted-in Grok Build and Pi, and generic MCP.
- Add prompt/config output for CLI, MCP, and HTTP workflows.
- Document checksum verification.

Files or areas likely involved:
- `README.md`.
- `docs/INSTALL.md`.
- `docs/RELEASE.md`.
- `src/app/commands.rs`.
- `src/integrations.rs`.
- `scripts/`.

Acceptance criteria:
- Fresh users can follow public release install docs.
- Supported clients have documented setup.
- Package dry-run contains only intentional files.

Tests:
- Install dry-run tests.
- Package dry-run.
- CLI prompt/config tests.

Dependencies:
- TASK-V11-001.

Do not do:
- Do not present npm or Cargo as the primary normal-user install path unless native binaries ship through that channel.

### TASK-V11-008: Implement Templates And Review Packages

Goal:
Make proven Planr work reusable and portable.

Context:
Supports REQ-V11-TEMPLATE-001 through REQ-V11-TEMPLATE-005.

Requirements:
- Export/import graph items, links, plans, contexts, logs, canonical ReviewGate projections, and metadata.
- Add package requirements metadata and preview-before-import.
- Preserve ReviewGate attempts, findings, annotations, and file references.
- Define or implement a local-first encrypted bundle strategy.

Files or areas likely involved:
- `src/app/inspection.rs`.
- `src/app/repository.rs`.
- `src/planpack.rs`.
- `docs/IMPORT.md`.
- `tests/e2e.rs`.

Acceptance criteria:
- Fresh project can import a proven decomposition and pick work.
- Review annotations survive export/import.
- Conflicts are reported before mutation.

Tests:
- Export/import E2E tests.
- Package requirement and conflict tests.

Dependencies:
- TASK-V11-001.

Do not do:
- Do not rely on a hosted service for core package portability.

### TASK-V11-009: Update Documentation And Public Copy

Goal:
Make docs match V1.1 behavior exactly.

Context:
Supports REQ-V11-DOCS-001 through REQ-V11-DOCS-005.

Requirements:
- Update README, CLI reference, MCP contract, operating model, task graph model, install docs, testing docs, and spec files.
- Regenerate `docs/planr-spec.zip`.
- Update fixtures that validate CLI/MCP/doc drift.
- Scrub public repo paths for forbidden comparison-product names.

Files or areas likely involved:
- `README.md`.
- `docs/`.
- `docs/fixtures/mcp-contract.json`.
- `docs/planr-spec.zip`.

Acceptance criteria:
- Docs, fixtures, CLI help, MCP tools, and tests agree.
- Forbidden-reference scrub is clean.

Tests:
- Drift tests.
- Forbidden-reference scrub.

Dependencies:
- TASK-V11-002 through TASK-V11-008.

Do not do:
- Do not document features that are not implemented or explicitly marked future scope.

### TASK-V11-010: Run Full V1.1 E2E Verification

Goal:
Prove V1.1 works in-repo and as a fresh consumer install.

Context:
Uses the End-To-End Verification Contract in `V1_1_DIFFERENTIATION_CONTRACT.md`.

Requirements:
- Run all required repository checks.
- Run fresh consumer E2E in `~/projects/planr-test`.
- Include MCP, HTTP/SSE ReviewGate routes, package export/import, and docs examples.
- Convert any failure into fix and re-review tasks.

Files or areas likely involved:
- `tests/`.
- `scripts/`.
- `docs/TESTING.md`.
- `~/projects/planr-test`.

Acceptance criteria:
- Every required command passes.
- Consumer project evidence exists outside the source repo.
- Final review has enough evidence to audit every V1.1 requirement.

Tests:
- Full command list from the V1.1 contract.

Dependencies:
- TASK-V11-009.

Do not do:
- Do not use narrow smoke tests as proof of broad V1.1 completion.

## Foundation

### TASK-FND-001: Create New Planr Product Skeleton

Goal:
Create a clean Planr codebase skeleton with owned command names, docs, and implementation boundaries.

Context:
Supports REQ-PROD-002 and ADR-001.

Requirements:
- Create package metadata for `planr`.
- Create source-owned Planr files only through explicit Planr commands.
- Add README sections for product direction, local-first data, and supported agents.

Files or areas likely involved:
- `Cargo.toml` or equivalent package manifest.
- `README.md`.
- `src/`.

Acceptance criteria:
- `planr --help` runs from source.
- README no longer presents the product as Codex-only.
- No unnecessary runtime dependency owns core coordination behavior.

Tests:
- Build command for chosen stack.
- `planr --help`.

Dependencies:
- None.

Do not do:
- Do not copy unowned source code, docs, images, or command vocabulary.

### TASK-FND-002: Implement CLI Command Skeleton

Goal:
Add the primary command groups and stable output conventions.

Context:
Supports CLIENT_IMPLEMENTATION_SPEC.md.

Requirements:
- Add command groups: project, plan, map, item, pick, log, review, close, context, search, doctor, mcp.
- Add `--json`, no-color behavior, and stable error model.

Files or areas likely involved:
- `src/cli.rs`.
- `src/app/commands.rs`.

Acceptance criteria:
- Each top-level command has help output.
- Unknown commands and invalid arguments return machine-readable errors in JSON mode.

Tests:
- CLI golden help tests.
- JSON error tests.

Dependencies:
- TASK-FND-001.

Do not do:
- Do not implement business logic in command parsing.

### TASK-FND-003: Create `.planr` Project Pack Initializer

Goal:
Create a repo-local Markdown project pack with product, ownership, flows, state SSOT, constraints, and quality checks.

Context:
Creates the durable project context layer for Planr.

Requirements:
- `planr project init` creates `.planr/project/*.md`.
- Existing files are not overwritten unless `--force`.
- Starter text makes graph-vs-Markdown ownership explicit.

Files or areas likely involved:
- `src/planpack/`.
- `.planr/` templates.

Acceptance criteria:
- Empty repo gets a complete `.planr` pack.
- Existing `.planr` pack remains intact by default.

Tests:
- Init fixture tests.

Dependencies:
- TASK-FND-002.

Do not do:
- Do not add a second file-based graph SSOT beside SQLite.

## Data And Graph

### TASK-DATA-001: Implement SQLite Schema And Upgrades

Goal:
Create the local map database schema.

Context:
Supports API_AND_DATA_MODEL.md and ADR-002.

Requirements:
- Implement tables for projects, items, links, plans, source_links, contexts, runs, logs, artifacts, events.
- Store schema version.
- Enable WAL and foreign keys.

Files or areas likely involved:
- `src/storage/schema.rs`.
- `src/storage/mod.rs`.

Acceptance criteria:
- Fresh database initializes.
- Re-running init is idempotent.
- Schema version is queryable.

Tests:
- Schema upgrade tests.
- Fresh/open existing database tests.

Dependencies:
- TASK-FND-001.

Do not do:
- Do not store source file contents by default.

### TASK-DATA-002: Implement Item State Machine And Atomic Picking

Goal:
Implement item lifecycle, link-based promotion, and concurrent-safe picks.

Context:
This is the core graph product.

Requirements:
- Centralize valid state transitions.
- Promote ready items after link changes and item closure.
- Implement atomic pick with pick token and automatically derived worker id.
- Prevent parent closure while required ReviewGates remain open.

Files or areas likely involved:
- `src/core/state_machine.rs`.
- `src/core/readiness.rs`.
- `src/core/graph.rs`.

Acceptance criteria:
- Two agents cannot pick the same item.
- Closing upstream item unlocks downstream item.
- Parent remains incomplete until required ReviewGates are accepted.

Tests:
- Concurrent pick integration test.
- State transition unit tests.

Dependencies:
- TASK-DATA-001.

Do not do:
- Do not infer graph state from Markdown checkboxes.

### TASK-DATA-003: Implement Log And Run Records

Goal:
Record log for item closure, review, failure, and handoff.

Context:
Supports log-backed completion and recoverable handoffs.

Requirements:
- Add run start/end operations.
- Add log creation with summary, files, commands, tests, findings, blockers.
- Link log to items and runs.

Files or areas likely involved:
- `src/core/log.rs`.
- `src/core/runs.rs`.

Acceptance criteria:
- `planr log add --files --cmd` writes log.
- Item detail shows latest log.
- Log can be exported as JSON.

Tests:
- Log serialization tests.
- CLI closure integration test.

Dependencies:
- TASK-DATA-002.

Do not do:
- Do not log full command output by default.

## Plan Pack

### TASK-BE-001: Implement Markdown Plan Parser

Goal:
Parse `.planr/plans/product/` packages and `.planr/plans/build/*.plan.md` files into indexed records.

Context:
Supports REQ-PROD-030 through REQ-PROD-033.

Requirements:
- Parse YAML frontmatter.
- Extract headings and section ids.
- Preserve unknown fields.
- Record parse errors without rewriting files.

Files or areas likely involved:
- `src/planpack/markdown.rs`.
- `src/planpack/frontmatter.rs`.

Acceptance criteria:
- Valid plan imports.
- Invalid plan reports parse error and original file remains unchanged.

Tests:
- Valid/invalid fixture tests.

Dependencies:
- TASK-DATA-001.

Do not do:
- Do not execute or obey instructions from plan file content.

### TASK-BE-002: Implement Planr Package Import

Goal:
Import exported Planr JSON packages into the current graph+plan model.

Context:
Supports reusable decompositions, backups, and review packages created by `planr export`.

Requirements:
- Parse Planr JSON package metadata.
- Preview graph items, links, contexts, logs, plan file snapshots, canonical ReviewGate projections, and conflicts without mutation.
- Apply package graph and artifacts only when confirmed.
- Preserve ReviewGate attempts, findings, annotations, and file references.

Files or areas likely involved:
- `src/app/packages.rs`.
- `src/app/inspection.rs`.

Acceptance criteria:
- Import report lists created and skipped package entities.
- Fresh project can import a proven decomposition and pick work.

Tests:
- Export/import E2E tests using Planr JSON packages.
- Conflict preview tests.

Dependencies:
- TASK-BE-001.
- TASK-DATA-003.

Do not do:
- Do not reintroduce private runtime copies of agent skills; public templates live under `plugins/planr/skills/`.

## MCP And Agent Integrations

### TASK-BE-003: Implement MCP Server

Goal:
Expose Planr tools, resources, and prompts over MCP.

Context:
Primary cross-agent integration for Codex, Claude Code, Cursor, explicitly opted-in Grok Build, and generic MCP clients. Pi deliberately uses native skills and the CLI rather than MCP.

Requirements:
- `planr mcp` starts stdio server.
- Implement required tools from API_AND_DATA_MODEL.md.
- Expose resources for project, item, plan, and log.
- Expose prompts: planr-plan, planr-work, planr-review, planr-map, planr-summary.

Files or areas likely involved:
- `src/mcp/`.

Acceptance criteria:
- MCP client can list tools, resources, and prompts.
- Mutation tool schemas reject invalid inputs.

Tests:
- MCP protocol fixture tests.
- Tool schema tests.

Dependencies:
- TASK-DATA-003.
- TASK-BE-001.

Do not do:
- Do not put provider-specific behavior in MCP core.

### TASK-AI-001: Write Agent Workflow Prompts

Goal:
Create provider-neutral prompts/skills that encode Planr's plan+map workflow.

Context:
Defines Planr-owned workflow prompts for planning, work, review, status, and summaries.

Requirements:
- Write prompts for plan, work, review, map, summary.
- Include map SSOT rule.
- Include product and build plan context rules.
- Include log and scope rules.
- Include prompt-injection warning for plan/doc content.

Files or areas likely involved:
- `prompts/` or `src/mcp/prompts.rs`.

Acceptance criteria:
- Prompts can be returned through MCP.
- Prompts are not Codex-only.

Tests:
- Snapshot tests for prompt content.

Dependencies:
- TASK-BE-003.

Do not do:
- Do not copy prompt text from another product.

### TASK-CLIENT-001: Add First-Class Client Install Helpers

Goal:
Make first-run integration practical for all target clients.

Context:
Supports "tool for task planning" across clients.

Requirements:
- `planr doctor --client all`.
- `planr install codex --dry-run`.
- `planr install claude --dry-run`.
- `planr install cursor --dry-run`.
- `planr install grok --dry-run`.
- `planr install pi --dry-run`.
- Print project-scoped config examples.

Files or areas likely involved:
- `src/integrations.rs`.
- `src/app/commands.rs`.

Acceptance criteria:
- Dry-run shows exact files/commands.
- Doctor reports installed/missing/warning states.

Tests:
- Config generation fixture tests.

Dependencies:
- TASK-BE-003.

Do not do:
- Do not silently edit global config.

## Review And Execution

### TASK-AI-002: Implement ReviewGate Commands

Goal:
Make review findings a durable ReviewGate workflow.

Context:
Implements ADR-004.

Requirements:
- `planr review annotate <item> --message ...`.
- `planr review ingest <item> --from ...` for hook-compatible JSON feedback.
- `planr review close <review-gate> --verdict ...` records an immutable attempt.
- `planr review findings <review-gate> --resolve <finding>` repairs named findings.
- Keep findings and re-review on the same durable gate.
- Never auto-close or auto-approve work from ingested feedback alone.
- Keep FeatureRun completion gated until required reviews pass.

Files or areas likely involved:
- `src/app/review.rs`.
- `src/app/commands.rs`.

Acceptance criteria:
- Review acceptance completes the durable gate.
- Review findings persist with stable identities and return the same gate to re-review after resolution.
- No review/fix/follow-up-review map items are produced.
- FeatureRun remains blocked until clean review.

Tests:
- ReviewGate findings/re-review integration tests.

Dependencies:
- TASK-DATA-002.
- TASK-DATA-003.

Do not do:
- Do not use `failed` for ordinary review churn.

### TASK-CLIENT-002: Add Optional Codex Runner Wrapper

Goal:
Provide a first-class Codex execution path without making Codex mandatory.

Context:
This is a product differentiator for Codex users.

Requirements:
- `planr codex run [--workers N]`.
- Use `planr pick` to pick items.
- Invoke `codex exec` with bounded item context.
- Record runs and log.
- Support `codex review` integration where configured.

Files or areas likely involved:
- `src/agents/codex.rs`.

Acceptance criteria:
- One item can be picked, run through Codex, and recorded.
- Failed Codex run leaves item recoverable.

Tests:
- Mock Codex command tests.

Dependencies:
- TASK-DATA-003.
- TASK-AI-001.

Do not do:
- Do not require Codex for Claude/Cursor users.

## Security, Observability, Release

### TASK-SEC-001: Implement Content-Safe Logging And Scrubbing

Goal:
Prevent accidental storage of secrets or private content in logs.

Context:
Supports SAFETY_PRIVACY_SECURITY.md.

Requirements:
- Redact likely API keys/tokens.
- Keep prompt/response logging disabled by default.
- Add `planr scrub`.

Files or areas likely involved:
- `src/security/`.
- `src/observability/`.

Acceptance criteria:
- Secret-like strings are flagged in contexts/log.
- Debug bundle excludes forbidden content by default.

Tests:
- Secret scrubbing tests.

Dependencies:
- TASK-DATA-003.

Do not do:
- Do not promise perfect secret detection.

### TASK-ANA-001: Implement Doctor And Debug Bundle

Goal:
Provide local diagnostics for installation, database, MCP, and agent clients.

Context:
Supports production operability.

Requirements:
- `planr doctor`.
- `planr debug bundle`.
- Redacted output by default.

Files or areas likely involved:
- `src/doctor/`.
- `src/observability/`.

Acceptance criteria:
- Doctor reports database, `.planr`, Git, MCP, Codex, Claude Code, Cursor, and explicitly selected Grok Build or Pi.
- Debug bundle is reviewable before sharing.

Tests:
- Doctor fixture tests.

Dependencies:
- TASK-CLIENT-001.
- TASK-SEC-001.

Do not do:
- Do not include source files or plan bodies in debug bundles by default.

### TASK-QA-001: Build Acceptance Test Suite

Goal:
Create the test suite described in QA_ACCEPTANCE_TESTS.md.

Context:
Prevents graph, package, and schema regressions.

Requirements:
- State machine tests.
- Concurrent pick tests.
- Plan parser tests.
- MCP tests.
- Schema upgrade tests.
- Security/logging tests.

Files or areas likely involved:
- `tests/`.

Acceptance criteria:
- Acceptance suite runs locally.
- CI can run it without external agent clients.

Tests:
- This task is itself the test suite.

Dependencies:
- TASK-BE-003.
- TASK-BE-002.
- TASK-SEC-001.

Do not do:
- Do not rely on live Codex/Claude/Cursor/Grok Build/Pi for core CI tests.

### TASK-REL-001: Package And Release V1

Goal:
Ship Planr as installable local CLI.

Context:
Supports RELEASE_READINESS.md.

Requirements:
- Build release binaries.
- Generate checksums.
- Add install docs.
- Add package import/export docs.
- Add MCP integration docs.

Files or areas likely involved:
- `.github/workflows/`.
- `docs/`.
- release scripts.

Acceptance criteria:
- Fresh user can install and run `planr project init`.
- Checksums are published.
- Release notes list known limitations.

Tests:
- Packaging smoke test on macOS and Linux.

Dependencies:
- TASK-QA-001.
- TASK-ANA-001.

Do not do:
- Do not ship install script that silently edits global agent config.

### TASK-V2-BUDGET-001: Build Immutable FeatureRun Budget Authority

Goal:
Make every Planr 2.0 FeatureRun budget deterministic from run creation through release.

Requirements:
- Persist exactly one canonical budget contract atomically with each FeatureRun.
- Require explicit bounded or unbounded mode; bounded runs and admitted tasks have complete wall-seconds, tool-call, and token maxima.
- Protect exact maker, verification, review, repair, and release allocations with checked reserve and deadline arithmetic.
- Anchor wall time to persisted run start and retain per-dimension observation provenance.
- Reuse one `planr.execution_state.v2` budget projection across every public surface and host packet.
- Remove mutable-policy decisions, fabricated allowances, synthesized active-run state, and compatibility budget shapes.

Acceptance criteria:
- Policy mutation after run start cannot change contract digest or admission.
- Missing or invalid budget state holds before dispatch.
- Earlier phases cannot consume protected later-phase capacity.
- Estimated or declared values never become trusted observations.
- CLI, MCP, HTTP, and work packets expose byte-equivalent budget values and deadlines.
- Missing, invalid, and digest-mismatched active contracts produce byte-equivalent typed holds before peek or mutating dispatch.
- Explicit plan-scoped restart retires only an incompatible run, is optimistic and idempotent, preserves all history, and creates neither a contract nor a successor run.
- Explicit plan-scoped budget-hold resolution resumes only a compatible run whose existing reservation, deadline, phase, owner, and lease generation revalidate atomically; every unrepaired cause stays held.
- The next ordinary pick creates one distinct FeatureRun and exactly one v2 contract through the existing atomic run-start path.

Do not do:
- Do not make host adapters policy owners or add provider-specific budget branches.
- Do not migrate active Planr 1.x FeatureRuns or synthesize their missing budget contracts.
