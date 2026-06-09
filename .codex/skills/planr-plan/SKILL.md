---
name: planr-plan
description: Create or update executable `.planr/plans/*.plan.md` contracts in this repository. Use when scope, ownership, phase breakdown, verification, or acceptance criteria must be defined before implementation, including bug-to-plan conversions and review-finding follow-up plans. Not for executing fixes (`planr-fix`), giving a verdict-only status answer (`planr-status`), or running a findings-first audit (`planr-review`).
---

# Planr Plan

Use this skill to write an execution contract, not a design memo.

A `planr-plan` is invalid if it skips the real code, omits per-phase checklists, leaves verification vague, or treats completion as something later agents can assume instead of prove.

## CLI-First Rule

- Read [../planr-shared.md](../planr-shared.md) first.
- If `.planr/project/*.md` is missing or still generic starter text, run `./.planr/tooling/planr project init`, then inspect the target repo and rewrite those files before making strong architecture or ownership decisions.
- Use `./.planr/tooling/planr plan new ...` to scaffold new plan files instead of hand-writing boilerplate.
- There is no general plan-update command today. After scaffolding, edit the existing plan body directly.

## Required Inputs

- the user request, task doc, bug doc, or review finding that the plan is based on
- the existing implementation files likely to own the change
- adjacent tests and architecture docs when ownership or boundaries matter
- nearby `.planr/plans/*.plan.md` examples when creating a new plan shape from scratch

## Required Plan Shape

Create or update a file under `.planr/plans/*.plan.md` with:

- frontmatter fields:
  - `name`
  - `overview`
  - `todos`
  - `isProject`
- body sections:
  - `## Scope Decision`
  - `## Ownership Target`
  - `## Existing Leverage`
  - `## Phase 1: ...` and later phases as needed
  - `## Out Of Scope`
  - `## Verification`
  - `## Acceptance Criteria`

Prefer `Acceptance Criteria` as the canonical heading. Do not rely on only `Success Criteria`.

Global live status belongs in `.planr/status/current.json`. The plan file stays the scoped execution contract.

## File Placement And Naming

- Store tracked plan artifacts in `.planr/plans/`
- Use the canonical filename shape `<slug>_<hash>.plan.md`
- Prefer the shared `.planr` CLI to scaffold the initial file when practical
- Do not create new numbered `docs/tasks/*.md` files
- If converting an existing `docs/tasks/*.md` artifact, promote one `.planr/plans/*.plan.md` successor and delete the old task doc only when the user explicitly asked for the hard cut

## Task-Shaped Plan Extensions

When the user explicitly wants a tracked task artifact, a bug-to-plan conversion, a review-finding follow-up, or a hard-cut implementation contract, `planr-plan` should produce the richer task-shaped variant rather than inventing a separate planning skill.

In that case, include these sections unless the user explicitly asked for a smaller variant:

- `## Source`
- `## Why this task exists`
- `## Hard-Cut`
- `## Relevant Files`
- `## Notes`

These sections support the same `.planr` execution contract; they do not replace `Scope Decision`, `Ownership Target`, phase checklists, verification, or acceptance criteria.

## Phase Rules

Every phase must contain a tickable checklist in the body using `- [ ]`.

Each checkbox must be:

- concrete
- falsifiable
- narrow enough that it can be proved with code or tests
- written so a reviewer can compare it against the repo diff

Do not:

- pre-check boxes in a new plan
- combine multiple meaningful outcomes into one checkbox
- hide verification as an implied final step
- use vague boxes like `clean up leftovers` without naming what that means
- leave a hard-cut deletion ambiguous between "remove the old value" and "delete the dead concept"; the phase checklist must name which one is intended

## Core Rules

- State what the plan covers and what competing interpretations were rejected.
- Read the real code before writing phases.
- Cite concrete files, symbols, and tests under `Existing Leverage`.
- Separate `Runtime owner`, `First fix owner`, and `Canonical long-term owner`.
- If the work removes a legacy mode, enum, config key, or selector option, decide explicitly whether the surrounding concept still has more than one meaningful current-state behavior.
- Treat frontmatter `todos` as summary state only. The phase checklist plus verification evidence is the real completion bar.

## Required Workflow

1. Start from the source request and inspect the real owner layers before planning.
2. If this is a new tracked plan, scaffold it with `./.planr/tooling/planr plan new ...`.
3. Write `Scope Decision`, `Ownership Target`, `Existing Leverage`, phase checklists, `Out Of Scope`, `Verification`, and `Acceptance Criteria`.
4. For task-shaped work, add `Source`, `Why this task exists`, `Hard-Cut`, `Relevant Files`, and `Notes` when they help later execution.
5. Make every checkbox concrete, falsifiable, and reviewable against repo evidence.
6. Make verification commands and blocked or unverified conditions explicit enough that later `planr-fix` and `planr-review` runs do not need to reinterpret the contract.

## Additional Resource

- Read [../planr-shared.md](../planr-shared.md) first for shared CLI coverage and shared `.planr` rules.
- For the full template, reconciliation gate, and planning checklists, see [reference.md](reference.md)
