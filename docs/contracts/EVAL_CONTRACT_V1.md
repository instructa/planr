# Eval Contract v1

Status: frozen for implementation; v1.1 additive efficiency-evidence amendment frozen for implementation

Eval Contract v1 is a Planr-owned, local-first contract for deciding whether a candidate revision is better than a frozen baseline. It defines authored suite inputs, immutable run evidence, recomputable comparisons, CLI/MCP response shapes, and the ownership boundaries that prevent eval verdicts from becoming a second source of truth for map work closure. The v1.1 amendment is additive: it freezes attempt lineage, metering basis, effective treatment identity, versioned estimate provenance, and derived failure-inclusive efficiency metrics without changing the four comparison verdicts.

## Ownership

- Repository files own authored suite manifests, fixtures, expected lifecycle assertions, and checked-in examples.
- The suite loader owns parsing, repository containment, normalization, canonical ordering, fixture hashing, and suite digests.
- SQLite owns immutable suite snapshots, runs, case results, samples, comparisons, invalidations, rescoring provenance, and evidence references.
- The comparison engine owns compatibility, freshness, coverage, correctness, quality, protected-dimension, performance, and cost gates.
- Existing Planr map state owns item status, dependencies, picks, logs, reviews, approvals, and closure. An eval verdict can be attached as evidence, but it cannot close or reopen work by itself.
- CLI, MCP, and HTTP adapters may render the contract; they must call shared application services and must not implement independent scoring or status transitions.
- Eval Contract v1.1 owns derived efficiency semantics over EvalSample evidence. It does not own a second evidence store, provider catalog, pricing service, router, model leaderboard, or closure authority.

## V1.1 Additive Efficiency Evidence

V1.1 extends the V1 objects with the smallest shape needed to compare cost and wall time per independently verified success while preserving backward compatibility. V1 readers may ignore the additive fields and still interpret V1 snapshots, runs, samples, comparisons, invalidations, evidence refs, exit codes, and the four existing comparison verdicts. V1.1 readers must retain the V1 verdict vocabulary exactly: `improved`, `regressed`, `no_material_difference`, and `insufficient_evidence`.

Attempt lineage is countable evidence, not a separate run model:

- One countable attempt is one case, repetition, and seed execution under one effective treatment, including failures, retries, availability fallbacks, quality escalations, quota downgrades, safety stops, and resumed attempts that re-execute the subject.
- Every countable attempt records `task_id`, `case_id`, `repetition_index`, `seed`, `attempt_index`, `retry_of`, `fallback_of`, `escalation_of`, `resume_of`, terminal status, reason codes, runner/harness version, and the effective treatment identity described below.
- Failed and retried attempts remain durable and participate in aggregate cost and wall-time totals. They cannot be silently dropped, overwritten by a later success, or double-counted as both their original attempt and their retry.
- Warmups may be retained for audit but are excluded from efficiency denominators unless the suite explicitly marks them countable.

Metering basis is per dimension:

- `actual_trusted`: value came from an allowlisted process timer, Planr route observation, host usage receipt, or scorer/event meter whose source and confidence are recorded and accepted by policy.
- `estimated`: value was computed from durable measured inputs and a versioned estimate reference, such as `pricing_reference_id`, `pricing_reference_version`, `currency`, and `effective_at`. Estimated values are visible but cannot masquerade as trusted actuals.
- `mixed`: an aggregate combines multiple accepted bases, such as trusted tool calls with estimated credits, or baseline and candidate runs with different bases. Mixed evidence is non-promotable unless the comparison policy explicitly permits the named mixed basis.
- `unavailable`: the value is unknown. Unavailable never coerces to `0`, never satisfies budget or material-improvement evidence, and must remain `null` or omitted with an `unavailable` basis in JSON projections.

Allowlisted dimensions are `wall_time_ms`, `tool_calls`, `input_tokens`, `output_tokens`, `total_tokens`, `credits_micros`, and `cost_micros`. Each EvalSample for these dimensions carries `metering_basis`, `basis_source`, `basis_confidence`, and optional versioned estimate provenance. Unknown or untrusted values stay unavailable rather than falling back to authored defaults, requested route declarations, package metadata, or provider catalog guesses.

Effective treatment identity is the comparison key:

- Each attempt records `client`, `provider`, `runtime`, `model`, `effort`, `profile_id`, `profile_digest`, `route_policy_digest`, `runner_version`, `harness_version`, task, repetition, and seed.
- Requested route values are not effective treatment evidence. A requested model, effort, provider, runtime, or profile may be stored as intent, but comparison identity uses only resolved/effective values with confidence provenance.
- Silent treatment fallback invalidates matched comparison for the affected cell. If the candidate requested one treatment but effectively ran another and every changed treatment field is not accounted for by a field-specific fallback, escalation, effort downgrade, or quota downgrade reason, the comparison records `treatment_fallback_unattributed` and returns `insufficient_evidence`. Retry and resume lineage alone do not attribute treatment changes, and fallback or escalation lineage pointers are audit context rather than blanket attribution for unrelated fields.

Derived efficiency metrics are recomputable from EvalSample and case/attempt status rows:

- `cost_per_attempt` and `wall_time_per_attempt` divide all countable attempt totals by countable attempts.
- `cost_per_verified_success` and `wall_time_per_verified_success` divide all countable attempt totals, including failed attempts, retries, fallbacks, escalations, and downgrades, by independently verified successful attempts.
- Zero verified successes produce `unavailable` or `unbounded`, never `0`, for per-success efficiency. The comparison must use `insufficient_evidence` or `regressed` according to policy, but it must not report a free success.
- Solve rate, verified-success count, sample count, confidence intervals, variance, and metering-basis mix stay alongside efficiency metrics. They are not collapsed into a single opaque score.

Schema and surface impact is additive:

- SQLite extends run evidence with attempt lineage and additional EvalSample metadata. It must reuse immutable EvalSample rows and route-observation metering rather than adding a second JSONL evidence store, usage ledger, or provider/pricing catalog.
- CLI, MCP, and HTTP eval responses expose the same efficiency, metering-basis, estimate-provenance, effective-treatment, and reason-code fields inside the existing eval envelope.
- Package export/import includes the attempt lineage, sample metadata, estimate references, and derived comparison outputs needed to recompute the same result locally. Import preflights reject unknown-to-zero coercion and missing effective treatment identity.
- Migrations are additive and preserve existing V1 rows. Missing V1.1 fields on older evidence read as unavailable or non-comparable, not as zero cost, zero time, or a successful matched treatment.

V1.1 executable contract vectors:

