# planr contract

The data and CLI contract of the planr engine. Change it here first, then the code.

## Planning repository layout

```
<planning-root>/
  planr.config.json        optional
  tasks/<ID>.md            one file per task (source of truth)
  plans/<name>-goal.md     goals (directory and suffix configurable)
  decisions.md             free-form, not parsed
```

`planr.config.json` (all keys optional):

```json
{ "tasksDir": "tasks", "goalsDir": "plans", "goalSuffix": "-goal.md", "boardOut": ".planr/board.html" }
```

Legacy `staleHours` config keys are accepted and ignored.

`--root <dir>` selects the planning root (default: walk up from cwd to the first directory containing `tasks/` or `planr.config.json`).

## Task file

Frontmatter (YAML subset: scalars, `[a, b]` inline lists, `- item` block lists; that is all planr parses):

| Field | Required | Notes |
|---|---|---|
| `id` | yes | equals the file name without `.md` |
| `title` | yes | |
| `status` | yes | `open`, `in-progress`, `review`, `blocked`, `decision`, `done`, `dropped` |
| `track` | yes | free text |
| `depends` | no | list of task IDs |
| `owner` | for `in-progress`, unless a goal covers it | stream or thread; see below |
| `refs` | no | passed through untouched |

Missing owner on an `in-progress` task is a warning (error with `--strict`) only when no `running` or
`approved` goal with a `stream` has the task in its scope (see `stream` below).
`review` tasks wait for a person's or delegated agent's check and need no owner.

Body sections are free Markdown and never parsed.

## Derived rules (only in `skills/planr/scripts/lib/graph.mjs`)

- `satisfied(d)`: status is `review` or `done`.
  `review` means built and tested; a person's or delegated agent's check never blocks further building.
  **`dropped` does not satisfy a dependency.**
  A task depending on a dropped task gets a warning: remove the edge.
- `openDeps(t)`: IDs in `t.depends` that are not satisfied.
- `ready(t)`: `t.status == "open"` and `openDeps(t)` is empty.
- `waitingOn(t)`: IDs of tasks with status not in {done, dropped} whose `depends` contains `t.id`.
- `stream(t)`: `t.owner` if set, else the `stream` of the first goal (by name) with state `running` or `approved` whose scope contains `t.id`, else `null`.
- `unlocks(t)`: number of tasks with status `open` whose only open dependency is `t` (they become ready when `t` is satisfied). Defined for every task; 0 for review/done/dropped.
- Cycles: any dependency cycle is an error listing the cycle path.

## Derived data (in memory, never committed)

planr does not write an index file. Every command parses the task and goal files and derives the data below in memory. Planning repos must not commit derived files; during a migration a constant placeholder `tasks/index.json` may exist and is ignored by planr.

`planr list/next --json` return task objects of this shape:

```json
{ "id": "AUTH-03", "title": "…", "status": "in-progress", "track": "release",
  "depends": ["AUTH-02"], "owner": null, "refs": ["…"], "file": "tasks/AUTH-03.md",
  "ready": false, "openDeps": [], "waitingOn": ["MAIL-02", "…"], "unlocks": 1, "stream": "S2" }
```

Goal objects: `{ name, file, title, state, stream, budget, thread, scope, done, total, ready, waiting }`.

Warnings include: in-progress task without derived stream; task in the scope of two `running` goals; dependency on a dropped task; legacy goal without frontmatter; history problems (shallow clone, unreadable snapshot).

## Goal file

```markdown
---
title: Offline sync (S1)
state: approved          # draft | approved | running | done | review
stream: S1
budget: 4h               # free text, read by humans and the skill
thread: thr_abc123       # set when a coordinator starts it
scope: [SYNC-01, SYNC-02, SYNC-04]
---
Free Markdown: context, rules, out of scope.
```

