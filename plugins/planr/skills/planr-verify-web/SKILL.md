---
name: planr-verify-web
description: Frozen-source live verification for a web FeatureRun. Consumes a canonical verification work packet, uses the configured Evidence capability, and records trusted proof without editing product source.
---

# Planr Verify Web

Prove the frozen feature runs. Planr owns the evidence contract and capability selection; the host executes the configured method. Never install or configure browser infrastructure on behalf of this skill.

## Run The Typed Verification Packet

Keep verification in the coordinator. Use one stable worker identity that differs from the responsible maker. Do not spawn another model.

```bash
PLANR_WORKER_ID="coordinator-verifier-1" planr evidence verify --scope plan --id <plan-id> --json
```

This command leases verification, probes readiness, seals the run index, executes the configured adapter, evaluates coverage, and settles the FeatureRun. Product source is read-only. Only Planr runtime state, receipts, logs, and artifacts may change.

Require `planr.execution_state.v2`; its budget and absolute deadline are opaque supplied authority. Skills must not recompute budget policy. If the selected adapter cannot honor required capability or the packet is held, stop with that exact classification.

If readiness is blocked, do not choose another unregistered tool or downgrade the observation. The FeatureRun enters a capability hold. Report the returned gap and `next_action`, then stop. Repair the policy, schema, adapter digest, runtime registration, or permissions before you run the same verify command again.

## Target Lifecycle

The configured Evidence adapter owns target startup, connection, and cleanup. Do not manually start a duplicate browser or application process unless the sealed work packet explicitly declares an externally managed target.

## Run The Verification

Exercise the flow the item changed — not the homepage. Interact and assert on the required rendered output. Capture a screenshot only for a visual criterion or failure diagnostics.

Use only the repository capability selected by the active obligation. Read `object.coverage`, `object.feature_run_verification_settlement`, and `object.verification_broker` from the verify result. Do not issue per-criterion coverage or explain commands on the normal path. They are diagnostic commands for a known gap, and `--scope criterion` accepts a criterion id, never a requirement id.

The observation contract decides what must be proved. Native Browser, CDP, Playwright, Computer Use, and HTTP probes are configurable methods, not interchangeable fallbacks. HTTP can prove an HTTP criterion but cannot satisfy rendered interaction, persistence, accessibility, console, or visual observations it never captured. `planr evidence verify` checks the canonical `SOURCE_PATHS` digest inside the transaction. A mismatch records a failed non-covering attempt and commits zero trusted receipts.

Attach screenshots or traces as artifacts on the item:

```bash
planr artifact add "verify-web screenshot" --item <item-id> --path <screenshot-path> --kind screenshot
planr artifact add "verify-web recording" --item <item-id> --path <recording.mp4> --kind video
```

The replay contract and trusted method identity are mandatory. A successful bounded live verification closes a Binding Evidence FeatureRun directly when all ordinary outcomes and coverage are settled. It does not trigger another model, reviewer, build, or bookkeeping gate. An explicitly required material ReviewGate remains independent of this normal closure path.

For a deployment oracle, require an approved deployment decision before the deploy begins. After deployment, keep the live check bounded to the changed routes, content, or interaction and record the deployed source/receipt identity in the summary.

## When Verification Is Impossible

If no configured capability is available or the runtime is unreachable, do not fake Evidence or downgrade it. The broker records the capability hold. Preserve that exact classification.

```bash
planr evidence verify --scope plan --id <plan-id> --json
planr context add "verification hold: <readiness gap code and capability>" --tag blocker
```

Then stop until the reported capability contract is repaired. A manual approval cannot convert a missing capability into trusted Evidence.

## Outcome

- Pass: the same `evidence verify --json` result reports satisfied plan coverage and a complete FeatureRun settlement. Do not call `planr done`; the implementation outcome was already settled before source freeze.
- Product failure: Planr routes a product finding back to the responsible maker. The maker receives an outcome repair packet and fixes only the finding. The coordinator then re-freezes the source and calls `evidence verify` for only invalidated Evidence.
- Verifier or environment failure: record the non-covering attempt and stop immediately. Do not retry, create a new verifier lease, or open an ad hoc ReviewGate. A later run is allowed only after the reported capability or environment state has materially changed.
