---
name: planr-review
description: Review a Planr item or scoped implementation against map state, plan acceptance criteria, logs, changed files, and verification evidence. Use for findings-first audits and review gates.
---

# Planr Review

Use this when a task needs a correctness and completion audit.

## Workflow

Export your checker identity once per session so the lease attributes to you:

```bash
export PLANR_WORKER_ID=checker-1
planr --json pick --work-type review
```

`--work-type review` leases only durable ReviewGates, so a checker never accidentally takes maker work. Continue only when `work_packet.kind` is `review_gate`; read the gate, FeatureRun, responsible maker, attempts, findings, phase, budget, and source revision from its canonical `execution_state`. Add `--plan <plan-id>` when your dispatch names a plan so the lease stays inside that scope. Use `planr review show <review-gate-id> --json`, `planr review evidence <scope-item-id> --json`, or `planr trace item <scope-item-id> --json` only for deeper reads.

Require `planr.execution_state.v2` and treat its budget projection as opaque. Skills must not recompute budget policy or reinterpret provenance, reserves, digest, or deadline.

Inspect the actual changed files and acceptance criteria, then independently judge whether the evidence proves them. When the repository owns a versioned verification policy, verify the logged receipt against its exact source revision, policy digest, changed-file digest, selected gates, command results, and artifact digests. Use the repository's receipt validator (for this repository, `npm run verification:verify -- --receipt <path> --base <base-revision> --head <source-revision>`), not a visual read of JSON.

Replay only evidence that is cheap, missing, failing, or explicitly high-risk. An already-green expensive check bound to the reviewed source is normally validated from its receipt rather than rerun. Receipt validation does not replace judgment: inspect the diff for security, correctness, scope, and acceptance-criteria gaps, and record a finding when the policy selection or receipt is inadequate. Then close the ReviewGate exactly once:

```bash
planr review close <review-gate-id> --verdict complete --reviewer <your-id>
```

`--reviewer` records the independent checker identity on the immutable attempt and event. An accepted risk checkpoint resumes the responsible maker's FeatureRun; an accepted final product gate completes the FeatureRun. A second close without a fresh leased gate fails — never retry a close that succeeded.

or:

```bash
planr review close <review-gate-id> --verdict not-complete --reviewer <your-id> --findings "specific actionable finding"
```

## Findings Rules

- Findings must be specific and actionable.
- Missing tests are findings when acceptance criteria need proof.
- A stale, mismatched, unvalidated, or insufficiently scoped receipt is a finding.
- Architecture or ownership drift is a finding when it creates duplicate policy or state owners.
- If evidence is insufficient, use `--verdict unclear` rather than complete.
- Deployment remains gated by explicit approval and a bounded live oracle where applicable. A successful live smoke does not by itself require another broad build or a second full review; replay it only under the same cheap/missing/failing/explicitly-high-risk rule.

## Independent Identity

ReviewGate closure requires a reviewer identity distinct from the responsible maker and matching the active reviewer lease. Keep one `PLANR_WORKER_ID` per agent instance and always pass `--reviewer <id>` when shell exports do not survive between tool calls. Never manufacture independence by changing identities inside one agent instance.

## Completion Rule

The FeatureRun may complete only after its required ReviewGates are accepted. Use the FeatureRun and ReviewGate projections as the source of truth.
Pending or denied approval is also a close blocker; treat an attempted close through that gate as a finding, not as completion.
