# Evidence Contract v1

Status: frozen for implementation

Evidence Contract v1 is Planr's local-first contract for proving acceptance criteria. It defines the durable vocabulary for proof obligations, observation requirements, capability declarations, execution attempts, untrusted proposals, trusted receipts, coverage verdicts, policy, presets, waivers, statuses, compatibility, and extension naming. It does not add public commands by itself and does not grant closure authority until later implementation slices wire the contract into storage, audit, review, trace, pick, and active-goal enforcement.

## Ownership

- Product and build plans own authored acceptance criteria and the reviewed `ProofObligation` records bound to those criteria.
- The Evidence domain owns `ProofObligation`, `ObservationRequirement`, `VerificationCapabilityManifest`, `VerificationCapabilityInstance`, `EvidenceAttempt`, `UntrustedEvidenceProposal`, `EvidenceReceipt`, `CoverageVerdict`, `EvidencePolicy`, `ProofPreset`, and `EvidenceWaiver`.
- Planr assigns trusted provenance only from Planr-observed execution, verified host events, accepted MCP attestation, validated artifact import, or explicit approval-backed user attestation.
- Public JSON, agent-authored JSON, adapter stdout, logs, and artifacts may propose claims, but they cannot construct trusted provenance, execution identity, freshness, target binding, receipt digest, or closure authority.
- `src/canonical_json.rs` is the future single production owner for RFC 8785/JCS-compatible canonical bytes and SHA-256 digests used by this contract.
- `src/execution.rs` is the future shared owner for bounded process execution, cancellation, process-tree cleanup, output limits, and stdout/stderr digests.
- CLI, MCP, HTTP, docs, audit, hooks, review, trace, and pick surfaces are renderers or consumers. They must use the same Evidence application service and cannot implement alternate coverage decisions.

## Existing-System Boundaries

- Eval remains the owner of comparative candidate evaluation, immutable eval runs, case results, samples, comparisons, invalidations, and eval evidence references. Eval evidence keeps `closure_authority = false`; an eval verdict may support review but cannot close or reopen Planr work by itself.
- Route Audit remains the owner of requested, resolved, and effective routing evidence. Evidence may consume a mapped provenance view, but it must not copy requested route declarations into effective execution proof.
- Agent Profiles, model-routing capability classes, usage-policy capability classes, MCP protocol capabilities, and context tags are dispatch or protocol metadata. They are not verification capability instances and do not prove runtime availability.
- Planr logs remain narrative and supporting records. A legacy `kind = verification` log is preserved as a claim and can be referenced, but it never automatically satisfies a binding observation.
- Pre-Evidence plans and non-binding scopes keep their historical closure compatibility until an operator explicitly migrates a plan into binding Evidence. Migration is plan-scoped, previewable, idempotent, and materializes ordinary immutable `ProofObligation` rows; it must not rewrite plans, logs, reviews, artifacts, or historical claims.
- Planr artifacts remain files or references with digests. An artifact alone is not trusted evidence unless a trusted receipt binds it to the source revision, target, environment, execution identity, observation results, and policy.

## Versioning And Compatibility

- `schema_version` for all v1 objects is `evidence.contract.v1`.
- Additive optional fields are allowed only when old readers can ignore them without changing coverage, trust, freshness, or closure decisions.
- Status enum values, failure reason codes, provenance source kinds, namespace rules, digest algorithms, and trusted/untrusted boundaries are frozen for v1.
- Removing fields, renaming statuses, weakening required trust checks, changing digest semantics, or treating legacy logs as binding proof requires a new major contract.
- Unknown object kinds, observation types, schema versions, status values, provenance kinds, or namespaces are invalid for trusted coverage unless an explicit project extension registers them.
- Canonical JSON bytes and digests are computed over the object without non-contractual display fields. The frozen schema fixture in `docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json` names each object's required fields.

## Contract Digests

Digest preimages use RFC 8785/JCS canonical JSON bytes over the contract object after the digest field being produced is removed. No other contract field is implicitly excluded.

