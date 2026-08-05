import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));

const fixturePath = path.join(repositoryRoot, 'tests/fixtures/evidence/docs/v1/examples.generated.json');
const evidenceSchemaPath = path.join(
  repositoryRoot,
  'docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json',
);
const hostMatrixPath = path.join(
  repositoryRoot,
  'tests/fixtures/evidence/host-capabilities/v1/expected/host-surface-matrix.json',
);
const scenarioHelperPath = path.join(docsRoot, 'scripts/create-evidence-scenario-files.mjs');

function digest(bytes) {
  return `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
}

const fixture = JSON.parse(await readFile(fixturePath, 'utf8'));
const scenarioPage = await readFile(
  path.join(docsRoot, 'content/docs/guides/evidence-scenarios.mdx'),
  'utf8',
);
const conceptPage = await readFile(
  path.join(docsRoot, 'content/docs/concepts/evidence-trust-model.mdx'),
  'utf8',
);
const referencePage = await readFile(path.join(docsRoot, 'content/docs/reference/evidence.mdx'), 'utf8');
const scenarioHelper = await readFile(scenarioHelperPath, 'utf8');
const scenarioBuilder = await readFile(path.join(docsRoot, 'scripts/evidence-fixture-builder.mjs'), 'utf8');
const exampleGenerator = await readFile(path.join(docsRoot, 'scripts/generate-evidence-examples.mjs'), 'utf8');
const allEvidenceDocs = `${scenarioPage}\n${conceptPage}\n${referencePage}`;

assert.equal(fixture.schema_version, 'planr.evidence_docs_examples.v1');
assert.equal(fixture.evidence_schema_digest, digest(await readFile(evidenceSchemaPath)));
assert.equal(fixture.host_matrix_digest, digest(await readFile(hostMatrixPath)));

for (const token of [
  fixture.schema_version,
  fixture.evidence_schema_digest,
  fixture.host_matrix_digest,
  fixture.candidate_binary,
  'docs/contracts/EVIDENCE_CONTRACT_V1.md',
  '/docs/reference/cli',
  '/docs/reference/mcp',
  '/docs/reference/http-api',
  '/docs/reference/cli-generated',
  '/docs/reference/mcp-schemas-generated',
  'apps/docs/scripts/create-evidence-scenario-files.mjs',
]) {
  assert.ok(allEvidenceDocs.includes(token), `Evidence docs must include ${token}`);
}

for (const token of [
  '--scenario api-only',
  '--scenario repository-custom-extension',
]) {
  assert.ok(scenarioPage.includes(token), `scenario docs must include reproducible reference ${token}`);
  assert.ok(scenarioHelper.includes(token), `scenario helper must document ${token}`);
}

for (const token of [
  '.planr/evidence/schemas/com.example.http.status.schema.json',
  '.planr/evidence/adapters/verifier-http-curl-v1.manifest.json',
  'pob-docs-api-http.obligation.json',
  '.planr/evidence/schemas/com.example.queue.depth.v2.schema.json',
  '.planr/evidence/adapters/verifier-queue-depth-v2.manifest.json',
  'pob-docs-queue-depth.obligation.json',
]) {
  assert.ok(scenarioPage.includes(token), `scenario docs must include reproducible reference ${token}`);
  assert.ok(scenarioBuilder.includes(token), `shared scenario builder must own documented file ${token}`);
}

const fullStackCase = fixture.cases.find((entry) => entry.id === 'full-stack-composition');
assert.ok(fullStackCase, 'fixture must include full-stack-composition');
assert.ok(
  JSON.stringify(fullStackCase.output).includes('pob-browser-cdp') ||
    JSON.stringify(fullStackCase.output).includes('obs-pob-browser-cdp-visible'),
  'full-stack fixture must include browser CDP obligation output',
);
assert.ok(
  scenarioPage.includes('<exact-readiness.run_index.repository_path>'),
  'scenario docs must execute only the exact repository_path returned by leased readiness',
);

for (const staleToken of [
  '$EDITOR',
  'pob-docs-full-browser.run.json',
  '.planr/evidence/runs/<sealed-digest>.json',
  'run-feature.json',
  'readiness-run-index-path',
]) {
  assert.ok(!scenarioPage.includes(staleToken), `scenario docs must not include stale placeholder ${staleToken}`);
}

for (const source of [scenarioHelper, exampleGenerator]) {
  assert.ok(
    source.includes("from './evidence-fixture-builder.mjs'"),
    'docs Evidence scenario entry points must import the shared fixture builder',
  );
  for (const duplicate of [
    'function adapterSpec',
    'function httpSpec',
    'function queueSpec',
    'function writeEvidencePolicy',
    'function obligation',
    'function writeRepositoryCustomExtensionFixture',
  ]) {
    assert.ok(!source.includes(duplicate), `docs Evidence entry points must not redefine ${duplicate}`);
  }
}

for (const exported of ['SCENARIOS', 'httpSpec', 'queueSpec', 'writeEvidencePolicy', 'obligation']) {
  assert.ok(scenarioBuilder.includes(`export `) && scenarioBuilder.includes(exported), `shared builder must export ${exported}`);
}

for (const entry of fixture.cases) {
  assert.ok(scenarioPage.includes(`\`${entry.id}\``), `scenario docs must include fixture id ${entry.id}`);
  assert.ok(scenarioPage.includes(entry.title), `scenario docs must include fixture title ${entry.title}`);
  for (const scope of entry.proven_scope) {
    assert.ok(scenarioPage.includes(scope), `scenario docs must include proven scope for ${entry.id}: ${scope}`);
  }
  for (const scope of entry.not_proven_scope) {
    assert.ok(scenarioPage.includes(scope), `scenario docs must include not-proven scope for ${entry.id}: ${scope}`);
  }
}

for (const rejected of [
  'attempt',
  'receipt',
  'receipt_json',
  'trusted_binding_json',
  'trusted_receipt',
  'receipt_status',
  'provenance',
]) {
  assert.ok(referencePage.includes(`\`${rejected}\``), `Evidence reference must include rejected field ${rejected}`);
}

for (const classification of [
  'missing_capability',
  'permission_denied',
  'sandbox_blocked',
  'environment_unavailable',
  'external_dependency_unavailable',
  'product_failed',
  'verifier_failed',
  'stale_policy',
  'stale_adapter_schema',
  'unsupported_runtime_target',
]) {
  assert.ok(allEvidenceDocs.includes(classification), `Evidence docs must include classification ${classification}`);
}

console.log(
  JSON.stringify(
    {
      ok: true,
      cases: fixture.cases.length,
      evidence_schema_digest: fixture.evidence_schema_digest,
      host_matrix_digest: fixture.host_matrix_digest,
    },
    null,
    2,
  ),
);
