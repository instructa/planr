# Planr Review Reference

Read [../planr-shared.md](../planr-shared.md) first for shared CLI coverage, live-scope lookup rules, and shared `.planr` rules.

Use this reference when reviewing whether an implementation really satisfies a task doc or execution plan in the current repository.

## Core Principle

Do not review the code in isolation.

Review the implementation against:

- the task doc
- `.planr/status/current.json`
- the execution plan under `.planr/plans/*.plan.md` when one exists
- the source bug doc, if one exists
- the architecture docs
- the actual changed code and tests

## Owned Review Scope

Define the exact `owned review scope` before doing any review work.

- Review only the files and hunks the agent changed in this task, or the exact files and hunks the user explicitly put in scope.
- Never treat the whole dirty worktree as the review target by default.
- If the repo has parallel edits, constrain the review to explicit paths with Git pathspecs.
- Adjacent files can be read for context, but findings and completion claims must stay inside the owned review scope.
- If the same file contains mixed agent and non-agent edits and you cannot isolate the owned hunks from a known base, explicit patch, or conversation context, the review is not fully verifiable.

## Git Diff Gate

A `planr-review` is incomplete without a Git comparison limited to the owned review scope.

At minimum:

- inspect `git diff -- <owned paths>` for working-tree reviews
- inspect `git diff --cached -- <owned paths>` when staged changes matter
- inspect `git diff <base>...HEAD -- <owned paths>` or `git show <commit> -- <owned paths>` for committed change sets

Rules:

- Never rely on a repo-wide diff when unrelated dirty files may exist.
- Use the scoped diff to enumerate every owned changed file before reading implementation details.
- If any owned changed file or hunk was not covered by the scoped diff plus direct file inspection, do not return `complete`.
- Quote the exact Git comparison command(s) in the review output.
- If you cannot deterministically isolate the owned scope in Git, do not return `complete` or `not complete`; use `unclear / partially verified` or ask for a cleaner comparison base.
- If ownership is ambiguous inside a shared file, use `unclear / partially verified` or ask for a cleaner comparison base.

## Optional Review Artifact

Persist a review under `.planr/review/<slug>.review.md` only when it adds execution value, for example:

- the user asked for a saved review artifact
- the review produced findings that will drive follow-up `planr-fix` work
- the review is a gate for a larger hard-cut, PR, or multi-phase task
- the review will likely be revisited in a `fix -> review -> fix` loop

Do not persist a review artifact for tiny clean reviews or scopes that are still ambiguous.

If you create one:

- reuse the same findings-first structure as the response
- include the exact Git comparison command(s)
- keep `.planr/status/current.json` as summary state only rather than duplicating the full review there

## Task-Completion Checklist

For every review, check:

- Did I inspect the governing `.planr/plans/*.plan.md` when one exists?
- Did I establish the owned review scope before evaluating the task?
- Did I inspect a path-scoped Git diff for every owned changed file?
- Did the implementation satisfy the `Summary`?
- Did checked plan phase items, completed `todos`, and `.planr/status/current.json` claims match the owned diff and recorded verification?
- Did it implement the `Canonical decisions` as written?
- Did it respect `Out of scope`?
- Did it do the required `Hard-Cut` cleanup?
- Did it satisfy each `Acceptance criteria` item?
- Did it leave any required work only implied rather than actually implemented?

Treat these as separate checks. Do not let one successful area hide failure in another.

## Architecture Checklist

When the task touches architecture or ownership, verify:

- the implementation lives in the correct runtime layer
- the implementation lives in the correct package or module boundary (per project conventions)
- reusable policy is not stranded in the wrong layer (per `.planr/project/ownership.md`)
- integration-specific behavior stays in integration boundaries
- UI or session-local state was not promoted to durable SSOT without an explicit task decision
- thin runtime or glue layers did not quietly become the long-term owner of reusable domain policy

## Hard-Cut Checklist

Look for:

