---
name: planr-task-graph
description: Coordinate Planr plans, map dependencies, leases, evidence, approvals, reviews, handoffs, and interruption recovery.
---

# Planr Task Graph

Use Planr as the canonical local coordinator. Markdown plans own scope and narrative; the map is the source of truth for live items, links, picks, approvals, logs, reviews, and closure.

Evaluation subcommands run only when the user explicitly requests them, the selected item's acceptance criteria require them, or the maintainer release workflow invokes them. Never invoke them as routine or opportunistic goal, loop, or task-graph work.

## Inspect And Work

Before changes:

```bash
planr project show --json
planr map show --json
planr map lane --critical
planr map pressure
```

If missing, initialize the project and run `planr doctor --client all`.

Work one item at a time:

```bash
planr pick --json
planr done <item-id> --summary "<outcome and decisive result>" \
  --files <path> --cmd "<build or live command>" \
  --tests "<test command>" --review
```

Every closure must be evidence-backed: changed files through `--files`, commands through `--cmd`, tests through `--tests`, remaining risk through context or findings, and review outcome through `planr review`. Use plain `done` only when review has no signal. Keep longer work current with `planr pick progress`, `pause`, and `resume`.

## Plans And Dependencies

Create/refine/check the plan, expand it into independently verifiable tasks, then `planr map build --from <plan-id>`. Annotate routed work types before mapping or retag afterward.

Create ordering explicitly with `planr link add <earlier> <later> --type blocks`; readiness comes from graph state, not Markdown checkboxes. Validate generated order and remove only incorrect links. Use `planr item breakdown <parent> --into <child>...` for a parent gate; later top-level work depends on the parent, which rolls up after children settle.

## Reviews And Approvals

Request review after completion evidence. A complete review may use `planr review close <review-id> --verdict complete --close-target`. Findings use `--verdict not-complete`; Planr creates fix and follow-up review work.

Human gates use `planr approval request <item-id> --reason <reason>`. Never close with an open or denied approval.

## Handoff And Recovery

Use item notes for nearby handoff and contexts for reusable decisions. Logs are evidence, not chat summaries.

After interruption inspect Git status, map state, then:

```bash
planr trace item <item-id>
planr log list --item <item-id>
planr context list --item <item-id>
planr pick stale --older-than-seconds 900
```

Release stale ownership only after inspection. Do not claim completion until children and reviews are closed, approvals are clear, verification commands ran, findings became settled follow-up work, and the user-facing summary matches Planr state.