```json
{
  "object": {
    "v1_1_contract_vectors": {
      "sample_identity_key": [
        "run_id",
        "case_id",
        "scorer_id",
        "scorer_version",
        "attempt_id",
        "measure"
      ],
      "attempts": [
        {
          "attempt_id": "evatt-baseline-000001",
          "task_id": "task-create-plan-map-close",
          "run_id": "evrun-20260721-baseline",
          "case_id": "create-plan-map-close",
          "repetition_index": 0,
          "seed": 12345,
          "attempt_index": 0,
          "countable": true,
          "retry_of": null,
          "fallback_of": null,
          "escalation_of": null,
          "resume_of": null,
          "terminal_status": "verified_success",
          "reasons": [],
          "requested_treatment": {
            "client": "codex",
            "provider": "openai",
            "runtime": "codex-cli",
            "model": "gpt-5.6-terra",
            "effort": "medium",
            "profile_id": "terra-medium",
            "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
          },
          "effective_treatment": {
            "client": "codex",
            "provider": "openai",
            "runtime": "codex-cli",
            "model": "gpt-5.6-terra",
            "effort": "medium",
            "profile_id": "terra-medium",
            "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "runner_version": "eval-runner-v1",
            "harness_version": "planr-eval-harness-v1",
            "confidence": "verified"
          }
        },
        {
          "attempt_id": "evatt-candidate-000001",
          "task_id": "task-create-plan-map-close",
          "run_id": "evrun-20260721-candidate",
          "case_id": "create-plan-map-close",
          "repetition_index": 0,
          "seed": 12345,
          "attempt_index": 0,
          "countable": true,
          "retry_of": null,
          "fallback_of": null,
          "escalation_of": null,
          "resume_of": null,
          "terminal_status": "failed",
          "reasons": ["process_failed"],
          "requested_treatment": {
            "client": "codex",
            "provider": "openai",
            "runtime": "codex-cli",
            "model": "gpt-5.6-terra",
            "effort": "medium",
            "profile_id": "terra-medium",
            "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
          },
          "effective_treatment": {
            "client": "codex",
            "provider": "openai",
            "runtime": "codex-cli",
            "model": "gpt-5.6-terra",
            "effort": "medium",
            "profile_id": "terra-medium",
            "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "runner_version": "eval-runner-v1",
            "harness_version": "planr-eval-harness-v1",
            "confidence": "verified"
          }
        },
        {
          "attempt_id": "evatt-candidate-000002",
          "task_id": "task-create-plan-map-close",
          "run_id": "evrun-20260721-candidate",
          "case_id": "create-plan-map-close",
          "repetition_index": 0,
          "seed": 12345,
          "attempt_index": 1,
          "countable": true,
          "retry_of": "evatt-candidate-000001",
          "fallback_of": null,
          "escalation_of": null,
          "resume_of": null,
          "terminal_status": "verified_success",
          "reasons": [],
          "requested_treatment": {
            "client": "codex",
            "provider": "openai",
            "runtime": "codex-cli",
            "model": "gpt-5.6-terra",
            "effort": "medium",
            "profile_id": "terra-medium",
            "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
          },
          "effective_treatment": {
            "client": "codex",
            "provider": "openai",
            "runtime": "codex-cli",
            "model": "gpt-5.6-terra",
            "effort": "medium",
            "profile_id": "terra-medium",
            "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "runner_version": "eval-runner-v1",
            "harness_version": "planr-eval-harness-v1",
            "confidence": "verified"
          }
        },
        {
          "attempt_id": "evatt-candidate-000003",
          "task_id": "task-create-plan-map-close",
          "run_id": "evrun-20260721-candidate",
          "case_id": "create-plan-map-close",
          "repetition_index": 0,
          "seed": 12345,
          "attempt_index": 2,
          "countable": true,
          "retry_of": "evatt-candidate-000002",
          "fallback_of": null,
          "escalation_of": null,
          "resume_of": null,
          "terminal_status": "failed",
          "reasons": ["metering_unavailable"],
          "requested_treatment": {
            "client": "codex",
            "provider": "openai",
            "runtime": "codex-cli",
            "model": "gpt-5.6-terra",
            "effort": "medium",
            "profile_id": "terra-medium",
            "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
          },
          "effective_treatment": {
            "client": "codex",
            "provider": "openai",
            "runtime": "codex-cli",
            "model": "gpt-5.6-terra",
            "effort": "medium",
            "profile_id": "terra-medium",
            "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "runner_version": "eval-runner-v1",
            "harness_version": "planr-eval-harness-v1",
            "confidence": "verified"
          }
        }
      ],
      "samples": [
        {
          "sample_id": "evsample-candidate-cost",
          "run_id": "evrun-20260721-candidate",
          "case_id": "create-plan-map-close",
          "scorer_id": "map-lifecycle",
          "scorer_version": "1.0.0",
          "attempt_id": "evatt-candidate-000001",
          "repetition_index": 0,
          "warmup": false,
          "seed": 12345,
          "measure": "cost_micros",
          "value": 300,
          "unit": "micros",
          "source": "metering",
          "metering_basis": "actual_trusted",
          "basis_source": "host_usage_receipt",
          "basis_confidence": "verified",
          "estimate_provenance": null,
          "captured_at": "2026-07-21T14:21:10Z"
        },
        {
          "sample_id": "evsample-candidate-wall-time",
          "run_id": "evrun-20260721-candidate",
          "case_id": "create-plan-map-close",
          "scorer_id": "map-lifecycle",
          "scorer_version": "1.0.0",
          "attempt_id": "evatt-candidate-000001",
          "repetition_index": 0,
          "warmup": false,
          "seed": 12345,
          "measure": "wall_time_ms",
          "value": 1200,
          "unit": "ms",
          "source": "process",
          "metering_basis": "actual_trusted",
          "basis_source": "allowlisted_process_timer",
          "basis_confidence": "verified",
          "estimate_provenance": null,
          "captured_at": "2026-07-21T14:21:11Z"
        },
        {
          "sample_id": "evsample-candidate-cost-estimated",
          "run_id": "evrun-20260721-candidate",
          "case_id": "create-plan-map-close",
          "scorer_id": "map-lifecycle",
          "scorer_version": "1.0.0",
          "attempt_id": "evatt-candidate-000002",
          "repetition_index": 0,
          "warmup": false,
          "seed": 12345,
          "measure": "cost_micros",
          "value": 700,
          "unit": "micros",
          "source": "metering",
          "metering_basis": "estimated",
          "basis_source": "pricing_reference",
          "basis_confidence": "estimated",
          "estimate_provenance": {
            "pricing_reference_id": "openai-prices",
            "pricing_reference_version": "2026-07-01",
            "currency": "USD",
            "effective_at": "2026-07-01T00:00:00Z"
          },
          "captured_at": "2026-07-21T14:22:11Z"
        },
        {
          "sample_id": "evsample-candidate-cost-unavailable",
          "run_id": "evrun-20260721-candidate",
          "case_id": "create-plan-map-close",
          "scorer_id": "map-lifecycle",
          "scorer_version": "1.0.0",
          "attempt_id": "evatt-candidate-000003",
          "repetition_index": 0,
          "warmup": false,
          "seed": 12345,
          "measure": "cost_micros",
          "value": null,
          "unit": "micros",
          "source": "metering",
          "metering_basis": "unavailable",
          "basis_source": "host_usage_receipt_absent",
          "basis_confidence": "unavailable",
          "estimate_provenance": null,
          "captured_at": "2026-07-21T14:23:11Z"
        }
      ],
      "efficiency_cases": [
        {
          "name": "zero-success-is-unavailable-not-zero",
          "attempt_ids": ["evatt-candidate-000001"],
          "measure": "cost_micros",
          "expected_countable_attempts": 1,
          "expected_verified_successes": 0,
          "expected_total": 300,
          "expected_per_verified_success": null,
          "expected_per_verified_success_state": "unavailable",
          "expected_basis": "unavailable",
          "expected_reasons": ["zero_verified_successes", "samples_below_minimum"]
        },
        {
          "name": "retry-accounting-includes-failed-attempt",
          "attempt_ids": ["evatt-candidate-000001", "evatt-candidate-000002"],
          "measure": "cost_micros",
          "expected_countable_attempts": 2,
          "expected_verified_successes": 1,
          "expected_total": 1000,
          "expected_per_verified_success": 1000,
          "expected_per_verified_success_state": "available",
          "expected_basis": "mixed",
          "expected_reasons": []
        },
        {
          "name": "unavailable-failed-attempt-cost-blocks-promotable-per-success",
          "attempt_ids": ["evatt-candidate-000002", "evatt-candidate-000003"],
          "measure": "cost_micros",
          "expected_countable_attempts": 2,
          "expected_verified_successes": 1,
          "expected_total": null,
          "expected_per_verified_success": null,
          "expected_per_verified_success_state": "unavailable",
          "expected_basis": "unavailable",
          "expected_reasons": ["samples_unavailable"]
        }
      ],
      "invalid_efficiency_cases": [
        {
          "name": "successful-only-cost-accounting",
          "attempt_ids": ["evatt-candidate-000001", "evatt-candidate-000002"],
          "measure": "cost_micros",
          "expected_countable_attempts": 2,
          "expected_verified_successes": 1,
          "expected_total": 700,
          "expected_per_verified_success": 700,
          "expected_per_verified_success_state": "available",
          "expected_basis": "estimated",
          "expected_error": "failed_attempt_cost_excluded"
        }
      ],
      "invalid_samples": [
        {
          "name": "unknown-to-zero-coercion",
          "sample": {
            "sample_id": "evsample-invalid-zero",
            "run_id": "evrun-20260721-candidate",
            "case_id": "create-plan-map-close",
            "scorer_id": "map-lifecycle",
            "scorer_version": "1.0.0",
            "attempt_id": "evatt-candidate-000001",
            "repetition_index": 0,
            "warmup": false,
            "seed": 12345,
            "measure": "cost_micros",
            "value": 0,
            "unit": "micros",
            "source": "metering",
            "metering_basis": "unavailable",
            "basis_source": "host_usage_receipt_absent",
            "basis_confidence": "unavailable",
            "estimate_provenance": null,
            "captured_at": "2026-07-21T14:21:10Z"
          },
          "expected_error": "unavailable_value_must_be_null"
        }
      ],
      "valid_treatment_transitions": [
        {
          "name": "field-specific-model-fallback-attributed",
          "attempt": {
            "attempt_id": "evatt-candidate-model-fallback-attributed",
            "task_id": "task-create-plan-map-close",
            "run_id": "evrun-20260721-candidate",
            "case_id": "create-plan-map-close",
            "repetition_index": 0,
            "seed": 12345,
            "attempt_index": 3,
            "countable": true,
            "retry_of": null,
            "fallback_of": "evatt-candidate-primary-unavailable",
            "escalation_of": null,
            "resume_of": null,
            "terminal_status": "verified_success",
            "reasons": ["model_fallback"],
            "requested_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-terra",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            },
            "effective_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-sol",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "runner_version": "eval-runner-v1",
              "harness_version": "planr-eval-harness-v1",
              "confidence": "verified"
            }
          }
        },
        {
          "name": "field-specific-escalation-and-downgrade-attributed",
          "attempt": {
            "attempt_id": "evatt-candidate-escalation-attributed",
            "task_id": "task-create-plan-map-close",
            "run_id": "evrun-20260721-candidate",
            "case_id": "create-plan-map-close",
            "repetition_index": 0,
            "seed": 12345,
            "attempt_index": 4,
            "countable": true,
            "retry_of": null,
            "fallback_of": null,
            "escalation_of": "evatt-candidate-quality-inconclusive",
            "resume_of": null,
            "terminal_status": "verified_success",
            "reasons": ["provider_escalation", "model_escalation", "effort_downgrade"],
            "requested_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-terra",
              "effort": "high",
              "profile_id": "terra-high",
              "profile_digest": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            },
            "effective_treatment": {
              "client": "codex",
              "provider": "azure-openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-sol",
              "effort": "medium",
              "profile_id": "terra-high",
              "profile_digest": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "runner_version": "eval-runner-v1",
              "harness_version": "planr-eval-harness-v1",
              "confidence": "verified"
            }
          }
        }
      ],
      "invalid_attempts": [
        {
          "name": "silent-treatment-fallback",
          "attempt": {
            "attempt_id": "evatt-candidate-fallback-unattributed",
            "task_id": "task-create-plan-map-close",
            "run_id": "evrun-20260721-candidate",
            "case_id": "create-plan-map-close",
            "repetition_index": 0,
            "seed": 12345,
            "attempt_index": 0,
            "countable": true,
            "retry_of": null,
            "fallback_of": null,
            "escalation_of": null,
            "resume_of": null,
            "terminal_status": "verified_success",
            "reasons": [],
            "requested_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-terra",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            },
            "effective_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-sol",
              "effort": "high",
              "profile_id": "sol-high",
              "profile_digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "runner_version": "eval-runner-v1",
              "harness_version": "planr-eval-harness-v1",
              "confidence": "verified"
            }
          },
          "expected_reason": "treatment_fallback_unattributed"
        },
        {
          "name": "silent-route-policy-digest-change",
          "attempt": {
            "attempt_id": "evatt-candidate-route-policy-unattributed",
            "task_id": "task-create-plan-map-close",
            "run_id": "evrun-20260721-candidate",
            "case_id": "create-plan-map-close",
            "repetition_index": 0,
            "seed": 12345,
            "attempt_index": 8,
            "countable": true,
            "retry_of": null,
            "fallback_of": null,
            "escalation_of": null,
            "resume_of": null,
            "terminal_status": "verified_success",
            "reasons": [],
            "requested_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-terra",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            },
            "effective_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-terra",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
              "runner_version": "eval-runner-v1",
              "harness_version": "planr-eval-harness-v1",
              "confidence": "verified"
            }
          },
          "expected_reason": "treatment_fallback_unattributed"
        },
        {
          "name": "retry-lineage-does-not-attribute-model-fallback",
          "attempt": {
            "attempt_id": "evatt-candidate-fallback-retry-only",
            "task_id": "task-create-plan-map-close",
            "run_id": "evrun-20260721-candidate",
            "case_id": "create-plan-map-close",
            "repetition_index": 0,
            "seed": 12345,
            "attempt_index": 1,
            "countable": true,
            "retry_of": "evatt-candidate-fallback-unattributed",
            "fallback_of": null,
            "escalation_of": null,
            "resume_of": null,
            "terminal_status": "verified_success",
            "reasons": ["process_failed"],
            "requested_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-terra",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            },
            "effective_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-sol",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "runner_version": "eval-runner-v1",
              "harness_version": "planr-eval-harness-v1",
              "confidence": "verified"
            }
          },
          "expected_reason": "treatment_fallback_unattributed"
        },
        {
          "name": "resume-lineage-does-not-attribute-effort-downgrade",
          "attempt": {
            "attempt_id": "evatt-candidate-downgrade-resume-only",
            "task_id": "task-create-plan-map-close",
            "run_id": "evrun-20260721-candidate",
            "case_id": "create-plan-map-close",
            "repetition_index": 0,
            "seed": 12345,
            "attempt_index": 2,
            "countable": true,
            "retry_of": null,
            "fallback_of": null,
            "escalation_of": null,
            "resume_of": "evatt-candidate-fallback-unattributed",
            "terminal_status": "verified_success",
            "reasons": ["host_resumed"],
            "requested_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-sol",
              "effort": "high",
              "profile_id": "sol-high",
              "profile_digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            },
            "effective_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-sol",
              "effort": "medium",
              "profile_id": "sol-high",
              "profile_digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "runner_version": "eval-runner-v1",
              "harness_version": "planr-eval-harness-v1",
              "confidence": "verified"
            }
          },
          "expected_reason": "treatment_fallback_unattributed"
        },
        {
          "name": "generic-fallback-lineage-does-not-attribute-model-change",
          "attempt": {
            "attempt_id": "evatt-candidate-generic-fallback-lineage",
            "task_id": "task-create-plan-map-close",
            "run_id": "evrun-20260721-candidate",
            "case_id": "create-plan-map-close",
            "repetition_index": 0,
            "seed": 12345,
            "attempt_index": 5,
            "countable": true,
            "retry_of": null,
            "fallback_of": "evatt-candidate-primary-unavailable",
            "escalation_of": null,
            "resume_of": null,
            "terminal_status": "verified_success",
            "reasons": ["treatment_fallback_attributed"],
            "requested_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-terra",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            },
            "effective_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-sol",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "runner_version": "eval-runner-v1",
              "harness_version": "planr-eval-harness-v1",
              "confidence": "verified"
            }
          },
          "expected_reason": "treatment_fallback_unattributed"
        },
        {
          "name": "generic-escalation-lineage-does-not-attribute-provider-change",
          "attempt": {
            "attempt_id": "evatt-candidate-generic-escalation-lineage",
            "task_id": "task-create-plan-map-close",
            "run_id": "evrun-20260721-candidate",
            "case_id": "create-plan-map-close",
            "repetition_index": 0,
            "seed": 12345,
            "attempt_index": 6,
            "countable": true,
            "retry_of": null,
            "fallback_of": null,
            "escalation_of": "evatt-candidate-quality-inconclusive",
            "resume_of": null,
            "terminal_status": "verified_success",
            "reasons": ["treatment_transition_attributed"],
            "requested_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-terra",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            },
            "effective_treatment": {
              "client": "codex",
              "provider": "azure-openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-terra",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "runner_version": "eval-runner-v1",
              "harness_version": "planr-eval-harness-v1",
              "confidence": "verified"
            }
          },
          "expected_reason": "treatment_fallback_unattributed"
        },
        {
          "name": "partial-field-attribution-does-not-cover-profile-change",
          "attempt": {
            "attempt_id": "evatt-candidate-partial-attribution",
            "task_id": "task-create-plan-map-close",
            "run_id": "evrun-20260721-candidate",
            "case_id": "create-plan-map-close",
            "repetition_index": 0,
            "seed": 12345,
            "attempt_index": 7,
            "countable": true,
            "retry_of": null,
            "fallback_of": "evatt-candidate-primary-unavailable",
            "escalation_of": null,
            "resume_of": null,
            "terminal_status": "verified_success",
            "reasons": ["model_fallback"],
            "requested_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-terra",
              "effort": "medium",
              "profile_id": "terra-medium",
              "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            },
            "effective_treatment": {
              "client": "codex",
              "provider": "openai",
              "runtime": "codex-cli",
              "model": "gpt-5.6-sol",
              "effort": "medium",
              "profile_id": "sol-medium",
              "profile_digest": "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
              "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "runner_version": "eval-runner-v1",
              "harness_version": "planr-eval-harness-v1",
              "confidence": "verified"
            }
          },
          "expected_reason": "treatment_fallback_unattributed"
        }
      ]
    }
  }
}
```

