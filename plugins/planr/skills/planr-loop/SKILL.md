---
name: planr-loop
description: Drive one Planr feature or scope autonomously to audit-backed completion through scoped work, live verification, and independent review.
---

# Planr Loop

This is the iteration protocol, not the re-prompting driver. Run one goal with a checkable stop condition and an iteration budget (default 10). Refuse multi-goal loops.

Evaluation subcommands run only when the user explicitly requests them, the selected item's acceptance criteria require them, or the maintainer release workflow invokes them. Never invoke them as routine or opportunistic goal, loop, or task-graph work.

## Contract And Iteration

Recover the stored `GOAL CONTRACT <plan-id>` from Planr every iteration. If absent, store one that requires settled outcomes with evidence, accepted required ReviewGates, clear approvals, and a live oracle. Never weaken it.

At the start of the host thread that drives an active goal, run `planr stop activate --plan <plan-id>` once. Codex hosts should let Planr use `CODEX_THREAD_ID`; otherwise provide a stable explicit session with `--session <id>`. Leave the binding active until `planr plan audit <plan-id> --json` returns `holds: true`. Run `planr stop deactivate --plan <plan-id>` only for explicit user cancellation or after a durable ownership transfer has activated the same plan in the successor session; a budgeted handoff alone must not deactivate an unfinished goal.

Each iteration follows the Planr stage protocols:

1. `planr plan audit <plan-id> --json`; `holds: true` exits. If audit reports the final product ReviewGate is missing after all outcomes and material gates have settled, run `planr plan final-review <plan-id>` and dispatch that gate instead of creating an ad hoc checkpoint.
2. For a binding Evidence plan, inspect existing `evidence coverage` / `evidence explain` output to understand current gaps. Do not run readiness or evidence runs before every mutable slice. Batch compatible implementation work first, settle source, then let the coordinator call `planr evidence readiness --scope plan --id <plan-id> --json` once to establish the source freeze. For a frozen FeatureRun, the expected lease-first response names `planr pick --plan <plan-id> --work-type verification --json`; dispatch that exact continuation instead of probing or running Evidence in the coordinator. The exception is a genuinely material ReviewGate that needs exact evidence earlier; bind only that gate's affected criterion and record why.
3. Use `$planr-plan` or `$planr-task-graph` only when scope or graph structure is missing.
4. Dispatch `$planr-work` for a compatible same-plan maker run. The maker keeps one `PLANR_WORKER_ID`, uses `planr pick --work-type code --plan <plan-id>`, and branches only on the returned typed `work_packet`. It settles `kind: "outcome"` packets one outcome at a time with `planr done --next`; `mode: "finding_repair"` returns named findings to the same maker and same ReviewGate without a fix item; `kind: "hold"` stops. `done --next` atomically rolls the internal three-outcome ExecutionBatch for the same maker and returns the next plan-scoped code packet, so the root does not wake at that boundary. Stop when settlement opens a material ReviewGate, returns `next.reason: "verification_handoff_source_frozen"`, ownership or scope becomes incompatible, work blocks, the pick is empty, or a budget boundary is reached. Each genuine stop records a compact durable handoff: settled item ids, changed files, commands/tests, source-freeze status, and the exact next command from the packet.
5. After `verification_handoff_source_frozen`, dispatch a fresh verification-only worker with a verifier identity distinct from the maker. Its first command is the packet's `commands.lease_verifier` (`planr pick --plan <plan-id> --work-type verification --json`); continue only for `work_packet.kind: "verification"` and use its bound `item_id`, `source_freeze`, and `verification_lease`. Under that same leased identity, run the packet's readiness command, treat its `run_index.repository_path` as the only executable Evidence input, and execute `planr evidence run --input <that-exact-path>`. Product source remains read-only by the canonical Evidence `SOURCE_PATHS` digest. The verifier may run build/oracle/Evidence commands and write receipts, logs, and artifacts, but source mismatch inside the Evidence transaction records a failed non-covering attempt and zero trusted receipts. Product findings stay on the existing ReviewGate and route back to its responsible maker; after repair the coordinator re-freezes, then a leased verifier reruns readiness before selectively rerunning only invalidated Evidence. Deployment still requires prior human approval and a bounded live oracle. Narrative logs never substitute for binding receipts.
6. Dispatch `$planr-review`; the checker independently inspects the diff and validates the exact-source receipt, replaying only cheap, missing, failing, or explicitly high-risk evidence. Findings remain on the same ReviewGate for responsible-maker repair and explicit resolution; an accepted risk gate resumes the FeatureRun. Keep exactly one final independent product ReviewGate for the plan.
7. Repeat from audit.

