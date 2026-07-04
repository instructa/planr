---
name: planr-plan
description: Create or refine Planr product plans and build plans before implementation. Use for app ideas, PRDs, architecture slices, scoped implementation contracts, and converting broad work into map-ready items.
---

# Planr Plan

Use this when scope, ownership, acceptance criteria, or verification must be defined before implementation.

## Workflow

```bash
planr project show --json
planr plan new "App idea" [--platform web] [--ai] [--backend]
planr plan refine <plan-id> --note "decision, constraint, or assumption"
planr plan check <plan-id>
planr plan split <plan-id> --slice "narrow implementation slice"
planr map build --from <build-plan-id>
```

## Product Plan Standard

A product plan package must include:

- manifest;
- product spec;
- UX flows;
- design system;
- architecture;
- ADRs;
- AI spec when relevant;
- safety/privacy/security;
- API/data model;
- client and backend implementation specs;
- observability;
- QA;
- release readiness;
- executable tasks;
- references.

## Build Plan Standard

A build plan must include:

- source plan;
- scope decision;
- ownership target;
- existing leverage;
- phases;
- out of scope;
- verification;
- acceptance criteria.

## Route-Aware Tagging

Before `map build`, check whether the project declares model routing: `planr agents list --json`. If routes exist, their `work_type` selectors are the project's use-case vocabulary (e.g. `frontend`, `backend`, `design`) — and tagging is your job, not the user's; never ask a human to name work types. `map build` seeds every item as `code`, so after building the map, retag each item whose work matches a declared route:

```bash
planr agents list --json          # route selectors = the use-case vocabulary
planr map build --from <build-plan-id>
planr item update <item-id> --work-type frontend   # per item that matches a route
```

Match by the item's actual work (UI/components/styling -> a `frontend` route, API/server/storage -> `backend`, and so on). Items matching no route keep `code` — the default route covers them. The payoff: every pick packet then carries the right profile, model, and paired skill for its use case, so dispatch needs no human routing knowledge.

## Done

Planning is complete only when `planr plan check <plan-id>` passes and the next command is clear: split further, build map, or ask the user for a blocking decision.

`plan check` rejects empty scaffolds: build plans must have content in `## Scope Decision`, `## Verification`, and `## Acceptance Criteria`; product plans must have content in `## Problem`, `## Requirements`, and `## Success Criteria` of `PRODUCT_SPEC.md`. Write those sections before checking — do not pad them to satisfy the gate.
