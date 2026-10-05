# CLI

Install with `npm install -g planr`, or use `npx planr <command>`.
Node.js 20 or newer is required. Git provides history & planning collaboration.

`planr --version` prints the installed version. `planr --help` (or `planr` without
arguments) lists commands & common options. Both work without a planning repository.

Run commands inside the planning repository. From elsewhere, add
`--root /path/to/planning`. From a source checkout, the entry point is
`node skills/planr/scripts/planr.mjs`.

## Start a planning repository

Use a separate repository for the plan, or keep it inside your code repository.
Run one of these in the chosen repository root.

```sh
planr init
# Or, for planning inside a code repository
planr init --in-repo
```

`init` creates task instructions in `tasks/README.md`, a draft goal in
`plans/example-goal.md` & `planr.config.json`. It also ignores the generated board
at `.planr/board.html`. If Git is initialized & no pre-commit hook exists, it adds
a hook that runs `check` through the local script.

`--in-repo` puts tasks & goals in `.planr/tasks/` & `.planr/plans/` instead.
The config stays at the repository root. Only the generated board is ignored;
the planning files belong in Git. Existing task instructions, goals, config &
pre-commit hooks are preserved.

## Read the plan

```sh
planr check
planr list --ready
planr list --status blocked,decision
planr next --goal example
planr next --stream S1 --json
planr goal example
```

`next` requires exactly one goal or stream. A goal query previews its scope,
including draft goals. A stream query uses only its approved or running goals.
Both exclude tasks explicitly owned by another stream.

`list` also accepts `--goal`, `--track` & `--owner` filters. `--owner` matches the
explicit task owner, not a stream inherited from a goal.

`check` validates planning data, including unknown dependencies & cycles.
It does not validate product work. Warnings are informational; `--strict`
treats them as errors. Exit codes are `0` for success, `1` for validation or
runtime errors & `2` for usage errors. Use `--json` for structured data from
`check`, `list`, `next`, `set` & `goal`.

## Change the plan

Create each task as a Markdown file, then add its ID to a goal's `scope`, for example in
`plans/example-goal.md`. There is no `add` command.

```sh
cat > tasks/MOB-01.md <<'EOF'
---
id: MOB-01
title: App shell & navigation
status: open
track: mobile
---
The app opens on a home screen with tabs for accounts, expenses & budgets.
EOF
```

Edit descriptions, dependencies & progress notes directly, then run `check`.

For a goal a person has approved, record the coordinator & update tasks as work progresses.

```sh
planr goal example running --thread your-session-id
planr set MOB-01 in-progress --owner S1
planr set MOB-01 review
planr check
```

Use these states when the work reaches that stage. See [concepts](concepts.md)
for their meaning. `set` rewrites status & an optional owner; `goal` rewrites
state & an optional thread. Both validate afterward. A validation error does
not roll back the edit. The CLI never commits or pushes.

## View the board

```sh
planr board --out .planr/board.html
planr board --serve
planr board --serve --port 4174
```

See [the board](board.md) for snapshots & live reload. For every option & JSON
shape, see the [technical contract](contract.md#cli-skillsplanrscriptsplanrmjs).
