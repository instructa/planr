---
name: planr-status
description: Report honest Planr project, map, item, or review state without implementing changes. Use when the user asks what is done, what remains, what is blocked, what is ready, or what should be picked next.
---

# Planr Status

Use read-only Planr commands first:

```bash
planr project show --json
planr map show --json
planr map lane --critical
planr map pressure
```

For one item:

```bash
planr trace item <item-id>
planr log list --item <item-id>
```

For active work, include runtime and approval state from `trace item`, `map status`, or:

```bash
planr approval list --open
planr pick stale --older-than-seconds 900
```

## Verdicts

Use one:

- `complete`: closed with evidence and no open required child/review/approval work.
- `in progress`: concrete work remains and the next step is available.
- `blocked`: progress needs an external decision or prerequisite.
- `unclear / partially verified`: evidence is incomplete or inconsistent.

Never treat checked Markdown boxes or optimistic summaries as proof. Map state and log evidence decide.