## Repository Manifest

An authored manifest is a repository-relative JSON or TOML file. V1 accepts one normalized semantic shape regardless of source syntax.

Required fields:

- `schema_version`: exactly `eval.suite.v1`.
- `suite_id`: stable kebab-case id scoped to the repository.
- `suite_version`: SemVer string.
- `description`: short human description.
- `fixtures`: repository-relative fixture list with `id`, `path`, `digest_algorithm`, and optional `role`.
- `scorers`: deterministic scorer list with `id`, `version`, `kind`, and scorer-specific `config`; scorer controls are declared on bounded cases that prove polarity.
- `cases`: non-empty ordered input before normalization; each case has identity, fixture/scorer references, command contract, assertions, measures, budgets, and sampling policy.
- `comparison_policy`: coverage, freshness, compatibility, quality, performance, cost, and verdict thresholds.
- `safety`: execution and retention limits.
- `metadata`: optional owner/team labels that are excluded from verdict math unless explicitly named in `comparison_policy`.

Example:

```json
{
  "schema_version": "eval.suite.v1",
  "suite_id": "planr-lifecycle-smoke",
  "suite_version": "1.0.0",
  "description": "Planr lifecycle cases for feature-promotion evidence.",
  "fixtures": [
    {
      "id": "basic-project",
      "path": "eval/fixtures/basic-project",
      "digest_algorithm": "sha256",
      "role": "repository"
    }
  ],
  "scorers": [
    {
      "id": "map-lifecycle",
      "version": "1.0.0",
      "kind": "planr_lifecycle",
      "config": {
        "required_events": ["project.created", "map.built", "item.closed"]
      }
    }
  ],
  "cases": [
    {
      "case_id": "create-plan-map-close",
      "title": "Create a project, build a map, close one item",
      "tags": ["lifecycle", "correctness"],
      "fixture_ids": ["basic-project"],
      "scorer_ids": ["map-lifecycle@1.0.0"],
      "subject": {
        "kind": "planr_cli",
        "argv": ["planr", "project", "init", "EvalDemo"]
      },
      "assertions": [
        {
          "kind": "exit_code",
          "expected": 0
        },
        {
          "kind": "event_sequence",
          "expected": ["project.created", "map.built", "item.closed"]
        }
      ],
      "measures": ["duration_ms", "tool_calls"],
      "budgets": {
        "duration_ms_p95_max": 5000,
        "tool_calls_max": 30
      },
      "sampling": {
        "repetitions": 5,
        "warmups": 1,
        "seed": 12345,
        "min_successful_samples": 4
      },
      "timeout_ms": 15000,
      "output_limit_bytes": 65536
    }
  ],
  "comparison_policy": {
    "freshness_max_age_hours": 168,
    "required_case_coverage": 1.0,
    "compatible_testbed_fields": ["os", "arch", "planr_version", "cpu_class"],
    "quality_non_inferiority_margin": 0.0,
    "material_improvement": {
      "duration_ms_p95_relative": -0.1
    },
    "material_regression": {
      "duration_ms_p95_relative": 0.1
    },
    "protected_dimensions": {
      "correctness_pass_rate_min": 1.0,
      "tool_calls_relative_max": 0.05
    }
  },
  "safety": {
    "allow_network": false,
    "allow_shell": false,
    "max_concurrency": 2,
    "capture_stdout": "digest_and_bounded_excerpt",
    "capture_stderr": "digest_and_bounded_excerpt",
    "forbid_prompt_capture": true,
    "forbid_source_capture": true,
    "forbid_secret_capture": true
  },
  "metadata": {
    "owner": "planr-core"
  }
}
```

