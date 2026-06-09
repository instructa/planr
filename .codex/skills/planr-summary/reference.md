# Planr Summary Reference

Read [../planr-shared.md](../planr-shared.md) first for shared CLI coverage, live-scope lookup rules, and shared `.planr` rules.

Use this reference when producing a user-facing recap of what an owned planr scope accomplished.

## Core Principle

A summary is not a verdict and not a review.

It is a narrative recap backed by scope and evidence.

A good `planr-summary`:

- stays inside the owned scope
- explains what changed and why
- makes the working end result clear
- makes intentional removals explicit
- keeps blocked or unverified items visible

## Summary Checklist

For every run of `planr-summary`, check:

- Did I define the exact summary scope first?
- Did I read the matching scope entry in `.planr/status/current.json`?
- Did I read the governing `.planr/plans/*.plan.md` file when one exists?
- Did I read the task doc when one exists?
- Did I inspect the recorded verification commands and results?
- Did I inspect a path-scoped Git diff if the recorded evidence was stale or insufficient?
- Did I separate `what changed` from `why`?
- Did I list only evidence-backed items in `what works now`?
- Did I identify intentional removals only when the evidence makes them explicit?
- Did I keep blocked or unverified items visible instead of smoothing them into success language?
- Did I choose the right next skill instead of turning the summary into status or review?

## Next-Skill Guide

Recommend:

- `none`
  - when the recap is sufficient and no immediate follow-up is needed
- `planr-fix`
  - when concrete owned work still remains and implementation should continue
- `planr-status`
  - when the missing question is completion verdict or current scope state
- `planr-review`
  - when the missing question is correctness, architecture, hard-cut cleanup, or findings

## Reusable Response Template

```markdown
## Summary Scope

- Current scope: ...
- Governing plan or task: ...
- Owned paths: ...

## What Changed

- ...

## Why

- ...

## What Works Now

- ...

## What Intentionally No Longer Works

- none / ...

## What Remains Blocked Or Unverified

- none / ...

## Evidence Checked

- `<exact command>` -> passed / failed / blocked / not run
- `<exact command>` -> passed / failed / blocked / not run

## Recommended Next Skill

- `none`: ...

## Brief End Result

...
```

## Good Habits

- Prefer plain language over internal implementation jargon when the user-facing meaning is clear.
- Prefer the net result over a file-by-file inventory.
- If the summary starts drifting into a verdict, switch to `planr-status`.
- If the summary starts drifting into findings, switch to `planr-review`.
- If the summary would hide real remaining work, stop and say so explicitly.
