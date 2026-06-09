---
name: planr-fix
description: Implement remaining scoped work in this repository to verified completion. Use for direct bug fixes, regressions, failing tests, `planr-review` findings, or unfinished `.planr` plan phases when the next step is to change code, tests, or docs, keep live `.planr` status honest, and prove the result. Not for writing a new execution contract (`planr-plan`), giving a verdict-only status answer (`planr-status`), or running a findings-first audit (`planr-review`).
---

# Planr Fix

Use this skill when the next job is to finish real implementation work, not just describe it.

A `planr-fix` is invalid if it claims completion without owned scope, honest live status, root-cause reasoning for bug-driven work, and exact verification evidence.

## CLI-First Rule

- Read [../planr-shared.md](../planr-shared.md) first.
- Use `./.planr/tooling/planr status show`, `ensure-scope`, `set-checklist`, `set-blocker`, and `set-verification` for `.planr` state whenever they fit.
- There is no `planr.py fix` command today. Use normal repo tools for implementation, tests, and scoped verification.

## Required Inputs

- the trigger: user request, failing test, bug doc, task note, or `planr-review` finding
- the governing `.planr/plans/*.plan.md` when one exists
- any explicitly referenced historical source doc still in scope
- the exact implementation files and tests for the owned scope

## Core Rules

- Convert every in-scope request, finding, or failing behavior into a real fix or an explicit blocked or unverified item.
- For bugs, regressions, contract failures, unexpected requests, restore or hydration issues, and hidden writes, debug root-cause first:
  - state the expected behavior and invariant
  - trace the call path
  - identify the canonical source of truth and the first unintended side effect or write
- Fix the correct owner layer, not only the downstream symptom.
- Do not make contracts, parsers, or validation more permissive unless that behavior is proven canonical.
- Apply the hard-cut policy by default: remove fallback, compatibility, shim, dual-path behavior, and one-value shells unless transition support was explicitly requested.
- Checked phase items, completed plan `todos`, and live-status entries are claims until the owned diff and verification evidence prove them.

## Required Workflow

1. Start from the concrete trigger, not from the diff.
2. Define the owned implementation scope and exclude unrelated dirty paths.
3. Use the CLI to ensure or inspect the live scope entry before major edits when the command surface fits.
4. Translate each in-scope issue into explicit checklist items with owned scope and required proof.
5. Implement the canonical fix in the correct layer, including hard-cut cleanup where required.
6. Update or add the smallest proof that can fail for the right reason.
7. After each substantial step, sync checklist, blocker, and verification state through the CLI when supported.
8. Reconcile checked plan items, completed `todos`, and live-status claims against the owned diff and actual verification before marking anything complete.
9. Return completed work separately from blocked or unverified work, quoting exact commands and real results.

## Output Format

Use this structure:

- `Execution scope`
- `Checklist status`
- `Verification evidence`
- `Remaining blockers or unverified items`
- `Brief summary`

Rules for the response:

- Say which findings, direct requests, or failing behaviors were closed.
- If the work started from a bug or regression, name the root cause and the layer fixed.
- Quote the exact verification command for each completed claim.
- If something could not be verified, say that explicitly rather than smoothing it over.
- Keep the summary short and factual.

## Additional Resource

- Read [../planr-shared.md](../planr-shared.md) first for shared CLI coverage and shared `.planr` rules.
- For the full execution checklist, verification ladder, and response template, see [reference.md](reference.md)
