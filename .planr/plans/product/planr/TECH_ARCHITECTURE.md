# Technical Architecture

Eval Contract v1 is frozen in [EVAL_CONTRACT_V1.md](EVAL_CONTRACT_V1.md). Eval implementation work must preserve its ownership split: repository manifests are authored inputs, SQLite owns immutable eval evidence, and Planr map state remains authoritative for work closure.

## Architecture Goals

- REQ-ARCH-001: Keep item state, picks, worker runtime state, approval gates, links, log, reviews, and events in one local SQLite source of truth.
- REQ-ARCH-002: Keep rich product and build plan context in repo-local Markdown files that remain useful without Planr installed.
- REQ-ARCH-003: Support CLI, MCP, and optional HTTP/SSE as lenses over the same core engine.
- REQ-ARCH-004: Make Codex, Claude Code, Cursor, Grok Build, Pi, and generic MCP clients first-class integration targets.
- REQ-ARCH-005: Avoid provider-specific logic in the core graph engine.

## System Context Diagram

```text
Developer
  -> planr CLI
     -> Core engine
        -> SQLite graph database
        -> .planr Markdown project
        -> Git repository

Agent clients
  -> MCP stdio / MCP HTTP
     -> Core engine
        -> SQLite graph database
        -> .planr Markdown project

Optional dashboard
  -> Local HTTP/SSE server
     -> Core engine
```

## Component Boundaries

- `core`: map graph operations, state machine, item readiness, worker runtime state, approval gates, log, reviews, contexts, search.
- `storage`: SQLite schema, schema upgrades, transactions, FTS indexes.
- `planpack`: `.planr` project pack, plan parsing, and Markdown frontmatter.
- `cli`: user commands and deterministic output.
- `mcp`: tools, resources, prompts, capability negotiation.
- `server`: optional local REST/SSE API.
- `agents`: integration helpers for Codex, Claude Code, Cursor, Grok Build, Pi, and generic clients.
- `git`: worktree, branch, diff, and changed-file log.
- `recovery`: stale-pick detection, timeout handling, retry policy, and manual condition reporting.
- `execution_run`: FeatureRun lifecycle, outcome batching, role leases, typed escalation, durable ReviewGate attempts/findings, and final product review projection.
- `packages`: reusable JSON export/import packages with plan snapshots, logs, contexts, and durable ReviewGate projections.

## Client Architecture

- CLI is the primary client and must be scriptable.
- TUI/dashboard are optional clients over the same API.
- MCP clients must receive concise, structured responses optimized for LLM context.
- All client layers must call core services, not mutate database or Markdown directly.

## Planr 2.0 Budget Ownership

- `usage_policy`: sole shared-core owner of `FeatureRunBudgetContract`, `ExecutionBudget`, `BudgetSnapshot`, provenance, contract validation, phase protection, and checked deadline/reserve arithmetic.
- `execution_policy`: pure pre-dispatch admission over typed policy, concurrency state, and a persisted budget snapshot; it never loads storage or authored policy.
- `app/execution_run`: resolves authored policy once at run start and creates the FeatureRun plus immutable budget contract in one transaction.
- `execution_run` owns the pure incompatible-contract retirement transition; `app/execution_run` owns plan-scoped compatibility diagnosis and restart orchestration; `app/repository/execution_run` applies run, batch, lease, and provenance changes in one optimistic transaction.
- `app/feature_run_evidence`: coordinates reservation, append-only observation, reconciliation, release, and budget-hold transitions against SQLite.
- `app/execution_state`: sole `planr.execution_state.v2` projection owner for CLI, MCP, HTTP, work packets, trace, status, roles, and skills.
- Storage owns insert-only contract, reservation, and observation mechanics but no policy. Host adapters enforce supplied task maxima/deadlines and report provenance but never choose policy.

Dependencies remain one-way: surfaces and host adapters -> application services -> pure policy/admission core. Runtime budget decisions never reload `.planr/policy.toml`, synthesize missing state, or fall back to a previous execution-state shape.

CLI, MCP, and HTTP restart adapters parse only the closed `incompatible-budget` reason and return the canonical application value byte-for-byte. They never cancel healthy runs, calculate budget compatibility, or create successor runs.

Compatible budget-hold resolution is a separate `app/feature_run_evidence` lifecycle. It resumes only the exact prior phase after one immediate transaction revalidates the persisted contract/snapshot, every active reservation and deadline, the canonical role owner, and lease generation. Incompatible contracts route to restart; capability holds, corrupt or exhausted state, missing reservations, expired deadlines, and ownership mismatch remain fail-closed. CLI, MCP, and HTTP are transport-only.

Stale source-freeze restart uses the same ownership direction. `app/execution_run` is the sole plan-scoped diagnosis and orchestration owner; it admits only an active source-frozen run with a stale immutable freeze, no verification item, and stranded code outcomes. `execution_run` owns the pure terminal transition, and `app/repository/execution_run` atomically retires runtime ownership and routes those outcomes without changing the freeze row. Pick creates a distinct successor run later; Evidence readiness freezes current source. CLI and MCP are typed consumers only.

## Binding Evidence Ownership

