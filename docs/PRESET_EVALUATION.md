# Preset Evaluation

Planr provides three explicit evidence levels over the same versioned fixture and lifecycle engine:

- the default offline policy simulation performs no external execution and can reach `verified` but never `recommended`;
- the opt-in live-host runner executes every task through a user-selected local adapter and evaluates Planr-owned workspace artifacts, but arbitrary-process route and cost claims cannot reach `recommended` without independent instrumentation.
- the instrumented live-host runner joins those artifacts to Ed25519-signed telemetry receipts and can reach `recommended` when trusted route, usage, and all other thresholds pass.

`evaluations/preset-suite-v1.toml` defines distinct inputs, artifact kinds, and candidate/task-bound output oracles for exploration, implementation, mechanical, browser, visual, security, and subagent workflows. Input and result artifacts are SHA-256 hashed.

```sh
planr agents preset evaluate
planr --json agents preset evaluate --at-unix 1783987200
planr agents preset evaluate --report-dir reports/preset-v1
planr agents preset evaluate \
  --live-host-command /absolute/path/to/host-adapter \
  --live-host-arg optional-argument \
  --trusted-telemetry-signer production-collector \
  --trusted-telemetry-collector /absolute/path/to/collector
```

Evaluation defaults to current wall-clock time, so evidence becomes `stale` after fixture expiry. `--at-unix` is the deterministic override for tests and reproducible exports. `--host` requires binding compatibility. The signer and collector flags are all-or-none and require live mode. MCP exposes the equivalent optional `at_unix`, `host`, `live_host_command`, `live_host_args`, `trusted_telemetry_signer`, and `trusted_telemetry_collector` inputs.

## Offline simulation

Offline artifacts say `evidence_scope = "policy_simulation"`, `task_executed = false`, `outcome_oracle_evaluated = false`, and `recommendation_eligible = false`. Projected tool, token, credit, and latency values carry `metrics_source = "estimated_projection"` and `metering_confidence = "estimated"`.

Binding configuration populates only requested and resolved route stages. Effective model, effort, and context-fork dimensions remain `unavailable`, so route coverage is zero and the recommendation list is empty. Offline `verified` means only that policy composition, transition simulation, and hashes are complete.

## Explicit live-host runner

`--live-host-command` must be an absolute executable path. Planr starts it once per candidate/task fixture, writes one JSON request to stdin, waits for process completion, measures elapsed time, and parses one JSON response from stdout. This is an explicit arbitrary-process boundary: only run a trusted adapter. No live process is started without the flag.

For each run, Planr creates a Planr-controlled temporary workspace containing `challenge.json`. The request contains schema/suite/candidate/policy/binding/task versions, task kind, input, input hash, required artifact kind, and absolute workspace/challenge/artifact paths. It deliberately does not expose the expected output or challenge contents. The adapter must read the challenge, write a strict schema-v1 artifact to `artifact_path`, hash those exact bytes, and bind its response back to that artifact:

```json
{
  "schema_version": 1,
  "host_id": "example-host",
  "host_version": "1.0.0",
  "candidate_id": "balanced-codex-openai",
  "task_id": "browser-report-smoke",
  "input_sha256": "<request input hash>",
  "artifact_kind": "browser_trace",
  "artifact_sha256": "<SHA-256 of the artifact file>",
  "output": "balanced-codex-openai:browser-report-smoke:browser-report-inspected",
  "effective_model": "model-id",
  "effective_effort": "high",
  "effective_context_fork": { "mode": "none" },
  "tool_calls": 1,
  "tokens": 10,
  "credits_micros": 100,
  "retries": 1,
  "availability_fallbacks": 1,
  "quality_escalations": 1,
  "corrections": 0,
  "violations": 0
}
```

The artifact file is separate from stdout and has this strict shape:

```json
{
  "schema_version": 1,
  "candidate_id": "balanced-codex-openai",
  "task_id": "browser-report-smoke",
  "input_sha256": "<request input hash>",
  "artifact_kind": "browser_trace",
  "challenge_sha256": "<SHA-256 of Planr's challenge file>",
  "output": "balanced-codex-openai:browser-report-smoke:browser-report-inspected"
}
```

Planr reads the artifact itself after the process exits, rejects missing, symlinked, oversized, malformed, or extra-field artifacts, computes its SHA-256 independently, verifies candidate/task/input/artifact and challenge binding, and finally evaluates the versioned task oracle. Neither a constant responder nor a request-aware mapper that only echoes dynamic request fields and public success labels writes this Planr-controlled artifact, so both have `task_executed = false`, `evidence_complete = false`, status `unverified`, and cannot recommend. A wholly failed live attempt also sets the report-level `reproducible_evidence = false`; merely attempting the adapter is never described as evaluated live evidence.

