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
/goal Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract). Continue until the contract holds or the iteration budget is exhausted. You are operating autonomously: the user is not watching, so never end a turn on a plan, a question, or a promise — proceed until the contract holds or you are blocked on input only the user can provide.
```

The stop condition lives in Planr (`--tag goal-contract`), so a dead session resumes with the same starter line from zero chat context.

Apply the native Codex preset in the repository before starting the driver:

```bash
planr agents preset apply balanced --binding codex-openai --live-host-command /absolute/codex-adapter --trusted-telemetry-signer codex --trusted-telemetry-collector /absolute/collector --preview --json
planr agents preset apply balanced --binding codex-openai --live-host-command /absolute/codex-adapter --trusted-telemetry-signer codex --trusted-telemetry-collector /absolute/collector --confirm --json
```

The preset generates repository-local Sol Medium, Terra Medium/High, Luna xHigh, Sol High, and Sol Ultra role TOMLs plus `.codex/skills/planr-native-routing/SKILL.md`. Dispatch the table's role with an explicit `agent_type` and `fork_turns: "none"` (or an evidenced positive bounded count); never repeat model or effort at the call site. Native Codex rejects `fork_turns: "all"` for these overridden roles. Restart after applying changed roles, then confirm child metadata reports the expected model/effort and a non-null `agent_path`. Full workflow and tiering rationale: [Long-Running Goals](GOALS.md).

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
