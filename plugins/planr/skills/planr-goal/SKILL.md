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
```

`plan refine` appends notes; the plan body is yours to edit. When `plan check` fails, each warning names the exact file and section — edit that file directly, fill the section with real content, and re-run the check. Scaffold sections (`## Scope Decision`, `## Verification`, `## Acceptance Criteria`) are filled by editing the plan markdown, not by more `refine` notes.

Before `map build`, expand the plan's task list: the scaffold ships a single placeholder task, and mapping it produces one coarse item that forces the worker to guess the breakdown later. Replace the placeholder with one `### TASK-00n: <slice>` heading (or `- [ ]` line) per verifiable slice — typically 4-8, in execution order, each one closeable with its own evidence. Derive the slices from the acceptance criteria; `plan check` flags the unexpanded placeholder.

`map build` creates one item per plan step and chains them in plan order with `blocks` links; the output lists the created items and links. Review that chain and adjust it only where execution order differs from document order:

```bash
planr link add <earlier-item> <later-item> --type blocks
```

## Store The Goal Contract

The contract must survive compaction, session loss, and host switches, so it lives in Planr, not in chat:

```bash
planr context add "GOAL CONTRACT <plan-id>: DONE when every in-scope map item is closed with log evidence, all reviews closed with verdict complete, no open approvals in scope, and a live verification log exists for <oracle>. Iteration budget: 10." --tag goal-contract
```

One contract per plan scope. Any agent on any host can recover it with `planr context list --tag goal-contract` or `planr search "GOAL CONTRACT"`. Never weaken a stored contract mid-run; scope changes go through `$planr-plan` and the user. During the run, workers lease with `planr pick --plan <plan-id>` so the loop never picks items outside this contract, even when other plans share the board. The loop checks the contract with `planr plan audit <plan-id>`, which evaluates exactly these clauses with evidence and answers `holds: true/false`.

"All reviews closed" audits review items that exist — it does not require a review gate on every item. An item closed with plain `done` (evidence still required) satisfies the contract without one; request reviews where they carry signal (implementation slices, user-facing work), not on trivial inspection or scaffold steps.

## Hand Off

Print the starter command, then stop. Do not start execution yourself; ask whether to start now, refine the plan, or stop.

With a native loop driver (Codex `/goal`, Claude Code `/goal` or `/loop`):

```text
/goal Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract). Continue until the contract holds or the iteration budget is exhausted. You are operating autonomously: the user is not watching, so never end a turn on a plan, a question, or a promise — proceed until the contract holds or you are blocked on input only the user can provide.
```

Without one (Cursor, plain MCP clients, Codex without /goal):

```text
Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract). You are operating autonomously: never end a turn on a plan, a question, or a promise — proceed until the contract holds or you are blocked on input only the user can provide.
```

Re-dispatch the same line after any session ends. The map, logs, and stored contract make every iteration resumable from zero context — nothing about the goal lives only in chat.
