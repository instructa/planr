# Flows

High-level flows planning work should respect. Adapt examples to **your** stack (web, CLI, mobile, backend, etc.).

## Change flow

1. User or ticket defines scope.
2. Read `.planr/project/` context when architecture matters.
3. Add or update a scoped plan under `.planr/plans/*.plan.md` when the work needs a contract.
4. Implement against owned paths only.
5. Record verification and update `.planr/status/current.json` when tracking execution.

## Plan → execute → verify

- **Plan**: explicit scope, ownership, phases, acceptance criteria.
- **Fix**: concrete changes + tests/docs.
- **Review**: Git-scoped evidence that the contract is satisfied.

## Status vs review vs fix

- **Status**: smallest honest verdict right now.
- **Review**: did the implementation satisfy the task correctly?
- **Fix**: what concrete work closes the gap?

Do not merge these flows silently.

## Boundaries

Prefer one canonical path per concern, explicit handoffs between layers, and a single owner per policy decision.

## Plan Notes

Derive from your repo’s docs (e.g. architecture overview, ADRs) when present.
