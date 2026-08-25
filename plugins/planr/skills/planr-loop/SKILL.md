---
name: planr-loop
description: Drive one Planr feature or scope autonomously to audit-backed completion through direct maker work and trusted verification, with independent review only for explicit material risk.
---

# Planr Loop

This skill is the complete sequential execution contract. Do not reload `$planr`
or `$planr-work` after the request has routed here.

The invoking session is the default maker. It inspects and changes product source
directly, keeps one stable worker identity, and consumes compatible typed packets
until a genuine stop. Do not spawn a coordinator or maker for the default path.

Evaluation subcommands run only when the user requests them, an acceptance
criterion requires them, or the maintainer release workflow invokes them. Never
run an eval as routine loop work.

## Evidence admission guard

Before product work that requires binding Evidence, inspect the repository's
existing Evidence policy and configured capability. If either is absent, record
the exact gap code, capability, and next action, then stop. Do not create or
weaken an Evidence policy, schema, capability, adapter, migration, or run index
to bypass a hold. Only the owner of explicitly requested verification
infrastructure may repair that contract before product work resumes.

## Execute the loop

Use one plan and a checkable stop condition. The default iteration budget is 10.
Refuse a request that combines unrelated goals.

1. Recover the stored `GOAL CONTRACT <plan-id>`. If it is absent, store one that
   requires settled outcomes, binding Evidence, clear approvals, and only the
   material ReviewGates required by policy or the plan. Run
   `planr stop activate --plan <plan-id>` once for the host thread. Codex hosts
   let Planr use `CODEX_THREAD_ID`; other hosts provide one stable explicit
   session.
2. Run `planr plan audit <plan-id> --json`. Exit when `holds: true`. If scope,
   the checked build plan, or the map is missing, use `$planr-plan` or
   `$planr-task-graph` in this session, then return to the audit.
3. Become the maker in this session. Export one stable identity, then lease only
   plan-scoped outcome work:

   ```bash
   export PLANR_WORKER_ID="maker-<stable-session-id>"
   planr pick --work-type code --plan <plan-id> --json
   ```

   Require `work_packet.execution_state.schema_version` to equal
   `planr.execution_state.v2`. Treat its budget as opaque authority. Branch only
   on `work_packet.kind` and optional `work_packet.mode`.

   - `kind: "outcome"` is maker work. Read the linked plan, inspect product
     source, implement the smallest correct slice, run the repository-selected
     checks once, and settle with `planr done <item-id> ... --next --json`.
   - `mode: "finding_repair"` repairs only the named findings on the same
     ReviewGate. It creates no fix item. Resolve the findings and stop for
     re-review.
   - `kind: "hold"` stops. Never replace policy with host behavior.

   `done --next` atomically settles the outcome, rolls the internal
   three-outcome ExecutionBatch when needed, and returns the next compatible
   packet to the same maker. Continue in this session. Do not wake a root driver
   or reload skills at the batch boundary.

   Stop only when settlement opens a material ReviewGate, returns
   `next.reason: "verification_handoff_source_frozen"`, ownership becomes
   incompatible, work blocks, the pick is empty, or the budget ends. At a genuine
   stop, write one compact durable handoff with settled item IDs, changed files,
   commands, results, source-freeze status, and the packet's exact next command.
4. After `verification_handoff_source_frozen`, keep verification in the invoking
   session. Switch to a stable verifier identity that differs from the maker, and
   run the packet's exact `commands.verify` command once:

   ```bash
   PLANR_WORKER_ID="coordinator-verifier-<stable-session-id>" \
     planr evidence verify --scope plan --id <plan-id> --json
   ```

   The Evidence broker leases verification, seals readiness, executes the
   configured adapter, evaluates Coverage, and settles the FeatureRun. It starts
   no model. Product source remains read-only under the canonical Evidence
   `SOURCE_PATHS` digest. A verifier or environment failure stops immediately.
   Resume only after the external state changes. Do not spawn a verifier agent
   or model. Route a ProductFinding back to the same responsible maker.
   After repair and a new freeze, verify only invalidated Evidence.
5. When a typed packet names an explicit material ReviewGate, dispatch one
   distinct checker through `$planr-review`. The maker never leases or closes its
   own gate. An accepted gate resumes the same maker identity when compatible
   work remains.
6. Return to the audit. Two iterations without durable map movement stop. On
   success or budget exhaustion, finish with `$planr-summary`.

## Optional host dispatch

Host-native maker dispatch is not part of the sequential default. Read
[host dispatch](references/host-dispatch.md) only when the user explicitly asks
for delegation, the plan has independent parallel branches with isolated
ownership, or the active session is unavailable, context-lost, or incompatible
with the next owner. A material ReviewGate still uses a separate checker.

Pick packets expose provider-neutral `routing.profile`. If an optional dispatch
uses a generated repository role that exactly matches the profile, dispatch that
profile identifier as the host-native role/`agent_type`. Pick packets do not
expose a host-owned `routing.agent_type`. If no role matches, do not invent one;
keep the active session for sequential work. Model, effort, profile, client, and
fallback fields are advisory declarations and evidence labels only. Workers
report their actual profile and attach route observations when available.

Read [recovery and platform details](references/recovery-and-verification.md)
only when interruption, lost context, or platform-specific recovery is active.

## Hard rules

- Keep one active write item unless the user authorizes isolated parallel work.
- Keep graph state, leases, budgets, ReviewGates, Evidence, and closure in Planr
  Core. Skills orchestrate those authorities; they do not reproduce them.
- Keep the same maker across compatible outcomes and internal batch rolls.
- Use a distinct checker only for an explicit material ReviewGate.
- Use the Evidence broker after source freeze. Do not start a verifier model.
- Scope changes go through `$planr-plan` and the user.
- Destructive or out-of-repository effects require `planr approval request`.
- Do not deactivate an unfinished goal for a budget handoff. Deactivate only
  after completion, explicit cancellation, or an activated durable successor.
  Use `planr stop deactivate --plan <plan-id>` only at that boundary.
