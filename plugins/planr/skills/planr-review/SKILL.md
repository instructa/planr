---
name: planr-review
description: Review a Planr item or scoped implementation against map state, plan acceptance criteria, logs, changed files, and verification evidence. Use for findings-first audits and review gates.
---

# Planr Review

Use this when a task needs a correctness and completion audit.

## Workflow

Export your checker identity once per session so the lease attributes to you:

```bash
export PLANR_WORKER_ID=checker-1
planr --json pick --work-type review
```

`--work-type review` leases only review items, so a checker never accidentally takes maker work. The pick packet inlines the target item and its evidence logs under `target` — one command is enough to see what is being audited, its status (`in_review` while waiting on you), files, and verification commands. If you already hold a review id, `planr --json trace item <review-id>` returns the same packet. Use `planr log list --item <target-id>` or `planr map show --json` only for deeper reads.

Inspect the actual changed files and re-run the logged verification evidence. Then close the review exactly once:

```bash
planr review close <review-id> --verdict complete --reviewer <your-id> --close-target
```

`--close-target` also closes the reviewed item when the verdict is complete and a completion log exists — the worker does not need a separate close round-trip. Omit it when the worker should close explicitly. `--reviewer` records your checker identity on the log, artifact, and event so maker and checker stay distinguishable in the audit trail. A second close of the same review fails with `already_closed` — never retry a close that succeeded.

or:

```bash
planr review close <review-id> --verdict not-complete --reviewer <your-id> --findings "specific actionable finding"
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
