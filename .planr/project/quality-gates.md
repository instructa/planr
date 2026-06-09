# Quality Gates

This file defines the minimum bar for claiming progress or completion in planr workflows.

## Completion Contract

Do not report success until every requested item is:

- completed
- explicitly blocked
- or explicitly unverified

Status files and plan todos are summary state, not proof.

## Verification Rules

- run the smallest relevant verification first
- record the exact command and actual result
- if a broader command is blocked by unrelated failures, say so explicitly
- do not convert blocked or skipped verification into a silent pass

## Evidence Rules

Strong evidence includes:

- exact test or verification commands
- exact pass, fail, blocked, or unverified results
- scoped diff evidence
- exact path-scoped Git comparison commands for review verdicts
- searches that prove the competing path is gone
- for hard-cut removals, proof that the dead concept itself is gone from live contracts and owner layers, not only that an old value is rejected

Weak evidence includes:

- checked boxes without proof
- "looks done"
- one nearby happy-path test
- large diffs without scoped verification
- a review verdict without an exact path-scoped Git comparison command
- a reject-test for a deleted legacy value when the surrounding one-value setting, enum, or binding still exists

## Default Validation Commands

Use the smallest affected scope first, then broaden when warranted.

Customize this section for the target repository. Good defaults are:

- the smallest relevant unit or integration test command for the owned scope
- the narrowest typecheck, compile, or build command that validates the changed surface
- the repo's linter or static-analysis command for the affected package, module, or file set
- the repo's format check when formatting is part of the gate
- a targeted smoke check or manual repro command when the change is user-visible or operational

Record the exact commands and outcomes in the plan, status, or review artifact instead of assuming another repo's stack.

## Status Tracking Rules

- `.planr/status/current.json` is the canonical live planr status source
- `.planr/plans/*.plan.md` contains scoped execution contracts
- material persisted reviews may live under `.planr/review/*.review.md`, but `current.json` remains the summary layer rather than a duplicate full review log
- after each substantial step, update the live status source honestly
- after interruption or resume, read `.planr/status/current.json` and `git status` before continuing

## Plan Notes

This file is derived from:

- `AGENTS.md`
- repo user rules for test execution
