# Planr Fix Reference

Read [../planr-shared.md](../planr-shared.md) first for shared CLI coverage, live-status mutation commands, and shared `.planr` rules.

Use this reference when converting a direct user request, bug report, `planr-review` finding, or plan item into actual implementation and evidence-backed completion.

## Core Principle

A requested fix is not complete when it is acknowledged.

A requested fix is complete only when:

- the canonical code change exists
- the relevant competing path is removed or proven intentional
- the root cause is identified for bug-driven or regression-driven work
- the affected behavior is verified with real evidence
- the remaining state is recorded honestly in `.planr/status/current.json`

## Execution Checklist

For every run of `planr-fix`, check:

- Did I read the request or findings, task doc, and current `.planr/status/current.json` first?
- Did I read the governing `.planr/plans/*.plan.md` when one exists?
- Did I define the exact owned implementation scope before editing?
- Did I translate each request, finding, or failing behavior into a concrete checklist item?
- If the trigger was a bug, regression, contract failure, or unexpected side effect, did I prove the code path was intended before patching it?
- Did I identify the first unintended side effect or write and the correct owner layer?
- Did I fix the root cause at the correct owner layer?
- Did I avoid making downstream contracts or parsers more permissive unless that behavior is proven canonical?
- Did I remove leftover fallback, compatibility, shim, or dual-path behavior when the task called for a hard cut?
- Did I add or update the tests that prove the behavior?
- Did I reconcile checked plan phase items and completed `todos` against the owned diff and verification evidence before marking them complete?
- Did I record exact verification commands and actual results?
- Did I mark blocked or unverified items honestly instead of smoothing them into `done`?

Treat these as separate gates. A strong code change does not erase a missing verification step.

## `current.json` Scope Template

Use a structured status entry, not a vague summary:

```json
{
  "id": "scope-id",
  "title": "Task name",
  "status": "in_progress",
  "plan_paths": [
    ".planr/plans/example.plan.md"
  ],
  "owned_paths": [
    "path/to/file"
  ],
  "checklist": [
    {
      "id": "read-scope",
      "content": "Read the request, findings, task doc, and current implementation.",
      "status": "completed"
    },
    {
      "id": "fix-item-a",
      "content": "Fix item A in the canonical owner and remove the competing path.",
      "status": "pending"
    }
  ],
  "verification": [
    {
      "command": "<exact command>",
      "status": "pending",
      "result": ""
    }
  ],
  "blocked_or_unverified": []
}
```

After each substantial step, update the checklist to match reality.

## Plan Doc Sync

When a `.planr/plans/*.plan.md` exists:

- keep `.planr/status/current.json` as the running live status
- keep the plan phase checklist as the phase contract
- keep frontmatter `todos` as coarse summary only
- after each phase, compare the owned diff and verification output to every checked phase item
- if the evidence does not support a checked item, uncheck it or rewrite it before claiming the phase is done
- do not mark a frontmatter `todo` `completed` until its mapped phase checklist is genuinely complete

## Verification Ladder

Start with the smallest check that can really fail for the right reason, then broaden when the change surface warrants it.

Examples:

- test the smallest affected unit, file, package, or service first
- run the narrowest typecheck, compile, or build command that validates the changed surface
- run the repo's linter or format check when the touched code is covered by those gates
- for cross-boundary changes, verify each changed side, not only the producer or only the consumer

If a broader verification command is blocked by unrelated workspace failures:

- run the narrowest credible command first
- capture the exact failing broader command
- record why it is blocked
- do not convert that blocked step into a silent pass

## Evidence Rules

Good evidence:

- exact commands
- exact pass or fail results
- tests that exercise the changed behavior
- targeted searches that prove the old path is gone
- scoped diff evidence that matches the checked plan items

Weak evidence:

- "the code path looks right"
- "typecheck probably covers it"
- "the task boxes were already checked"
- "the plan phase looked basically done"
- "one nearby test passed"

## Final Response Template

```markdown
## Execution Scope

- Closed items: ...
- Owned paths: ...

## Checklist Status

- Completed: ...
- Blocked or unverified: ...

## Verification Evidence

- `<exact command>` -> passed / failed / blocked
- `<exact command>` -> passed / failed / blocked

## Remaining Blockers Or Unverified Items

- ...

## Brief Summary

The task is complete / not complete / partially verified because ...
```

## Good Habits

- Prefer the smallest honest claim over an overstated success claim.
- If an action-driven request has no prior review, write your own explicit checklist items before editing.
- If the symptom is probably downstream noise, trace the root cause before hardening parsing or validation.
- If a shared file has mixed authorship, keep the owned scope explicit.
- If the user asked for full completion, keep going until only real blockers remain.
