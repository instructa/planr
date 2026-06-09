# Product Specification

## Vision

Planr is the planning and coordination layer coding agents are missing: a local-first system that turns an app idea into a production plan, narrows it into a build plan, and runs the work on a dependency-aware map with log-backed review.

## Product Promise

Planr turns broad product ideas and coding work into a coherent flow: product plan -> build plan -> map -> pick -> log -> review/evidence -> recovery/package -> close. It prevents duplicate work through atomic picks and makes closure auditable through logs, reviews, verification, and explicit recovery.

## Target Users

- Individual developers running one or more coding agents locally.
- Power users coordinating Codex, Claude Code, Cursor, Gemini CLI, or custom MCP agents in the same repo.
- Teams that want repo-local planning artifacts before adopting a hosted workflow.
- Agent builders who need a small coordination primitive for local or CI-based agent workers.

## Problems Solved

- Agents lose context across sessions, compaction, and handoffs.
- Flat todo lists cannot model dependency order or safe parallelism.
- Markdown plans have rich context but weak live status and no atomic picking unless connected to a map.
- Issue trackers are too human-centric for fast, mid-flight agent replanning.
- Agent completions often lack proof: no files, commands, review, or blocked-state record.

## Product Principles

- Local first: the repository plus a local database should be enough.
- Product plans capture intent, build plans capture implementation context, and the map coordinates live work.
- Log over optimism: completion is proven, not declared.
- Cross-agent by default: Codex, Claude Code, Cursor, and MCP clients are peers.
- Hard-cut bias: avoid duplicate sources of truth and transitional shells.
- Human-readable artifacts: all important plans and decisions must be inspectable without a proprietary UI.

## V1 Scope

- A `planr` CLI.
- A local SQLite map graph with items, links, picks, contexts, artifacts, logs, reviews, runs, and events.
- A `.planr/` repo pack for plans, project context, review artifacts, and skill/prompt templates.
- MCP server exposing tools, resources, and prompts for Claude Code, Cursor, Codex, and compatible clients.
- Optional HTTP/SSE local server for dashboard and automation clients.
- Codex, Claude Code, and Cursor install/config helpers.
- Import of existing `.planr` data.
- Export/import of map graph and Markdown plan packs.
- Explicit recovery sweeps for stale, timed-out, and retryable work.
- Scoped Git/PR review evidence and a local browser review workspace.
- Reusable template packages with preview-first import.
- Prompt output for CLI, MCP, and HTTP agent setup without hidden global config edits.

## Explicit Non-Goals

- REQ-PROD-001: V1 must not require a cloud account, hosted database, or network service.
- REQ-PROD-002: V1 must not depend on unowned coordination-layer packages or hosted services.
- REQ-PROD-003: V1 must not be a general project management SaaS.
- REQ-PROD-004: V1 must not store full agent transcripts by default.
- REQ-PROD-005: V1 must not privilege one vendor as the only supported workflow.

## User Personas

- Solo operator: runs Codex and Claude Code in one repo and wants clean handoffs.
- Multi-agent power user: launches several workers and wants atomic picking plus conflict visibility.
- Reviewer: audits whether an item is actually closed against a plan, diff, log, and tests.
- Toolsmith: wants MCP/HTTP primitives to embed Planr in another agent system.

## Product Flow

The canonical Planr flow is:

```text
idea -> product plan -> build plan -> map -> pick -> log -> review/evidence -> recovery/package -> close
```

- Idea: raw user request, startup concept, feature request, bug, refactor, or product slice.
- Product plan: broad product/spec package for app ideas and major initiatives.
- Build plan: focused implementation contract for a buildable slice.
- Map: live dependency graph of executable items.
- Pick: atomic assignment of one ready item to one agent.
- Log: proof bundle for implementation, verification, review, or handoff.
- Review: approval or audit condition that blocks closure until satisfied.
- Evidence: scoped Git, PR URL, file, command, test, and artifact proof attached to the item.
- Recovery: explicit preview/apply operation for stale, timed-out, retryable, or condition-gated work.
- Package: reusable local export/import bundle for graph state, plans, logs, contexts, and review artifacts.
- Close: log-backed completion of an item or parent slice.

## Core Objects And Vocabulary

- Project: one repository or multi-root project tracked by Planr.
- Plan: Markdown artifact under `.planr/plans/` with an internal stage such as `product`, `build`, or `review`.
- Map: live graph for work items, links, picks, reviews, log, and status.
- Item: graph node with status, work type, owner, acceptance summary, and optional plan links.
- Link: directed relationship between items; blocking or non-blocking.
- Context: project-wide discovery, decision, constraint, or pattern.
- Log: proof bundle produced when an agent implements, verifies, reviews, or hands off work.
- Run: one agent execution attempt against an item.
- Review: approval node or policy requiring log before closure.

## Core User Journeys

