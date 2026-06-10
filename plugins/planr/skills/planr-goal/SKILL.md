---
name: planr-goal
description: Prepare a long-running goal for autonomous execution on Planr. Use when the user states a broad goal, wants to start a /goal run, or asks to set up a goal. Compiles intent into a checked plan, a linked map, and a durable goal contract, then prints the starter command for the host's loop driver without implementing anything.
---

# Planr Goal

Prep only. This skill compiles a goal into durable Planr state and stops. Execution belongs to the host's loop driver — Codex or Claude Code `/goal`, an automation, or repeated `$planr-loop` dispatches. This boundary is strict: goal prep is a board compiler, not a lightweight loop.

## Boundary

During a goal-prep turn, do not implement, refactor, verify, or touch product code — even when the work looks read-only or obviously useful. That work belongs to the loop run. Allowed actions: read Planr state, ask intake questions, create/refine/check plans, build and link the map, store the goal contract, print the starter command.

## Intake

Classify the input before creating anything:

- specific: outcome and proof are clear -> compile directly.
- vague: ask at most two material questions (which outcome matters most? what proof would convince you it works?), then proceed with labeled assumptions recorded as `plan refine` notes.
- existing plan: preserve user-provided steps, files, and constraints as `plan refine` notes; validate them, do not rediscover from scratch, and do not discard them.
- recovery: read `planr map show --json` and `planr recover sweep` first — the board may already exist. Repair state instead of duplicating it.

Always capture the goal oracle: the observable signal that proves the outcome actually runs — a test run, a live browser flow, an executed CLI binary, real API responses. Weak proof creates weak goals. The oracle becomes the live-verification half of the stop condition and maps to the loop's platform verification (`$planr-verify-web` for web, simulator for ios, executed binary for cli, real requests for api).

## Compile The Board

```bash
planr project show --json                 # init only if no project exists
planr plan new "<goal>" --platform <p>
planr plan refine <plan-id> --note "constraint, assumption, or user-provided plan fact"
planr plan check <plan-id>                # strict: empty required sections fail
planr map build --from <plan-id>          # idempotent: safe to re-run
planr link add <earlier-item> <later-item> --type blocks   # for every execution-order dependency
```

Linking is part of prep, not optional: an unlinked map makes the loop pick items in arbitrary order.

## Store The Goal Contract

The contract must survive compaction, session loss, and host switches, so it lives in Planr, not in chat:

```bash
planr context add "GOAL CONTRACT <plan-id>: DONE when every in-scope map item is closed with log evidence, all reviews closed with verdict complete, no open approvals in scope, and a live verification log exists for <oracle>. Iteration budget: 10." --tag goal-contract
```

One contract per plan scope. Any agent on any host can recover it with `planr context list` or `planr search "GOAL CONTRACT"`. Never weaken a stored contract mid-run; scope changes go through `$planr-plan` and the user. During the run, workers lease with `planr pick --plan <plan-id>` so the loop never picks items outside this contract, even when other plans share the board.

## Hand Off

Print the starter command, then stop. Do not start execution yourself; ask whether to start now, refine the plan, or stop.

With a native loop driver (Codex `/goal`, Claude Code `/goal` or `/loop`):

```text
/goal Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract). Continue until the contract holds or the iteration budget is exhausted.
```

Without one (Cursor, plain MCP clients, Codex without /goal):

```text
Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract).
```

Re-dispatch the same line after any session ends. The map, logs, and stored contract make every iteration resumable from zero context — nothing about the goal lives only in chat.
