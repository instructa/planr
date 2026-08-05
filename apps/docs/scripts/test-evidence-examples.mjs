import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const fixturePath = path.join(repositoryRoot, 'tests/fixtures/evidence/docs/v1/examples.generated.json');
const hostMatrixPath = path.join(repositoryRoot, 'tests/fixtures/evidence/host-capabilities/v1/expected/host-surface-matrix.json');

function digest(bytes) {
  return `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
}

const fixtureText = await readFile(fixturePath, 'utf8');
assert.equal(
  fixtureText.includes(repositoryRoot),
  false,
  'generated evidence docs examples must not leak the repository root',
);
const fixture = JSON.parse(fixtureText);
assert.equal(fixture.schema_version, 'planr.evidence_docs_examples.v1');
assert.equal(fixture.host_matrix_digest, digest(await readFile(hostMatrixPath)));
assert.equal(fixture.cases.length, 7);
assert.equal(fixtureText.includes('/planr-docs-'), false, 'generated commands must redact disposable workspaces');

const byId = new Map(fixture.cases.map((entry) => [entry.id, entry]));
for (const id of [
  'api-only-success',
  'repository-custom-extension',
  'full-stack-composition',
  'forged-claim-rejection',
  'stale-evidence',
  'missing-capability',
  'curl-http-not-browser',
]) {
  assert.ok(byId.has(id), `missing generated docs example ${id}`);
  assert.ok(byId.get(id).proven_scope.length > 0, `${id} must state proven scope`);
  assert.ok(byId.get(id).not_proven_scope.length > 0, `${id} must state not-proven scope`);
}

assert.equal(byId.get('api-only-success').exit_code, 0);
assert.equal(byId.get('api-only-success').output.run.object.verdict, 'passed');
assert.equal(byId.get('api-only-success').output.run.object.schema_version, 'planr.evidence.run-index.result.v1');
const apiReceipt = byId.get('api-only-success').output.run.object.results[0].receipt;
assert.equal(apiReceipt.receipt_status, 'trusted');
assert.equal(byId.get('api-only-success').output.coverage.object.verdict, 'satisfied');
assert.equal(
  apiReceipt.vantage_point.identity,
  'verifier-http-curl-v1',
);

const custom = byId.get('repository-custom-extension');
assert.equal(custom.exit_code, 0);
assert.equal(custom.output.run.object.verdict, 'passed');
assert.equal(custom.output.coverage.object.verdict, 'satisfied');
assert.ok(
  custom.output.run.object.results[0].receipt.observations.some(
    (observation) => observation.type === 'com.example.queue.depth.v2' && observation.outcome === 'passed',
  ),
  'repository custom example must execute and cover its namespaced observation',
);

const fullStack = byId.get('full-stack-composition');
assert.equal(fullStack.exit_code, 0);
assert.equal(fullStack.output.before.object.verdict, 'unsatisfied');
assert.equal(fullStack.output.after.object.verdict, 'satisfied');
const browserReceipt = fullStack.output.runs[1].object.receipt;
assert.equal(browserReceipt.vantage_point.identity, 'verifier-browser-cdp-v1');
assert.equal(browserReceipt.observations.length, 6);
assert.ok(browserReceipt.observations.every((observation) => observation.outcome === 'passed'));
assert.ok(
  browserReceipt.observations.every(
    (observation) => observation.actual.runtime_identity.kind === 'chrome-cdp',
  ),
  'full-stack fixture must contain actual Chrome/CDP runtime identity on every browser observation',
);

const forged = byId.get('forged-claim-rejection');
assert.notEqual(forged.exit_code, 0);
assert.match(
  `${forged.stderr} ${forged.output.error.message}`,
  /cannot construct trusted receipt field: (attempt|receipt)/,
);

const stale = byId.get('stale-evidence');
assert.equal(stale.exit_code, 2);
assert.equal(stale.output.run.object.verdict, 'passed');
assert.equal(stale.output.coverage.object.verdict, 'stale');
assert.equal(
  stale.output.coverage.object.coverage.observation_coverage[0].gap_reason,
  'stale_policy',
);

const missing = byId.get('missing-capability');
assert.equal(missing.exit_code, 1);
assert.match(missing.output.run.error.message, /capability instance is not available/);
assert.ok(
  missing.output.capabilities.object.instances.some(
    (instance) =>
      instance.manifest_id === 'verifier-unavailable-queue-v1' &&
      instance.availability_status === 'unavailable',
  ),
  'missing capability example must preserve the unavailable capability classification',
);
assert.equal(missing.output.coverage.object.verdict, 'unsatisfied');

const curlNegative = byId.get('curl-http-not-browser');
assert.equal(curlNegative.exit_code, 1);
assert.equal(curlNegative.output.http_run.object.verdict, 'passed');
assert.match(
  curlNegative.output.browser_rejection.error.message,
  /observation type is not supported by capability instance/,
);
assert.ok(
  curlNegative.not_proven_scope.some((scope) => scope.includes('browser')),
  'curl/browser negative example must explicitly withhold browser proof',
);
assert.equal(
  curlNegative.output.host_matrix_digest,
  fixture.host_matrix_digest,
);

console.log(`evidence_docs_examples_fixture_test=passed cases=${fixture.cases.length}`);
