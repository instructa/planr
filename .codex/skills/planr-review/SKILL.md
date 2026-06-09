---
name: planr-review
description: Review agent-owned implementation scope in this repository against `.planr` plans or live status, path-scoped Git evidence, and acceptance criteria. Use for findings-first audits of completion, correctness, architecture, hard-cut cleanup, and test sufficiency. Not for implementing fixes (`planr-fix`), writing a new execution contract (`planr-plan`), or giving a verdict-only status answer (`planr-status`).
---

# Planr Review

Use this skill to audit whether scoped work is actually done and done cleanly.

A `planr-review` is invalid if it skips explicit owned scope or skips a path-scoped Git comparison for that scope.

## CLI-First Rule

- Read [../planr-shared.md](../planr-shared.md) first.
- Use `./.planr/tooling/planr status show [--scope ...]` for live scope context before dropping to raw `.planr/status/current.json`.
- There is no `planr.py review` command today. The actual verdict still requires path-scoped Git evidence plus direct inspection of code and tests.

## Owned Review Scope

Before reviewing any implementation, define the exact `owned review scope`.

- The default scope is only the files and hunks the agent changed in this task, or the exact files and hunks the user explicitly asked to review.
- Never assume the whole dirty worktree belongs to the review.
- If the repo is dirty and the agent cannot distinguish its own changed paths with high confidence, stop and ask for the review scope.
- If the same file contains both agent and non-agent edits, only review the agent-owned hunks when they can be isolated from conversation context, an explicit patch, or a known base commit.
- If mixed authorship inside a file cannot be isolated, do not guess. Mark the review `unclear / partially verified` or ask for a clean comparison base.
- Adjacent files may be read for context, but findings and completion claims must stay limited to the owned review scope.

## Deterministic Git Evidence Gate

Before producing findings or a completion verdict:

- define the `owned review scope`
- run one or more path-scoped Git comparison commands for that scope
- record the exact command text that was used
- inspect the diff before reading implementation details deeply enough to form conclusions

Allowed Git evidence shapes:

- `git diff -- <owned paths>` for working-tree reviews
- `git diff --cached -- <owned paths>` when staged changes matter
- `git diff <base>...HEAD -- <owned paths>` for branch or multi-commit review
- `git show <commit> -- <owned paths>` for committed change sets

Rules:

- Never rely on a repo-wide diff when unrelated dirty files may exist.
- Never return `complete` or `not complete` without quoting the exact path-scoped Git command(s) used.
- If the owned scope is ambiguous or the Git evidence cannot be isolated deterministically, downgrade the verdict to `unclear / partially verified` or ask the user for a cleaner base.

## Required Inputs

- the governing `.planr/plans/*.plan.md` when one exists
- the relevant live scope entry from `.planr/status/current.json`
- any explicitly referenced historical source doc still in scope
- the path-scoped Git diff for the owned review scope
- the changed implementation files and relevant tests in that scope

## Required Workflow

1. Read the governing plan and live scope context first.
2. Define the exact owned review scope before reading implementation details deeply.
3. Run and record one or more path-scoped Git comparison commands for that scope.
4. Inspect every owned changed file and relevant test through the scoped diff plus direct file reads.
5. Check acceptance criteria, checked phase items, completed `todos`, and live-status claims against the actual diff and recorded verification.
6. Review architecture, hard-cut cleanup, correctness, and test sufficiency.
7. Produce findings-first output and persist a `.planr/review/*.review.md` artifact only when it adds execution value.

## Output Format

Use this structure:

- `Findings`
- `Completion verdict`
- `Review scope and Git evidence`
- `Open questions or assumptions`
- `Brief summary`

Rules for the response:

- Findings come first, ordered by severity.
- Each finding should explain why the implementation fails the task, architecture, hard-cut direction, correctness, or cleanliness standard.
- The completion verdict must say one of:
  - `complete`
  - `not complete`
  - `unclear / partially verified`
- The `Review scope and Git evidence` section must list the exact owned paths and the exact Git comparison command(s) used.
- If shared files required hunk-level review, say whether the owned hunks were isolated or remained ambiguous.
- If a plan doc was in scope, say whether its checked phase items were confirmed or overclaimed.
- If a persisted review artifact was created, include its path.
- If there are no findings, say that explicitly and still mention residual risks or testing gaps.
- Keep the summary short. The findings are the main output.

## Additional Resource

- Read [../planr-shared.md](../planr-shared.md) first for shared CLI coverage and shared `.planr` rules.
- For the detailed review checklist, hard-cut checks, and report templates, see [reference.md](reference.md)
