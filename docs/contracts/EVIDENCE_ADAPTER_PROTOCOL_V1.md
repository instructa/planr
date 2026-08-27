# Evidence adapter protocol v1

Status: frozen for the `planr.evidence.adapter-request.v1` request and the
`planr.structured_observation_results.v2` result.

This reference defines the process protocol between Planr Core and a registered
Evidence adapter. It extends `EVIDENCE_CONTRACT_V1.md`. It does not change the
Evidence domain schema.

## Trust ownership

Planr Core creates the adapter request, starts the registered process, and
validates the result. The adapter reports observations. It cannot assign
provenance, decide coverage, or close work.

Planr treats adapter output as untrusted input until all execution and binding
checks pass. A valid JSON result without a Planr-observed process execution
cannot create trusted Evidence.

## Process transport

Planr starts the adapter with bounded time, stdout, and stderr limits. Planr
writes one JSON request to stdin and then closes stdin. The adapter writes one
JSON result to stdout.

An availability probe starts the same executable with closed, empty stdin. The
adapter can use that invocation to check runtime availability. A probe result
cannot satisfy an Evidence requirement.

Planr reserves the complete `PLANR_` environment variable namespace. An adapter
request does not use environment variables for target, environment, or contract
bindings.

A process capability can declare `permissions.environment` as
`read_env:NAME[,NAME]`. This repository-authored declaration is necessary but
not sufficient. The host or user must independently include every requested
name in `PLANR_ADAPTER_ENV_ALLOWLIST`; repository content cannot change that
authorization. A manifest that reads host variables must also declare
`permissions.secrets = host_allowlist` to acknowledge that values remain
host-authorized even when a variable name appears non-secret.

Planr captures only the intersection of the manifest request and the host
authorization. The capability instance and sealed run index bind the variable
names, the host's public/non-public classification, and a digest of the present
values. Planr does not persist the values in that binding. At execution, Planr
clears the process environment, adds `PATH`, and forwards the captured values. A
changed value, classification, or host authorization invalidates the sealed run
before the adapter starts. Requested names in the complete `PLANR_` namespace
are invalid because only Planr Core can own that namespace.

Forwarded values are non-public by default. The host may independently list
known non-secret configuration names in `PLANR_ADAPTER_PUBLIC_ENV_ALLOWLIST`.
This list does not authorize access; it must remain a subset of the names that
the manifest requests and `PLANR_ADAPTER_ENV_ALLOWLIST` authorizes. Before
result validation, Planr rejects an adapter result that exposes an exact
non-public value, records `verifier_failed`, and redacts that value from durable
stdout and stderr excerpts while retaining the digests of the observed output.
Forwarded adapter values are never passed to a supervised target process.

This allowlist configures the adapter transport. It does not replace the
`target`, `environment`, or `execution_contract_digest` fields in the sealed
request.

## `planr.evidence.adapter-request.v1`

The request has these fields:

- `schema_version`: `planr.evidence.adapter-request.v1`.
- `request_id`: A new Planr-assigned ID for this process execution.
- `request_digest`: The SHA-256 canonical JSON digest of the request without
  `request_digest`.
- `obligation_id` and `criterion_id`: The exact Evidence obligation identity.
- `requirements`: The selected `ObservationRequirement` objects. The array
  includes each subject, expected value, target, payload schema, and optional
  execution method.
- `target` and `environment`: The runtime bindings selected by Planr.
- `fixture_disclosure`: The fixture and mock disclosure admitted by policy.
- `assurance_policy`: The policy that controls required capture strength.
- `result_contract`: The registered outer result schema binding.
- `execution_contract_digest`: The digest of the registered process contract.
- `execution_binding`: The sealed run-index subset and its requirement IDs.
- `retry`: The exact attempt number, maximum attempts, and predecessor IDs.

The request contains no provider-specific fields. A Browser Harness, native
browser, Playwright, mobile, desktop, game-engine, API, or CLI adapter receives
the same request shape.

## `planr.structured_observation_results.v2`

A structured adapter result has these required fields:

- `schema_version`: `planr.structured_observation_results.v2`.
- `request_id` and `request_digest`: Exact copies from the current request.
- `target`, `observed_target`, and `environment`: The declared and observed
  runtime identity.
- `execution_contract_digest`: An exact copy from the current request.
- `fixture_disclosure`: The actual fixture and mock use.
- `observations`: One result for every selected requirement and no other
  results.

Each observation contains only `requirement_id`, `type`, and `actual`. The
`actual` object names its payload schema. Planr validates the object against the
registered JSON Schema and evaluates the expected predicate.

If a requirement selects an agent skill as its execution method, the result
also includes the structured `agent_skill` invocation record required by the
Evidence domain contract. The supervised adapter creates this record. Agent or
user JSON cannot submit it through another trusted path.

Adapters can include diagnostic top-level fields. Diagnostics do not affect
provenance or coverage.

## Failure behavior

Planr rejects the result if the process fails or if any required binding does
not match. This includes a stale request ID, a stale request digest, a wrong
target, a wrong environment, a wrong execution contract, missing observations,
extra observations, or a schema mismatch.

An `actual` object that is valid for its registered payload schema but does not
satisfy the requirement's expected predicate is a product observation, not a
protocol rejection. Planr preserves that validated `actual`, assigns `failed`
only to that observation, records `product_failed` for its coverage gap, and
creates a trusted non-passing receipt. Other observations from the same exact
execution retain their own validated outcomes. The containing attempt is
failed when any selected observation fails.

Planr records failed attempts. A runtime target binding failure is
`target_mismatch`; malformed or schema-invalid observation data is
`schema_mismatch`. Neither code may replace `product_failed`, and one failed
observation may not contaminate sibling observations with a receipt-wide gap.
No non-passing result can create a trusted passing receipt or satisfy complete
coverage.

## Version transition

`planr.structured_observation_results.v2` replaces
`planr.structured_observation_results.v1`. Planr does not keep a second trusted
compatibility path. Adapter manifests and policy registrations must use the v2
schema reference and artifact name.