Production policy checks remain authoritative: tool-budget and write-capability checks are evaluated from the candidate policy, never adapter claims. A read-only candidate fails the implementation and mechanical write fixtures even if the adapter returns otherwise successful JSON.

The adapter reports effective route and usage values, but process exit does not make those claims trusted. Effective route fields are labeled `host_report` with `estimated` enforcement, so they do not increment `verified_route_runs`. Tool/token/credit metering is also `estimated`; only elapsed process latency is independently measured by Planr. Retry/fallback/escalation/correction/violation counts remain visible host reports. The process-adapter boundary therefore always sets `recommendation_eligible = false`, even when every workspace artifact passes.

## Independently signed telemetry

Recommendation-capable live evaluation uses a separately provisioned telemetry collector. The repository trust registry at `.planr/trusted-telemetry.toml` pins both an Ed25519 signer key and the SHA-256 of the collector executable; the evaluation invocation supplies only the registry signer id and the absolute collector path. A caller-selected public key, run id, or precomputed receipt bundle is not accepted. The registry shape is:

```toml
schema_version = 1

[[signers]]
id = "production-collector"
public_key_hex = "<64-hex-character-ed25519-public-key>"
collector_sha256 = "<64-hex-character-executable-sha256>"
```

Planr creates a fresh run UUID before evaluation and a fresh challenge nonce for every task. Only after the live adapter exits and Planr reads and hashes the workspace artifact does it invoke the hash-pinned collector. The collector receives a strict JSON identity request containing the Planr-owned run id, evaluation and suite identity, candidate/task/input/artifact identity, exact artifact SHA-256, and challenge nonce. It independently adds host route, usage, transition, correction, and violation measurements, then returns `{ "payload": ..., "signature_hex": "..." }`. The adapter never receives a signing key, and pre-run receipts cannot know the fresh run/challenge/artifact tuple.

Before invocation Planr verifies that the collector is a regular absolute-path file whose SHA-256 matches the selected registry entry, and it rechecks the digest for every task. It rejects the trust upgrade for a failed collector, malformed response, bad signature, wrong run/suite/time, wrong task/input/artifact/challenge binding, blank host identity, or host identity mismatch with the live adapter response. Missing or rejected receipts do not abort artifact evaluation: that result falls back to `host_reported`/`estimated`, gets zero verified route credit, and is recommendation-ineligible. Registry, signer, and collector configuration errors fail the evaluation before execution.

For a verified post-run receipt, Planr uses only signed route, usage, transition, correction, and violation values. Effective route dimensions carry `evidence = "telemetry_receipt"` and `enforcement = "verified"`; tool/token/credit dimensions and result `metering_confidence` are `trusted`; process latency remains Planr-observed. These results use `metrics_source = "trusted_telemetry"` and may satisfy recommendation gates.

## Lifecycle and thresholds

- `recommended`: every task artifact and oracle passes, every effective route and usage measurement has a valid signed telemetry receipt, and all thresholds pass;
- `verified`: offline simulation is complete, or every required live workspace artifact and outcome oracle passed but recommendation-grade instrumentation is unavailable or a threshold failed;
- `stale`: evaluation time is later than fixture expiry;
- `incompatible`: requested host is unsupported by the binding;
- `unverified`: required composition, transition, hash, live task execution, challenge-bound artifact, or outcome-oracle evidence is incomplete.

Only independently instrumented live task evidence can set `recommendation_evidence_complete = true`. Hashing simulator-authored JSON, process stdout, or an adapter-authored route/cost claim cannot promote it.

## Immutable output

`--report-dir` accepts only a normalized repository-relative directory and rejects absolute paths, traversal, non-directory components, and symlink crossings. It creates `verification.json` and `report.md` without overwrite; either existing target fails the command.

The report includes suite/Planr/configured-model provenance. Live results additionally include host id/version, Planr-read workspace artifact and oracle hashes, host route/usage evidence with explicit estimated or trusted provenance, observed process latency, corrections/transitions/violations, policy-capability checks, and result hashes.

## Sol/Luna Codex contract

`evaluations/sol-luna-codex-v1.toml` independently proves that cross-tier `fork_turns = "all"` is rejected, `none` preserves Luna parameters, missing effective evidence cannot route-verify, and process-evidenced effective model/effort/fork values can verify.
