---
name: planr-work
description: Execute one Planr map item to evidence-backed completion. Use after a task has been picked or when the next step is implementation, docs, tests, scripts, integration work, or a fix item.
---

# Planr Work

Use this for one picked item at a time.

## Workflow

Export your worker identity once per session so picks, logs, and heartbeats attribute to you instead of `client:host:user`:

```bash
export PLANR_WORKER_ID=maker-1
planr pick --json
```

The pick output is one flat work packet — item, links, logs, runtime, recovery, conditions, recall context (`upstream_handoffs`, `relevant_contexts`, `review_history`), and `remaining` progress. Each fact appears once; a missing key means "empty". No separate `trace item` call needed. Add `--work-type code` to skip review items when checker agents work the same board, and `--plan <plan-id>` when your dispatch names a plan so the lease stays inside that scope. A null pick explains itself: `{"item": null}` carries a `reason` (`all_settled`, `nothing_ready`, `ready_items_excluded_by_filter`) plus the `remaining` snapshot; when filters excluded ready work, `excluded` names each item and cause and `repair` carries the exact pick command to run instead. Read the linked plan/context, implement the smallest correct slice, then finish the step in one command:

```bash
planr done <item-id> --summary "what changed" --files path-a --files path-b --cmd "exact verification command" --tests "exact test command" --review
```

Put build/serve commands in `--cmd` and test runs in `--tests` — both are recorded as evidence. `done --review` writes the completion log, requests the review, and moves the item to `in_review` (you keep ownership; it is waiting on the gate, not abandoned); add `--next` to pick the following item in the same call. Without `--review` it closes the item directly (only for items that need no review gate). The response reports what your settlement `unlocked`, echoes the item's post condition, and hints when downstream work depends on an item closed without command/test evidence.

Live verification (browser flow, executed binary, real requests) gets its own log kind so `plan audit` can find it:

```bash
planr log add --item <item-id> --kind verification --summary "verified <flow>: <observed outcome>" --cmd "<exact command>"
```

Log persistent evidence, not transient noise: a failure you immediately fixed belongs in the final log's narrative, not as a standalone failure log. Only record a failure separately when it blocks the item.

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
