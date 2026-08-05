---
name: planr-verify-web
description: Frozen-source live verification for a web FeatureRun. Consumes a canonical verification work packet, uses the configured Evidence capability, and records trusted proof without editing product source.
---

# Planr Verify Web

Prove the frozen feature runs. Planr owns the evidence contract and capability selection; the host executes the configured method. Never install or configure browser infrastructure on behalf of this skill.

## Lease The Typed Verification Packet

Use a fresh verifier identity distinct from the responsible maker:

```bash
export PLANR_WORKER_ID="verifier-web-1"
planr pick --plan <plan-id> --work-type verification --json
```

This typed pick is the verifier's first action. Continue only when `work_packet.kind` is `verification`. Treat its `execution_state`, `source_freeze`, and `verification_lease` as the complete runtime contract. Product source is read-only for this pass; only Planr runtime state, receipts, logs, and artifacts may be written.

Run readiness before starting the configured method:

```bash
planr evidence readiness --scope plan --id <plan-id> --json
```

Read the returned `readiness.run_index.repository_path` and preserve it exactly. It is the only executable Evidence input; do not derive a path from the digest or substitute a repository-authored obligation, declarative index, or remembered filename.

If readiness is blocked, do not choose a different unregistered tool or downgrade the observation. The FeatureRun enters a capability hold; report the returned gap and `next_action`, then stop. Repairing policy, schema, adapter digest, runtime registration, or permissions and rerunning the same readiness command is the only resume path.

## Dev Server

After the typed pick and leased readiness, detect a running dev server and use it. Never start a second instance. Only start one (in the background, and stop it afterwards) when none is running and the loop is unattended.

## Run The Verification

Exercise the flow the item changed — not the homepage. Interact, assert on rendered output, capture a screenshot when the tier supports it.

Use only the repository capability selected by the active obligation to create trusted Evidence, then evaluate coverage:

```bash
planr evidence run --input <exact-readiness.run_index.repository_path>
planr evidence coverage --scope criterion --id <criterion-id>
planr evidence explain --scope criterion --id <criterion-id>
```

The observation contract decides what must be proved. Native Browser, CDP, Playwright, Computer Use, and HTTP probes are configurable methods, not interchangeable fallbacks. HTTP can fully prove an HTTP criterion but cannot satisfy rendered interaction, persistence, accessibility, console, or visual observations it never captured. `planr evidence run` checks the canonical `SOURCE_PATHS` digest inside the transaction; a mismatch records a failed non-covering attempt and commits zero trusted receipts.

Attach screenshots or traces as artifacts on the item:

```bash
planr artifact add "verify-web screenshot" --item <item-id> --path <screenshot-path> --kind screenshot
planr artifact add "verify-web recording" --item <item-id> --path <recording.mp4> --kind video
```

The replay contract and trusted method identity are mandatory. The reviewer validates the receipt and reruns it only when it is cheap, missing, failing, or explicitly high-risk; a verification that cannot be replayed when needed is not evidence. A successful bounded live smoke joins the existing coherent FeatureRun/ReviewGate boundary and does not automatically trigger another full build or gate replay.

For a deployment oracle, require an approved deployment decision before the deploy begins. After deployment, keep the live check bounded to the changed routes, content, or interaction and record the deployed source/receipt identity in the summary.

## When Verification Is Impossible

No configured capability or unreachable runtime: do not fake it and do not downgrade silently. Readiness records the capability hold; preserve that exact classification.

```bash
planr evidence readiness --scope plan --id <plan-id> --json
planr context add "verification hold: <readiness gap code and capability>" --tag blocker
```

Then stop until the reported capability contract is repaired. A manual approval cannot convert a missing capability into trusted Evidence.

## Outcome

- Pass: trusted receipts satisfy coverage and the FeatureRun advances toward its single final independent ReviewGate. Do not call `planr done`; the implementation outcome was already settled before source freeze.
- Product failure: Planr routes a product finding back to the responsible maker. The maker receives an outcome repair packet, fixes only the finding, then readiness re-freezes the source and the verifier selectively reruns only invalidated Evidence.
- Verifier or environment failure: record the non-covering attempt and stop. It is not protected product risk and must not open an ad hoc ReviewGate.
