# Planr Status Reference

Read [../planr-shared.md](../planr-shared.md) first for shared CLI coverage, status mutation commands, and shared `.planr` rules.

Use this reference when answering whether the current scope in this repo is actually done, which scopes are still open, or what open scope should come next.

## Core Principle

Status is not a vibe.

It is a scope verdict backed by evidence.

A scope is `complete` only when:

- the scope is explicit
- every in-scope item is done
- the recorded verification supports those claims
- no in-scope blockers or unverified items remain

## Status Questions

Answer these in order:

1. What exact scope is being judged?
2. What plan, task, or `.planr/status/current.json` scope defines done?
3. What verification evidence already exists?
4. What is still pending?
5. What is blocked or unverified?
6. Is this really a status question, or has it turned into `planr-fix`, `planr-review`, or `planr-plan` work?

Queue-style variants:

- Which scopes are still open?
- Which open scope should come next?
- Is the next thing implementation, review, or plan work?

## Status-Scope Checklist

For every run of `planr-status`, check:

- Did I define the exact scope first?
- Did I identify the governing `.planr/status/current.json` scope?
- Did I read the relevant `.planr/plans/*.plan.md` file when one exists?
- Did I exclude unrelated dirty paths?
- Did I inspect the recorded verification?
- Did I inspect a path-scoped Git diff if the recorded evidence was stale or insufficient?
- Did I decide whether this should escalate to `planr-fix`, `planr-review`, or `planr-plan` instead?

## Verdict Rules

Use:

- `complete`
  - only when every in-scope item is done, the evidence is recorded, and no in-scope blockers or unverified steps remain
- `in progress`
  - when concrete work remains and the next step is available now
- `blocked`
  - when the next required step is stopped by an external dependency, unrelated failure, missing prerequisite, or ambiguity that needs user input
- `unclear / partially verified`
  - when the scope looks close but key evidence, scope isolation, or ownership certainty is missing

Do not call a scope `complete` just because:

- the diff is large
- `.planr/status/current.json` has optimistic checklist state
- a plan frontmatter `todo` says `completed`
- one happy-path test passed
- the code "looks done"

## Escalation Rules

Recommend:

- `planr-fix`
  - when the missing work is concrete, in scope, and implementation should continue
- `planr-review`
  - when the status question requires fresh findings about correctness, hard-cut cleanup, ownership, or code quality
- `planr-plan`
  - when the scope, phase contract, or acceptance criteria are missing or too vague to judge completion
- `none`
  - when the scope is actually complete

## Evidence Hierarchy

Strong evidence:

- the governing `.planr/status/current.json` scope entry
- the governing `.planr/plans/*.plan.md` section when one exists
- exact verification commands and recorded pass, fail, blocked, or unverified results
- path-scoped Git diff evidence when status depends on unreviewed changes

Weak evidence:

- checked boxes without verification
- `todos` marked `completed`
- "looks finished"
- repo-wide impressions from a dirty worktree

## Read-Only Queue Commands

When the user asks queue-style status questions, start with the shared CLI:

```bash
./.planr/tooling/planr status open
./.planr/tooling/planr status next
```

Interpretation:

- `status open` lists scopes that are still open by status or that have status drift, such as a closed scope with unfinished checklist, blocker, or failed verification state.
- `status next` selects the next deterministic open scope, preferring actionable `in_progress`, then `pending`, then `blocked` scopes before falling back to a closed scope with status drift.

## Reusable Response Template

```markdown
## Status Scope

- Current scope: ...
- Governing plan or task: ...
- Owned paths: ...

## Status Verdict

`in progress`

Reason:

- ...

## Evidence Checked

- `<exact command>` -> passed / failed / blocked / not run
- `<exact command>` -> passed / failed / blocked / not run

## Completed

- ...

## Remaining

- ...

## Blocked Or Unverified

- ...

## Recommended Next Skill

- `planr-fix`: ...

## Brief Summary

The scope is / is not complete because ...
```

## Good Habits

- Prefer the smallest honest verdict over a flattering one.
- If one blocked command is unrelated and genuinely out of scope, do not let it erase otherwise complete in-scope proof.
- If one blocked or unverified step is still in scope, `complete` is off the table.
- If the scope question becomes "is this code actually correct and clean?", escalate to `planr-review`.
- If the scope question is "what should we do next among existing scopes?", stay in `planr-status` and use the queue commands first.
- If the scope question is "what new work should exist that is not yet represented by a scope or plan?", escalate to `planr-plan`.