- old fallback paths that still exist
- compact shims preserving old behavior
- compatibility bridges or temporary adapters
- dual-write or dual-read behavior
- silent fallback or silent coercion
- dead code paths left after the new path was introduced
- leftover comments like `temporary`, `compat`, `legacy`, `fallback`, `bridge`, `adapter`, `shim`, `repair`, `best effort`

If any of these remain, decide whether they are:

- justified by the task's current-state design
- explicitly surfaced and intentional
- or a violation of the hard-cut direction

## Correctness And Cleanliness Checklist

Check for:

- correctness and behavioral regressions
- incomplete propagation of the new policy
- inconsistent naming or semantics across layers
- hidden writes or unexpected side effects
- suspicious complexity or over-compact abstraction
- missing or weak error handling
- missing targeted tests
- tests that prove only the happy path but not the task's architectural promises

## Completion Verdict Rules

Use:

- `complete`
  - only when the acceptance criteria are satisfied, no material architecture or hard-cut gaps remain, and the owned review scope was verified through a scoped Git comparison
- `not complete`
  - when one or more acceptance criteria are unmet, or when the implementation violates the task's required architecture or hard-cut direction
- `unclear / partially verified`
  - when the code looks close but key criteria are unverified because tests, behavior, runtime evidence, or ownership of the changed hunks are missing or ambiguous

Do not call something complete just because:

- the code compiles
- the diff is large
- the task checklist has boxes checked
- the plan phase checklist has boxes checked
- a frontmatter `todo` says `completed`
- one happy-path test passes

## Findings Categories

Useful labels for findings:

- `Task completion gap`
- `Architecture violation`
- `Hard-cut violation`
- `Correctness risk`
- `Code smell`
- `Missing test or verification`

## Reusable Report Template

```markdown
## Findings

- High: [Task completion gap] Acceptance criterion `<criterion>` has no proving implementation because ...
- Medium: [Architecture violation] Reusable policy still lives in `path/to/file` even though the task requires ...
- Medium: [Hard-cut violation] Old fallback path still exists in `path/to/file` and competes with the new canonical path.
- Low: [Missing test or verification] There is no coverage proving ...

## Completion Verdict

`not complete`

Unmet or unverified task requirements:

- `Acceptance criteria`: ...
- `Hard-Cut`: ...
- `Canonical decisions`: ...

## Review Scope And Git Evidence

- Owned paths: `path/to/file_a`, `path/to/file_b`
- Git compare used: `git diff -- path/to/file_a path/to/file_b`
- Scope note: no unrelated dirty files were included in the verdict
- Review artifact: `.planr/review/example.review.md` (optional)

## Open Questions Or Assumptions

- Assumed ...
- Could not verify ...

## Brief Summary

The implementation moves in the right direction but does not yet complete the task because ...
```

## No-Findings Template

```markdown
## Findings

- No blocking findings.

## Completion Verdict

`complete`

## Review Scope And Git Evidence

- Owned paths: `path/to/file_a`, `path/to/file_b`
- Git compare used: `git diff -- path/to/file_a path/to/file_b`
- Scope note: review limited to agent-owned changes in the listed paths
- Review artifact: none

## Open Questions Or Assumptions

- Could not fully verify runtime behavior X without executing Y.

## Brief Summary

The implementation appears to satisfy the task, matches the intended architecture, and does not leave obvious hard-cut leftovers. Residual risk is limited to ...
```

## Good Review Habits

- Establish the owned review scope before reading the diff.
- Quote the exact Git comparison used for the verdict.
- Quote the exact task requirement that failed.
- If a plan doc was in scope, say whether its checked items were confirmed or overclaimed.
- Prefer concrete evidence over broad impressions.
- If the task says user-visible behavior must exist, verify the user-visible surface.
- If the task says a policy must move layers, verify the old layer no longer owns it.
- If the task claims cleanup, search for leftovers rather than assuming they are gone.
