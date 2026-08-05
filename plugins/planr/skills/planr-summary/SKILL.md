---
name: planr-summary
description: Summarize a Planr scope after work, review, or status inspection. Use when the user wants what changed, why, what works now, verification evidence, and what remains.
---

# Planr Summary

Build the summary from current Planr evidence:

```bash
planr map show --json
planr plan audit <plan-id> --json
planr trace item <item-id>
planr log list --item <item-id>
```

## Output

Include:

- scope;
- what changed;
- why;
- what works now;
- verification commands and results;
- open blockers or unverified items;
- final product ReviewGate state when the summary is plan-scoped;
- next recommended Planr command.

If completion is not proven, say so directly and recommend `planr-status`, `planr-work`, or `planr-review`.
