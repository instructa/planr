#!/usr/bin/env node
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { lstatSync, realpathSync } from 'node:fs'
import { mkdir, readFile, realpath, writeFile } from 'node:fs/promises'
import { dirname, isAbsolute, relative, resolve } from 'node:path'

const root = process.cwd()
const planId = process.argv[2]
const planrBinary = process.argv[3]
const planrBinaryDigest = process.argv[4]
if (!planId || !planrBinary || !planrBinaryDigest) {
  throw new Error('usage: ac014-configure-sparziele-evidence.mjs <build-plan-id> <absolute-planr-binary> <sha256-digest>')
}

const canonical = (value) => {
  if (value === null || typeof value !== 'object') return JSON.stringify(value)
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`
  return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(',')}}`
}
const sha = (value) => `sha256:${createHash('sha256').update(value).digest('hex')}`
const shaJson = (value) => sha(canonical(value))
const requireExactPlanrBinary = async () => {
  if (!isAbsolute(planrBinary)) throw new Error('Planr binary path must be absolute; PATH lookup is forbidden')
  if (!/^sha256:[0-9a-f]{64}$/.test(planrBinaryDigest)) throw new Error('Planr binary digest must be an exact sha256 digest')
  const metadata = lstatSync(planrBinary)
  if (!metadata.isFile() || metadata.isSymbolicLink()) throw new Error('Planr binary must be a regular non-symlink file')
  if ((metadata.mode & 0o222) !== 0) throw new Error('Planr binary must be immutable (no write bits)')
  const canonicalPath = realpathSync(planrBinary)
  if (canonicalPath !== planrBinary) throw new Error('Planr binary path must already be canonical')
  const observedDigest = sha(await readFile(canonicalPath))
  if (observedDigest !== planrBinaryDigest) throw new Error(`Planr binary digest mismatch: expected ${planrBinaryDigest}, observed ${observedDigest}`)
  return canonicalPath
}
const writeJson = async (path, value) => {
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`)
}

const schemaPath = '.planr/evidence/schemas/project.sparziele.verifier-result.v1.schema.json'
const schema = JSON.parse(await readFile(schemaPath, 'utf8'))
const schemaDigest = shaJson(schema)
const schemaRef = schema.schema_ref
const envelopeSchema = {
  schema_version: 'evidence.contract.v1',
  type: 'planr.structured_observation_results',
  schema_ref: 'schema://planr.structured_observation_results.v1',
  json_schema: { type: 'object' },
}
const envelopePayload = {
  type: envelopeSchema.type,
  schema_ref: envelopeSchema.schema_ref,
  schema_digest: shaJson(envelopeSchema),
}

async function fileIdentity(argument, index) {
  const cwd = await realpath(root)
  const path = await realpath(resolve(root, argument))
  return {
    argument_index: index,
    argument,
    resolved_relative_to: 'command_cwd',
    cwd,
    path,
    cwd_relative_path: relative(cwd, path),
    path_digest: sha(path),
    content_digest: sha(await readFile(path)),
  }
}

async function capability({ id, script, type, surface, runtimeTarget, interactions, timeout, permissions, blindSpots }) {
  const payload = { type, schema_ref: schemaRef, schema_digest: schemaDigest }
  const execution = {
    kind: 'process', executable: 'node', args: [script], working_directory: '.',
    timeout_ms: timeout, stdout_limit_bytes: 131072, stderr_limit_bytes: 131072,
    payload_schema: envelopePayload,
  }
  const files = [await fileIdentity(script, 0)]
  const adapterDigest = shaJson({
    schema_version: 'planr.process_adapter.binding.v1',
    execution_contract: execution,
    file_arguments: files,
  })
  const manifest = {
    id, schema_version: 'evidence.contract.v1', version: `1.0.3-${adapterDigest.slice('sha256:'.length, 'sha256:'.length + 12)}`,
    adapter_kind: 'process', adapter_digest: adapterDigest,
    supported_surfaces: surface, supported_observations: [payload],
    supported_interactions: interactions,
    supported_artifacts: ['stdout', 'planr.structured_observation_results.v1'],
    runtime_targets: [runtimeTarget], provenance_path: 'planr_observed_execution',
    permissions, costs: {}, determinism: 'deterministic against the bound source and local target',
    repeatability: 'replayable through the repository-owned adapter',
    independence: 'Planr executes and binds the repository-owned verifier independently of agent-authored claims',
    blind_spots: blindSpots,
    availability_probe: { kind: 'process', execution },
  }
  return { id, type, script, payload, execution, runtimeTarget, manifest, manifestDigest: shaJson(manifest) }
}

const build = await capability({
  id: 'verifier-sparziele-build-v1',
  script: '.planr/evidence/adapters/sparziele-build.mjs',
  type: 'project.sparziele.build.completed',
  surface: ['local-process'], runtimeTarget: { kind: 'process', id: 'sparziele-production-build' },
  interactions: ['build'], timeout: 120000,
  permissions: { network: 'package-manager-cache-only', filesystem: 'read_write_workspace' },
  blindSpots: ['build success does not prove rendered browser behavior'],
})
const browser = await capability({
  id: 'verifier-sparziele-browser-cdp-v1',
  script: '.planr/evidence/adapters/sparziele-browser.mjs',
  type: 'project.sparziele.browser.acceptance',
  surface: ['local-process', 'chrome-cdp'], runtimeTarget: { kind: 'browser', id: 'chrome-cdp' },
  interactions: ['render', 'form_input', 'click', 'reload', 'storage_clear', 'console_observe', 'network_observe', 'viewport'],
  timeout: 180000,
  permissions: { network: 'loopback', filesystem: 'read_workspace', browser: 'chrome-cdp' },
  blindSpots: ['does not prove behavior outside the canonical German EUR acceptance scenarios'],
})
const capabilities = [build, browser]
for (const entry of capabilities) {
  await writeJson(`.planr/evidence/adapters/${entry.id}.manifest.json`, entry.manifest)
}

const policy = {
  id: 'epolicy-sparziele-benchmark-v1', schema_version: 'evidence.contract.v1',
  policy_digest: 'sha256:0000000000000000000000000000000000000000000000000000000000000000',
  defaults: { preset_id: 'preset-sparziele-browser', binding: true, assurance_level: 'standard' },
  named_presets: capabilities.map((entry) => ({
    id: `preset-${entry === browser ? 'sparziele-browser' : 'sparziele-build'}`,
    schema_version: 'evidence.contract.v1', namespace: 'project.sparziele',
    observations: [{
      id: `obs-preset-${entry === browser ? 'browser' : 'build'}`, type: entry.type,
      subject: entry === browser ? 'canonical Sparziele acceptance criterion' : 'production build',
      expected: { passed: true },
      target: entry === browser
        ? { kind: 'browser', uri: 'http://127.0.0.1:3000/#AC-012' }
        : { kind: 'process', uri: 'local://pnpm-build' },
    }],
  })),
  observation_schema_registrations: capabilities.map((entry) => ({
    type: entry.type, schema_ref: schemaRef, schema_digest: schemaDigest, owning_namespace: 'project.sparziele',
  })),
  adapter_registrations: capabilities.map((entry) => ({
    manifest_id: entry.id,
    manifest_path: `.planr/evidence/adapters/${entry.id}.manifest.json`,
    manifest_digest: entry.manifestDigest,
    observation_types: [entry.type], payload_schemas: [entry.payload],
    provenance_path: 'planr_observed_execution', execution_contract: entry.execution,
  })),
  extension_namespaces: ['project.sparziele'],
  trust_policy: { accepted_provenance: ['planr_observed_execution'], min_receipt_status: 'trusted', allow_user_attestation: false },
  freshness_policy: { max_age_seconds: 604800, invalidate_on: ['source_change', 'target_change', 'policy_change', 'adapter_schema_change', 'configuration_change'] },
  fixture_policy: { fixtures_allowed: false, mocks_allowed: false, disclosure_required: true },
  completion_policy: { require_satisfied_or_waived: true, allow_inconclusive_completion: false, require_review_evidence: true },
  layering_policy: {
    mode: 'monotonic_strengthening', weakening_requires_waiver: true,
    layers: [{
      scope: { kind: 'plan', id: planId },
      policy_digest: 'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    }],
  },
}
delete policy.policy_digest
policy.policy_digest = shaJson(policy)
await writeJson('.planr/evidence.yaml', policy)

// Revalidate immediately before the only Planr subprocess. Never resolve through PATH.
const exactPlanrBinary = await requireExactPlanrBinary()
const run = spawnSync(exactPlanrBinary, ['evidence', 'capability', 'list', '--json'], { cwd: root, encoding: 'utf8' })
if (run.status !== 0) throw new Error(`capability probe failed\n${run.stdout}\n${run.stderr}`)
const capabilityResult = JSON.parse(run.stdout).object
const instanceFor = (manifestId) => {
  const probe = capabilityResult.registry.probes.find((entry) => entry.manifest_id === manifestId)
  const instance = capabilityResult.instances.find((entry) => entry.id === probe?.instance_id)
    ?? capabilityResult.instances.findLast((entry) =>
      entry.capability?.manifest_id === manifestId
      && entry.capability?.availability?.status === 'available')
  if (!instance || instance.capability?.availability?.status !== 'available') {
    throw new Error(`capability ${manifestId} is not available: ${JSON.stringify(probe)}`)
  }
  return instance
}
const buildInstance = instanceFor(build.id)
const browserInstance = instanceFor(browser.id)

const browserObligations = Array.from({ length: 12 }, (_, index) => {
  const criterion = `AC-${String(index + 1).padStart(3, '0')}`
  const target = { kind: 'browser', uri: `http://127.0.0.1:3000/#${criterion}` }
  return {
    id: `pob-sparziele-${criterion.toLowerCase()}-browser`, schema_version: 'evidence.contract.v1',
    criterion_id: criterion, plan_id: planId,
    title: `${criterion} canonical browser acceptance`, binding: true,
    observations: [{
      id: `obs-${criterion}-browser`, type: browser.type,
      subject: `${criterion} required browser observations from QA_ACCEPTANCE_TESTS.md`,
      expected: { passed: true, criterion }, target,
      payload_schema: { schema_ref: schemaRef },
      freshness_policy: { source: 'source_tree', max_age_seconds: 604800 },
      assurance_policy: { min_provenance: 'planr_observed_execution', independent_methods: 1 },
    }],
    fixture_policy: { fixtures_allowed: false, mocks_allowed: false, disclosure_required: true },
    freshness_policy: { invalidate_on: ['source_change', 'target_change', 'policy_change', 'adapter_schema_change', 'configuration_change'] },
    assurance_policy: { min_receipt_status: 'trusted', allow_user_attestation: false },
  }
})
const buildTarget = { kind: 'process', uri: 'local://pnpm-build' }
const buildObligation = {
  id: 'pob-sparziele-ac-012-build', schema_version: 'evidence.contract.v1',
  criterion_id: 'BUILD-001', plan_id: planId, title: 'Required production build succeeds', binding: true,
  observations: [{
    id: 'obs-AC-012-build', type: build.type, subject: 'production build completes successfully',
    expected: { passed: true, command: 'pnpm build', exit_code: 0 }, target: buildTarget,
    payload_schema: { schema_ref: schemaRef },
    freshness_policy: { source: 'source_tree', max_age_seconds: 604800 },
    assurance_policy: { min_provenance: 'planr_observed_execution', independent_methods: 1 },
  }],
  fixture_policy: { fixtures_allowed: false, mocks_allowed: false, disclosure_required: true },
  freshness_policy: { invalidate_on: ['source_change', 'target_change', 'policy_change', 'adapter_schema_change', 'configuration_change'] },
  assurance_policy: { min_receipt_status: 'trusted', allow_user_attestation: false },
}
const obligations = [...browserObligations, buildObligation]
await writeJson('.planr/evidence/obligations/sparziele.migration.json', {
  schema_version: 'planr.evidence.migration.v1', plan_id: planId, obligations,
})
await writeJson('.planr/evidence/obligations/sparziele.run-index.json', {
  schema_version: 'project.sparziele.evidence_run_index.v1', plan_id: planId,
  runs: obligations.map((obligation) => ({
    obligation_id: obligation.id,
    manifest_id: obligation.id.endsWith('-build') ? build.id : browser.id,
    target: obligation.observations[0].target,
  })),
})

process.stdout.write(JSON.stringify({
  plan_id: planId, policy_digest: policy.policy_digest,
  capability_manifests: capabilities.map((entry) => entry.id),
  obligations: obligations.length,
}, null, 2) + '\n')