## Normalized Suite Snapshot

The normalized snapshot is the immutable object embedded in each run. It is derived by validation followed by canonicalization:

- Reject unknown `schema_version`, duplicate ids, dangling fixture/scorer references, empty case lists, invalid SemVer, ambiguous scorer ids, unsupported status/reason values, and mixed source syntaxes that normalize to different values.
- Reject scorer controls with incorrect polarity, duplicate controls, missing oracle or negative coverage for an opted-in scorer, dangling scorer identities, or nondeterministic bounds.
- Resolve every path against the canonical repository root. Reject absolute paths, `..`, symlink escapes, home paths, user/global config targets, device paths, and non-file fixture targets.
- Normalize path separators to `/`, ids to exact case-sensitive strings, durations to integer milliseconds, booleans to booleans, and absent optional fields to explicit documented defaults.
- Sort maps by key, fixtures by `id`, scorers by `id@version`, and cases by `case_id`; preserve explicit per-case assertion order.
- Hash fixture bytes with `sha256`, then hash the canonical UTF-8 JSON snapshot with `sha256`.
- Exclude comments, file formatting, and `metadata` keys not named by policy from the digest.

Documented defaults applied before digesting:

- `fixtures[].role`: `"repository"`.
- `cases[].tags`: `[]`.
- `cases[].measures`: `[]`.
- `cases[].budgets`: `{}`.
- `cases[].scorer_control`: omitted for ordinary cases. When present, `kind` is `oracle`, `known_bad`, or `no_op` (`null` normalizes to `no_op`), and `expect` is `pass` for oracle controls and `fail` for negative controls.
- `cases[].sampling.repetitions`: `1`.
- `cases[].sampling.warmups`: `0`.
- `cases[].sampling.seed`: `0`.
- `cases[].sampling.min_successful_samples`: equal to `repetitions`.
- `cases[].timeout_ms`: `30000`.
- `cases[].output_limit_bytes`: `65536`.
- `comparison_policy.freshness_max_age_hours`: `168`.
- `comparison_policy.required_case_coverage`: `1.0`.
- `comparison_policy.compatible_testbed_fields`: `["os", "arch", "planr_version"]`.
- `comparison_policy.quality_non_inferiority_margin`: `0.0`.
- `comparison_policy.material_improvement`: `{}`.
- `comparison_policy.material_regression`: `{}`.
- `comparison_policy.protected_dimensions`: `{}`.
- `safety.allow_network`: `false`.
- `safety.allow_shell`: `false`.
- `safety.max_concurrency`: `1`.
- `safety.capture_stdout`: `"digest_and_bounded_excerpt"`.
- `safety.capture_stderr`: `"digest_and_bounded_excerpt"`.
- `safety.forbid_prompt_capture`: `true`.
- `safety.forbid_source_capture`: `true`.
- `safety.forbid_secret_capture`: `true`.
- `metadata`: `{}`.

### Scorer Controls

Scorer controls are additive case metadata used to keep weak or inverted scorers from producing promotion evidence:

- `oracle` controls must pass under the scorer and prove known-good evidence is accepted.
- `known_bad` controls must fail under the scorer and prove bad evidence is rejected.
- `no_op` controls must fail and prove null/no-output subject behavior cannot pass accidentally.
- Any scorer that opts into controls must include at least one oracle control and at least one negative control (`known_bad` or `no_op`).
- Controls must be bounded and repeatable: at least two repetitions and no retry/fallback attempts (`max_attempts = 1`).
- Stored comparisons project controls from the frozen suite snapshot. Failed oracle controls, negative controls that pass, no-op controls that pass, or changed control metadata return explicit non-promotable reasons before ordinary quality and performance gates.

Canonical serialization is RFC 8785 JSON Canonicalization Scheme (JCS): UTF-8 JSON, no insignificant whitespace, object properties sorted lexicographically by unsigned UTF-16 code units, ECMAScript-compatible string escaping, ECMAScript `JSON.stringify` number serialization, and lowercase literals. Inputs must satisfy I-JSON constraints before digesting: no duplicate object names after parsing, strings must be valid Unicode scalar values, numbers must be finite and integer values must remain within the interoperable `[-9007199254740991, 9007199254740991]` range unless represented as strings. TOML manifests normalize to this JSON value before serialization, so semantically identical JSON and TOML inputs produce the same canonical bytes and digest.

Digest vectors:

```json
{
  "object": {
    "canonicalization_vectors": [
      {
        "name": "utf16-key-order-with-supplementary-plane-key",
        "input_json": "{\"\":1,\"𐀀\":2,\"a\":3}",
        "semantic_json": {
          "": 1,
          "𐀀": 2,
          "a": 3
        },
        "canonical_json": "{\"a\":3,\"𐀀\":2,\"\":1}",
        "sha256": "e7286ce065b2cc74ec502be56a09b4210f1533604cb4929d1ca01559ef1b8b76"
      },
      {
        "name": "number-edge-cases",
        "input_json": "{\"minusZero\":-0,\"small\":0.000001,\"int\":9007199254740991}",
        "semantic_json": {
          "minusZero": -0.0,
          "small": 0.000001,
          "int": 9007199254740991
        },
        "canonical_json": "{\"int\":9007199254740991,\"minusZero\":0,\"small\":0.000001}",
        "sha256": "7b423780480affcc244cb986f75813eca804812768b2b8f85b6a94b90144f24a"
      },
      {
        "name": "rfc8785-appendix-b-rounded-ieee754-values",
        "input_json": "{\"rfc333\":333333333.33333329,\"rfcfrac\":1.2345678901234567}",
        "semantic_json": {
          "rfc333": 333333333.33333329,
          "rfcfrac": 1.2345678901234567
        },
        "canonical_json": "{\"rfc333\":333333333.3333333,\"rfcfrac\":1.2345678901234567}",
        "sha256": "2da2a5a90d21b3687fa8882b65959bd4e24ce0a72997255b464f3529a33493cf"
      }
    ],
    "toml_equivalence": {
      "fixture_bytes_utf8": "fixture:basic-project:v1\n",
      "json_manifest": {
        "schema_version": "eval.suite.v1",
        "suite_id": "planr-lifecycle-smoke",
        "suite_version": "1.0.0",
        "fixtures": [
          {
            "id": "basic-project",
            "path": "eval/fixtures/basic-project",
            "digest_algorithm": "sha256"
          }
        ],
        "scorers": [
          {
            "id": "map-lifecycle",
            "version": "1.0.0",
            "kind": "planr_lifecycle",
            "config": {}
          }
        ],
        "cases": [
          {
            "case_id": "create-plan-map-close",
            "fixture_ids": ["basic-project"],
            "scorer_ids": ["map-lifecycle@1.0.0"],
            "subject": {
              "kind": "planr_cli",
              "argv": ["planr"]
            },
            "assertions": []
          }
        ],
        "comparison_policy": {},
        "safety": {}
      },
      "toml_manifest": "schema_version = \"eval.suite.v1\"\nsuite_id = \"planr-lifecycle-smoke\"\nsuite_version = \"1.0.0\"\n\n[[fixtures]]\nid = \"basic-project\"\npath = \"eval/fixtures/basic-project\"\ndigest_algorithm = \"sha256\"\n\n[[scorers]]\nid = \"map-lifecycle\"\nversion = \"1.0.0\"\nkind = \"planr_lifecycle\"\nconfig = {}\n\n[[cases]]\ncase_id = \"create-plan-map-close\"\nfixture_ids = [\"basic-project\"]\nscorer_ids = [\"map-lifecycle@1.0.0\"]\nassertions = []\n[cases.subject]\nkind = \"planr_cli\"\nargv = [\"planr\"]\n\n[comparison_policy]\n\n[safety]\n",
      "json_manifest_digest": "sha256:e2f7f013c72b6519499dfb45df0628d9bb823544a88695b0543464bd1745d79a",
      "toml_manifest_digest": "sha256:e2f7f013c72b6519499dfb45df0628d9bb823544a88695b0543464bd1745d79a"
    }
  }
}
```

Example response:

