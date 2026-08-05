import { mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';

export const SCENARIO_IDS = ['api-only', 'repository-custom-extension'];

export const SCENARIOS = {
  'api-only': {
    policyId: 'epolicy-docs-api-v1',
    planTitle: 'Evidence docs API',
    obligationId: 'pob-docs-api-http',
    expected: { status: 'ok' },
    files: [
      '.planr/evidence/schemas/com.example.http.status.schema.json',
      '.planr/evidence/adapters/verifier-http-curl-v1.manifest.json',
      '.planr/evidence.yaml',
      'pob-docs-api-http.obligation.json',
    ],
  },
  'repository-custom-extension': {
    policyId: 'epolicy-docs-custom-v1',
    planTitle: 'Evidence docs custom extension',
    obligationId: 'pob-docs-queue-depth',
    expected: { status: 'drained' },
    files: [
      '.planr/evidence/schemas/com.example.queue.depth.v2.schema.json',
      '.planr/evidence/adapters/verifier-queue-depth-v2.manifest.json',
      '.planr/evidence.yaml',
      'pob-docs-queue-depth.obligation.json',
    ],
  },
};

export function canonicalJson(value) {
  if (value === null || typeof value !== 'object') return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map((entry) => canonicalJson(entry)).join(',')}]`;
  return `{${Object.keys(value)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
    .join(',')}}`;
}

export function sha256(bytes) {
  return `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
}

export function sha256Json(value) {
  return sha256(canonicalJson(value));
}

export function sha256JsonWithoutField(value, field) {
  const copy = structuredClone(value);
  delete copy[field];
  return sha256Json(copy);
}

export function processAdapterDigest(execution, fileArguments = []) {
  return sha256Json({
    schema_version: 'planr.process_adapter.binding.v1',
    execution_contract: execution,
    file_arguments: fileArguments,
  });
}

export async function writeJson(file, value) {
  await mkdir(path.dirname(file), { recursive: true });
  await writeFile(file, `${JSON.stringify(value, null, 2)}\n`);
}

export function adapterSpec({
  id,
  version = '1.0.0',
  adapterKind = 'process',
  observationType,
  schemaRef,
  jsonSchema,
  executable,
  args,
  runtimeKind,
  runtimeId,
  target,
  provenance = 'planr_observed_execution',
  network = 'none',
  independence,
  blindSpot,
}) {
  const schema = {
    schema_version: 'evidence.contract.v1',
    type: observationType,
    schema_ref: schemaRef,
    json_schema: jsonSchema,
  };
  const payloadSchema = {
    type: observationType,
    schema_ref: schemaRef,
    schema_digest: sha256Json(schema),
  };
  const execution = {
    kind: 'process',
    executable,
    args,
    working_directory: '.',
    timeout_ms: 5000,
    stdout_limit_bytes: 4096,
    stderr_limit_bytes: 4096,
    payload_schema: payloadSchema,
  };
  const manifest = {
    id,
    schema_version: 'evidence.contract.v1',
    version,
    adapter_kind: adapterKind,
    adapter_digest: processAdapterDigest(execution),
    supported_surfaces: ['local-process'],
    supported_observations: [payloadSchema],
    supported_interactions: ['process'],
    supported_artifacts: ['stdout', 'planr.generic_adapter_predicate.v1'],
    runtime_targets: [{ kind: runtimeKind, id: runtimeId }],
    provenance_path: provenance,
    permissions: { network, filesystem: 'read_workspace' },
    costs: {},
    determinism: 'deterministic',
    repeatability: 'repeatable',
    independence,
    blind_spots: [blindSpot],
    availability_probe: { kind: 'process', execution },
  };
  return {
    id,
    schema,
    payloadSchema,
    execution,
    manifest,
    manifestDigest: sha256Json(manifest),
    observationType,
    runtimeTarget: { kind: runtimeKind, id: runtimeId },
    target,
  };
}

export function httpSpec(apiUrl) {
  return adapterSpec({
    id: 'verifier-http-curl-v1',
    observationType: 'com.example.http.status',
    schemaRef: 'schema://com.example.http.status',
    jsonSchema: {
      type: 'object',
      required: ['status'],
      additionalProperties: false,
      properties: { status: { const: 'ok' } },
    },
    executable: 'curl',
    args: ['-fsS', apiUrl],
    runtimeKind: 'process',
    runtimeId: 'runtime-http-curl',
    target: { kind: 'process', uri: 'local://health' },
    network: 'loopback',
    independence: 'repository-defined curl HTTP adapter',
    blindSpot: 'fixture HTTP server returns static health body',
  });
}

export function queueSpec() {
  return adapterSpec({
    id: 'verifier-queue-depth-v2',
    version: '2.1.0',
    adapterKind: 'artifact_import',
    observationType: 'com.example.queue.depth.v2',
    schemaRef: 'schema://com.example.queue.depth.v2',
    jsonSchema: {
      type: 'object',
      required: ['status'],
      additionalProperties: false,
      properties: { status: { const: 'drained' } },
    },
    executable: 'sh',
    args: ['-c', 'printf \'{"status":"drained"}\''],
    runtimeKind: 'process',
    runtimeId: 'runtime-queue-v2',
    target: { kind: 'process', uri: 'local://queue-depth' },
    independence: 'repository-defined queue extension adapter',
    blindSpot: 'fixture adapter checks a deterministic disposable queue state',
  });
}

