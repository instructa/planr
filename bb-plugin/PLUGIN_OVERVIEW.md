Keep goals and task readiness beside the products your agents build.

## What you get

- A Planr board for a project's Markdown planning repository.
- Ready and waiting work, goal progress, task history and repository state from the same engine as the CLI.
- Human actions to approve a draft goal, close a reviewed goal or mark a reviewed task as checked.
- Read-only `bb planr next`, `bb planr check` and `bb planr board` commands, plus the bundled Planr skill.

## How it works

Choose an enrolled host and a planning folder for each bb project. Files stay on that host;
only the project mapping lives in bb storage. Native file watches invalidate the board.

Board actions compare the file version you saw against origin, change one frontmatter line
in a dedicated git worktree, then commit and push. They never edit the main checkout.
Refresh fetches and fast-forwards the main checkout only when it is clean.
Without an origin remote, actions create a local commit and report that nothing was pushed.

Planr does not build products, dispatch agents or accept work automatically.
Agents use the skill script in their own worktree; the bb CLI cannot apply human acceptance actions.
