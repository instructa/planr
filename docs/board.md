# The board

The board shows work waiting on you, ready tasks, goal progress & status history.
Use the overview to see what needs attention, the task view to inspect
dependencies, or the run view to see what changed.

## Standalone board

From the planning repository, generate a self-contained HTML file.

```sh
planr board --out .planr/board.html
```

Open the file in a browser. It includes its data & fonts, so it can be read
offline. It is a snapshot; run the command again to update it.

For live reload, start the local server.

```sh
planr board --serve
```

Open <http://127.0.0.1:4173>. The server binds to loopback only & the page reloads
when tasks or goals change. Stop it with Ctrl+C. Use `--port` to choose another port.

Both versions are read-only. Change the Markdown files or use the CLI to update
the plan. Search tasks by ID or title & filter them by status, goal or track.

## Runs & history

A board run is a planning commit that adds or removes a task, or changes a task's
status. It is reconstructed from Git's first-parent history. Body-only edits do
not create runs. Uncommitted task additions, removals & status changes appear
as a final run.

The first run is the baseline. These runs are not a count of agent sessions.
A coordinator's notes under `## Runs` in a goal file are separate, human-readable
summaries. Without Git history, run history is unavailable; a shallow clone
can make it incomplete.

## In bb

Use the [bb plugin](plugins/bb.md) to see the board beside your threads, approve goals & check work.
