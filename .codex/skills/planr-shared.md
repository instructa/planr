# Planr Shared Baseline

Use this with any repo-local `planr-*` skill.

## Non-Negotiable

- Prefer `./.planr/tooling/planr` whenever its command surface fits the task.
- The wrapper prefers `python3` and falls back to `python` when `python` is Python 3.
- Do not hand-edit `.planr/status/current.json` or scaffold a new `.planr/plans/*.plan.md` file by hand when the CLI already covers that operation.
- Status mutations are single-writer serialized through the shared CLI; do not bypass it with parallel hand-edits to `.planr/status/current.json`.
- Do not invent ascending plan ids. When a completed scope needs plan-folder hygiene, archive it through the shared CLI into `.planr/plans/done/DD-MM/`.
- Do not invent unsupported `planr.py` subcommands. Today the CLI supports `project`, `plan`, and `status`.

## Current CLI Surface

Use these commands as the deterministic first path:

```bash
./.planr/tooling/planr project init [--force]

./.planr/tooling/planr plan path --title "..." [--slug slug]
./.planr/tooling/planr plan new --title "..." --overview "..." [--todo id=content] [--slug slug] [--project]
./.planr/tooling/planr plan archive --scope scope-id [--archive-date YYYY-MM-DD]

./.planr/tooling/planr status show [--scope scope-id]
./.planr/tooling/planr status open
./.planr/tooling/planr status next

./.planr/tooling/planr status ensure-scope --id scope-id [--title "..."] [--status pending|in_progress|blocked|completed|cancelled] [--source "..."] [--clear-plan-paths] [--plan-path path] [--clear-owned-paths] [--owned-path path]
./.planr/tooling/planr status delete-scope --id scope-id
./.planr/tooling/planr status set-checklist --scope scope-id --item-id item-id --content "..." --status pending|in_progress|blocked|completed|cancelled
./.planr/tooling/planr status delete-checklist --scope scope-id --item-id item-id
./.planr/tooling/planr status set-blocker --scope scope-id --item-id item-id --content "..." --status blocked|unverified
./.planr/tooling/planr status delete-blocker --scope scope-id --item-id item-id
./.planr/tooling/planr status set-verification --scope scope-id --verification-id verification-id --status not_run|passed|failed|blocked --result "..." [--command "..."]
./.planr/tooling/planr status delete-verification --scope scope-id --verification-id verification-id
```

## What Each Source Means

- `.planr/plans/*.plan.md`: scoped execution contract
- `.planr/status/current.json`: live scope state
- `.planr/project/*.md`: project context pack
- path-scoped Git diff, implementation files, and tests: proof of the actual code state

## Shared Read Order

1. Use the CLI first for project, plan, or status questions and for supported `.planr` mutations.
2. Read the governing `.planr` artifacts next: relevant plan file, live scope state, and any explicitly referenced historical source doc still in scope.
3. If `.planr/project/*.md` is missing or still generic starter text, run `./.planr/tooling/planr project init`, then inspect the target repo and rewrite the pack before making strong architecture or ownership decisions.
4. Read the project-context pack only when ownership, runtime boundaries, state boundaries, or quality gates matter:
   - `.planr/project/product.md`
   - `.planr/project/ownership.md`
   - `.planr/project/flows.md`
   - `.planr/project/state-ssot.md`
   - `.planr/project/constraints.md`
   - `.planr/project/quality-gates.md`
5. Drop to implementation files, tests, and path-scoped Git evidence only when the CLI and recorded `.planr` state are insufficient.

## When CLI Coverage Ends

- There is no `planr.py fix` command today.
- There is no `planr.py review` command today.
- `project init` only scaffolds or refreshes `.planr/project/*.md` and canonical project-context metadata. It does not inspect the codebase or make repo-specific ownership decisions by itself.
- There is no general-purpose command for editing an existing plan body beyond `plan new`.
- `planr-review` still requires direct path-scoped Git evidence.
- `planr-fix` still requires direct code, test, and verification work.
- `planr-summary` still requires a synthesized narrative from the recorded evidence.

In those cases, keep the `.planr` parts deterministic with the CLI, then use files, diff, and tests for the parts the CLI does not cover.