- States: `draft` (proposal), `approved` (a person or their delegate approved it, no coordinator running),
  `running` (a coordinator works on it; `thread` set),
  `review` (over budget or waiting for a person's check), `done`.
- Agents create goals as `draft`.
- Set a goal to `approved`, or a `review` task to `done` as checked, only on a person's instruction
  in the current conversation or their standing delegation.
- Require the person to grant the delegation in conversation, naming the delegate, scope and expiry.
  Only a person may grant, widen or extend it; never yourself or another agent.
  Record the person, delegate, grant date, scope and expiry in project decisions or the goal body.
  Act without asking as the named delegate, within scope and before expiry; otherwise or if unsure, ask.
  Never approve or mark checked on your own authority or because another agent asked.
- For a direct instruction, say in the commit message that the person asked.
  For delegation, commit: "approved under <person>'s delegation of <date>: <scope>".
  The bb board is an optional way to do the same.
  Tasks with nothing for a person to look at may go straight to `done`.
- The CLI does not enforce who sets states; the skill governs agents.
  The bb plugin can enforce identity for board actions (see `bb-plugin/README.md`).
- A goal file without frontmatter is listed with `state: "legacy"`, an empty scope and a warning. No text heuristics.
- `goals[].ready`: scope ∩ ready tasks. `goals[].waiting`: scope tasks with status `decision` or `blocked`.

## CLI (`skills/planr/scripts/planr.mjs`)

Human-readable output by default, `--json` for agents (exact JSON shapes below). Exit 0 on success, 1 on validation errors, 2 on usage errors. Messages are one line each, prefixed `planr:`.

| Command | Behavior | `--json` |
|---|---|---|
| `planr check [--strict]` | Parse and validate tasks and goals; writes nothing. Exit 1 on errors (and on warnings with `--strict`). `planr index` is a deprecated alias. | `{ "ok", "errors": [], "warnings": [] }` |
| `planr list [--status a,b] [--track x] [--owner x] [--goal name] [--ready]` | Filtered task list, sorted like the board (blocked, decision, in-progress, review, open, done, dropped; then most `waitingOn`). | array of task objects |
| `planr next --goal <name>` / `planr next --stream <S>` | Ready tasks in the goal's scope (or in the scopes of the stream's `approved`/`running` goals), most `unlocks` first, plus waiting tasks. Tasks whose `owner` names a different stream are excluded. | `{ "goal" or "stream", "ready": [tasks], "waiting": [tasks] }` |
| `planr set <id> <status> [--owner <x>]` | Rewrite only the `status:` (and `owner:`) line in the task's frontmatter (temp file + rename), then validate. Writes nothing else. Rejects unknown status. | `{ "ok", "id", "from", "to" }` |
| `planr goal <name> [<state>] [--thread <id>]` | Show a goal, or rewrite its `state:` / `thread:` frontmatter lines, then validate. | goal object |
| `planr board [--out <file>] [--serve] [--port <n>]` | Render the board HTML. `--serve`: loopback-only server, re-renders when `tasks/`, the goals dir or `.git/HEAD` change, the page reloads itself. | — |
| `planr init [--in-repo]` | Scaffold `tasks/README.md` (format), `planr.config.json`, a goal template, `.gitignore` entry for `.planr/board.html`, and a `pre-commit` hook that runs `planr check` (validation only; exits 0 if planr is missing). Never overwrites existing files. |  — |

`planr` never runs `git commit`, `push` or `merge`. It reads git only for history (`lib/runs.mjs`).

## Board data (input of `board/template.html`)

`skills/planr/assets/board/template.html` contains `/*DATA*/null` and `/*FONT:<file>*/` placeholders. The renderer (`skills/planr/scripts/lib/board.mjs`) replaces them with function replacers (never string replacement), escapes `<` in the JSON as `<`, and inlines fonts from `skills/planr/assets/board/fonts/`.

```ts
type BoardData = {
  project: string;               // planning root directory name
  commit: string;                // "<short hash> · <ISO date>" of planning HEAD
  dirty: boolean;                // working tree differs from HEAD for tasks/ or goals
  generatedAt: string;           // ISO
  tasks: Array<IndexTask & {
    goal: string[];              // names of non-done goals whose scope contains the task
    history: Array<{ run: number; from: string | null; to: string }>;
    lastRun: number | null;
  }>;
  goals: IndexGoal[];            // derived in memory, order: running, approved, draft, review, legacy, done
  runs: Array<{ commit: string | null; date: string; subject: string;
                counts: Record<string, number>; total: number; initial: boolean;
                changes: Array<{ id: string; from: string | null; to: string }> }>;
  warnings: string[];
};
```

Runs: one per first-parent commit (oldest first) that adds or removes a task file or changes its `status`.
They are reconstructed from task file history; body-only edits are not runs.
Plus a final `commit: null` run when the working tree's task statuses differ from HEAD.
A synthetic sample is in `fixtures/board-data.json`.