```json
{
  "schema_version": "eval.suite.snapshot.v1",
  "suite_id": "planr-lifecycle-smoke",
  "suite_version": "1.0.0",
  "digest": "sha256:7c4b44e9f6a6113fdf1344f3e14c65b07e32e63e8b22c34784d51a6b9a4ec001",
  "case_order": ["create-plan-map-close"],
  "fixture_digests": {
    "basic-project": "sha256:2bb80d537b1da3e38bd30361aa855686bde0ba89dd8c7d463f537c41b30b25a9"
  },
  "scorer_identities": ["map-lifecycle@1.0.0"],
  "defaults": {
    "allow_network": false,
    "allow_shell": false,
    "capture_stdout": "digest_and_bounded_excerpt",
    "capture_stderr": "digest_and_bounded_excerpt"
  }
}
```

## Status Vocabulary

Run status:

- `started`: run record exists and at least one case may still execute.
- `success`: every required case reached a terminal usable status and run-level persistence completed.
- `error`: infrastructure or runner failure prevented a usable run.
- `inconclusive`: run completed but has no comparison-eligible evidence.
- `invalidated`: operator or service provenance marks the run unusable for future comparisons.

Case status:

- `pass`: deterministic assertions passed and samples are comparison-eligible.
- `fail`: deterministic assertions prove candidate behavior wrong for the case.
- `error`: command, fixture, parser, timeout, or scorer infrastructure failed.
- `skipped`: policy excluded the case before execution.
- `inconclusive`: execution completed but evidence cannot support pass or fail.

Comparison verdict:

- `improved`: correctness/safety/coverage/freshness/compatibility pass, protected dimensions stay inside budget, and at least one target dimension improves materially.
- `regressed`: correctness, safety, quality, protected dimension, performance, or cost is worse beyond the declared threshold.
- `no_material_difference`: evidence is compatible and adequate but does not prove material improvement or regression.
- `insufficient_evidence`: missing, stale, incompatible, under-covered, under-sampled, invalidated, or excessively noisy evidence prevents a stronger verdict.

## Reason Codes

Reason codes are stable snake-case strings. V1 reason arrays are additive and ordered by gate.

- Input and safety: `manifest_invalid`, `unsupported_schema_version`, `duplicate_identity`, `dangling_reference`, `unsafe_path`, `symlink_escape`, `absolute_path`, `shell_not_allowed`, `network_not_allowed`, `output_limit_exceeded`, `secret_like_output`.
- Run and case: `process_timeout`, `process_failed`, `missing_output`, `invalid_output`, `scorer_error`, `assertion_failed`, `case_skipped`, `case_inconclusive`, `run_interrupted`.
- Compatibility and evidence: `suite_digest_mismatch`, `fixture_digest_mismatch`, `scorer_version_mismatch`, `subject_identity_mismatch`, `testbed_incompatible`, `evidence_stale`, `run_invalidated`, `coverage_below_minimum`, `samples_below_minimum`, `samples_unavailable`, `variance_too_high`, `missing_baseline`, `missing_candidate`.
- Scorer controls: `scorer_control_incorrect_polarity`, `scorer_control_missing_oracle`, `scorer_control_missing_negative`, `scorer_control_nondeterministic`, `scorer_control_oracle_failed`, `scorer_control_negative_passed`, `scorer_control_unexpected_no_op_success`.
- V1.1 treatment and metering: `effective_treatment_missing`, `treatment_fallback_unattributed`, field-specific fallback reasons such as `client_fallback`, `provider_fallback`, `runtime_fallback`, `model_fallback`, `effort_fallback`, `profile_id_fallback`, `profile_digest_fallback`, and `route_policy_digest_fallback`, field-specific escalation reasons such as `client_escalation`, `provider_escalation`, `runtime_escalation`, `model_escalation`, `effort_escalation`, `profile_id_escalation`, `profile_digest_escalation`, and `route_policy_digest_escalation`, effort transition reasons `effort_downgrade` and `quota_downgrade`, `unavailable_value_must_be_null`, `mixed_basis_not_allowed`, `estimate_provenance_missing`, `attempt_identity_conflict`.
- Verdict gates: `correctness_regressed`, `safety_regressed`, `quality_regressed`, `protected_dimension_regressed`, `performance_regressed`, `cost_regressed`, `no_material_effect`, `candidate_improved`.

## Evidence Identity

Every stored object carries enough identity to prove where it came from without retaining secrets or full transcripts.

Run identity fields:

- `run_id`, `suite_digest`, `subject_kind`, `subject_revision`, `runner_version`, `planr_version`, `created_at`, `completed_at`.
- `testbed_fingerprint`: OS, arch, Planr version, CPU class, memory class, and allowlisted environment names without values unless the value is explicitly non-secret and hashed.
- `source_state`: repository root digest inputs, commit when available, dirty-state summary, and fixture digests.
- `lineage`: optional parent run id, resume reason, invalidation id, rescore id, or recompute id.

Case/sample identity fields:

- `run_id`, `case_id`, `scorer_id`, `scorer_version`, `fixture_digest`, `repetition_index`, `warmup`, `seed`.
- `command`: argv array, working directory relative to repository root, timeout, output limit, exit status, stdout digest, stderr digest, and optional bounded artifact ids.
- `attempts`: attempt id, task/case/repetition/seed identity, attempt index, countable flag, retry/fallback/escalation/resume lineage, terminal status, reason codes, runner/harness versions, requested treatment intent, and effective treatment evidence.
- `samples`: measure name, nullable value, unit, source (`process`, `planr_event`, `scorer`, `metering`), attempt id, capture timestamp, metering basis, basis source/confidence, and optional estimate provenance.

Comparison identity fields:

- `comparison_id`, baseline run id, candidate run id, comparison policy digest, runner version, created timestamp, gate results, verdict, reason codes, matched effective-treatment identity, efficiency summary, and recompute/rescore lineage.

## Stored Objects

The fields below are normative for V1 storage and wire projections. Required fields must be present in JSON responses. Optional fields may be `null` only when the value is unknowable; otherwise defaults are applied before persistence.

### EvalSuiteSnapshot

Required fields:

- `schema_version` string: exactly `eval.suite.snapshot.v1`.
- `suite_id` string.
- `suite_version` string.
- `digest` string: `sha256:<64 lowercase hex>`.
- `normalized_manifest` object: canonical semantic manifest after defaults.
- `fixture_digests` object: fixture id to `sha256:<hex>`.
- `scorer_identities` array of `id@version` strings.
- `case_order` array of case ids.
- `baseline_selector` object: `kind` (`explicit_run_id | latest_successful | git_ref`) plus selector-specific fields.
- `comparison_policy_digest` string.
- `created_at` RFC 3339 timestamp.

Optional fields:

- `metadata` object, default `{}`.
- `source_path` repository-relative string, default `null`.

```json
{
  "object": {
    "schema_version": "eval.suite.snapshot.v1",
    "suite_id": "planr-lifecycle-smoke",
    "suite_version": "1.0.0",
    "digest": "sha256:7c4b44e9f6a6113fdf1344f3e14c65b07e32e63e8b22c34784d51a6b9a4ec001",
    "normalized_manifest": {
      "schema_version": "eval.suite.v1",
      "suite_id": "planr-lifecycle-smoke",
      "suite_version": "1.0.0"
    },
    "fixture_digests": {
      "basic-project": "sha256:2bb80d537b1da3e38bd30361aa855686bde0ba89dd8c7d463f537c41b30b25a9"
    },
    "scorer_identities": ["map-lifecycle@1.0.0"],
    "case_order": ["create-plan-map-close"],
    "baseline_selector": {
      "kind": "explicit_run_id",
      "run_id": "evrun-20260721-000001"
    },
    "comparison_policy_digest": "sha256:5a5d4f6ad2c4e98d4f16d857f0f6e2c3f4e5d6c7b8a90123456789abcdef0001",
    "created_at": "2026-07-21T14:20:00Z",
    "metadata": {},
    "source_path": "eval/suites/planr-lifecycle-smoke.json"
  }
}
```

### EvalRun

Required fields:

- `run_id` string.
- `suite_digest` string.
- `suite_snapshot` EvalSuiteSnapshot object or immutable snapshot id.
- `subject` object: `kind`, `revision`, `path`, `argv`, and optional `label`.
- `runner_version` string.
- `planr_version` string.
- `status` run status.
- `created_at`, `started_at`, `completed_at` RFC 3339 timestamps; `completed_at` is `null` until terminal.
- `testbed_fingerprint` object.
- `source_state` object: commit, dirty summary, root path digest, and fixture digests.
- `case_counts` object with explicit zeros for every case status.
- `lineage` object with `parent_run_id`, `resume_of`, `rescore_of`, `recompute_of`, all nullable.

Optional fields:

- `invalidated_by` invalidation id, default `null`.
- `aggregate_summary` object, default `{}`.

