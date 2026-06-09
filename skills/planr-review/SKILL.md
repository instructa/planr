---
name: planr-review
description: Review a Planr item or scoped implementation against map state, plan acceptance criteria, logs, changed files, and verification evidence. Use for findings-first audits and review gates.
---

# Planr Review

Use this when a task needs a correctness and completion audit.

## Workflow

```bash
planr trace item <item-id>
planr log list --item <item-id>
planr map show --json
```

Inspect the actual changed files and verification evidence. Then close the review:

```bash
planr review close <review-id> --verdict complete
```

or:

```bash
planr review close <review-id> --verdict not-complete --findings "specific actionable finding"
```

## Findings Rules

- Findings must be specific and actionable.
- Missing tests are findings when acceptance criteria need proof.
- Architecture or ownership drift is a finding when it creates duplicate policy or state owners.
- If evidence is insufficient, use `--verdict unclear` rather than complete.

## Completion Rule

The target item may close only after required review items are closed. Use the map as the source of truth.
Pending or denied approval is also a close blocker; treat an attempted close through that gate as a finding, not as completion.
