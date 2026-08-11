---
name: planr-work
description: Execute one Planr outcome item to evidence-backed settlement in its FeatureRun. Use after code work has been picked or when the next step is implementation, docs, tests, scripts, integration work, or finding repair.
---

# Planr Work

Use this for one picked item at a time.

## Workflow

Export your worker identity once per session so picks, logs, and heartbeats attribute to you instead of `client:host:user`:

```bash
export PLANR_WORKER_ID=maker-1
planr pick --json
```

The pick output is one flat typed packet. Branch only on `work_packet.kind` and, for outcome work, optional `work_packet.mode`; read lifecycle facts only from `work_packet.execution_state`. `kind: "outcome"` with an item id is ordinary maker work. `kind: "outcome", mode: "finding_repair"` has no synthetic fix item: repair the named findings on the same ReviewGate, log evidence against the gate's scoped outcome, resolve those finding ids, and stop for re-review. `kind: "hold"` is a stop, not permission to replace the maker or bypass policy. Each fact appears once; a missing key means "empty". No separate `trace item` call is needed. Add `--work-type code` to lease only outcome work while checker agents lease ReviewGates, and `--plan <plan-id>` when your dispatch names a plan. A null pick explains itself with a reason, remaining snapshot, exclusions, and a repair command when applicable. Read the linked plan/context, implement the smallest correct slice, then finish ordinary outcome work in one command:

Require `work_packet.execution_state.schema_version` to be `planr.execution_state.v2`. Treat its `budget` object—including consumed, reserved, protected, available, provenance, contract digest, and absolute task deadline—as opaque runtime authority. Skills must not recompute budget policy; a hold or unavailable required value is a stop.

```bash
planr done <item-id> --summary "what changed" --files path-a --files path-b --cmd "exact verification command" --tests "exact test command" --next
```

Use `--next` when the dispatch authorizes a compatible same-plan maker run. Settlement, the internal three-outcome batch rollover, and the next plan-scoped code lease then happen atomically. Omit `--next` for a standalone outcome.

Before choosing ad hoc checks, inspect the repository's versioned verification policy. When it supplies a runner, use that runner once for the changed-file set and preserve its exact-source receipt. For this repository the canonical flow is:

```bash
npm run verification:run -- --receipt .planr/receipts/<name>.json --base <base-revision> --head <source-revision>
npm run verification:verify -- --receipt .planr/receipts/<name>.json --base <base-revision> --head <source-revision>
```

Record the receipt path, digest, source revision, selected profile/gates, and copy-paste replayable validation command in the completion evidence. Do not manually add broader suites that the selected policy does not require, and do not rerun an expensive green gate merely to hand work to the reviewer.

Before relying on materiality outcomes, confirm the project has selected a committed `.planr/policy.toml`. Missing policy is fail-closed and may require review, but it is not an efficiency baseline; dogfood maker runs should use the repository policy profile and `planr done --next` so materiality, not unconditional escalation, decides.

Put build/serve commands in `--cmd` and test runs in `--tests` — both are recorded as evidence. When the pick packet carries a `routing` block, also report the registry profile you actually ran on: add `--profile <profile-id>` to `done`/`log add`, or export `PLANR_PROFILE` once per session. It is part of the evidence — a mismatch with the declared route is advisory (never blocks the close) and surfaces in `planr trace item` so silent host overrides get caught. Include the decisive output line in `--summary` (e.g. "12 tests passed", "GET /videos returned 3 entries"): reviewers see your recorded command strings, not your terminal, so the summary must carry what you observed, not just what you ran. Single-quote `--files` values that contain `$` (route files like `watch.$videoId.tsx`), or the shell expands them before planr sees them. `done --next` is the standard settlement path inside an authorized compatible maker run: Planr records the completion once, lets FeatureRun materiality decide whether to interrupt, atomically rolls an internal capped batch for the same maker when required, and leases only compatible code work from the same plan. Plain `done` settles a standalone outcome without leasing more work. Intentional escalation beyond computed materiality uses `--escalate <reason>` with `--escalation-ref` and `--escalation-explanation`. If settlement returns a review gate or reports `materiality.effective_review.required: true`, stop; the FeatureRun is waiting on that gate. If settlement returns `next.reason: "verification_handoff_source_frozen"`, stop: source is frozen and the response carries the exact fresh-verifier `pick` and `readiness` commands. Running `done` on a ready outcome you never picked adopts it under your worker id. The response reports the FeatureRun transition and what the settlement unlocked. Do not add a second ordinary `log add` after `done`; the settlement already persisted the completion log.

