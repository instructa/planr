---
name: planr-plan
description: Create or refine Planr product plans and build plans before implementation. Use for app ideas, PRDs, architecture slices, scoped implementation contracts, and converting broad work into map-ready items.
---

# Planr Plan

Read and apply the canonical [Evidence ownership guard](../planr/SKILL.md#evidence-ownership-guard) before product planning.

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
- a non-empty `criteria` frontmatter list whose entries contain only a stable `id` and `title`;
- scope decision;
- ownership target;
- existing leverage;
- phases;
- out of scope;
- verification;
- acceptance criteria.

Author criterion identity only in frontmatter, for example:

```yaml
criteria:
  - id: criterion-api-health
    title: API health is observable from the target service
```

Keep the `## Acceptance Criteria` section as readable narrative, never an identity source. Do not infer criterion IDs from prose or decide obligation completeness in this skill; `plan check`, explicit Evidence migration, and the canonical `app/proof` authority own those decisions.

## Route-Aware Tagging

Before writing the task list, check whether the project declares model routing: `planr agents list --json`. If routes exist, their `work_type` selectors are the project's use-case vocabulary (e.g. `frontend`, `backend`, `design`) — and tagging is your job, not the user's; never ask a human to name work types.

Declare the use case in the task list itself — `map build` seeds annotated tasks with that work type directly:

```markdown
### TASK-001 (backend): REST API for todos
- [ ] (frontend) Build the form and list view
```

Match by the task's actual work (UI/components/styling -> a `frontend` route, API/server/storage -> `backend`, and so on); unannotated tasks stay `code`, which the default route covers. For maps that were already built without annotations, retag instead: `planr item update <item-id> --work-type frontend`. The payoff: every pick packet then carries the right profile, model, and paired skill for its use case, so dispatch needs no human routing knowledge.

## Done

Planning is complete only when `planr plan check <plan-id>` passes and the next command is clear: split further, build map, or ask the user for a blocking decision.

When the map is built, linked, and tagged, end by naming the execution handoff explicitly — the user should never have to guess the next prompt: `Use $planr-loop on plan <build-plan-id>. Stop condition: all items closed with evidence, reviews complete, canonical Evidence coverage holds.` (On hosts with a /goal primitive, `$planr-goal` wraps the same loop for long-running autonomous runs.)

`plan check` rejects empty scaffolds: build plans need a valid unique `criteria` frontmatter list plus content in `## Scope Decision`, `## Verification`, and `## Acceptance Criteria`; product plans must have content in `## Problem`, `## Requirements`, and `## Success Criteria` of `PRODUCT_SPEC.md`. Write those sections before checking — do not pad them to satisfy the gate.