- `app/evidence` owns repository-policy parsing and whether the policy requires binding Evidence.
- `planpack` owns the checked build-plan criterion identity list; Markdown acceptance prose is not an identity source.
- `evidence/coverage` owns authoritative active-obligation row selection and exposes typed rows; no application surface duplicates its binding/supersession query.
- `app/proof` is the sole owner of `PlanEvidenceAuthority`: `nonbinding`, `binding_unsatisfied`, or `binding_active`, derived by joining policy, the checked declared criterion set, and authoritative active obligation rows.
- FeatureRun handoff, verifier admission, audit, final-review admission, accepted-risk handoff, and stop activation consume that classification. They do not infer authority from an empty coverage list or verification logs.
- Explicit Evidence migration remains the sole obligation writer and accepts only the exact declared criterion set. An incomplete or invalid binding set becomes a durable capability hold, not a compatibility route.

## Backend Architecture

V1 is a local backend packaged into the CLI binary:

- SQLite database at `.planr/planr.sqlite` by default.
- WAL mode enabled for concurrent readers and serialized writers.
- Local service optional for dashboards and long-running agent orchestration.
- Canonical FeatureRun and ReviewGate projections are served from the localhost REST/SSE boundary.
- No cloud backend in V1.

## Data Architecture

- Map state, worker heartbeats, progress, stale ownership data, and approval gates: SQLite.
- Rich product plans, build plans, and project context: `.planr/*.md`.
- Live status summaries: SQLite.
- Search: SQLite FTS over items, contexts, plan metadata/frontmatter/headings, logs, and review summaries.
- Large artifacts: paths and metadata by default; inline content only for small explicitly provided artifacts.

## AI Architecture

Planr does not call model providers by default. It guides external agents through:

- MCP tools for map, plan, log, and review operations.
- MCP prompts for `plan`, `work`, `review`, `map`, and `summary` workflows.
- Client-specific install snippets for Codex, Claude Code, Cursor, Grok Build, and Pi.
- Optional runner wrappers for local Codex/Claude/Cursor CLIs when explicitly configured.

## Auth And Identity

- V1 local mode uses OS user access and file permissions.
- Worker identity is an explicit `worker_id` string, not an auth boundary.
- HTTP server binds to localhost by default.
- Remote HTTP mode is post-V1 and must require authentication.

## Integrations

- Codex: CLI instructions, MCP registration, optional `codex exec` runner, optional `codex review` integration.
- Claude Code: `.mcp.json` or CLI-based MCP registration guidance.
- Cursor: `.cursor/mcp.json` project config and global config guidance.
- Grok Build: native `.grok/skills/` and `.grok/agents/` plus a document-preserving project `.grok/config.toml` MCP merge. Grok is explicit opt-in, has no Planr hooks in v1, and introduces no provider runtime dependency.
- Pi: native `.pi/skills/` plus optional `pi-subagents` roles under `.pi/agents/`. Pi is explicit opt-in, uses CLI-backed skills because core Pi intentionally omits MCP, has no Planr hooks/extensions/settings in v1, and introduces no runtime dependency.
- Generic MCP: stdio first; streamable HTTP optional.
- Git: worktree isolation and scoped diff log.

## Security Architecture

- Host hooks are repository-local and optional where supported; Grok Build and Pi have no Planr hook contract in v1.
- Any command runner must show command, cwd, environment policy, and worker id.
- Secrets must not be stored in database, plans, logs, or analytics.
- MCP tools that mutate state must be separated from read-only resources/prompts.
- Destructive graph operations require preview or explicit flags.

## Privacy Architecture

- Local-only by default.
- No provider telemetry by default.
- No content logging by default.
- Export and delete commands must remove Planr database state and `.planr` artifacts when requested.

## Observability

- Local structured event log in SQLite.
- Optional JSONL debug log with content scrubbing.
- `planr doctor` for installation, database, MCP, and client integration checks.
- `planr trace item <id>` for item lifecycle debugging.

## Deployment Environments

- Local development: source checkout and debug binary.
- Local production: installed binary via package manager or release asset.
- CI: headless CLI mode with explicit db/path.
- Hosted/team: out of scope for V1.

## Failure Modes

- Database locked: retry bounded writes, then return actionable diagnostic.
- Corrupt Markdown plan: preserve file, mark parse status degraded, keep map usable.
- Missing MCP client support: print manual CLI instructions.
- Agent run interrupted: keep item picked/running with heartbeat timeout and release/re-pick command.
- Recovery sweep interrupted: preview is non-mutating; apply mutates only listed recoverable work and records events.
- Changes-requested verdict: persist findings on the same ReviewGate and return it to re-review after explicit resolution.
- Package import cancelled: preview leaves database and `.planr` files unchanged.

## Scalability Assumptions

- V1 target: hundreds of projects, tens of thousands of items, thousands of plan files per machine.
- SQLite and FTS are sufficient for V1.
- Remote multi-user concurrency is post-V1.

## Open Technical Decisions

- OD-ARCH-001: Whether to implement worktree management in core V1 or as an extension.
- OD-ARCH-002: Whether dashboard uses server-rendered HTML, TUI, or a small SPA.
