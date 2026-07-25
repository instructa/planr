---
name: planr-loop
description: Drive one Planr feature or scope autonomously to audit-backed completion through scoped work, live verification, and independent review.
---

# Planr Loop

This is the iteration protocol, not the re-prompting driver. Run one goal with a checkable stop condition and an iteration budget (default 10). Refuse multi-goal loops.

Evaluation subcommands run only when the user explicitly requests them, the selected item's acceptance criteria require them, or the maintainer release workflow invokes them. Never invoke them as routine or opportunistic goal, loop, or task-graph work.

## Contract And Iteration

Recover the stored `GOAL CONTRACT <plan-id>` from Planr every iteration. If absent, store one that requires settled items with evidence, complete reviews, clear approvals, and a live oracle. Never weaken it.

Each iteration follows the Planr stage protocols:

1. `planr plan audit <plan-id> --json`; `holds: true` exits.
2. Use `$planr-plan` or `$planr-task-graph` only when scope or graph structure is missing.
3. Dispatch `$planr-work` for exactly one ready item scoped to `<plan-id>`; makers must use `planr pick --work-type code --plan <plan-id>`, never an unscoped pick, and finish implementation with `planr done <item-id> ... --review`.
4. Run the target-platform oracle and record `planr log add --item <id> --kind verification --summary ... --cmd ...`.
5. Dispatch `$planr-review`; findings create fix work, while `complete --close-target` settles the target.
6. Repeat from audit.

One picked item per iteration. Use plain `done` only for low-signal setup/inspection work. Maker and checker stay separate when the host supports another agent; a maker never self-reviews when an independent checker is available, and never manufactures independence by changing worker identity. A worker may use `done --next`, which never returns its own review.

Pick packets explain null results and include `remaining`; follow their repair command. Destructive or out-of-repository effects require `planr approval request`. Two iterations without map movement must stop. On success or budget exhausted, finish with `$planr-summary`.

## Provider-Neutral Dispatch

The driver dispatches and audits; it does not implement when subagents are available. Pick packets expose provider-neutral `routing.profile`; they do not expose a host-owned `routing.agent_type`. If a generated repository role exactly matches that profile, dispatch that profile identifier as the host-native role/`agent_type`. If no matching repository role exists, keep the host's default dispatch contract and treat routing as advisory.

Model, effort, profile, client, and fallback fields are advisory declarations and evidence labels only. Planr chooses none of them. Never infer effective model or effort from declarations. Workers report their actual profile and attach route observations when available.

Dispatch messages stay minimal:

- maker: `Use $planr-work on item <item-id>. Stop after requesting review.`
- checker: `Use $planr-review on item <item-id>. Close the review with a verdict.`

For generated Codex roles: The `spawn_agent` tool call itself must include `agent_type` set exactly to the matching `routing.profile`, `fork_turns: "none"`, a stable lowercase task name, and the maker/checker message. Read [host dispatch](references/host-dispatch.md) only when choosing host-specific wiring.

## Verification And Recovery

“Done” means the feature ran. For web dispatch `$planr-verify-web`; for CLI execute the built binary; for API use real requests; for iOS launch the simulator. Log the replayable command. If the capability is missing, record a blocker context, request approval, and pause—never fake proof.

Recovery starts in a fresh session with audit, map state, the stored contract, and the next scoped pick. Read [recovery and platform details](references/recovery-and-verification.md) only when that branch is active.

## Hard Rules

- Keep one active write item unless separate worktrees and loop instances were explicitly authorized.
- Scope changes go through `$planr-plan` and the user.
- Do not close your own review when independent review is available.
- Keep final claims aligned with map, logs, approvals, reviews, and oracle evidence.