- `EvidenceReceipt.receipt_digest`: hash the complete receipt object without the top-level `receipt_digest` field. Source, target, environment, capability, provenance, execution identity, observations, attempts, artifacts, raw result, config digest, fixture disclosure, permissions, sandbox, proof gaps, and timings all remain in the preimage.
- `EvidencePolicy.policy_digest`: hash the complete policy object without the top-level `policy_digest` field. Defaults, presets, observation schema registrations, adapter registrations, extension namespaces, trust/freshness/fixture/completion/layering policy all remain in the preimage. Nested `PolicyLayer.policy_digest` values are included as references to already-materialized lower-layer policies and must not repeat the enclosing policy digest.

The canonical fixture vectors are executable through `node scripts/verify-evidence-contract-vectors.mjs`.

## Namespaces And Extensions

Built-in observation types and proof presets are starter vocabulary only. They are conveniences for common API, process, browser, desktop, mobile, deployment, filesystem, database, queue, and artifact cases, not a closed enum.

Observation type names and preset names use one of these namespaces:

- `planr.*`: reserved for Planr-owned built-ins.
- `mcp.*`: reserved for MCP attestation vocabulary that Planr validates.
- `host.*`: reserved for verified host-native capture contracts.
- Reverse-domain namespaces such as `com.example.queue.job.processed`.
- Project-local namespaces such as `project.queue.job.processed` inside the repository that defines them.

Custom observation types must reference a versioned JSON Schema and a registration digest in repository-local Evidence policy. Unknown, unregistered, or unavailable custom types remain uncovered and cannot be silently accepted as a weaker built-in.

## Object Model

### ProofObligation

A reviewed, binding or advisory contract for one acceptance criterion.

Required fields:

- `id`, `schema_version`, `criterion_id`, `plan_id`, optional `item_id`.
- `title`, `binding`, `observations`, `fixture_policy`, `freshness_policy`, `assurance_policy`.
- `policy_digest`, `config_digest`, `created_at`, and optional `supersedes`.

`binding = true` means closure must evaluate Evidence coverage. `binding = false` may be rendered as guidance but cannot block or satisfy closure.

### ObservationRequirement

One atomic fact that must be observed.

Required fields:

- `id`, namespaced `type`, `subject`, `expected`, `target`, `environment`, `runtime_target`.
- Optional `payload_schema`, `state_transitions`, `persistence`, `negative_assertions`, `freshness_policy`, and `assurance_policy`.

The `type` says what fact is being observed, not which tool must observe it. Tool preference belongs in policy.

### VerificationCapabilityManifest

A versioned static declaration from an adapter, host, or attestor.

Required fields:

- `id`, `schema_version`, `version`, `adapter_kind`, `adapter_digest`.
- Supported surfaces, observation type/schema/digest triples, interactions, artifacts, runtime targets, provenance path, permissions, costs, determinism, repeatability, independence, blind spots, and availability probe contract.
- Process adapters declare a closed `availability_probe.kind = process` contract with executable name, arguments, optional working directory, timeout, stdout/stderr byte limits, and the payload schema binding for emitted observations.

A manifest is a claim about what a method can observe. It is not proof that the method is available now.

### VerificationCapabilityInstance

A runtime availability snapshot for one exact manifest, host, surface, version, environment, and permission set.

Required fields:

- `id`, `schema_version`, `manifest_id`, `manifest_digest`, `host`, `surface`, `host_version`, `adapter_version`, `environment`, `permissions`, `availability`, `probe_result`, `observed_payload_contract`, `limitations`, `captured_at`.

`probe_result.probe_execution_id` is the single authoritative runtime probe execution identity for a capability instance. A separate top-level probe execution ID is not part of the contract.

Instances expire or become stale when manifest, host, environment, permission, schema, or policy inputs change.

### EvidenceAttempt

An immutable record of each verification attempt.

Required fields:

