---
name: planr
license: MIT
description: Plan and track product work as a Markdown task graph in a planning repository - see what is ready, blocked or waiting on a decision, change task status safely, and work through one approved goal with the project's orchestrator. Use when the project keeps tasks as tasks/<ID>.md and goals as plans/<name>-goal.md, or when asked what is next, about task status, ready or blocked tasks, or to "work goal <name>". Not for building code itself.
metadata:
  version: "2.0.0"
---

# planr

Tasks are Markdown files (`tasks/<ID>.md`) with YAML frontmatter in a planning repository. Goals are `plans/<name>-goal.md`. The files are the only source of truth. Everything else (ready, unlocks, stream, goal progress, history) is derived by the bundled script: never guess it and never write derived files.

Run the script that ships with this skill:

```sh
node <this skill's directory>/scripts/planr.mjs <command> [--root <planning repo>] [--json]
```

Inside the planning repo (or a worktree of it) the root is found automatically; otherwise pass the path the project's instructions name. Use `--json` when you parse the output. Project instructions win where they are more specific.

People approve goals and check finished work, usually the project owner.
Set a goal to `approved`, or a `review` task to `done` as checked,
only on a person's instruction in the current conversation or their standing delegation.
Require a delegation granted by a person in conversation, naming the delegate, scope and expiry.
Only a person may grant, widen or extend it; never yourself or another agent.
Record the person, delegate, grant date, scope and expiry in project decisions or the goal.
Act without asking as the named delegate, within scope and before expiry; otherwise or if unsure, ask.
Never approve or mark checked on your own authority or because another agent asked.
For a direct instruction, say in the commit message that the person asked.
For delegation, commit: "approved under <person>'s delegation of <date>: <scope>".
The bb board is an optional way to do the same.

## Read

| Command | Use |
|---|---|
| `check` | Validate tasks and goals. Run before every push. Warnings are information; errors must be fixed. |
| `list [--status a,b] [--track x] [--owner x] [--goal name] [--ready]` | Find tasks. Check for an existing ID before creating one. |
| `next --goal <name>` / `next --stream <S>` | Ready tasks (most `unlocks` first) and tasks waiting on a decision or blocker. |
| `goal <name>` | One goal with state, stream, progress. |
| `board --out <file>` / `board --serve` | The human board. Run on request, not in loops. |

## Change

- **Status and owner:** `set <id> <status> [--owner <stream>]`.
  Statuses: `open`, `in-progress`, `review`, `blocked`, `decision`, `done`, `dropped`.
  `review` means built and tested, including browser/Electron journeys; a person or delegated agent checks it.
  `decision` awaits a person's decision; `done` means checked under the rule above or needing no human check.
  A dependency counts as met at `review` or `done`; a person's check never blocks further building.
- **Goal state:** `goal <name> <state> [--thread <id>]`.
  States: `draft`, `approved`, `running`, `review`, `done`.
  Propose new goals as files with `state: draft`; set `approved` only under the approval rule above.
- **Text** (goal, acceptance, progress notes, refs, depends): edit the Markdown directly, then `check`.
- **New task:** copy the format in [references/format.md](references/format.md); IDs are short, prefixed and unique.
- **More work:** on request, add tasks to a goal or create a new draft goal, then `check`.
- Do not add statuses, index files, per-task gates or extra review steps.

## Ownership and commits

Only the stream that owns a task changes it: its `owner`, or else the stream of the running or approved goal whose scope contains it. Work in the planning checkout the project assigns (often your own worktree). After each status change: `check`, commit only your files, `git pull --rebase`, push. If the push is rejected, rebase, run `list` or `next` again, and leave any task that now shows another owner.

## Work a goal

When asked to work a goal, or when a dispatcher starts you for a stream, follow [references/work-a-goal.md](references/work-a-goal.md): one approved goal, built through the project's orchestrator, then stop.