```json
{
  "object": {
    "run_id": "evrun-20260721-000002",
    "suite_digest": "sha256:7c4b44e9f6a6113fdf1344f3e14c65b07e32e63e8b22c34784d51a6b9a4ec001",
    "suite_snapshot": "evsuite-7c4b44e9",
    "subject": {
      "kind": "planr_cli",
      "revision": "candidate-abc123",
      "path": ".",
      "argv": ["planr"],
      "label": "candidate"
    },
    "runner_version": "eval-runner-v1",
    "planr_version": "1.5.2",
    "status": "success",
    "created_at": "2026-07-21T14:21:00Z",
    "started_at": "2026-07-21T14:21:01Z",
    "completed_at": "2026-07-21T14:22:00Z",
    "testbed_fingerprint": {
      "os": "macos",
      "arch": "aarch64",
      "planr_version": "1.5.2",
      "cpu_class": "apple-silicon",
      "memory_class": "16gb",
      "env": {
        "PLANR_PROFILE": "sha256:7d9c"
      }
    },
    "source_state": {
      "commit": "abc123",
      "dirty": false,
      "root_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
      "fixture_digests": {
        "basic-project": "sha256:2bb80d537b1da3e38bd30361aa855686bde0ba89dd8c7d463f537c41b30b25a9"
      }
    },
    "case_counts": {
      "pass": 1,
      "fail": 0,
      "error": 0,
      "skipped": 0,
      "inconclusive": 0
    },
    "lineage": {
      "parent_run_id": null,
      "resume_of": null,
      "rescore_of": null,
      "recompute_of": null
    },
    "invalidated_by": null,
    "aggregate_summary": {}
  }
}
```

### EvalCaseResult

Required fields:

- `object_kind` string: exactly `eval_case_result`.
- `run_id` string.
- `case_id` string.
- `scorer_id` string.
- `scorer_version` string.
- `fixture_digest` string.
- `status` case status.
- `repetition_count` integer, required and non-negative.
- `warmup_count` integer, required and non-negative.
- `assertions` array of assertion result objects, default `[]`.
  Safety verdict replay reads a durable scorer observation from either `{"safety_pass": true|false}` or a `{"kind": "safety_pass", "status": "pass"|"fail"}` assertion.
  Quality verdict replay reads a durable scorer observation from either `{"quality_pass": true|false}` or a `{"kind": "quality_pass", "status": "pass"|"fail"}` assertion.
- `command` object with `argv`, `cwd`, `timeout_ms`, `output_limit_bytes`, `exit_code`, `stdout_digest`, `stderr_digest`, and `artifact_ids`.
- `sample_ids` array of strings, default `[]`.
- `reasons` array of reason codes, default `[]`.

```json
{
  "object": {
    "object_kind": "eval_case_result",
    "run_id": "evrun-20260721-000002",
    "case_id": "create-plan-map-close",
    "scorer_id": "map-lifecycle",
    "scorer_version": "1.0.0",
    "fixture_digest": "sha256:2bb80d537b1da3e38bd30361aa855686bde0ba89dd8c7d463f537c41b30b25a9",
    "status": "pass",
    "repetition_count": 5,
    "warmup_count": 1,
    "assertions": [
      {
        "kind": "exit_code",
        "status": "pass",
        "expected": 0,
        "actual": 0
      }
    ],
    "command": {
      "argv": ["planr", "project", "init", "EvalDemo"],
      "cwd": ".",
      "timeout_ms": 15000,
      "output_limit_bytes": 65536,
      "exit_code": 0,
      "stdout_digest": "sha256:3333333333333333333333333333333333333333333333333333333333333333",
      "stderr_digest": "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "artifact_ids": []
    },
    "sample_ids": ["evsample-000001"],
    "reasons": []
  }
}
```

### EvalAttempt

Required fields:

- `object_kind` string: exactly `eval_attempt`.
- `attempt_id` string.
- `task_id` string.
- `run_id` string.
- `case_id` string.
- `repetition_index` integer, required and non-negative.
- `seed` integer.
- `attempt_index` integer, required and non-negative within the same case/repetition/seed.
- `countable` boolean: false only for explicit warmups or retained non-denominator audit attempts.
- `retry_of`, `fallback_of`, `escalation_of`, `resume_of` nullable attempt ids. At most one lineage pointer should be non-null for a single transition unless the runner records a combined transition reason.
- `terminal_status` string enum: `verified_success | failed | error | skipped | inconclusive | stopped`.
- `reasons` array of reason codes, default `[]`.
- `requested_treatment` object: client/provider/runtime/model/effort/profile intent as requested before execution. This is audit context only.
- `effective_treatment` object: client, provider, runtime, model, effort, profile_id, profile_digest, route_policy_digest, runner_version, harness_version, and confidence provenance actually observed for comparison identity.

Optional fields:

- `started_at`, `completed_at` RFC 3339 timestamps, default `null`.
- `lineage_note` string, default `null`.

```json
{
  "object": {
    "object_kind": "eval_attempt",
    "attempt_id": "evatt-candidate-000002",
    "task_id": "task-create-plan-map-close",
    "run_id": "evrun-20260721-000002",
    "case_id": "create-plan-map-close",
    "repetition_index": 0,
    "seed": 12345,
    "attempt_index": 1,
    "countable": true,
    "retry_of": "evatt-candidate-000001",
    "fallback_of": null,
    "escalation_of": null,
    "resume_of": null,
    "terminal_status": "verified_success",
    "reasons": [],
    "requested_treatment": {
      "client": "codex",
      "provider": "openai",
      "runtime": "codex-cli",
      "model": "gpt-5.6-terra",
      "effort": "medium",
      "profile_id": "terra-medium",
      "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    },
    "effective_treatment": {
      "client": "codex",
      "provider": "openai",
      "runtime": "codex-cli",
      "model": "gpt-5.6-terra",
      "effort": "medium",
      "profile_id": "terra-medium",
      "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "runner_version": "eval-runner-v1",
      "harness_version": "planr-eval-harness-v1",
      "confidence": "verified"
    },
    "started_at": "2026-07-21T14:21:30Z",
    "completed_at": "2026-07-21T14:22:00Z",
    "lineage_note": null
  }
}
```

### EvalSample

Required fields:

- `sample_id` string.
- `run_id` string.
- `case_id` string.
- `scorer_id` string.
- `scorer_version` string.
- `attempt_id` string: references EvalAttempt and is part of sample identity.
- `repetition_index` integer, required and non-negative.
- `warmup` boolean.
- `seed` integer.
- `measure` string.
- `value` number, boolean, or `null`; finite when numeric. `null` is required when `metering_basis` is `unavailable`.
- `unit` string.
- `source` string enum: `process | planr_event | scorer | metering`.
- `metering_basis` string enum: `actual_trusted | estimated | mixed | unavailable`.
- `basis_source` string.
- `basis_confidence` string enum: `verified | estimated | mixed | unavailable`.
- `estimate_provenance` object or `null`: required for estimated values and carries pricing/reference id, version, currency, and effective timestamp when cost or credit estimates are used.
- `captured_at` RFC 3339 timestamp.

Identity invariant:

- The database uniqueness key is `(run_id, case_id, scorer_id, scorer_version, attempt_id, measure)`. `attempt_id` includes task, repetition, seed, and attempt_index identity, so retries and multiple measures within one repetition can coexist without overwriting each other.
- `metering_basis = unavailable` requires `value = null`, `basis_confidence = unavailable`, and no estimate provenance. Missing or unavailable values must not be imported, exported, aggregated, or compared as `0`.

```json
{
  "object": {
    "sample_id": "evsample-000001",
    "run_id": "evrun-20260721-000002",
    "case_id": "create-plan-map-close",
    "scorer_id": "map-lifecycle",
    "scorer_version": "1.0.0",
    "attempt_id": "evatt-candidate-000002",
    "repetition_index": 0,
    "warmup": false,
    "seed": 12345,
    "measure": "wall_time_ms",
    "value": 912,
    "unit": "ms",
    "source": "process",
    "metering_basis": "actual_trusted",
    "basis_source": "allowlisted_process_timer",
    "basis_confidence": "verified",
    "estimate_provenance": null,
    "captured_at": "2026-07-21T14:21:10Z"
  }
}
```

### EvalComparison

Required fields:

- `comparison_id` string.
- `baseline_run_id` string.
- `candidate_run_id` string.
- `policy_digest` string.
- `runner_version` string.
- `created_at` RFC 3339 timestamp.
- `verdict` comparison verdict.
- `reasons` array of reason codes.
- `gates` array of gate result objects.
- `effect_estimates` object of measure name to numeric effect.
- `uncertainty` object of measure name to interval or variance metadata.
- `protected_dimensions` object of measure name to numeric result.
- `matched_effective_treatment` object with baseline and candidate effective-treatment identities used for compatible matching.
- `efficiency_summary` object with countable attempts, verified successes, solve rate, total cost/wall-time, per-attempt and per-verified-success metrics, metering-basis mix, sample count, confidence intervals, and reason codes. Per-success values are `null` with `basis = unavailable` or `state = unbounded` when verified successes are zero.
- `lineage` object with nullable `recompute_of` and `rescore_of`.

