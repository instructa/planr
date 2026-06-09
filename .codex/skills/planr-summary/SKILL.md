---
name: planr-summary
description: Produce a user-facing recap of an owned `.planr` scope in this repository. Use when the user asks what changed, why it changed, what works now, what intentionally no longer works, or what remains blocked after `planr-fix`, `planr-status`, or `planr-review`. Not for deciding completion (`planr-status`), running a findings-first audit (`planr-review`), or continuing implementation (`planr-fix`).
---

# Planr Summary

Use this skill when the user wants the end result in plain language, not a verdict-only status report or a findings-first audit.

A `planr-summary` is invalid if it invents outcomes, hides blockers, collapses into `planr-status`, or turns into a findings list that belongs in `planr-review`.

## CLI-First Rule

- Read [../planr-shared.md](../planr-shared.md) first.
- Start with `./.planr/tooling/planr status show [--scope ...]` for live scope context before reading raw status JSON.
- There is no summary command today. Build the recap from the recorded `.planr` evidence, then drop to plan files, diff, and code only when the recorded state is insufficient.

## Required Inputs

- the user's summary request
- the live scope entry
- the governing `.planr/plans/*.plan.md` when one exists
- any explicitly referenced historical source doc still in scope
- the recorded verification commands and results for the scope

## Core Rules

- Define an explicit `summary scope` before writing anything.
- Stay read-only by default.
- Summarize only evidence-backed changes and outcomes.
- Distinguish clearly between:
  - what changed
  - why it changed
  - what works now
  - what was intentionally removed or no longer works
  - what remains blocked or unverified
- Do not turn blocked or unverified items into celebratory success language.
- Do not claim an intentional removal unless the plan, diff, or recorded evidence supports it.
- When a concept was deleted end-to-end, say that plainly; do not summarize it as merely "old values now fail" unless the boundary itself intentionally still exists in the current product shape.
- Do not restate every diff hunk; synthesize the result into user-facing language.
- If completion itself is unclear, say so in the summary and recommend `planr-status`.
- If correctness, cleanup quality, or architecture placement are in question, recommend `planr-review`.
- Do not auto-run this skill. Use it only when the user explicitly wants the recap.

## Required Workflow

1. Start from the user's summary request, not from the whole worktree.
2. Define the exact `summary scope`:
   - current scope entry in `.planr/status/current.json`
   - governing plan phases and acceptance criteria when one exists
   - owned paths or hunks when known
   - excluded dirty paths
3. Gather the minimum evidence needed:
   - checklist and verification state from `.planr/status/current.json`
   - governing plan phase items and `todos`
   - exact verification commands and results
   - path-scoped diff or direct file inspection when the recorded summary inputs are stale or incomplete
4. Synthesize the result into the required output sections.
5. Choose the smallest honest next step:
   - `none` when the summary can simply stand on its own
   - `planr-fix` when concrete owned work still remains
   - `planr-status` when the missing question is completion verdict or scope state
   - `planr-review` when the missing question is correctness, architecture, or hard-cut quality
6. Keep the result narrative and user-facing, but still quote exact evidence where it materially supports the summary.

## Output Format

Use this structure:

- `Summary scope`
- `What changed`
- `Why`
- `What works now`
- `What intentionally no longer works`
- `What remains blocked or unverified`
- `Evidence checked`
- `Recommended next skill`
- `Brief end result`

Rules for the response:

- `What changed` should describe the net result, not a file-by-file changelog.
- `Why` should explain the purpose of the change in one short paragraph or a few flat bullets.
- `What works now` should only include evidence-backed outcomes.
- `What intentionally no longer works` should explicitly say `none` when nothing was intentionally removed in scope.
- `What remains blocked or unverified` should explicitly say `none` only when that is true.
- `Evidence checked` should quote exact commands when verification materially supports the recap.
- `Recommended next skill` should be on-demand guidance, not an automatic workflow jump.
- `Brief end result` should read like the final takeaway a user actually wanted to understand.

## Additional Resource

- Read [../planr-shared.md](../planr-shared.md) first for shared CLI coverage and shared `.planr` rules.
- For the full checklist and response template, see [reference.md](reference.md)
