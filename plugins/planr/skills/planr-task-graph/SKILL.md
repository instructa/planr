---
name: planr-task-graph
description: Use Planr as the live local task graph for coding-agent coordination. Trigger when work needs project planning, build-plan splitting, map creation, dependency links, picking, log-backed closure, review gates, handoff context, interruption recovery, or multi-client coordination.
---

# Planr Task Graph

Use `planr` as the canonical local coordination system for this repository.

Planr has two first-class layers:

- Markdown plans for product context, build scope, verification, and narrative decisions.
- SQLite map state for items, links, picks, runtime heartbeats, approvals, contexts, logs, reviews, runs, and closure.

The map is the source of truth for live state. Markdown explains scope and acceptance.

## Start Here

Inspect the project and map before changing work:

```bash
planr project show --json
planr map show --json
planr map lane --critical
planr map pressure
```

If no project exists:

```bash
planr project init "Project Name" --client all
planr doctor --client all
```

## Core Loop

Use one item at a time. The short path is two commands per step:

```bash
planr pick --json
planr done <item-id> --summary "what changed" --files a --files b --cmd "exact verification command" --tests "exact test command" --review [--next]
```

`pick --json` returns one flat work packet (item, links, logs, runtime, recovery, conditions, recall context, `remaining` progress); empty collections are omitted. `done` writes the completion log (test runs belong in `--tests`, build/serve commands in `--cmd`), requests review (`--review`, which moves the item to `in_review`) or closes directly, and `--next` picks the following item. Evidence logs refresh the heartbeat automatically.

For longer work, keep the live claim visible:

```bash
planr pick progress <item-id> --percent 50 --note "tests running"
planr pick pause <item-id> --note "waiting for human input"
planr pick resume <item-id>
```

Capture decisions and discoveries another agent may need:

```bash
planr context add "decision or discovery" --item <item-id> --tag discovery
```

Record evidence before review:

```bash
planr log add --item <item-id> \
  --summary "what changed" \
  --files file-a,file-b \
  --cmd "exact verification command"
```

Granular alternative: request review, close review, then close the item:

```bash
planr review request <item-id>
planr review close <review-id> --verdict complete --close-target
planr close <item-id> --summary "Verified with evidence"
```

`--close-target` closes the reviewed item together with the review when the verdict is complete and a completion log exists; the separate `planr close` is then unnecessary.

If human approval is part of the gate, request it and do not close until it is approved:

```bash
planr approval request <item-id> --reason "release approval"
planr approval list --open
```

If review finds issues:

```bash
planr review close <review-id> \
  --verdict not-complete \
  --findings "specific actionable finding"
planr map show --json
planr pick --json
```

Planr creates fix and follow-up review work instead of pretending the parent item is done.

## Planning Flow

For broad app or product work:

```bash
planr plan new "App idea" --platform web --ai --backend
planr plan refine <plan-id> --note "Assumption or decision"
planr plan split <plan-id> --slice "MVP implementation"
planr plan check <build-plan-id>
planr map build --from <build-plan-id>
```

Product plan work lists are candidates. They become live commitments only after `planr map build --from ...`.

`map build` creates items without ordering: everything starts ready. Linking is mandatory before the first pick — add a `blocks` link for every real execution dependency:

```bash
planr link add <earlier-item> <later-item> --type blocks
planr map lane --critical
```

Do not pick from a freshly built map that has zero links unless the items are genuinely independent.

## Parent Gate Pattern

Model material changes as parent gates. The parent is the completion gate; linked children do the work.

Default shape:

```text
parent gate
`- implementation or test child
   `- review item linked to that child
      |- pass -> child can close -> parent gate auto-closes
      `- findings -> fix item -> follow-up review -> ...
```

Rules:

- create a parent item for the change;
- use `planr item breakdown <parent-id> --into "Implement, Verify"` to create child work under that parent;
- request review on the implementation or test child after evidence exists;
- if review finds issues, let Planr create fix and follow-up review work from the review verdict;
- make later top-level work depend on the parent gate, not only the first child.

Parent gates roll up on their own: when every child is settled, the gate auto-closes unless a review or approval on the gate itself is still open. Do not pick a parent gate as work; `planr pick` skips them.

## Dependencies

Create ordering explicitly:

```bash
planr item create "Design API" --description "Define endpoints and data ownership."
planr item create "Implement API" --description "Build endpoints after design is closed."
planr link add <design-item> <implementation-item> --type blocks
```

Readiness comes from graph links and item state, not Markdown checkboxes.

Use:

```bash
planr item breakdown <item-id> --into "Trace owner, Implement, Verify"
planr link remove <from-item> <to-item> --type blocks
```

## Handoff Evidence

Every closure must be evidence-backed:

- changed files through `--files`;
- commands through `--cmd`;
- tests through `--tests`;
- remaining risks through context, notes, or findings;
- review outcome through `planr review ...`.

Use task-local notes for nearby handoff:

```bash
planr note add "Reviewer asked for an extra package dry-run before closure." --item <item-id>
```

Use contexts for reusable project knowledge:

```bash
planr context add "Do not edit global client config without explicit operator approval." --tag constraint
```

Use a story log only when map state, logs, and contexts are too thin to preserve the decision chain. Story logs are narrative memory, not status authority.

## Recovery

After interruption or handoff:

```bash
git status --short
planr project show --json
planr map show --json
planr map lane --critical
planr map pressure
```

Then inspect the item:

```bash
planr trace item <item-id>
planr log list --item <item-id>
planr context list --item <item-id>
```

If a stale pick must be reset:

```bash
planr pick stale --older-than-seconds 900
planr pick stale --older-than-seconds 900 --release
planr pick release <item-id> --force
```

## Completion Rule

Do not call work complete until:

- required child and review items are closed;
- approval gates are approved or absent;
- log evidence exists;
- verification commands were actually run;
- review findings are closed or converted into follow-up work;
- `planr map show --json` shows no in-scope blocker;
- the user-facing summary matches map, logs, and review state.

For release-grade scopes, rerun the full verification ladder from the repository testing guide.
