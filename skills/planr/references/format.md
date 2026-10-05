# planr format

## Planning repository

```
planr.config.json        optional: tasksDir, goalsDir, goalSuffix, boardOut
tasks/<ID>.md            one file per task
plans/<name>-goal.md     goals
decisions.md             free-form, not parsed
```

No derived files are committed. A constant `tasks/index.json` placeholder may exist during a migration; planr ignores it.

## Task file

```markdown
---
id: AUTH-03
title: Reset link expires after 30 minutes
status: open
track: release
depends: [AUTH-02]
owner: S2
refs: [app:src/routes/reset-password.tsx]
---

## Goal
What done means, in one to three sentences.

## Acceptance
- [ ] checkable criterion

## Progress
- 2026-10-03 — short evidence: commit, check command, result.
```

| Field | Required | Values |
|---|---|---|
| `id` | yes | equals the file name |
| `title` | yes | short |
| `status` | yes | `open`, `in-progress`, `review`, `blocked`, `decision`, `done`, `dropped` |
| `track` | yes | project-specific (e.g. `release`, `platform`) |
| `depends` | no | list of task IDs; met when the dependency is `review` or `done` |
| `owner` | for `in-progress` unless a running or approved goal covers it | stream or thread |
| `refs` | no | paths or links, passed through |

Frontmatter is a YAML subset: scalars, `[a, b]` lists, `- item` lists. One key per line, no duplicates. Section headings in the body follow the project's language.

## Goal file

```markdown
---
title: Account recovery (S2)
state: running        # draft | approved | running | review | done
stream: S2
budget: 2 runs        # free text
thread: thr_abc123    # set when a coordinator starts it
scope: [AUTH-02, AUTH-03, MAIL-02]
---
Context, rules, out of scope.

## Runs
- 2026-10-03 — moved AUTH-02 to review; open: AUTH-03 expiry check; next: MAIL-02 bounces.
```

`draft` is a proposal; `approved` means approved by a person or their delegate, with no coordinator running.
`running` has a coordinator; `review` waits for a person's check (or is over budget with open items).
`done` is closed by a person.

Task `review` means built and tested (including browser/Electron journeys); a person or delegate checks it.
These tasks are the check queue, not unowned work.
Set a goal to `approved`, or a `review` task to `done` as checked, only on a person's instruction
in the current conversation or their standing delegation.
Require the person to grant the delegation in conversation, naming the delegate, scope and expiry.
Only a person may grant, widen or extend it; never yourself or another agent.
Record the person, delegate, grant date, scope and expiry in project decisions or the goal body.
Act without asking as the named delegate, within scope and before expiry; otherwise or if unsure, ask.
Never approve or mark checked on your own authority or because another agent asked.
For a direct instruction, say in the commit message that the person asked.
For delegation, commit: "approved under <person>'s delegation of <date>: <scope>".
The bb board is an optional way to do the same.

## Derived fields (`--json`)

Task: `ready`, `openDeps`, `waitingOn` (open tasks that depend on it), `unlocks` (tasks that become ready when it is done), `stream` (owner, else the covering goal's stream).
Goal: `done`, `total`, `ready`, `waiting`.

Common warnings: in-progress task without a stream; task in two running goals; dependency on a dropped task; goal without frontmatter (legacy); incomplete git history.
