# `.planr`

This directory is the canonical repo-local **planr** workspace: plans, live status, optional reviews, and the shared CLI. Copy `.planr/` and the matching planr skills into any codebase that wants the same workflow.

Use it to store:

- durable project context
- scoped execution plans
- the live status summary
- optional persisted review artifacts
- the shared deterministic CLI that reads and mutates `.planr`

## Directory Layout

- `project/`
  - Durable project context used by planning, fixing, review, and status work.
  - Typical files: product direction, ownership boundaries, flows, state ownership, constraints, and quality gates.
- `plans/`
  - Scoped execution contracts.
  - File shape: `<slug>_<hash>.plan.md`
  - These are not loose notes. They are the contracts that later `fix`, `review`, and `status` work should reconcile against.
- `status/current.json`
  - The canonical live planr status source.
  - This is a summary layer, not proof by itself.
- `review/`
  - Optional persisted review artifacts for material reviews.
  - Use this only when the review has durable follow-up value.
- `tooling/planr.py`
  - The canonical shared CLI for deterministic `.planr` operations.
- `tooling/test_planr.py`
  - Focused tests for the shared CLI.

## Core Rules

- Keep one canonical current-state workflow. Do not create a second queue, status file, or helper path.
- `.planr/status/current.json` is the live summary source. It does not replace plan checklists, diffs, or verification evidence.
- `.planr/plans/*.plan.md` are execution contracts. They should contain explicit scope, ownership, phases, verification, and acceptance criteria.
- Prefer the shared CLI for plan scaffolding and status mutation when the command exists.
- Edit plan body markdown directly when the scaffold needs richer task-specific detail.
- Record exact verification commands and real results. Checked boxes without proof are not enough.
- On resume or interruption, read `.planr/status/current.json` and `git status` before continuing.
- Do not use `docs/tasks/*.md` as the live planr tracking system.
- Follow the repo hard-cut policy: when only one meaningful current-state behavior remains, delete the dead concept end-to-end instead of keeping a one-value shell.

## Typical Workflow

1. Inspect live state.
2. Create or update the scoped plan in `plans/`.
3. Ensure the scope exists in `status/current.json`.
4. Implement work and keep checklist / verification state honest.
5. Run focused verification and record exact commands/results.
6. Use `status open` or `status next` to decide what remains or what comes next.
7. Persist a review under `review/` only when it adds execution value.

## Shared CLI

The canonical path is:

```bash
./.planr/tooling/planr
```

The wrapper prefers `python3` and falls back to `python` when `python` is Python 3.

If you are not running from the repo root, pass `--repo-root <path>`.

### Project Context Init

Bootstrap or refresh the starter pack under `.planr/project/`:

```bash
./.planr/tooling/planr project init
```

Overwrite existing `.planr/project/*.md` files with the starter templates:

```bash
./.planr/tooling/planr project init --force
```

`project init` creates or refreshes the starter pack and canonical project-context metadata. It does **not** analyze the target repository or make repo-specific ownership decisions by itself.

After running it, the agent must inspect the target codebase and rewrite `.planr/project/*.md` so product, ownership, flows, state sources of truth, and quality gates match the real repo.

### Read-Only Inspection

Show the full live status:

```bash
./.planr/tooling/planr status show
```

Show one scope:

```bash
./.planr/tooling/planr status show --scope <scope-id>
```

Show all open scopes:

```bash
./.planr/tooling/planr status open
```

Show the next deterministic open scope:

```bash
./.planr/tooling/planr status next
```

### Plan Scaffolding

Create a new deterministic plan scaffold:

```bash
./.planr/tooling/planr plan new \
  --title "Example Scope" \
  --overview "One-sentence plan overview." \
  --todo phase-one="First concrete outcome." \
  --todo verify="Verification outcome."
```

### Scope Mutation

Create or update a scope shell:

```bash
./.planr/tooling/planr status ensure-scope \
  --id <scope-id> \
  --title "Scope Title" \
  --status in_progress \
  --source "user-requested planr-fix" \
  --plan-path ".planr/plans/example_scope_deadbeef.plan.md" \
  --owned-path ".planr/status/current.json"
```

Set one checklist item:

```bash
./.planr/tooling/planr status set-checklist \
  --scope <scope-id> \
  --item-id <item-id> \
  --content "Concrete checklist item." \
  --status in_progress
```

Optional ordering flags for checklist, blocker, and verification mutations:

- `--before-id <sibling-item-id>`
- `--after-id <sibling-item-id>`

Use these when a new or existing item must sit in the middle of a deterministic list without hand-editing JSON.

Set one blocker or unverified item:

```bash
./.planr/tooling/planr status set-blocker \
  --scope <scope-id> \
  --item-id <item-id> \
  --content "What remains blocked or unverified." \
  --status blocked \
  --before-id <sibling-item-id>
```

Set one verification record:

```bash
./.planr/tooling/planr status set-verification \
  --scope <scope-id> \
  --verification-id <verification-id> \
  --status passed \
  --command "./.planr/tooling/planr --help" \
  --result "CLI help rendered successfully." \
  --after-id <sibling-item-id>
```

## Queue Semantics

`status open` lists:

- scopes whose top-level status is `pending`, `in_progress`, or `blocked`
- closed scopes with status drift, such as a `completed` scope that still has unfinished checklist items, blocker/unverified items, or non-passed verification records

`status next` selects the next deterministic open scope with this precedence:

1. `in_progress`
2. `pending`
3. `blocked`

Within each bucket, current file order in `status/current.json` is preserved.

If no actionable open scope remains, `status next` falls back to the first drifted closed scope.

If everything is cleanly closed, it returns:

```json
null
```

## Manual Editing Guidance

Prefer the CLI for:

- plan scaffolding
- creating or updating a scope shell
- checklist mutation
- blocker / unverified mutation
- verification record mutation
- read-only queue inspection

Edit files directly only when:

- you need richer plan body content than the scaffold provides
- the desired change is not expressible through the current CLI

If the CLI does not support a needed change, extend the canonical CLI or edit the file directly. Do not add a second helper path.

## Reviews

Material reviews may be persisted under:

```text
.planr/review/<slug>.review.md
```

Persist a review only when it adds follow-up value, for example:

- the user explicitly asked for a saved review artifact
- the review produced findings that will drive later `fix` work
- the review gates a larger hard-cut, PR, or multi-phase task

Keep `status/current.json` as the summary layer rather than duplicating the full review there.

## Tool Verification

When the shared CLI changes, run:

```bash
./.planr/tooling/planr --help
./.planr/tooling/test_planr
python3 -m py_compile .planr/tooling/planr.py .planr/tooling/test_planr.py
```

For the raw `py_compile` check, use whichever command on your machine points to Python 3 (`python3` or `python`).

For scoped diff hygiene, use:

```bash
git diff --check -- <owned .planr paths>
```
