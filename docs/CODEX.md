# Codex Integration

## Plugin (preferred for skills)

The Planr repository is a Codex plugin. Install it to get the `$planr`, `$planr-loop`, and stage skills without copying folders:

```bash
codex plugin marketplace add instructa/planr
codex plugin add planr@planr
```

See [Skills](SKILLS.md) for the skill workflow and subagent role templates.

## Long-Running Goals With `/goal`

Codex `/goal` is the recommended orchestrator for autonomous Planr runs: `/goal` supplies continuation pressure, Planr supplies durable state, evidence, reviews, and recovery. Prep once with `$planr-goal`, then start:

```text
$planr-goal <your goal>
/goal Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract). Continue until the contract holds or the iteration budget is exhausted.
```

The stop condition lives in Planr (`--tag goal-contract`), so a dead session resumes with the same starter line from zero chat context. Full workflow, recovery, and per-host variants: [Long-Running Goals](GOALS.md).

## MCP

```bash
planr install codex --dry-run
planr doctor --client codex
```

The dry-run prints the MCP server snippet for `planr mcp`. Verify the client-side registration with the Codex CLI command shown by your local Codex installation.

Codex should use the same public flow as every other client:

```text
map -> pick -> work -> log -> review -> close
```

Review hooks can feed Planr without changing global Codex settings:

```bash
codex review --json > .planr/tmp/codex-review.json
planr review ingest <item-id> --from .planr/tmp/codex-review.json
planr review annotate <item-id> --message "Needs regression coverage" --severity blocking
```

Ingested feedback is evidence only. A reviewer or agent must still close the review item explicitly with `planr review close`.