One active write item at a time inside a compatible same-plan maker run. Each internal ExecutionBatch remains durably capped at three outcomes, but `done --next` rolls that boundary for the same maker without a root round trip or a second completion log. A small coherent change stays one implementation outcome with one signal-bearing ReviewGate; do not create a new gate for every mechanical stage or for an already-verified successful live smoke. Use `done --next` for ordinary outcomes in the authorized run and plain `done` for standalone settlement; only a real protected-risk interrupt uses structured escalation flags. Maker and checker stay separate when the host supports another agent; a maker never leases its own ReviewGate and never manufactures independence by changing worker identity. The reviewer must exercise independent judgment even when it relies on a green receipt rather than replaying an expensive gate.

Pick packets explain null results and include `remaining`; follow their repair command. Destructive or out-of-repository effects require `planr approval request`. Two iterations without map movement must stop. On success or budget exhausted, finish with `$planr-summary`.

## Provider-Neutral Dispatch

The driver dispatches and audits from Planr state; it does not implement or inspect product source when subagents are available. Pick packets expose provider-neutral `routing.profile`; they do not expose a host-owned `routing.agent_type`. If a generated repository role exactly matches that profile, dispatch that profile identifier as the host-native role/`agent_type`. If no matching repository role exists, keep the host's default dispatch contract and treat routing as advisory.

Model, effort, profile, client, and fallback fields are advisory declarations and evidence labels only. Planr chooses none of them. Never infer effective model or effort from declarations. Workers report their actual profile and attach route observations when available.

Dispatch messages stay minimal:

- maker: `Use $planr-work on item <item-id> as the first item in a compatible same-plan maker run. Keep one worker identity, settle each ordinary outcome with planr done --next, write a compact durable handoff only at a genuine stop, and stop when settlement opens a material ReviewGate, work blocks, ownership is incompatible, the pick is empty, or the budget is reached.`
- checker: `Use $planr-review on ReviewGate <review-gate-id>. Close the gate with a verdict.`
- verifier: `Verification-only pass for plan <plan-id>. Keep one fresh verifier identity distinct from the maker. Your first command is planr pick --plan <plan-id> --work-type verification --json; continue only for work_packet.kind verification. Under that same lease run planr evidence readiness --scope plan --id <plan-id> --json, then execute only readiness.run_index.repository_path with planr evidence run --input. Product source is read-only by the canonical Evidence SOURCE_PATHS digest. Log receipts/artifacts, route product findings back to the responsible maker, and preserve exactly one final independent product ReviewGate.`

For generated Codex roles: The `spawn_agent` tool call itself must include `agent_type` set exactly to the matching `routing.profile`, `fork_turns: "none"`, a stable lowercase task name, and the maker/checker message. Read [host dispatch](references/host-dispatch.md) only when choosing host-specific wiring.

For every Codex Planr maker or checker dispatch, including default-role dispatch when no generated role exists, set `fork_turns: "none"` on the `spawn_agent` call. Spawn each maker or checker role once. Put the scoped instructions in the initial task. `planr done --next` owns the internal batch-cap rollover and next compatible lease; that internal transition is transparent to the driver and does not wake the root. When a material outcome pauses for independent review, keep the maker agent live if the host still reports it available; route review findings back to that same maker with `followup_task`, then after accepted re-review reuse that maker again for the next compatible same-plan outcome. Do not spawn a replacement maker unless the host reports the original unavailable, its context is lost, ownership is incompatible, or Planr state no longer has compatible work for that worker identity. Use one completion-length `wait_agent` with `timeout_ms: 3600000` for the spawned role; do not implement a 60-second polling loop. Call `list_agents`, another `wait_agent`, or recovery checks only after that wait times out, the role reports lost state, or explicit user steering. Do not repair an invalid spawn with repeated spawn attempts; return to durable Planr state, audit/pick again, and dispatch a fresh role only when the map still requires it.

## Verification And Recovery

“Done” means the feature ran, but binding live verification belongs after the mutable implementation run reaches source freeze. Do not launch an opportunistic browser/live smoke between compatible outcomes. After freeze, for web dispatch `$planr-verify-web`; for CLI execute the built binary; for API use real requests; for iOS launch the simulator. Log the replayable command. A passing bounded live oracle is evidence for the existing ReviewGate, not a reason to start another gate. If the capability is missing, record a blocker context, request approval, and pause—never fake proof.

Recovery starts in a fresh session with audit, map state, Evidence readiness/explain, the stored contract, and the next scoped pick. A terminal unchanged Stop gap remains terminal; use `planr stop resume --plan <plan-id>` only after an explicit operator decision to reopen its bounded continuation window. Read [recovery and platform details](references/recovery-and-verification.md) only when that branch is active.

## Hard Rules

- Keep one active write item unless separate worktrees and loop instances were explicitly authorized.
- Scope changes go through `$planr-plan` and the user.
- Do not lease or close your own ReviewGate.
- Keep final claims aligned with map, logs, approvals, reviews, and oracle evidence.