- `id`, `schema_version`, `criterion_id`, `obligation_id`, `capability_instance_id`.
- `started_at`, `ended_at`, `status`, resolved command or tool call, exit/signal/error details, retry lineage, stdout/stderr digests, raw result reference, artifact references, and output bounds.

Attempts are durable whether they pass, fail, time out, abort, skip, become unavailable, or end inconclusively. A later passing attempt does not erase earlier failures.

Attempt status and execution result must agree: `passed` requires clean exit code `0`; `failed` requires a non-zero exit code; `timed_out`, `skipped`, and `unavailable` require no exit code or signal and the matching error value; `aborted` requires no exit code plus an abort signal and error; `inconclusive` requires a completed exit with an inconclusive error marker.

### UntrustedEvidenceProposal

Adapter, host, import, or agent input before Planr assigns provenance.

Required fields:

- `id`, `schema_version`, `source_kind`, `submitted_at`, `claims`, `artifact_refs`, `producer_metadata`.

Forbidden authoritative fields:

- Trusted provenance, execution identity, source revision binding, target digest binding, environment identity, freshness result, policy result, coverage result, receipt digest, or closure authority.

### EvidenceReceipt

The trusted outer envelope created only by Planr or an accepted attestation verifier after validation.

Required fields:

- `id`, `schema_version`, `criterion_id`, `obligation_id`, `receipt_status`.
- Source revision/tree state, target digest/URI/deployment ID, environment identity, vantage point.
- Capability manifest and instance digests, Planr-assigned provenance, execution/tool-call ID.
- Observation results, attempt IDs and retry history, artifact references and digests.
- Fixture/mock disclosure, permission and sandbox state, proof gaps, started/ended timestamps, and `receipt_digest`.

Receipts can satisfy coverage only when trusted, fresh, policy-compliant, target-matched, schema-valid, and observation-equivalent to the requirement.

### CoverageVerdict

The canonical coverage result for a criterion, item, plan, or goal.

Required fields:

- `id`, `schema_version`, `scope`, `evaluated_at`, `status`, `observation_coverage`, `validation_details`, `suggested_next_action`, `actionable_now`.

Allowed statuses:

- `satisfied`: every binding observation is covered by trusted, fresh, policy-compliant receipts.
- `unsatisfied`: one or more observations lack sufficient proof and a product or test failure is indicated.
- `blocked`: proof cannot currently be produced because a required capability, permission, sandbox, environment, or dependency is unavailable.
- `inconclusive`: attempted evidence cannot decide the required observation.
- `waived`: every remaining binding gap is covered by a valid scoped waiver.
- `stale`: previously sufficient evidence was invalidated by source, target, environment, policy, adapter, schema, configuration, or freshness change.

Gap coverage must match the verdict status:

- `unsatisfied` uses `missing` or `unsatisfied` gap entries with missing/product/verifier/policy/trust/schema/fixture/waiver/unsupported-observation reasons.
- `blocked` uses `blocked` gap entries with capability, permission, sandbox, environment, dependency, or runtime-target reasons.
- `inconclusive` uses `inconclusive` gap entries with timeout, abort, inconclusive-result, or verifier-failed reasons.
- `waived` uses `waived` gap entries, requires `waiver_id`, and records the underlying waived reason rather than `waiver_missing` or `waiver_expired`.
- `stale` uses `stale` gap entries with stale source, target, environment, policy, adapter schema, or configuration reasons.

### EvidencePolicy, ProofPreset, EvidenceWaiver

Repository policy lives in `.planr/evidence.yaml`. It owns defaults, named presets, observation schema registrations, adapter registrations, trust/freshness/fixture/completion policy, and extension namespaces.

Adapter registrations bind the manifest ID, repository-local manifest path, manifest digest, observation types, payload schema bindings, provenance path, and closed execution contract used by Planr-observed process adapters.

Policy layer scope uses the frozen `EvidenceScope` chain. Repository policy defaults and extension declarations are document-level policy. Product-level policy is represented by `goal`, plan-level policy by `plan`, item-specific branches by `item`, and criterion-level policy by `criterion`.