export function scenarioSpec(scenario, { apiUrl } = {}) {
  if (scenario === 'api-only') return httpSpec(apiUrl ?? 'http://127.0.0.1:3000/health');
  if (scenario === 'repository-custom-extension') return queueSpec();
  throw new Error(`unsupported evidence docs scenario: ${scenario}`);
}

export async function writeEvidencePolicy(workspace, specs, { policyId = 'epolicy-docs-v1', unavailable = false } = {}) {
  const evidenceDir = path.join(workspace, '.planr', 'evidence');
  for (const spec of specs) {
    for (const schema of spec.schemas ?? [spec.schema]) {
      await writeJson(path.join(evidenceDir, 'schemas', `${schema.type}.schema.json`), schema);
    }
    await writeJson(path.join(evidenceDir, 'adapters', `${spec.id}.manifest.json`), spec.manifest);
  }

  const policy = buildEvidencePolicy(specs, { policyId, unavailable });
  await writeFile(path.join(workspace, '.planr', 'evidence.yaml'), `${JSON.stringify(policy, null, 2)}\n`);
  return policy.policy_digest;
}

export function buildEvidencePolicy(specs, { policyId = 'epolicy-docs-v1', unavailable = false } = {}) {
  const policy = {
    id: policyId,
    schema_version: 'evidence.contract.v1',
    policy_digest: 'sha256:0000000000000000000000000000000000000000000000000000000000000000',
    defaults: {
      preset_id: `preset-${specs[0].id}`,
      binding: true,
      assurance_level: 'standard',
    },
    named_presets: specs.map((spec) => {
      const namespace = spec.observationType.split('.').slice(0, -1).join('.');
      return {
        id: `preset-${spec.id}`,
        schema_version: 'evidence.contract.v1',
        namespace,
        observations: [
          {
            id: `obs-${spec.id}`,
            type: spec.observationType,
            subject: `docs fixture ${spec.observationType}`,
            expected: spec.schema.json_schema.properties?.status?.const
              ? { status: spec.schema.json_schema.properties.status.const }
              : { visible: true },
            target: spec.target,
          },
        ],
      };
    }),
    observation_schema_registrations: specs.flatMap((spec) =>
      (spec.payloadSchemas ?? [spec.payloadSchema]).map((payloadSchema) => ({
        type: payloadSchema.type,
        schema_ref: payloadSchema.schema_ref,
        schema_digest: payloadSchema.schema_digest,
        owning_namespace: payloadSchema.type.split('.').slice(0, -1).join('.'),
      })),
    ),
    adapter_registrations: specs.map((spec) => ({
      manifest_id: spec.id,
      manifest_path: `.planr/evidence/adapters/${spec.id}.manifest.json`,
      manifest_digest: spec.manifestDigest,
      observation_types: spec.observationTypes ?? [spec.observationType],
      payload_schemas: spec.payloadSchemas ?? [spec.payloadSchema],
      provenance_path: spec.manifest.provenance_path,
      execution_contract: spec.execution,
    })),
    extension_namespaces: [
      ...new Set(
        specs.flatMap((spec) =>
          (spec.observationTypes ?? [spec.observationType]).map((type) =>
            type.split('.').slice(0, -1).join('.'),
          ),
        ),
      ),
    ],
    trust_policy: {
      accepted_provenance: ['planr_observed_execution'],
      min_receipt_status: 'trusted',
      allow_user_attestation: false,
    },
    freshness_policy: {
      max_age_seconds: unavailable ? 1 : 3600,
      invalidate_on: ['source_change', 'target_change', 'policy_change', 'adapter_schema_change'],
    },
    fixture_policy: {
      fixtures_allowed: specs.some((spec) => spec.fixtureAllowed),
      mocks_allowed: false,
      disclosure_required: true,
    },
    completion_policy: {
      require_satisfied_or_waived: true,
      allow_inconclusive_completion: false,
      require_review_evidence: true,
    },
    layering_policy: {
      mode: 'monotonic_strengthening',
      weakening_requires_waiver: true,
      layers: [
        {
          scope: { kind: 'plan', id: 'pln-docs-generated' },
          policy_digest: 'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
        },
      ],
    },
  };
  policy.policy_digest = sha256JsonWithoutField(policy, 'policy_digest');
  return policy;
}

export function obligation({
  id,
  planId,
  spec,
  expected,
  invalidateOn = ['policy_change', 'adapter_schema_change'],
}) {
  return {
    id,
    schema_version: 'evidence.contract.v1',
    criterion_id: `crit-${id}`,
    plan_id: planId,
    title: `Evidence obligation ${id}`,
    binding: true,
    observations: [
      {
        id: `obs-${id}`,
        type: spec.observationType,
        subject: `docs fixture ${spec.observationType}`,
        expected,
        target: spec.target,
        payload_schema: { schema_ref: spec.payloadSchema.schema_ref },
      },
    ],
    fixture_policy: { fixtures_allowed: false, mocks_allowed: false, disclosure_required: true },
    freshness_policy: { invalidate_on: invalidateOn },
    assurance_policy: {},
  };
}
