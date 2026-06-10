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

## Done

Planning is complete only when `planr plan check <plan-id>` passes and the next command is clear: split further, build map, or ask the user for a blocking decision.

`plan check` rejects empty scaffolds: build plans must have content in `## Scope Decision`, `## Verification`, and `## Acceptance Criteria`; product plans must have content in `## Problem`, `## Requirements`, and `## Success Criteria` of `PRODUCT_SPEC.md`. Write those sections before checking — do not pad them to satisfy the gate.
