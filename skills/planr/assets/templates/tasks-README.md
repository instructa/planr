# Tasks

Create one Markdown file per task, named `<ID>.md`. Frontmatter supports scalars,
inline lists (`[A, B]`) and block lists (`- A`). The body is free Markdown.

Required: `id` (matches the filename), `title`, `status`, `track`.
Optional: `depends` (task IDs), `owner` (stream or thread), `refs`.
An in-progress task without an owner warns unless a running/approved goal supplies its stream.
Statuses: open, in-progress, review, blocked, decision, done, dropped.
Review and done satisfy dependencies; review awaits manual acceptance.
A dropped dependency remains unsatisfied; remove its edge deliberately.

Run `planr check` to validate without writing files. `planr board` renders the board.
Task data is derived in memory; no index is written. `planr next --goal example` lists ready tasks and decisions/blockers.
