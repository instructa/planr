---
name: planr-work
description: Execute one Planr map item to evidence-backed completion. Use after a task has been picked or when the next step is implementation, docs, tests, scripts, integration work, or a fix item.
---

# Planr Work

Use this for one picked item at a time.

## Workflow

```bash
planr pick --json
```

The pick output is one flat work packet — item, links, logs, runtime, recovery, conditions, recall context (`upstream_handoffs`, `relevant_contexts`, `review_history`), and `remaining` progress. Each fact appears once; a missing key means "empty". No separate `trace item` call needed. Read the linked plan/context, implement the smallest correct slice, then finish the step in one command:

```bash
planr done <item-id> --summary "what changed" --files path-a --files path-b --cmd "exact verification command" --tests "exact test command" --review
```

Put build/serve commands in `--cmd` and test runs in `--tests` — both are recorded as evidence. `done --review` writes the completion log, requests the review, and moves the item to `in_review` (you keep ownership; it is waiting on the gate, not abandoned); add `--next` to pick the following item in the same call. Without `--review` it closes the item directly (only for items that need no review gate).

Evidence logging refreshes the heartbeat automatically — a separate `planr pick heartbeat` is only needed for long silent stretches without logs.

The granular commands remain available when you need them:

```bash
planr log add --item <item-id> --summary "..." --files a --files b --cmd "..."
planr review request <item-id>
planr close <item-id> --summary "Verified"
```

For longer work, keep runtime state current:

```bash
planr pick progress <item-id> --percent 50 --note "tests running"
planr pick pause <item-id> --note "waiting for human input"
planr pick resume <item-id>
```

If a human approval gate is required, request it before close and wait for an approved decision:

```bash
planr approval request <item-id> --reason "release approval"
planr approval list --open
```

## Rules

- Do not work on multiple picked items unless the user explicitly asks.
- Do not close without evidence.
- Do not treat a failed review as failure of the original item; let Planr create the fix and follow-up review chain.
- Do not close items with pending or denied approval.
- Use `planr context add ... --item <item-id>` for discoveries another client needs.
- Use `planr pick stale --older-than-seconds 900` before resetting abandoned ownership.
- Use `planr pick release <item-id> --force` only when ownership must be reset.
