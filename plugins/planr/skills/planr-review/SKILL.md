---
name: planr-review
description: Review a Planr item or scoped implementation against map state, plan acceptance criteria, logs, changed files, and verification evidence. Use for findings-first audits and review gates.
---

# Planr Review

Use this when a task needs a correctness and completion audit.

## Workflow

```bash
planr --json trace item <review-id>
```

Tracing the review item inlines the target item and its evidence logs under `target` — one trace is enough to see what is being audited, its status (`in_review` while waiting on you), files, and verification commands. Use `planr log list --item <target-id>` or `planr map show --json` only for deeper reads.

Inspect the actual changed files and re-run the logged verification evidence. Then close the review:

```bash
planr review close <review-id> --verdict complete --close-target
```

`--close-target` also closes the reviewed item when the verdict is complete and a completion log exists — the worker does not need a separate close round-trip. Omit it when the worker should close explicitly.

or:

```bash
planr review close <review-id> --verdict not-complete --findings "specific actionable finding"
```

## Findings Rules

- Findings must be specific and actionable.
- Missing tests are findings when acceptance criteria need proof.
- Architecture or ownership drift is a finding when it creates duplicate policy or state owners.
- If evidence is insufficient, use `--verdict unclear` rather than complete.

## Single-Agent Mode

When no independent reviewer instance is available (single-agent host), do not pretend a second instance reviewed the work. Re-read the diff, logs, and evidence with fresh eyes, then record the mode honestly before closing the review:

```bash
planr context add "review performed single-agent: same session re-read diff, logs, and evidence" --item <review-id> --tag review-mode
```

## Completion Rule

The target item may close only after required review items are closed. Use the map as the source of truth.
Pending or denied approval is also a close blocker; treat an attempted close through that gate as a finding, not as completion.