```json
{
  "object": {
    "comparison_id": "evcmp-20260721-000001",
    "baseline_run_id": "evrun-20260721-000001",
    "candidate_run_id": "evrun-20260721-000002",
    "policy_digest": "sha256:5a5d4f6ad2c4e98d4f16d857f0f6e2c3f4e5d6c7b8a90123456789abcdef0001",
    "runner_version": "eval-runner-v1",
    "created_at": "2026-07-21T14:23:00Z",
    "verdict": "insufficient_evidence",
    "reasons": ["samples_unavailable"],
    "gates": [
      {
        "gate": "compatibility",
        "status": "pass",
        "reasons": []
      },
      {
        "gate": "correctness",
        "status": "pass",
        "reasons": []
      },
      {
        "gate": "performance",
        "status": "blocked",
        "reasons": ["samples_unavailable"]
      }
    ],
    "effect_estimates": {
      "duration_ms_p95_relative": -0.12
    },
    "uncertainty": {
      "duration_ms_p95_ci95": [-0.18, -0.07],
      "variance": "acceptable"
    },
    "protected_dimensions": {
      "correctness_pass_rate": 1.0,
      "tool_calls_relative": 0.0
    },
    "matched_effective_treatment": {
      "baseline": {
        "client": "codex",
        "provider": "openai",
        "runtime": "codex-cli",
        "model": "gpt-5.6-terra",
        "effort": "medium",
        "profile_id": "terra-medium",
        "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "runner_version": "eval-runner-v1",
        "harness_version": "planr-eval-harness-v1",
        "confidence": "verified"
      },
      "candidate": {
        "client": "codex",
        "provider": "openai",
        "runtime": "codex-cli",
        "model": "gpt-5.6-terra",
        "effort": "medium",
        "profile_id": "terra-medium",
        "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "runner_version": "eval-runner-v1",
        "harness_version": "planr-eval-harness-v1",
        "confidence": "verified"
      }
    },
    "efficiency_summary": {
      "countable_attempts": 2,
      "verified_successes": 1,
      "solve_rate": 0.5,
      "total_cost_micros": {
        "value": null,
        "basis": "unavailable",
        "reasons": ["samples_unavailable"]
      },
      "cost_per_attempt_micros": {
        "value": null,
        "basis": "unavailable",
        "state": "unavailable"
      },
      "cost_per_verified_success_micros": {
        "value": null,
        "basis": "unavailable",
        "state": "unavailable",
        "reasons": ["samples_unavailable"]
      },
      "wall_time_per_verified_success_ms": {
        "value": 2112,
        "basis": "actual_trusted"
      },
      "sample_count": 4,
      "basis_mix": ["actual_trusted", "estimated", "unavailable"],
      "confidence_intervals": {},
      "reasons": ["samples_unavailable"]
    },
    "lineage": {
      "recompute_of": null,
      "rescore_of": null
    }
  }
}
```

### EvalInvalidation

Required fields:

- `invalidation_id` string.
- `target_kind` string enum: `run | comparison`.
- `target_id` string.
- `reason` string.
- `reason_codes` array of reason codes.
- `created_at` RFC 3339 timestamp.
- `created_by` string.
- `replacement_hint` string or `null`.

```json
{
  "object": {
    "invalidation_id": "evinv-20260721-000001",
    "target_kind": "run",
    "target_id": "evrun-20260721-000001",
    "reason": "baseline captured on incompatible testbed",
    "reason_codes": ["testbed_incompatible"],
    "created_at": "2026-07-21T14:24:00Z",
    "created_by": "maker-1",
    "replacement_hint": "recapture baseline on the current testbed"
  }
}
```

### EvalEvidenceRef

Required fields:

- `evidence_ref` string.
- `target_kind` string enum: `run | comparison`.
- `target_id` string.
- `planr_attachment_kind` string enum: `log | artifact`.
- `planr_attachment_id` string.
- `item_id` string.
- `created_at` RFC 3339 timestamp.
- `closure_authority` boolean, always `false` in V1.

```json
{
  "object": {
    "evidence_ref": "eval-comparison:evcmp-20260721-000001",
    "target_kind": "comparison",
    "target_id": "evcmp-20260721-000001",
    "planr_attachment_kind": "log",
    "planr_attachment_id": "log-2ae87225",
    "item_id": "i-freeze-eval-contract-v1-and-owne-e51f",
    "created_at": "2026-07-21T14:25:00Z",
    "closure_authority": false
  }
}
```

## CLI and MCP Responses

All eval CLI JSON and MCP tool responses use one envelope:

- `ok` boolean.
- `command` string: canonical command name.
- `object` object or `null`: primary result.
- `warnings` array, default `[]`.
- `reasons` array, default `[]`.
- `error` object or `null`: `{ code, message, reasons, field }` on failure.

MCP tools mirror the CLI object exactly inside the normal MCP text content wrapper. HTTP, if added, returns the same envelope as the response body.

Suite check:

```json
{
  "ok": true,
  "command": "eval.suite.check",
  "object": {
    "suite_id": "planr-lifecycle-smoke",
    "suite_version": "1.0.0",
    "digest": "sha256:7c4b44e9f6a6113fdf1344f3e14c65b07e32e63e8b22c34784d51a6b9a4ec001",
    "case_count": 1,
    "scorer_count": 1,
    "fixture_count": 1
  },
  "warnings": [],
  "reasons": [],
  "error": null
}
```

Run:

```json
{
  "ok": true,
  "command": "eval.run",
  "object": {
    "run_id": "evrun-20260721-000001",
    "suite_digest": "sha256:7c4b44e9f6a6113fdf1344f3e14c65b07e32e63e8b22c34784d51a6b9a4ec001",
    "status": "success",
	    "attempt_summary": {
	      "countable_attempts": 2,
	      "verified_successes": 1,
	      "failed_attempts": 1,
	      "retry_attempts": 1,
	      "basis_mix": ["actual_trusted", "estimated"]
	    },
	    "attempt_lineage": [
	      {
	        "attempt_id": "evatt-candidate-000001",
	        "attempt_index": 0,
	        "terminal_status": "failed",
	        "retry_of": null,
	        "fallback_of": null,
	        "escalation_of": null,
	        "resume_of": null,
	        "requested_treatment": {
	          "client": "codex",
	          "provider": "openai",
	          "runtime": "codex-cli",
	          "model": "gpt-5.6-terra",
	          "effort": "medium",
	          "profile_id": "terra-medium",
	          "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	          "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
	        },
	        "effective_treatment": {
	          "client": "codex",
	          "provider": "openai",
	          "runtime": "codex-cli",
	          "model": "gpt-5.6-terra",
	          "effort": "medium",
	          "profile_id": "terra-medium",
	          "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	          "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
	          "runner_version": "eval-runner-v1",
	          "harness_version": "planr-eval-harness-v1",
	          "confidence": "verified"
	        },
	        "reasons": ["process_failed"]
	      },
	      {
	        "attempt_id": "evatt-candidate-000002",
	        "attempt_index": 1,
	        "terminal_status": "verified_success",
	        "retry_of": "evatt-candidate-000001",
	        "fallback_of": null,
	        "escalation_of": null,
	        "resume_of": null,
	        "requested_treatment": {
	          "client": "codex",
	          "provider": "openai",
	          "runtime": "codex-cli",
	          "model": "gpt-5.6-terra",
	          "effort": "medium",
	          "profile_id": "terra-medium",
	          "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	          "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
	        },
	        "effective_treatment": {
	          "client": "codex",
	          "provider": "openai",
	          "runtime": "codex-cli",
	          "model": "gpt-5.6-terra",
	          "effort": "medium",
	          "profile_id": "terra-medium",
	          "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	          "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
	          "runner_version": "eval-runner-v1",
	          "harness_version": "planr-eval-harness-v1",
	          "confidence": "verified"
	        },
	        "reasons": []
	      }
	    ],
	    "sample_metering": [
	      {
	        "sample_id": "evsample-candidate-cost",
	        "attempt_id": "evatt-candidate-000001",
	        "measure": "cost_micros",
	        "metering_basis": "actual_trusted",
	        "basis_source": "host_usage_receipt",
	        "basis_confidence": "verified",
	        "estimate_provenance": null
	      },
	      {
	        "sample_id": "evsample-candidate-cost-estimated",
	        "attempt_id": "evatt-candidate-000002",
	        "measure": "cost_micros",
	        "metering_basis": "estimated",
	        "basis_source": "pricing_reference",
	        "basis_confidence": "estimated",
	        "estimate_provenance": {
	          "pricing_reference_id": "openai-prices",
	          "pricing_reference_version": "2026-07-01",
	          "currency": "USD",
	          "effective_at": "2026-07-01T00:00:00Z"
	        }
	      }
	    ],
	    "efficiency_summary": {
	      "countable_attempts": 2,
	      "verified_successes": 1,
	      "total_cost_micros": {
	        "value": 1000,
	        "basis": "mixed",
	        "estimate_provenance": ["openai-prices@2026-07-01"]
	      },
	      "cost_per_verified_success_micros": {
	        "value": 1000,
	        "basis": "mixed",
	        "state": "available",
	        "estimate_provenance": ["openai-prices@2026-07-01"]
	      },
	      "reasons": []
	    },
	    "case_counts": {
      "pass": 1,
      "fail": 0,
      "error": 0,
      "skipped": 0,
      "inconclusive": 0
    },
    "evidence_ref": "eval-run:evrun-20260721-000001"
  },
  "warnings": [],
  "reasons": [],
  "error": null
}
```

Show run or comparison:

```json
{
  "ok": true,
  "command": "eval.show",
  "object": {
    "target_kind": "comparison",
    "target_id": "evcmp-20260721-000001",
		    "summary": {
		      "verdict": "insufficient_evidence",
		      "first_blocking_gate": "performance"
		    },
	    "matched_effective_treatment": {
	      "baseline": {
	        "client": "codex",
	        "provider": "openai",
	        "runtime": "codex-cli",
	        "model": "gpt-5.6-terra",
	        "effort": "medium",
	        "profile_id": "terra-medium",
	        "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	        "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
	        "runner_version": "eval-runner-v1",
	        "harness_version": "planr-eval-harness-v1",
	        "confidence": "verified"
	      },
	      "candidate": {
	        "client": "codex",
	        "provider": "openai",
	        "runtime": "codex-cli",
	        "model": "gpt-5.6-terra",
	        "effort": "medium",
	        "profile_id": "terra-medium",
	        "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	        "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
	        "runner_version": "eval-runner-v1",
	        "harness_version": "planr-eval-harness-v1",
	        "confidence": "verified"
	      }
	    },
	    "attempt_lineage": [
	      {
	        "attempt_id": "evatt-candidate-000002",
	        "retry_of": "evatt-candidate-000001",
	        "fallback_of": null,
	        "escalation_of": null,
	        "resume_of": null,
	        "reasons": ["process_failed"],
	        "effective_treatment": {
	          "client": "codex",
	          "provider": "openai",
	          "runtime": "codex-cli",
	          "model": "gpt-5.6-terra",
	          "effort": "medium",
	          "profile_id": "terra-medium",
	          "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	          "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
	          "runner_version": "eval-runner-v1",
	          "harness_version": "planr-eval-harness-v1",
	          "confidence": "verified"
	        }
	      }
	    ],
	    "sample_metering": [
		      {
		        "attempt_id": "evatt-candidate-000002",
		        "measure": "cost_micros",
		        "metering_basis": "unavailable",
		        "basis_source": "provider_unavailable",
		        "basis_confidence": "unavailable",
		        "estimate_provenance": null
		      }
	    ],
	    "efficiency_summary": {
	      "countable_attempts": 2,
	      "verified_successes": 1,
		      "cost_per_verified_success_micros": {
		        "value": null,
		        "basis": "unavailable",
		        "state": "unavailable",
		        "reasons": ["samples_unavailable"]
		      },
		      "reasons": ["samples_unavailable"]
	    },
	    "evidence_ref": "eval-comparison:evcmp-20260721-000001"
	  },
  "warnings": [],
  "reasons": [],
  "error": null
}
```

Compare:

```json
{
  "ok": true,
  "command": "eval.compare",
  "object": {
    "comparison_id": "evcmp-20260721-000001",
    "baseline_run_id": "evrun-20260721-000001",
    "candidate_run_id": "evrun-20260721-000002",
	    "verdict": "insufficient_evidence",
	    "reasons": ["samples_unavailable"],
    "matched_effective_treatment": {
      "baseline": {
        "client": "codex",
        "provider": "openai",
        "runtime": "codex-cli",
        "model": "gpt-5.6-terra",
        "effort": "medium",
        "profile_id": "terra-medium",
        "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "runner_version": "eval-runner-v1",
        "harness_version": "planr-eval-harness-v1",
        "confidence": "verified"
      },
      "candidate": {
        "client": "codex",
        "provider": "openai",
        "runtime": "codex-cli",
        "model": "gpt-5.6-terra",
        "effort": "medium",
        "profile_id": "terra-medium",
        "profile_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "route_policy_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "runner_version": "eval-runner-v1",
        "harness_version": "planr-eval-harness-v1",
        "confidence": "verified"
      }
    },
    "efficiency_summary": {
      "countable_attempts": 2,
      "verified_successes": 1,
      "solve_rate": 0.5,
		      "cost_per_verified_success_micros": {
		        "value": null,
		        "basis": "unavailable",
		        "state": "unavailable",
		        "reasons": ["samples_unavailable"]
		      },
      "wall_time_per_verified_success_ms": {
        "value": 2112,
        "basis": "actual_trusted"
      },
		      "basis_mix": ["actual_trusted", "estimated", "unavailable"],
		      "reasons": ["samples_unavailable"]
	    },
    "gates": [
      {
        "gate": "compatibility",
        "status": "pass",
        "reasons": []
      },
      {
        "gate": "correctness",
        "status": "pass",
        "reasons": []
      },
      {
        "gate": "performance",
	        "status": "blocked",
	        "reasons": ["samples_unavailable"]
      }
    ],
    "evidence_ref": "eval-comparison:evcmp-20260721-000001"
  },
  "warnings": [],
  "reasons": [],
  "error": null
}
```

Gate:

```json
{
  "ok": true,
  "command": "eval.gate",
  "object": {
    "comparison_id": "evcmp-20260721-000001",
	    "verdict": "insufficient_evidence",
	    "first_blocking_gate": "performance",
	    "promotion_allowed": false,
	    "reasons": ["samples_unavailable"]
  },
  "warnings": [],
  "reasons": ["samples_unavailable"],
  "error": null
}
```

Invalidate:

```json
{
  "ok": true,
  "command": "eval.invalidate",
  "object": {
    "invalidation_id": "evinv-20260721-000001",
    "target_kind": "run",
    "target_id": "evrun-20260721-000001",
    "status": "invalidated",
    "reason_codes": ["testbed_incompatible"]
  },
  "warnings": [],
  "reasons": ["testbed_incompatible"],
  "error": null
}
```

Rescore:

```json
{
  "ok": true,
  "command": "eval.rescore",
  "object": {
    "source_run_id": "evrun-20260721-000002",
    "rescored_run_id": "evrun-20260721-000003",
    "scorer_identity": "map-lifecycle@1.1.0",
    "lineage": {
      "rescore_of": "evrun-20260721-000002"
    }
  },
  "warnings": [],
  "reasons": [],
  "error": null
}
```

Invalid input:

```json
{
  "ok": false,
  "command": "eval.suite.check",
  "object": null,
  "warnings": [],
  "reasons": ["unsafe_path", "symlink_escape"],
  "error": {
    "code": "invalid_input",
    "message": "manifest contains an unsafe fixture path",
    "reasons": ["unsafe_path", "symlink_escape"],
    "field": "fixtures[0].path"
  }
}
```

## Exit Codes

- `0`: successful command; gate may be `improved` or another non-error verdict depending on subcommand.
- `1`: regression or gate failure for commands intended to enforce promotion.
- `2`: insufficient evidence.
- `3`: invalid input or unsafe manifest.
- `4`: infrastructure error.

## Database Ownership

The first implementation slice must add SQLite tables or typed repository methods for run evidence only. It must not add item status columns, alternate closure state, or eval-owned review gates.

Required persistence rules:

- Insert one immutable suite snapshot per digest.
- Insert one run transition or one completed case plus its samples in a transaction.
- Insert one immutable attempt row per `(run_id, case_id, repetition_index, seed, attempt_index)` and preserve `retry_of`, `fallback_of`, `escalation_of`, and `resume_of` references as nullable foreign-key-style ids. Attempt rows own terminal status, reason codes, requested treatment intent, and effective treatment evidence.
- Use uniqueness constraints for `(run_id, case_id, scorer_id, scorer_version, attempt_id, measure)` samples. Multiple measures in one attempt and multiple retry attempts in one repetition must coexist; importing another row with the same tuple is an identity conflict unless every field is byte-identical.
- Preserve nullable sample values. `metering_basis = unavailable` requires `value IS NULL`; `actual_trusted`, `estimated`, and `mixed` require a finite number or boolean according to the measure. Do not materialize unavailable cost, token, time, or tool-call values as zero in migration defaults, package imports, CLI output, MCP output, HTTP output, aggregate caches, or comparison math.
- Append invalidation, rescore, and recompute provenance; never update historical verdicts in place.
- Store aggregate summaries as derived cache fields only when raw samples remain available.
- Store Planr log/review attachments as `EvalEvidenceRef` rows or existing artifact/log references that point to eval ids.

Package and migration projection rules:

- Eval packages include `suite_snapshots`, `runs`, `case_results`, `attempts`, `samples`, `comparisons`, `invalidations`, and `evidence_refs` arrays. `attempts` and `samples` carry the same field names and nullable invariants as the stored objects above.
- Package import preflight validates every sample against its attempt, rejects `unavailable` samples with numeric or boolean values, rejects estimated samples without estimate provenance, and rejects silent treatment fallback unless field-specific transition reasons account for every requested/effective treatment field mismatch. Generic fallback/escalation pointers, retry lineage, resume lineage, `treatment_fallback_attributed`, and `treatment_transition_attributed` do not satisfy this preflight.
- Additive migrations create attempt and sample metadata columns with nullable defaults. Backfilled V1 rows are marked non-comparable for V1.1 efficiency until effective treatment identity and metering basis are supplied by durable evidence.
- CLI, MCP, HTTP, package export, package import preview, and migration diagnostics must all use the stable reason-code vocabulary above; adapters may not invent surface-local strings for v1.1 metering or treatment failures.

## Security Constraints

- Execute only explicit argv arrays; do not pass authored commands through a shell.
- Bound time, output bytes, concurrency, repetitions, and retained artifact size before execution.
- Reject path traversal, absolute paths, symlink escapes, and user/global configuration targets before any execution or database mutation.
- Do not capture prompts, responses, source files, secrets, full transcripts, or unrestricted stdout/stderr by default.
- Scrub bounded excerpts with the existing secret detection policy before persistence.
- Treat fabricated result files, stale baselines, and incompatible testbeds as `insufficient_evidence`, not as passes.
