# Planr Plan Reference

Read [../planr-shared.md](../planr-shared.md) first for shared CLI coverage, plan scaffolding rules, and shared `.planr` rules.

Use this reference when creating or updating a repo-local execution plan under `.planr/plans/*.plan.md` that later agents must actually execute and verify.

This is the canonical planning reference for both general execution plans and richer task-shaped tracked artifacts. Do not split those into separate planning workflows.

## Core Principle

The plan is not a motivational checklist.

It is an execution contract that later work must reconcile against:

- the owned repo diff
- the recorded verification commands and results
- the acceptance criteria

If a plan item cannot be checked against repo evidence later, it is too vague now.

## Project Context Pack

Before finalizing architecture-sensitive planning, confirm that the repo already has equivalent durable context or run `./.planr/tooling/planr project init` to scaffold the bootstrap pack under `.planr/project/`:

- `product.md`
- `ownership.md`
- `flows.md`
- `state-ssot.md`
- `constraints.md`
- `quality-gates.md`

After `project init`, inspect the actual target codebase and rewrite those files so ownership, product, flow, state, and quality-gate guidance matches the repo instead of generic starter text.

`.planr/status/current.json` is the global live status source. It does not replace the scoped plan document.

## Frontmatter Template

Use this shape:

```yaml
---
name: short plan name
overview: One-paragraph overview of the chosen direction and why it is separate from adjacent work.
todos:
  - id: phase-1-summary
    content: Short summary of the first major workstream.
    status: pending
  - id: phase-2-summary
    content: Short summary of the second major workstream.
    status: pending
isProject: false
---
```

Rules:

- `todos` are a coarse summary, not the execution truth
- do not mark a todo `completed` until the mapped phase checklist and verification evidence are complete
- if the surrounding plan consumer expects only `pending` and `completed`, keep blocked detail in the markdown body

## Body Template

```markdown
# Plan Title

## Source

- Optional for task-shaped plans: bug doc, review finding, or user task source

## Why this task exists

- Optional for task-shaped plans: why this deserves tracked implementation work

## Scope Decision

- This plan covers ...
- It does not expand into ...
- Chosen direction: ...

## Ownership Target

- `Runtime owner`: ...
- `First fix owner`: ...
- `Canonical long-term owner`: ...
- Wrong competing owners: ...

## Existing Leverage

- `path/to/file`: existing owner or leverage point
- `path/to/test`: existing coverage or missing proof surface
- Current conflict: ...

## Hard-Cut

- Optional for task-shaped plans: canonical path, removed path, and whether a dead concept must be deleted end-to-end

## Phase 1: <short name>

Phase goal:

- One short paragraph explaining what becomes true after this phase.

Tasks:

- [ ] Make the first concrete change in the canonical owner.
- [ ] Remove or narrow the competing path if this phase requires a hard cut.
- [ ] Add or update the smallest tests or docs needed to prove the phase.
- [ ] Reconcile this phase against the owned repo diff and recorded verification before marking it complete.

Phase verification:

- `git diff -- <owned paths>`
- `<targeted test command>`

## Phase 2: <short name>

Tasks:

- [ ] ...

## Out Of Scope

- ...

## Verification

- Phase-local:
  - `<command>`
- End-state:
  - `<command>`
  - `<command>`

## Acceptance Criteria

- ...
- ...

## Relevant Files

- Optional for task-shaped plans: `path/to/file`

## Notes

- Optional for task-shaped plans: clarifications that should survive into execution
```

Rules:

- `## Source`, `## Why this task exists`, `## Hard-Cut`, `## Relevant Files`, and `## Notes` are the default extension when the user wants a tracked task artifact, bug conversion, review-finding follow-up, or hard-cut implementation contract
- For simpler planning requests, omit these sections instead of creating a second planning skill

## Phase Checklist Rules

Every phase checklist item should answer one of these:

- what code or ownership move must happen
- what old path must be removed or narrowed
- what proof must exist before the phase can be claimed complete

Avoid checkboxes like:

- `refactor things`
- `clean up`
- `verify later`
- `finish remaining issues`

For task-shaped plans, also avoid:

- `follow bug doc`
- `handle review comments`
- `hard-cut leftovers`

Name the actual implementation outcome instead.

## Existing Leverage Checklist

Before finalizing the plan, check:

- Did I read the current owner layer?
- Did I read the likely destination layer if ownership will move?
- Did I cite concrete files or symbols instead of describing imaginary architecture?
- Did I capture the existing tests or identify the missing proof surface?

## Phase-End Reconciliation Checklist

This is the anti-false-completion gate.

At the end of every phase, the executing agent should be able to answer:

- Which owned paths changed for this phase?
- Which exact diff or hunk closes each checked box?
- Which exact verification command proves each checked box?
- Which boxes stayed unchecked because the code or proof is still missing?
- Which frontmatter `todos` can honestly move to `completed` now?

If those answers are fuzzy, the phase is not truly closed.

## Acceptance Criteria Checklist

Good acceptance criteria:

- describe end-state behavior or architecture
- mention the canonical owner when ownership matters
- can be disproved by code inspection or targeted tests

Weak acceptance criteria:

- summarize effort instead of end state
- repeat phase names
- rely on checked boxes as evidence

## Good Habits

- Prefer an extra phase over a bloated phase with ambiguous boxes.
- Keep `Out Of Scope` explicit so later agents do not grow the task while claiming progress.
- If a requirement cannot be verified later, make it more concrete now.
- Write the plan so `planr-fix` and `planr-review` can both use it without reinterpretation.
- When the plan comes from a bug, task, or review finding, preserve the source context but rewrite it into an execution contract instead of copying the original text verbatim.
