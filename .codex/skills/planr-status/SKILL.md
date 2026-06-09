---
name: planr-status
description: Assess the honest current state of a scoped `.planr` task in this repository. Use when the user asks what is done, what remains, what is blocked, whether a scope is complete, which scopes are open, or what should come next. Start with the deterministic `.planr` CLI where it has command coverage. Not for implementing fixes (`planr-fix`) or running a findings-first audit (`planr-review`).
---

# Planr Status

Use this skill to answer "are we actually done with this scope?" without silently turning a status check into implementation work or a full audit.

A `planr-status` is invalid if it skips explicit scope, trusts checked boxes without evidence, or says `complete` while in-scope blockers or unverified items remain.

## CLI-First Rule

- Read [../planr-shared.md](../planr-shared.md) first.
- Start with `./.planr/tooling/planr status show`, `open`, and `next` before reading raw status JSON by hand.
- Only use `ensure-scope`, `set-checklist`, `set-blocker`, or `set-verification` when the user explicitly asked for status reconciliation and the command surface fits.
- Drop to plan files, path-scoped Git diff, and implementation files only when the CLI output and recorded verification are insufficient.

## Required Inputs

- the user's status question
- the governing live scope entry
- the governing `.planr/plans/*.plan.md` when one exists
- any explicitly referenced historical source doc still in scope
- the recorded verification commands and results for the candidate scope

## Core Rules

- Define an explicit `status scope` before judging completion.
- Prefer the smallest honest verdict over an optimistic one.
- Do not treat checked boxes, completed `todos`, status entries, or a large diff as proof.
- Never treat unrelated dirty files as part of the status scope.
- If an in-scope item is pending, blocked, or unverified, the scope is not `complete`.
- If the task requires hard-cut cleanup, leftover fallback, compatibility, shim, or dual-path behavior means the scope is not `complete`.
- If the task removed a mode, value, or setting but left a one-value enum, one-option selector, or dead pass-through field with no remaining current-state meaning, the scope is not `complete`.
- If the work is bug-driven and root-cause proof is missing, do not upgrade the status beyond `unclear / partially verified`.
- Default to read-only status assessment.
- If deciding status requires fresh findings about correctness, architecture, or hard-cut quality, escalate to `planr-review` instead of inventing a verdict from intuition.

## Status Verdicts

Use exactly one:

- `complete`
  - every in-scope item is done, verified, and consistent with the governing plan or task contract
- `in progress`
  - concrete owned work still remains, but the next step is available now
- `blocked`
  - the next required step cannot proceed because of an external dependency, unrelated failure, ambiguous ownership requiring user input, or another prerequisite outside the scope
- `unclear / partially verified`
  - the work may be close, but scope isolation, verification evidence, or ownership is ambiguous or incomplete

## Required Workflow

1. Start from the user's scope question, not from the whole worktree.
2. Use the CLI first:
   - `status open` or `status next` for queue-style questions
   - `status show [--scope ...]` for scope-state questions
3. Define the exact `status scope`:
   - current scope entry in `.planr/status/current.json`
   - mapped plan phases and acceptance criteria when one exists
   - owned paths or hunks when known
   - excluded dirty paths
4. Gather the minimum evidence needed:
   - current checklist states from the live scope entry
   - governing plan phase checklists and frontmatter `todos`
   - recorded verification commands and results
   - path-scoped diff or direct file inspection when the recorded evidence is stale or insufficient
5. Ask the status questions:
   - what must be true for this scope to count as done?
   - which items are complete with evidence?
   - which items remain pending?
   - which items are blocked or unverified?
   - do any checked boxes, completed `todos`, or status entries overclaim the actual evidence?
6. Choose the smallest honest verdict.
7. Recommend the next skill:
   - `planr-fix` when concrete owned work remains
   - `planr-review` when completion depends on fresh findings or a stronger audit
   - `planr-plan` when the scope or phase contract is missing or too vague
   - `none` when the scope is actually complete
8. If the user asked to synchronize status, update live state only after the evidence supports it, and prefer the CLI over hand-editing.

## Output Format

Use this structure:

- `Status scope`
- `Status verdict`
- `Evidence checked`
- `Completed`
- `Remaining`
- `Blocked or unverified`
- `Recommended next skill`
- `Brief summary`

Rules for the response:

- Quote exact verification or Git compare commands when they matter to the verdict.
- Say which plan phase or `.planr/status/current.json` items support the verdict.
- If the scope is not complete, name the narrowest missing step.
- If recommending another skill, say why.

## Additional Resource

- Read [../planr-shared.md](../planr-shared.md) first for shared CLI coverage and shared `.planr` rules.
- For verdict rules, queue-command interpretation, and response templates, see [reference.md](reference.md)
