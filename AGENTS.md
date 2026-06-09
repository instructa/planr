# Planr Agent Instructions

Planr is being shaped into a local-first planning and execution coordination tool for Codex, Claude Code, Cursor, and generic MCP clients.

Use the specification package in `docs/planr-spec/` as the product source of truth for new implementation work:

- `PRODUCT_SPEC.md` for scope and non-goals.
- `TECH_ARCHITECTURE.md` for ownership boundaries.
- `API_AND_DATA_MODEL.md` for project, plan, map, item, log, review, and API contracts.
- `TASKS.md` for executable coding-agent tasks.

Core rules:

- Build Planr as a self-owned product with original implementation, docs, and naming.
- Use the public product flow: idea -> product plan -> build plan -> map -> pick -> log -> review/evidence -> recovery/package -> close.
- Keep map graph state authoritative for item status, links, picks, reviews, approvals, and completion.
- Keep product and build plans first-class for rich scope, ownership, verification, and narrative context.
- Require log-backed closure: files changed, commands run, tests/review results, and blocked or unverified items.
- Support Codex, Claude Code, and Cursor as peers through MCP and CLI workflows.
