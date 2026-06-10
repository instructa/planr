---
name: planr-work
description: Execute one Planr map item to evidence-backed completion. Use after a task has been picked or when the next step is implementation, docs, tests, scripts, integration work, or a fix item.
---

# Planr Work

Use this for one picked item at a time.

## Workflow

```bash
planr pick --json
planr trace item <item-id>
planr pick heartbeat <item-id>
```

Read the linked plan/context, implement the smallest correct slice, then record evidence:

```bash
planr log add --item <item-id> --summary "what changed" --files path-a --files path-b --cmd "exact verification command"
planr review request <item-id>
```

Only close after review is complete:

```bash
planr review close <review-id> --verdict complete
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