- Initialize Planr in a repo and configure Codex, Claude Code, and Cursor.
- Create a plan from a broad app idea or PRD request.
- Convert product plan slices into build plans.
- Seed map items from a plan.
- Pick and execute the next ready item.
- Run multiple agents concurrently without duplicate picks.
- Add log with files, commands, tests, and result summary.
- Review an item against its plan and create fix/review follow-up items when needed.
- Inspect item-scoped Git evidence and optional PR URL context before approving work.
- Recover safely after interruption with explicit stale-pick and retry sweeps.
- Export a reusable package/template and preview import before mutating a project.
- Resume after interruption and see the current map, active plans, and blockers.

## Feature Requirements

### Initialization

- REQ-PROD-010: `planr project init` must create `.planr/`, `.planr/project/`, `.planr/plans/`, `.planr/reviews/`, and a local database without overwriting user content unless `--force` is provided.
- REQ-PROD-011: `planr project init --client codex|claude|cursor|all` must print or apply integration instructions for the selected client.
- REQ-PROD-012: Initialization must detect existing `.planr` data and offer import commands.

### Product Plans

- REQ-PROD-020: Product plans must support PRD/product spec, UX flows, design system plan, technical architecture, ADRs, AI spec where relevant, safety/privacy/security, API/data model, implementation specs, QA, release readiness, executable task checklist, and references.
- REQ-PROD-021: Product plan generation must ask only blocking questions and mark assumptions explicitly.
- REQ-PROD-022: Product plan requirements must use stable IDs and testable language.
- REQ-PROD-023: Product plan work lists must be convertible into build plans or map candidate items, but must not automatically become live map commitments without user/agent selection.

### Build Plans

- REQ-PROD-030: Build plans must support frontmatter, source, scope decision, ownership target, existing leverage, phases, verification, acceptance criteria, out-of-scope, and notes.
- REQ-PROD-031: A build plan may be linked to one or more map items.
- REQ-PROD-032: The project context pack must preserve product, ownership, flows, state SSOT, constraints, and quality checks.
- REQ-PROD-033: Plan closure claims must be reconciled against map item state and log.

### Map Planning

- REQ-PROD-040: Map items must support statuses: pending, ready, picked, running, in_review, blocked, closed, closed_partial, failed, cancelled.
- REQ-PROD-041: Links must support hard blocking order and soft contextual relationships.
- REQ-PROD-042: Item readiness must be computed from map graph state, not from Markdown checkboxes.
- REQ-PROD-043: Picking a ready item must be atomic across concurrent agents.
- REQ-PROD-044: Parent items must not close until required child code, fix, and review items are closed.

### Agent Execution

- REQ-PROD-050: Planr must provide agent-specific prompts or MCP prompts for Codex, Claude Code, and Cursor.
- REQ-PROD-051: Runs must record worker id, client, model/profile when available, item id, command surface, start/end time, and result status.
- REQ-PROD-052: Item closure must require or allow a log entry with files changed, tests run, commands run, result summary, and blocked/unverified items.
- REQ-PROD-053: Review findings must create fix items rather than failing ordinary code items.
- REQ-PROD-054: `planr prompt` must expose CLI, MCP, and HTTP operating instructions without editing global configuration.

### Search And Recall

- REQ-PROD-060: Planr must search items, plans, contexts, logs, and review artifacts.
- REQ-PROD-061: Picking an item must surface relevant upstream context and linked plan sections.
- REQ-PROD-062: Recovery sweep must preview stale, timed-out, retryable, and condition-gated work before applying mutations.
- REQ-PROD-063: Review evidence must distinguish item-scoped changed files from unrelated dirty worktree files.
- REQ-PROD-064: Package import must be preview-first and confirmed explicitly.

## Plans, Tiers, Monetization

- V1 is an open-source local tool.
- Commercial hosted sync, team dashboards, or enterprise policy packs are explicitly post-V1.

## Integrations

- Codex CLI and Codex MCP configuration.
- Claude Code MCP project/user configuration.
- Cursor MCP project/global configuration.
- Generic MCP clients via stdio and optional streamable HTTP.
- Git worktrees and Git diff logs.
- Optional CI invocation for verification tasks.

## Content, Moderation, And Safety Boundaries

Planr handles developer artifacts and may reference private code. It must minimize stored content and avoid collecting prompts, responses, source file contents, secrets, or private transcripts unless the user explicitly enables retention.

## Success Metrics

- A broad request can become a product plan, build plan, and map items without manual file editing.
- Two or more agents can pick independent items without collision.
- A reviewer can determine what changed, why, what was verified, and what remains.
- A repo can be resumed after interruption using only Planr state and Git state.
- A fresh consumer project can prove CLI, MCP, HTTP, review workspace, recovery, package, and installer behavior without relying on maintainer-only state.

## Analytics Constraints

- V1 analytics are local diagnostics only.
- No source code, prompt text, response text, file contents, secrets, or private plan body text may be sent to analytics.

## Open Decisions

- OD-PROD-001: Final implementation language: Rust is assumed, but Go remains viable.
- OD-PROD-002: Whether the local HTTP server ships in V1 or behind a feature flag.
- OD-PROD-003: Whether Codex-specific worker orchestration is V1 or V1.1.