Policy layers may strengthen lower-level requirements. Weakening requires an explicit waiver. Waivers include identity, reason, scope, exact observation IDs, source/target binding, created-at, expires-at, approval reference, and audit trail.

## Status And Failure Vocabularies

`EvidenceAttempt.status` values:

- `passed`, `failed`, `skipped`, `timed_out`, `aborted`, `unavailable`, `inconclusive`.

`EvidenceReceipt.receipt_status` values:

- `trusted`, `rejected`, `untrusted`, `stale`, `superseded`.

`VerificationCapabilityInstance.availability.status` values:

- `available`, `unavailable`, `degraded`, `permission_denied`, `sandbox_blocked`, `unsupported`, `probe_failed`.

Coverage gap/failure reason codes:

- `missing_observation`, `missing_capability`, `permission_denied`, `sandbox_blocked`, `environment_unavailable`, `external_dependency_unavailable`, `product_failed`, `verifier_failed`, `timed_out`, `aborted`, `inconclusive_result`, `stale_source`, `stale_target`, `stale_environment`, `stale_policy`, `stale_adapter_schema`, `stale_configuration`, `target_mismatch`, `schema_mismatch`, `manifest_mismatch`, `untrusted_provenance`, `fixture_disallowed`, `mock_disallowed`, `insufficient_assurance`, `waiver_missing`, `waiver_expired`, `unknown_observation_type`, `unsupported_runtime_target`.

Legacy/operator aliases are rendered as aliases only and resolve to the canonical codes above: `capability_unavailable` -> `missing_capability`, `dependency_unavailable` -> `external_dependency_unavailable`, `policy_failed` -> `stale_policy`, `trust_failed` -> `untrusted_provenance`, and `stale_evidence` -> `stale_source`. Unknown legacy reasons are classified as `verifier_failed` rather than `product_failed`.

Explicit pre-Evidence migration input uses `schema_version = "planr.evidence.migration.v1"`, a single `plan_id`, and an `obligations[]` array of full `ProofObligation` objects whose `plan_id` matches the migration plan and whose `binding` is `true`. Preview is non-mutating. Apply is atomic for the migration payload: any conflict or malformed obligation leaves the plan with no newly bound partial obligations. Reapplying an identical payload is `unchanged`.

Process adapters may report a host boundary failure only with an exact single-field JSON line on stdout or stderr:

- `{"planr_adapter_boundary":"permission_denied"}`
- `{"planr_adapter_boundary":"sandbox_blocked"}`

Planr treats those structured lines as adapter boundary signals for failed process attempts and capability probes. Numeric exit codes alone do not imply `sandbox_blocked`; shell exit code 126 remains permission denied.

## Trust And Provenance Rules

- Public deserialization always produces an `UntrustedEvidenceProposal` or authored policy object, never a trusted receipt.
- Adapter output can report observed facts and artifact references, but Planr binds the actual source revision, target, environment, execution identity, timings, retries, sandbox, permissions, output bounds, and receipt digest.
- Requested route metadata, declared agent profile metadata, and model names are not effective execution proof.
- A user approval can attest a fact only through the waiver or user-attestation path, with identity, reason, scope, expiry where applicable, and audit trail. It cannot silently convert agent JSON into Planr-observed execution.
- Artifacts referenced by trusted receipts require content digests and repository/root containment or explicit external URI policy.
- Fixture and mock use must be disclosed and accepted by policy for the exact observation.

## Canonical Fixtures

The v1 frozen fixtures are:

- `docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json`
- `docs/contracts/fixtures/evidence/v1/examples/proof-obligation.json`
- `docs/contracts/fixtures/evidence/v1/examples/verification-capability-manifest.json`
- `docs/contracts/fixtures/evidence/v1/examples/verification-capability-instance.json`
- `docs/contracts/fixtures/evidence/v1/examples/evidence-attempt.json`
- `docs/contracts/fixtures/evidence/v1/examples/untrusted-evidence-proposal.json`
- `docs/contracts/fixtures/evidence/v1/examples/evidence-receipt.json`
- `docs/contracts/fixtures/evidence/v1/examples/coverage-verdict.json`
- `docs/contracts/fixtures/evidence/v1/examples/evidence-policy.json`
- `docs/contracts/fixtures/evidence/v1/examples/proof-preset.json`
- `docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json`
- `docs/contracts/fixtures/evidence/v1/negative/available-empty-probe-capability.json`
- `docs/contracts/fixtures/evidence/v1/negative/mismatched-probe-execution-id-capability.json`
- `docs/contracts/fixtures/evidence/v1/negative/empty-trust-bindings-receipt.json`
- `docs/contracts/fixtures/evidence/v1/negative/failed-zero-exit-attempt.json`
- `docs/contracts/fixtures/evidence/v1/negative/forged-trusted-receipt.json`
- `docs/contracts/fixtures/evidence/v1/negative/timed-out-zero-exit-attempt.json`
- `docs/contracts/fixtures/evidence/v1/negative/aborted-zero-exit-attempt.json`
- `docs/contracts/fixtures/evidence/v1/negative/skipped-zero-exit-attempt.json`
- `docs/contracts/fixtures/evidence/v1/negative/unavailable-zero-exit-attempt.json`
- `docs/contracts/fixtures/evidence/v1/negative/inconclusive-clean-exit-attempt.json`
- `docs/contracts/fixtures/evidence/v1/negative/passed-empty-attempt.json`
- `docs/contracts/fixtures/evidence/v1/negative/unbound-waiver.json`
- `docs/contracts/fixtures/evidence/v1/negative/minimal-policy-missing-owned-inputs.json`
- `docs/contracts/fixtures/evidence/v1/negative/policy-default-preset-unregistered.json`
- `docs/contracts/fixtures/evidence/v1/negative/policy-weakening-without-waiver.json`
- `docs/contracts/fixtures/evidence/v1/negative/satisfied-empty-coverage-verdict.json`
- `docs/contracts/fixtures/evidence/v1/negative/unsatisfied-coverage-without-gap.json`
- `docs/contracts/fixtures/evidence/v1/negative/process-probe-missing-output-limits.json`
- `docs/contracts/fixtures/evidence/v1/negative/adapter-registration-missing-execution-contract.json`
- `docs/contracts/fixtures/evidence/v1/negative/waived-coverage-missing-waiver-id.json`
- `docs/contracts/fixtures/evidence/v1/negative/waived-coverage-forbidden-gap-reason.json`
- `docs/contracts/fixtures/evidence/v1/negative/blocked-coverage-with-stale-reason.json`
- `docs/contracts/fixtures/evidence/v1/negative/inconclusive-coverage-with-blocked-reason.json`
- `docs/contracts/fixtures/evidence/v1/negative/stale-coverage-with-inconclusive-reason.json`

Replayable schema validation:

```bash
set -e
schema='docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json'
node scripts/verify-evidence-contract-vectors.mjs
npx --yes ajv-cli@5.0.0 validate --spec=draft2020 --validate-formats=false -s "$schema" -d 'docs/contracts/fixtures/evidence/v1/examples/*.json'
for fixture in docs/contracts/fixtures/evidence/v1/negative/*.json; do
  case "$fixture" in
    *policy-default-preset-unregistered.json)
      npx --yes ajv-cli@5.0.0 validate --spec=draft2020 --validate-formats=false -s "$schema" -d "$fixture"
      echo "$fixture accepted by schema for semantic validation"
      continue
      ;;
  esac
  if npx --yes ajv-cli@5.0.0 validate --spec=draft2020 --validate-formats=false -s "$schema" -d "$fixture" >/tmp/planr-evidence-negative.out 2>&1; then
    cat /tmp/planr-evidence-negative.out
    echo "unexpected valid negative fixture: $fixture" >&2
    exit 1
  fi
  echo "$fixture rejected as expected"
done
```

Later implementation slices must use these fixtures as no-drift inputs for production canonicalization, schema validation, and coverage behavior.
