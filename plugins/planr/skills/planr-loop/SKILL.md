---
name: planr-loop
description: Drive one feature or scope to verified completion without per-step human prompting. Use when the user says build until done, loop on this, or finish autonomously. Sequences plan, map, work, live verification, and independent review until the map is clean and evidence proves the feature actually runs.
---

# Planr Loop

A closing loop: the agent prompts itself with Planr skills until a verifiable stop condition holds. The human supplies the goal at the start and reviews at the end; the map is the loop memory between iterations.

## Loop Contract

Before iterating, pin the contract:

1. One goal: a single feature, fix scope, or build plan. Refuse multi-goal loops; split them first.
2. A stop condition that a different agent can check from map state and evidence, not from the worker's claims.
3. An iteration budget (default: 10 iterations). When exhausted, stop and report honestly instead of grinding.

Stop condition template:

```text
DONE when: every in-scope map item is closed with log evidence,
all reviews closed with verdict complete, no open approvals in scope,
and a live verification log exists for the feature on its target platform.
```

## Iteration Shape

Each iteration is one dispatch through the routing skill — never a hand-written prompt:

```text
1. $planr-status      read honest state; if stop condition holds -> exit loop
2. $planr-plan / $planr-task-graph   only if scope or map structure is missing
3. $planr-work        pick exactly one ready item, implement, log evidence
4. live verify        run the platform verification (below), log it with planr log add --cmd
5. $planr-review      independent audit; complete -> close, findings -> Planr creates fix items
6. repeat             fix items are just the next ready items
```

After any `planr map build`, dependency linking is part of step 2, not optional: add `blocks` links for every execution-order dependency before the first pick. An unlinked map makes the loop pick items in arbitrary order.

The loop never closes its own reviews when the host supports a second agent. Maker and checker stay separate.

## Skills Are The Prompts

When the host supports subagents, delegate with skill references plus an item id, nothing more:

- Worker dispatch: `Use $planr-work on item <item-id>. Stop after requesting review.`
- Checker dispatch: `Use $planr-review on item <item-id>. Close the review with a verdict.`

Host wiring:

- Codex: project agents in `.codex/agents/*.toml` preload the skill via `[[skills.config]]` (TOML templates in `agents/` next to this skill). Spawn explicitly: "spawn the planr_worker agent for item X". Keep `[agents] max_depth = 1`.
- Claude Code: subagents preload via the `skills:` frontmatter field. The Planr plugin registers `planr-worker` and `planr-reviewer` automatically from its `agents/` directory; standalone installs copy them to `.claude/agents/`. The reviewer subagent is read-only except for `planr review` commands.
- Single-agent hosts: run worker and checker as separate sequential dispatches with a fresh read of map state in between; never carry the worker's self-assessment into the review step. Record the mode honestly per `$planr-review` single-agent mode (`planr context add ... --tag review-mode`).

## Live Verification By Platform

"Done" means the feature ran, not that it compiles. Pick the verification from the plan's platform (`planr plan new ... --platform <p>`), run it inside step 4, and log the exact command and outcome:

| Platform | Verification |
| --- | --- |
| `web` | dispatch `$planr-verify-web`: discovers the host's browser capability, runs the changed flow against the dev server, logs a replayable command |
| `ios` | build and launch in the simulator (`xcodebuild` + `xcrun simctl`), exercise the changed flow |
| `cli` | execute the built binary with the real flags the feature added; assert on output |
| `api`/`backend` | start the service, hit the changed endpoints with real requests, assert responses |

```bash
planr log add --item <item-id> \
  --summary "live verification on <platform>" \
  --cmd "<exact command actually run>"
```

If the needed capability is missing (no simulator, no browser tooling), do not fake it: log the gap as context, request human approval, and pause the loop:

```bash
planr context add "live verification blocked: <missing capability>" --item <item-id> --tag blocker
planr approval request <item-id> --reason "manual live verification required"
```

## Native Loop Runners

Prefer the host's loop primitive over a bash while-loop so a separate model checks the stop condition:

- Claude Code: `/goal <stop condition>` with this skill loaded; or `/loop` for a fixed cadence.
- Codex: `/goal`, or an Automation whose prompt is `Use $planr-loop on <scope>`.
- Anywhere else: re-dispatch `$planr-loop` manually; the map makes every iteration resumable from zero context.

## Hard Rules

- One picked item per iteration. Parallel work needs separate worktrees and separate loop instances.
- Every iteration must move map state (a log, a review verdict, a closed item, or a recorded blocker). Two iterations without state movement -> stop and report.
- Never weaken the stop condition mid-loop. Scope changes go through `$planr-plan` and the user.
- Destructive or out-of-repo side effects (deploys, migrations, infra) always go behind `planr approval request`.
- On exit — success or budget exhausted — finish with `$planr-summary` so the human gets an evidence-backed account.
