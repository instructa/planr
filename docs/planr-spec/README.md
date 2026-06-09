# Planr Specification Package

Generated: 2026-06-09

## Purpose

This package defines Planr as a production-grade, local-first planning and execution coordination tool for coding agents. Planr combines product plans, build plans, and a live dependency-aware map with reviewable logs and integration surfaces for Codex, Claude Code, Cursor, and other MCP-capable agents.

## Package Contents

- PRODUCT_SPEC.md: product scope, flow, users, features, non-goals, and requirements.
- UX_FLOWS.md: CLI, MCP, and optional dashboard flows.
- DESIGN_SYSTEM_SPEC.md: UI and TUI design direction.
- TECH_ARCHITECTURE.md: system architecture and core ownership boundaries.
- ADRS.md: major product and architecture decisions.
- AI_SPEC.md: agent behavior, prompts, context, and evals.
- SAFETY_PRIVACY_SECURITY.md: data, privacy, security, and tool execution boundaries.
- API_AND_DATA_MODEL.md: project, plan, map, item, log, review, and API contracts.
- CLIENT_IMPLEMENTATION_SPEC.md: CLI, TUI, editor, and agent-client surfaces.
- BACKEND_IMPLEMENTATION_SPEC.md: local service, MCP server, HTTP/SSE, and storage implementation.
- ANALYTICS_OBSERVABILITY_SPEC.md: no-content telemetry, local logs, and diagnostics.
- QA_ACCEPTANCE_TESTS.md: acceptance suites and regression strategy.
- RELEASE_READINESS.md: packaging, install, upgrade, and release checks.
- V1_1_DIFFERENTIATION_CONTRACT.md: V1.1 product differentiation requirements, acceptance criteria, and final verification contract.
- TASKS.md: executable implementation tasks for coding agents.
- REFERENCES.md: sources and source-derived assumptions.

## Global Assumptions

- V1 is a local-first developer tool, not a hosted SaaS.
- The first implementation may be a Rust CLI and local daemon using SQLite.
- Codex, Claude Code, and Cursor should all work through standard CLI and MCP integration paths.
- Existing `.planr` data may be imported, but the V1 CLI, data model, and docs define the final product API.
- Planr should ship with its own name, code, docs, and architecture.

## Global Non-Goals

- Do not build cloud accounts, billing, team sync, or hosted dashboards in V1.
- Do not depend on unowned coordination-layer code.
- Do not make the map graph the only planning artifact; product and build plans are first-class.
- Do not store full prompts, private code, or agent transcripts by default.
- Do not make Codex the only supported client.

## Critical Invariants

- The map database is the source of truth for item state, links, picks, approvals, reviews, and closure.
- Product plans are the source of PRD, architecture, UX, security, QA, and release context.
- Build plans are the source of implementation-level scope, ownership, acceptance criteria, and narrative decisions.
- Agent closures require log: changed files, commands run, review outcome, and remaining blockers.
- Multiple agents must not pick the same ready item.
- Every external command/tool execution path must be explicit, inspectable, and attributable.
- Parent items are completion gates. Executable code, test, and review work should live in child items, and downstream work should depend on the parent gate when review cleanliness matters.
- Story logs and handoff documents explain narrative history only. They never override map state, logs, reviews, or plan acceptance criteria.

## How Coding Agents Should Use This Package

1. Read PRODUCT_SPEC.md, TECH_ARCHITECTURE.md, API_AND_DATA_MODEL.md, and TASKS.md first.
2. Use UX_FLOWS.md and CLIENT_IMPLEMENTATION_SPEC.md when implementing the CLI, TUI, MCP prompts, or dashboard.
3. Use AI_SPEC.md for agent prompt, context, and review-loop behavior.
4. Treat TASKS.md acceptance criteria as the build checklist.
5. Keep implementation aligned with ADRS.md unless a new ADR supersedes a decision.

## Operating References

- `../OPERATING_MODEL.md`: daily operator flow, parent gates, completion rules, and recovery.
- `../TASK_GRAPH_MODEL.md`: map objects, readiness, links, picks, reviews, and evidence.
- `../HANDOFFS_AND_STORIES.md`: log, context, note, story, and handoff policy.
