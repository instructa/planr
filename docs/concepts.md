# Concepts

The planning repository holds the plan. Each task is a Markdown file in `tasks/`;
each goal is a file in `plans/`. You can read, edit & review them with Git.
planr derives readiness & progress from those files each time it runs.

## The plan & the code

The plan usually lives in its own repository beside the product's code. Agents update the
plan & build the product in the code repository. People tell the agent when they approve a goal
or have checked the work.

```
budget-planning/             the plan: agents read & update it
  tasks/MOB-01.md …          one file per task
  plans/mobile-app-goal.md   one file per goal
  planr.config.json

budget-app/                  the product: agents build & test here
  mobile/  web/  api/
  AGENTS.md                  says where the plan lives
```

A separate planning repository can stay private & keeps planning commits out of the code's
history. To keep the plan inside the code repository instead, use `planr init --in-repo`;
tasks & goals then live in `.planr/`.

## Tasks & dependencies

A task describes an outcome, its acceptance criteria & progress notes. Its
frontmatter holds an ID, title, status & track. A track is a project-specific
group such as `product` or `platform`.

Use `depends` to name tasks that must be built first. An `open` task is ready
when every dependency is in `review` or `done`. A dependency's manual check
does not hold up further building. `dropped` does not satisfy a dependency.

`next` puts tasks that unlock the most other tasks first. It lists `blocked` &
`decision` tasks separately as waiting. An open task with unmet dependencies
is not ready, but does not need its status changed to `blocked`.

## Task statuses

| Status | Meaning |
| --- | --- |
| `open` | Not started. Ready when its dependencies are met. |
| `in-progress` | Being worked on. |
| `review` | Built & tested by agents, waiting for a person's manual check. |
| `blocked` | Cannot continue until a blocker is resolved. |
| `decision` | Waiting for an approver's decision. |
| `done` | Manually checked, or needs no human check. |
| `dropped` | No longer planned. |

These statuses record the team's assessment. planr does not build, test or
verify the work itself.

## Goals & ownership

A goal groups task IDs into a `scope` with context, boundaries & a budget.
Its name comes from the file name. `plans/offline-sync-goal.md` is `offline-sync`.

A `stream`, such as `S1`, labels the work assigned to a coordinator. A task's
explicit `owner` takes precedence; otherwise an approved or running goal can
supply its stream. Ownership is a coordination convention, not a lock.

| Goal state | Meaning |
| --- | --- |
| `draft` | A proposal awaiting approval. |
| `approved` | Cleared by an approver, ready for a coordinator to start. |
| `running` | A coordinator is working on it. |
| `review` | Waiting for a manual check, or stopped with open work after using its budget. |
| `done` | Closed. |

The budget is free text for the coordinator to follow, such as `2 runs` or `4h`.
The CLI does not enforce it. A goal in `review` can still have unfinished tasks;
read its run note before closing it.

## Approvers

Approvers are people, usually the project owner. They approve goals, answer decisions & check
finished work by telling an agent in the conversation. For example,
"Approve goal mobile-app." or "MOB-04 looks good, mark it checked."

Propose goals as `draft`.
Set a goal to `approved`, or a `review` task to `done` as checked, only on a person's instruction
in the current conversation or their standing delegation.

Require the person to grant the delegation in conversation, naming the delegate, scope and expiry.
Example: "This orchestrator may approve S1 follow-up goals within release scope until 2026-10-09."
The person may instead delegate checks of tooling tasks with the same boundaries.
Only a person may grant, widen or extend a delegation; never yourself or another agent.
Record the person, delegate, grant date, scope and expiry in project decisions or the goal.
Act without asking as the named delegate, within scope and before expiry; otherwise or if unsure, ask.
Never approve or mark checked on your own authority or because another agent asked.

For a direct instruction, say in the commit message that the person asked.
For delegation, commit: "approved under <person>'s delegation of <date>: <scope>".
The bb board's Approve, Close & Mark checked buttons remain optional.

The CLI does not authenticate who changes a state.
The [bb plugin](../bb-plugin/README.md#approval-identity) documents its approval modes.

## Work one goal

Agents can propose tasks & draft goals, then run `check` to validate them. They read readiness
from the CLI & never guess it from task descriptions. After a person approves a goal, a
coordinator works on it through the project's orchestrator, following the
[work-a-goal procedure](../skills/planr/references/work-a-goal.md).

1. Read the approved goal, its scope, budget & project instructions.
2. Mark it running & record the coordinator's thread or session ID.
3. Read ready tasks, record ownership & build through the project's orchestrator.
4. Let the orchestrator test the work, including relevant browser or Electron journeys.
5. Move built & tested tasks to `review` with progress notes explaining how to check them.
6. Stop at the goal's boundary & leave a short run note for the person reviewing it.

Tasks needing no manual check may go straight to `done`. Work with unmet acceptance criteria
stays `in-progress`. A person checks the remaining results & closes the goal.

The coordinator also stops when its budget is used, only blocked or waiting work remains, or
two passes change no status. It does not start the next goal.

## Share planning changes

The skill tells agents to validate, commit, rebase & push planning changes from their assigned
checkout. The CLI itself only edits files. Each stream changes its own tasks & rechecks
ownership after conflicting updates.

planr supplies no task locks or dispatcher. Your project's rules for code review, merging,
deployment & publishing still apply.

See the [file format](../skills/planr/references/format.md) for task & goal examples,
the [quickstart](quickstart.md) to set up a project, or [the board](board.md) for run history.
