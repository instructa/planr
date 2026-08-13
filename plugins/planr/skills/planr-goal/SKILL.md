---
name: planr-goal
description: Prepare a long-running Planr goal as a checked plan, linked map, durable contract, and loop handoff without implementing it.
---

# Planr Goal

Read and apply the canonical [Evidence ownership guard](../planr/SKILL.md#evidence-ownership-guard) before goal preparation.

Prep only. Compile intent into durable Planr state, then stop. During goal prep, do not implement, refactor, verify, or touch product code. Allowed work is reading Planr state, clarifying material ambiguity, creating/refining/checking plans, building/linking the map, storing the contract, and printing the loop handoff.

Evaluation subcommands run only when the user explicitly requests them, the selected item's acceptance criteria require them, or the maintainer release workflow invokes them. Never invoke them as routine or opportunistic goal, loop, or task-graph work.

## Intake

Classify the request:

- specific: compile directly;
- vague: ask at most two questions about outcome and proof, then record labeled assumptions;
- existing plan: preserve its steps, files, and constraints as refinement notes;
- recovery: inspect `planr map show --json` and `planr recover sweep` before creating anything.

Capture a goal oracle: the observable test, browser flow, executed CLI, or real API response that proves the outcome runs.

## Compile

```bash
planr project show --json
planr plan new "<goal>" --platform <platform>
planr plan refine <plan-id> --note "<constraint or assumption>"
planr plan check <plan-id>
planr map build --from <plan-id>
```

Fill required plan sections directly. Replace the placeholder task with independently verifiable `TASK-00n` slices before `planr map build`. A small coherent change is one implementation item plus one signal-bearing independent review; do not split mechanical stages into separate implementation/review pairs. Larger scopes still use multiple slices where ownership, dependencies, or independently observable outcomes genuinely differ. Preserve real execution order with `blocks` links. When registry routes use `work_type`, annotate tasks before mapping or retag them afterward; this is prep work, not a user question.

When the repository provides a versioned verification policy and source-bound receipt runner, make that policy the verification owner in the plan. Record the selected profile, exact receipt path/digest, source revision, and the command that validates the receipt. Do not enumerate broad suites independently in every task when the policy already selects them.

## Durable Contract

For plans with binding Evidence, require the repository to define the observation obligations and capability policy before execution, then run readiness. Store one contract per plan:

```bash
planr evidence readiness --scope plan --id <plan-id>
planr context add "GOAL CONTRACT <plan-id>: DONE when every in-scope item is closed with implementation evidence, all reviews are complete, approvals are clear, and canonical Evidence coverage proves <goal oracle>. Iteration budget: 10." --tag goal-contract
```

Never weaken it mid-run. Workers use `planr pick --plan <plan-id>`; termination uses `planr plan audit <plan-id> --json`. Reviews are required only where they add signal; evidence-backed setup work may close directly. Where deployment is in scope, the contract must retain human deployment approval and a bounded live oracle against the deployed result.

## Hand Off

Print: `Use $planr-loop on plan <plan-id>. Recover the goal-contract from Planr and continue until its audit holds or the iteration budget is exhausted.` Ask whether to start, refine, or stop; then stop. Execution belongs to the loop driver.
