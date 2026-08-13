---
name: planr-task-graph
description: Coordinate Planr plans, map dependencies, leases, evidence, approvals, reviews, handoffs, and interruption recovery.
---

# Planr Task Graph

Read and apply the canonical [Evidence ownership guard](../planr/SKILL.md#evidence-ownership-guard) before product graph preparation or making.

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
  --tests "<test command>"
```

Continue only from the typed packet you were authorized to perform. Makers consume `work_packet.kind: "outcome"`; `mode: "finding_repair"` repairs named findings on the same ReviewGate without a fix item, and `kind: "hold"` stops. Read lifecycle state only from `work_packet.execution_state`.

Require `planr.execution_state.v2`. Its budget projection is opaque; skills must not recompute budget policy or replace supplied consumed, reserved, protected, available, provenance, digest, or deadline values.

Every closure must be evidence-backed: changed files through `--files`, commands through `--cmd`, tests through `--tests`, remaining risk through context or findings, and review outcome through `planr review`. Plain `planr done` is the standard outcome settlement. Branch on the returned `work_packet.transition` and `work_packet.kind`: continue only for another compatible outcome; stop for a ReviewGate, verification packet, hold, incompatible ownership or scope, or an empty pick. Use `--escalate <reason>` only for an allowed intentional override, always with `--escalation-ref <stable-reference>` and `--escalation-explanation <why-the-override-is-required>`; never use escalation to replace computed materiality. Keep longer work current with `planr pick progress`, `pause`, and `resume`.

## Plans And Dependencies

Create/refine/check the plan, expand it into independently verifiable tasks, then `planr map build --from <plan-id>`. Annotate routed work types before mapping or retag afterward.

Create ordering explicitly with `planr link add <earlier> <later> --type blocks`; readiness comes from graph state, not Markdown checkboxes. Validate generated order and remove only incorrect links. Use `planr item breakdown <parent> --into <child>...` for a parent gate; later top-level work depends on the parent, which rolls up after children settle.

## Reviews And Approvals

Plain `planr done` settles outcome evidence and lets FeatureRun materiality open or reuse a durable ReviewGate. Close a leased gate with `planr review close <review-gate-id> --verdict complete --reviewer <reviewer-id>`. Findings use `--verdict not-complete`; Planr appends an attempt and durable findings to that same gate. The responsible maker repairs the named findings and resolves them with `planr review findings <review-gate-id> --resolve <finding-id>` before re-review.

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