Binding verification (browser flow, executed binary, real requests) must produce a trusted Evidence receipt and coverage verdict. During a compatible multi-outcome maker run, implementation settlement uses the repository's ordinary changed-file checks and `planr done --next`; it must not run Evidence readiness, collection, or an opportunistic browser/live smoke while later compatible implementation outcomes can still change the source. Planr durably caps each internal ExecutionBatch at three outcomes and rolls it for the same maker inside `done --next`; that internal boundary is not a reason to wake the root or reload skills. Keep the same maker through stable source freeze. Write a compact durable handoff only when the run genuinely stops: material review, incompatible ownership, blocker, empty pick, budget, or stable source freeze. Do not re-read a skill already loaded in the same agent session merely because work continued. After the coordinator establishes source freeze, a fresh verification-only worker first leases `planr pick --plan <plan-id> --work-type verification --json`, then runs readiness under that same identity and executes only `readiness.run_index.repository_path`. Product source is read-only by the canonical Evidence `SOURCE_PATHS` source digest. The verifier may write Planr runtime state, receipts, logs, and artifacts, but `planr evidence run` enforces the frozen source digest inside the real Evidence transaction before any trusted receipt commit; any product-source mutation records a failed non-covering attempt and zero new trusted receipts. Product findings route back to the responsible maker; after maker fixes, the coordinator re-freezes and a leased verifier reruns readiness before selectively rerunning only invalidated Evidence. The only exception is a material review that explicitly requires an affected exact-source criterion before further implementation; record that boundary and stop for its review.

```bash
planr evidence readiness --scope criterion --id <criterion-id>
planr evidence run --input <exact-readiness.run_index.repository_path>
planr evidence coverage --scope criterion --id <criterion-id>
planr evidence explain --scope criterion --id <criterion-id>
```

The configured method must be copy-paste replayable: a real shell command, host capture, or committed verifier, never prose such as "start server; curl /; check stats". Reviewers validate the exact-source receipt and selectively replay cheap, missing, failing, or explicitly high-risk evidence. `planr log add` remains useful for narrative handoffs, but a free-form `kind=verification` claim has no binding closure authority.

Log persistent evidence, not transient noise: a failure you immediately fixed belongs in the final log's narrative, not as a standalone failure log. Only record a failure separately when it blocks the item.

Evidence logging refreshes the heartbeat automatically — a separate `planr pick heartbeat` is only needed for long silent stretches without logs.

Finding-repair packets use the granular evidence command because they have no outcome item to settle:

```bash
planr log add --item <item-id> --summary "..." --files a --files b --cmd "..."
```

For longer work, keep runtime state current:

```bash
planr pick progress <item-id> --percent 50 --note "tests running"
planr pick pause <item-id> --note "waiting for human input"
planr pick resume <item-id>
```

If a human approval gate is required, request it before close and wait for an approved decision:

```bash
planr approval request <item-id> --reason "release approval"
planr approval list --open
```

Deployment is always such a gate: obtain approval before deployment and record a bounded live oracle afterward. Keep that oracle on the same coherent FeatureRun/ReviewGate boundary; a successful smoke does not automatically require another full build or gate replay.

## Rules

- Do not work on multiple picked items unless the user explicitly asks.
- Do not close without evidence.
- Do not confuse item completion evidence with frozen-source binding Evidence; defer the latter until the batch freeze and fresh verification-only worker unless a material review explicitly requires it.
- Do not create workflow-only repair items for a failed review. Repair the named findings under the responsible maker, resolve their ids on the same ReviewGate, and return that gate to re-review.
- Do not call `planr done` for a finding-repair packet that has no item. Log the repair on the ReviewGate's scoped outcome, then use `planr review findings <gate-id> --resolve <finding-id>`.
- Do not close items with pending or denied approval.
- Use `planr context add ... --item <item-id>` for discoveries another client needs.
- Use `planr pick stale --older-than-seconds 900` before resetting abandoned ownership.
- Use `planr pick release <item-id> --force` only when ownership must be reset.
