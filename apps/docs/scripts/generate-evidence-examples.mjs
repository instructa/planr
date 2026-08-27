import assert from 'node:assert/strict';
import { access, mkdir, mkdtemp, readFile, realpath as pathRealpath, rm, writeFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { spawn, spawnSync } from 'node:child_process';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  SCENARIOS,
  httpSpec,
  obligation,
  processAdapterDigest,
  queueSpec,
  sha256,
  sha256Json,
  sha256JsonWithoutField,
  writeEvidencePolicy,
  writeJson,
} from './evidence-fixture-builder.mjs';
import { resolvePlanrBinary } from '../../../scripts/resolve-planr-binary.mjs';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const fixtureRoot = path.join(repositoryRoot, 'tests', 'fixtures', 'evidence', 'docs', 'v1');
const outputPath = path.join(fixtureRoot, 'examples.generated.json');
const planrBin = resolvePlanrBinary(repositoryRoot);
const checkOnly = process.argv.includes('--check');
const liveMode = process.argv.includes('--live');

await access(planrBin, constants.X_OK);

async function fileDigest(relative) {
  return sha256(await readFile(path.join(repositoryRoot, relative)));
}

function displayCandidateBinary() {
  const relative = path.relative(repositoryRoot, planrBin);
  if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
    return `<cargo-target>/debug/${path.basename(planrBin)}`;
  }
  return relative.split(path.sep).join('/');
}

function run(workspace, args, input) {
  const result = spawnSync(planrBin, args, {
    cwd: workspace,
    encoding: 'utf8',
    input,
    env: {
      ...process.env,
      PLANR_WORKER_ID: 'docs-fixture-generator',
      PLANR_PROFILE: 'docs-fixture',
    },
  });
  return {
    command: ['planr', ...args],
    exit_code: result.status,
    stdout: result.stdout.trim() ? JSON.parse(result.stdout) : null,
    stderr: result.stderr.trim(),
  };
}

function requireSuccess(result) {
  assert.equal(
    result.exit_code,
    0,
    `${result.command.join(' ')}\nstdout=${JSON.stringify(result.stdout)}\nstderr=${result.stderr}`,
  );
  return result;
}

async function disposableWorkspace(name) {
  const root = await mkdtemp(path.join(tmpdir(), `planr-docs-${name}-`));
  requireSuccess(run(root, ['project', 'init', `Evidence docs ${name}`, '--json']));
  const gitEnv = {
    ...process.env,
    GIT_AUTHOR_DATE: '2026-07-29T00:00:00Z',
    GIT_COMMITTER_DATE: '2026-07-29T00:00:00Z',
  };
  for (const args of [
    ['init', '--quiet'],
    ['add', '.'],
    ['-c', 'user.name=Planr Docs', '-c', 'user.email=docs@planr.invalid', 'commit', '--quiet', '-m', 'fixture baseline'],
  ]) {
    const result = spawnSync('git', args, { cwd: root, encoding: 'utf8', env: gitEnv });
    assert.equal(result.status, 0, `git ${args.join(' ')}\n${result.stderr}`);
  }
  return root;
}

function redactWorkspace(value, workspace) {
  return JSON.parse(JSON.stringify(value).replaceAll(workspace, '<workspace>'));
}

function redactCommand(command, workspace) {
  return redactVolatileEvidence(redactWorkspace(command, workspace));
}

function redactVolatileEvidence(value) {
  if (Array.isArray(value)) {
    return value.map((entry) => redactVolatileEvidence(entry));
  }
  if (value && typeof value === 'object') {
    const result = {};
    for (const [key, entry] of Object.entries(value)) {
      if (key.endsWith('_at') && typeof entry === 'string') {
        result[key] = '<timestamp>';
      } else if (key.endsWith('_excerpt') && typeof entry === 'string') {
        result[key] = '<captured-output>';
      } else if (key === 'revision' && typeof entry === 'string' && /^[0-9a-f]{40}$/.test(entry)) {
        result[key] = '<source-revision>';
      } else if (key === 'receipt_digests' && Array.isArray(entry)) {
        result[key] = entry.map(() => '<sha256>');
      } else if (
        (key === 'digest' || key.endsWith('_digest') || key.endsWith('_key')) &&
        typeof entry === 'string' &&
        entry.startsWith('sha256:')
      ) {
        result[key] = '<sha256>';
      } else if (key === 'probe_execution_id') {
        result[key] = '<probe-execution-id>';
      } else if ((key === 'id' || key === 'instance_id') && typeof entry === 'string' && entry.startsWith('capinst-')) {
        result[key] = '<capability-instance-id>';
      } else if (
        ['product', 'protocol_version', 'user_agent', 'executable_path', 'debug_endpoint'].includes(key) &&
        value.kind === 'chrome-cdp'
      ) {
        result[key] = `<browser-${key.replaceAll('_', '-')}>`;
      } else {
        result[key] = redactVolatileEvidence(entry);
      }
    }
    return result;
  }
  if (typeof value === 'string') {
    if (/^\d{4,5}$/.test(value)) return '<port>';
    if (value.startsWith('receipt-')) return '<receipt-id>';
    if (value.startsWith('erec-')) return '<receipt-id>';
    if (value.startsWith('attempt-')) return '<attempt-id>';
    if (value.startsWith('eatt-')) return '<attempt-id>';
    if (value.startsWith('cverdict-')) return '<coverage-id>';
    if (value.startsWith('capinst-')) return '<capability-instance-id>';
    if (value.startsWith('pln-')) return '<plan-id>';
    if (value.startsWith('p-')) return '<project-id>';
    return value
      .replace(/http:\/\/127\.0\.0\.1:\d+/g, 'http://127.0.0.1:<port>')
      .replace(/\.planr\/evidence\/runs\/[0-9a-f]{64}\.json/g, '.planr/evidence/runs/<sealed-digest>.json');
  }
  return value;
}

async function freePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const { port } = server.address();
  await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
  return port;
}

async function findHeadlessBrowser() {
  const configured = process.env.PLANR_TEST_HEADLESS_BROWSER;
  if (!configured) {
    throw new Error('set PLANR_TEST_HEADLESS_BROWSER to a dedicated headless_shell executable');
  }
  const executable = await pathRealpath(configured);
  const allowed = new Set(['chrome-headless-shell', 'chromium-headless-shell', 'headless_shell']);
  if (executable.includes('.app/Contents/MacOS/') || !allowed.has(path.basename(executable))) {
    throw new Error('PLANR_TEST_HEADLESS_BROWSER must never point to a GUI browser app');
  }
  await access(executable, constants.X_OK);
  return executable;
}

async function createPlan(workspace, title, criteria) {
  assert.ok(criteria.length > 0, 'docs fixture build plans require at least one criterion');
  const product = requireSuccess(run(workspace, ['plan', 'new', title, '--json']));
  const build = requireSuccess(
    run(workspace, ['plan', 'split', product.stdout.plan.id, '--slice', 'Evidence example', '--json']),
  );
  const planPath = path.resolve(workspace, build.stdout.plan.path);
  const source = await readFile(planPath, 'utf8');
  const criteriaStart = source.indexOf('criteria:\n');
  const frontmatterEnd = source.indexOf('\n---\n', criteriaStart);
  assert.notEqual(criteriaStart, -1, `missing criteria frontmatter in ${planPath}`);
  assert.notEqual(frontmatterEnd, -1, `missing closing frontmatter in ${planPath}`);
  const renderedCriteria = criteria
    .map(({ id, title: criterionTitle }) => {
      assert.match(id, /^[A-Za-z0-9][A-Za-z0-9._:-]*$/, `invalid criterion id: ${id}`);
      assert.ok(criterionTitle.trim(), `criterion ${id} requires a title`);
      return `  - id: ${id}\n    title: ${JSON.stringify(criterionTitle)}`;
    })
    .join('\n');
  const acceptanceCriteria = criteria.map(({ title: criterionTitle }) => `- ${criterionTitle}`).join('\n');
  const checkedSource = `${source.slice(0, criteriaStart)}criteria:\n${renderedCriteria}${source.slice(frontmatterEnd)}`
    .replace(
      '## Scope Decision\n\n',
      '## Scope Decision\n\nExercise the declared Evidence criteria in an isolated documentation fixture.\n\n',
    )
    .replace(
      /## Phase 1\n\n- \[ \] Implement [^\n]+\n/,
      '## Phase 1\n\n- [ ] Execute and record the declared Evidence observations.\n',
    )
    .replace(
      '## Verification\n\n',
      '## Verification\n\nRun the configured capability and inspect canonical Evidence coverage.\n\n',
    )
    .replace(
      '## Acceptance Criteria\n\n',
      `## Acceptance Criteria\n\n${acceptanceCriteria}\n\n`,
    );
  await writeFile(planPath, checkedSource);
  const check = requireSuccess(run(workspace, ['plan', 'check', build.stdout.plan.id, '--json']));
  assert.equal(check.stdout.ok, true, `generated build plan did not pass plan check: ${JSON.stringify(check.stdout)}`);
  return build.stdout.plan.id;
}

async function migrateObligations(workspace, planId, obligations) {
  const file = path.join(workspace, `${planId}.evidence-migration.json`);
  await writeJson(file, {
    schema_version: 'planr.evidence.migration.v1',
    plan_id: planId,
    obligations,
  });
  return requireSuccess(run(workspace, ['evidence', 'migrate', '--input', file, '--apply', '--json']));
}

function runReadinessIndex(workspace, readiness) {
  const repositoryPath = readiness.stdout.object.run_index.repository_path;
  return run(workspace, ['evidence', 'run', '--input', path.join(workspace, repositoryPath), '--json']);
}

async function runMutatedReadinessIndex(workspace, readiness, mutate) {
  const runIndex = structuredClone(readiness.stdout.object.run_index);
  mutate(runIndex);
  delete runIndex.repository_path;
  delete runIndex.run_index_digest;
  const pathDigest = sha256Json(runIndex).slice('sha256:'.length);
  runIndex.repository_path = `.planr/evidence/runs/${pathDigest}.json`;
  runIndex.run_index_digest = sha256Json(runIndex);
  const file = path.join(workspace, runIndex.repository_path);
  await writeJson(file, runIndex);
  return run(workspace, ['evidence', 'run', '--input', file, '--json']);
}

function capabilityInstance(capabilities, manifestId) {
  const probe = capabilities.stdout.object.registry.probes.find((entry) => entry.manifest_id === manifestId);
  const instance = capabilities.stdout.object.instances.find((entry) => entry.id === probe?.instance_id);
  assert.ok(
    instance,
    `missing capability instance for ${manifestId}: ${JSON.stringify(capabilities.stdout.object)}`,
  );
  return instance;
}

function browserCdpObligation({ planId, spec }) {
  const requirements = [
    ['visible', 'com.example.browser.rendered_visibility', 'rendered page content is visible', { visible: true }],
    ['interaction', 'com.example.browser.user_interaction', 'user-equivalent click mutates rendered state', { clicked: true }],
    ['navigation', 'com.example.browser.navigation', 'browser navigation is observed', { path: '/next' }],
    ['network', 'com.example.browser.network', 'browser network result is observed', { api_status: 200 }],
    ['console', 'com.example.browser.console', 'browser console has no relevant errors', { error_count: 0 }],
    ['reload', 'com.example.browser.reload_storage', 'local storage persists across reload', { persisted: true }],
  ];
  return {
    id: 'pob-browser-cdp',
    schema_version: 'evidence.contract.v1',
    criterion_id: 'crit-pob-browser-cdp',
    plan_id: planId,
    title: 'Real Chrome CDP rendered workflow',
    binding: true,
    observations: requirements.map(([suffix, type, subject, expected]) => ({
      id: `obs-pob-browser-cdp-${suffix}`,
      type,
      subject,
      expected,
      target: spec.target,
      payload_schema: {
        schema_ref: spec.payloadSchemas.find((schema) => schema.type === type).schema_ref,
      },
    })),
    fixture_policy: { fixtures_allowed: false, mocks_allowed: false, disclosure_required: true },
    freshness_policy: { invalidate_on: ['policy_change', 'adapter_schema_change'] },
    assurance_policy: {},
  };
}

async function writeBrowserCdpSpec(workspace, port, debugPort, chromePath) {
  const sourcePath = path.join(
    repositoryRoot,
    'tests',
    'fixtures',
    'evidence',
    'browser-cdp',
    'v1',
    'browser-cdp-live.cjs',
  );
  const relativeHelper = '.planr/evidence/adapters/browser-cdp-live.cjs';
  const helperPath = path.join(workspace, relativeHelper);
  const source = await readFile(sourcePath, 'utf8');
  const helper = source.replace(
    '__PLANR_HEADLESS_BROWSER_PATH__',
    chromePath.replaceAll('\\', '\\\\').replaceAll('"', '\\"'),
  );
  await mkdir(path.dirname(helperPath), { recursive: true });
  await writeFile(helperPath, helper, { mode: 0o755 });

  const envelopeSchema = {
    schema_version: 'evidence.contract.v1',
    type: 'planr.structured_observation_results',
    schema_ref: 'schema://planr.structured_observation_results.v2',
    json_schema: { type: 'object' },
  };
  const observationSchemas = [
    ['com.example.browser.rendered_visibility', 'visible', { type: 'boolean' }],
    ['com.example.browser.user_interaction', 'clicked', { type: 'boolean' }],
    ['com.example.browser.navigation', 'path', { type: 'string' }],
    ['com.example.browser.network', 'api_status', { type: 'integer' }],
    ['com.example.browser.console', 'error_count', { type: 'integer' }],
    ['com.example.browser.reload_storage', 'persisted', { type: 'boolean' }],
  ].map(([type, property, propertySchema]) => ({
    schema_version: 'evidence.contract.v1',
    type,
    schema_ref: `schema://${type}`,
    json_schema: {
      type: 'object',
      required: ['schema_ref', property],
      additionalProperties: true,
      properties: {
        schema_ref: { const: `schema://${type}` },
        [property]: propertySchema,
      },
    },
  }));
  const observationTypes = observationSchemas.map((schema) => schema.type);
  const payloadSchemas = observationSchemas.map((schema) => ({
    type: schema.type,
    schema_ref: schema.schema_ref,
    schema_digest: sha256Json(schema),
  }));
  const envelopePayloadSchema = {
    type: envelopeSchema.type,
    schema_ref: envelopeSchema.schema_ref,
    schema_digest: sha256Json(envelopeSchema),
  };
  const target = { kind: 'browser', uri: `http://127.0.0.1:${port}/workflow` };
  const execution = {
    kind: 'process',
    executable: 'node',
    args: [relativeHelper, target.uri, String(port), String(debugPort)],
    working_directory: '.',
    timeout_ms: 20000,
    stdout_limit_bytes: 65536,
    stderr_limit_bytes: 65536,
    payload_schema: envelopePayloadSchema,
  };
  const fileArguments = [
    {
      argument_index: 0,
      argument: relativeHelper,
      resolved_relative_to: 'command_cwd',
      cwd_relative_path: relativeHelper,
      content_digest: sha256(helper),
    },
  ];
  const manifest = {
    id: 'verifier-browser-cdp-v1',
    schema_version: 'evidence.contract.v1',
    version: '1.0.0',
    adapter_kind: 'process',
    adapter_digest: processAdapterDigest(execution, fileArguments),
    supported_surfaces: ['local-process', 'chrome-cdp'],
    supported_observations: payloadSchemas,
    supported_interactions: ['render', 'click', 'navigate', 'reload', 'network_observe', 'console_observe'],
    supported_artifacts: ['stdout', 'planr.structured_observation_results.v2'],
    runtime_targets: [{ kind: 'browser', id: 'chrome-cdp' }],
    provenance_path: 'planr_observed_execution',
    permissions: { network: 'loopback', filesystem: 'read_workspace', browser: 'chrome-cdp' },
    costs: {},
    determinism: 'deterministic',
    repeatability: 'repeatable',
    independence: 'repository-defined raw CDP browser adapter',
    blind_spots: ['process-observed CDP cannot claim host-native VerifiedHostEvent provenance'],
    availability_probe: { kind: 'process', execution },
  };
  return {
    id: manifest.id,
    schema: observationSchemas[0],
    schemas: observationSchemas,
    envelopeSchema,
    payloadSchema: payloadSchemas[0],
    payloadSchemas,
    observationType: observationTypes[0],
    observationTypes,
    execution,
    manifest,
    manifestDigest: sha256Json(manifest),
    runtimeTarget: { kind: 'browser', id: 'chrome-cdp' },
    target,
    fixtureAllowed: false,
  };
}

const hostMatrix = JSON.parse(
  await readFile(
    path.join(repositoryRoot, 'tests/fixtures/evidence/host-capabilities/v1/expected/host-surface-matrix.json'),
    'utf8',
  ),
);
const hostMatrixDigest = await fileDigest(
  'tests/fixtures/evidence/host-capabilities/v1/expected/host-surface-matrix.json',
);
const evidenceSchemaDigest = await fileDigest(
  'docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json',
);

const cases = [];
const workspaces = [];
const servers = [];

async function startFixtureServer() {
  const server = spawn(
    process.execPath,
    [
      '-e',
      `
        const { createServer } = require('node:http');
        const server = createServer((request, response) => {
          if (request.url === '/health') {
            response.writeHead(200, { 'content-type': 'application/json' });
            response.end('{"status":"ok"}');
            return;
          }
          if (request.url === '/workflow') {
            response.writeHead(200, { 'content-type': 'text/html' });
            response.end('<!doctype html><title>Evidence workflow</title><main data-visible="true">ready</main>');
            return;
          }
          response.writeHead(404, { 'content-type': 'application/json' });
          response.end('{"error":"not_found"}');
        });
        server.listen(0, '127.0.0.1', () => console.log(server.address().port));
      `,
    ],
    { stdio: ['ignore', 'pipe', 'pipe'] },
  );
  const port = await new Promise((resolve, reject) => {
    let output = '';
    server.stdout.on('data', (chunk) => {
      output += chunk.toString('utf8');
      const match = output.match(/\d+/);
      if (match) resolve(Number(match[0]));
    });
    server.once('exit', (code) => reject(new Error(`fixture server exited before ready: ${code}`)));
  });
  servers.push(server);
  return port;
}

async function closeServers() {
  await Promise.all(
    servers.map(
      (server) =>
        new Promise((resolve) => {
          server.once('exit', resolve);
          server.kill();
        }),
    ),
  );
}

try {
  if (checkOnly && !liveMode) {
    const current = await readFile(outputPath, 'utf8');
    const fixture = JSON.parse(current);
    assert.equal(fixture.schema_version, 'planr.evidence_docs_examples.v1');
    assert.equal(fixture.generated_by, 'apps/docs/scripts/generate-evidence-examples.mjs');
    assert.equal(fixture.evidence_schema_digest, evidenceSchemaDigest);
    assert.equal(fixture.host_matrix_digest, hostMatrixDigest);
    assert.equal(Array.isArray(fixture.cases), true);
    assert.equal(fixture.cases.length, 7);
    console.log(`evidence_docs_examples_check=passed cases=${fixture.cases.length} host_matrix=${hostMatrixDigest}`);
    process.exit(0);
  }
  if (!liveMode) {
    throw new Error('Evidence docs example generation launches the local Chrome/CDP proof; rerun with --live.');
  }

  const chromePath = await findHeadlessBrowser();
  const apiPort = await startFixtureServer();
  const api = await disposableWorkspace('api-only');
  workspaces.push(api);
  const apiDefinition = SCENARIOS['api-only'];
  const apiSpec = httpSpec(`http://127.0.0.1:${apiPort}/health`);
  await writeEvidencePolicy(api, [apiSpec], { policyId: apiDefinition.policyId });
  const apiPlanId = await createPlan(api, apiDefinition.planTitle, [
    {
      id: `crit-${apiDefinition.obligationId}`,
      title: `Evidence obligation ${apiDefinition.obligationId}`,
    },
  ]);
  const apiPolicy = requireSuccess(run(api, ['evidence', 'policy', '--json']));
  await migrateObligations(
    api,
    apiPlanId,
    [
      obligation({
        id: apiDefinition.obligationId,
        planId: apiPlanId,
        spec: apiSpec,
        expected: apiDefinition.expected,
      }),
    ],
  );
  const apiReadiness = requireSuccess(
    run(api, ['evidence', 'readiness', '--scope', 'plan', '--id', apiPlanId, '--json']),
  );
  const apiRun = requireSuccess(runReadinessIndex(api, apiReadiness));
  const apiCoverage = requireSuccess(
    run(api, ['evidence', 'coverage', '--scope', 'obligation', '--id', apiDefinition.obligationId, '--json']),
  );
  cases.push({
    id: 'api-only-success',
    title: 'API-only HTTP success',
    command: redactCommand(apiCoverage.command, api),
    exit_code: apiCoverage.exit_code,
    output: redactVolatileEvidence(
      redactWorkspace(
        {
          policy: apiPolicy.stdout,
          readiness: apiReadiness.stdout,
          run: apiRun.stdout,
          coverage: apiCoverage.stdout,
        },
        api,
      ),
    ),
    proven_scope: [
      'curl-backed repository API/HTTP capability produced a trusted receipt',
      'obligation coverage is satisfied through the candidate binary',
    ],
    not_proven_scope: [
      'no browser-rendered observation was attempted',
    ],
  });

  const custom = await disposableWorkspace('custom-extension');
  workspaces.push(custom);
  const customDefinition = SCENARIOS['repository-custom-extension'];
  const customSpec = queueSpec();
  await writeEvidencePolicy(custom, [customSpec], { policyId: customDefinition.policyId });
  const customPlanId = await createPlan(custom, customDefinition.planTitle, [
    {
      id: `crit-${customDefinition.obligationId}`,
      title: `Evidence obligation ${customDefinition.obligationId}`,
    },
  ]);
  await migrateObligations(
    custom,
    customPlanId,
    [
      obligation({
        id: customDefinition.obligationId,
        planId: customPlanId,
        spec: customSpec,
        expected: customDefinition.expected,
      }),
    ],
  );
  const customReadiness = requireSuccess(
    run(custom, ['evidence', 'readiness', '--scope', 'plan', '--id', customPlanId, '--json']),
  );
  const customRun = requireSuccess(runReadinessIndex(custom, customReadiness));
  const customCoverage = requireSuccess(
    run(custom, ['evidence', 'coverage', '--scope', 'obligation', '--id', customDefinition.obligationId, '--json']),
  );
  cases.push({
    id: 'repository-custom-extension',
    title: 'Repository custom namespaced schema and adapter execution',
    command: redactCommand(customRun.command, custom),
    exit_code: customRun.exit_code,
    output: {
      manifest_digest: customSpec.manifestDigest,
      readiness: redactVolatileEvidence(redactWorkspace(customReadiness.stdout, custom)),
      run: redactVolatileEvidence(redactWorkspace(customRun.stdout, custom)),
      coverage: redactVolatileEvidence(redactWorkspace(customCoverage.stdout, custom)),
    },
    proven_scope: [
      'repository-owned com.example.queue.depth.v2 schema is registered outside Planr core',
      'repository-owned verifier-queue-depth-v2 adapter executed and satisfied its obligation',
    ],
    not_proven_scope: [
      'the extension only proves its declared com.example.queue.depth.v2 observation type',
      'no Planr core registry/status ownership is modified by the repository fixture',
    ],
  });

  const fullStackPort = await startFixtureServer();
  const fullStack = await disposableWorkspace('full-stack');
  workspaces.push(fullStack);
  const fullHttp = httpSpec(`http://127.0.0.1:${fullStackPort}/health`);
  const fullBrowser = await writeBrowserCdpSpec(
    fullStack,
    await freePort(),
    await freePort(),
    chromePath,
  );
  await writeEvidencePolicy(fullStack, [fullHttp, fullBrowser], {
    policyId: 'epolicy-docs-full-stack-v1',
  });
  const fullPlanId = await createPlan(fullStack, 'Evidence docs full stack', [
    { id: 'crit-pob-docs-full-http', title: 'Evidence obligation pob-docs-full-http' },
    { id: 'crit-pob-browser-cdp', title: 'Real Chrome CDP rendered workflow' },
  ]);
  await migrateObligations(
    fullStack,
    fullPlanId,
    [
      obligation({
        id: 'pob-docs-full-http',
        planId: fullPlanId,
        spec: fullHttp,
        expected: { status: 'ok' },
      }),
      browserCdpObligation({
        planId: fullPlanId,
        spec: fullBrowser,
      }),
    ],
  );
  const beforeFull = run(fullStack, [
    'evidence',
    'coverage',
    '--scope',
    'plan',
    '--id',
    fullPlanId,
    '--json',
  ]);
  const fullReadiness = requireSuccess(
    run(fullStack, ['evidence', 'readiness', '--scope', 'plan', '--id', fullPlanId, '--json']),
  );
  const fullRun = requireSuccess(runReadinessIndex(fullStack, fullReadiness));
  const afterFull = requireSuccess(run(fullStack, ['evidence', 'coverage', '--scope', 'plan', '--id', fullPlanId, '--json']));
  cases.push({
    id: 'full-stack-composition',
    title: 'Full-stack composition from unsatisfied to satisfied plan coverage',
    command: redactCommand(afterFull.command, fullStack),
    exit_code: afterFull.exit_code,
    output: redactVolatileEvidence(
      redactWorkspace(
        {
          before: beforeFull.stdout,
          readiness: fullReadiness.stdout,
          run: fullRun.stdout,
          after: afterFull.stdout,
        },
        fullStack,
      ),
    ),
    proven_scope: [
      'initial plan coverage is not satisfied before receipts exist',
      'HTTP API and six real Chrome/CDP observations compose into satisfied plan coverage',
    ],
    not_proven_scope: [
      'browser proof covers the disposable workflow target, not an arbitrary product URL',
    ],
  });

  const forged = await disposableWorkspace('forged-claim');
  workspaces.push(forged);
  const forgedPort = await startFixtureServer();
  const forgedSpec = httpSpec(`http://127.0.0.1:${forgedPort}/health`);
  await writeEvidencePolicy(forged, [forgedSpec], { policyId: 'epolicy-docs-forged-v1' });
  const forgedPlanId = await createPlan(forged, 'Evidence docs forged input', [
    { id: 'crit-pob-docs-forged', title: 'Evidence obligation pob-docs-forged' },
  ]);
  await migrateObligations(
    forged,
    forgedPlanId,
    [
      obligation({
        id: 'pob-docs-forged',
        planId: forgedPlanId,
        spec: forgedSpec,
        expected: { status: 'ok' },
      }),
    ],
  );
  const forgedReadiness = requireSuccess(
    run(forged, ['evidence', 'readiness', '--scope', 'obligation', '--id', 'pob-docs-forged', '--json']),
  );
  const forgedRun = await runMutatedReadinessIndex(forged, forgedReadiness, (runIndex) => {
    runIndex.runs[0].input.receipt = {
      id: 'receipt-forged-by-caller',
      receipt_status: 'trusted',
    };
    runIndex.runs[0].input.attempt = {
      id: 'attempt-forged-by-caller',
      attempt_status: 'passed',
    };
  });
  cases.push({
    id: 'forged-claim-rejection',
    title: 'Forged trusted-field run input is rejected before trust',
    command: redactCommand(forgedRun.command, forged),
    exit_code: forgedRun.exit_code,
    output: redactWorkspace(forgedRun.stdout, forged),
    stderr: forgedRun.stderr.replaceAll(forged, '<workspace>'),
    proven_scope: [
      'the binary rejects caller-supplied trusted receipt/attempt fields in run input',
      'no trusted receipt is constructed from caller-supplied data',
    ],
    not_proven_scope: [
      'does not test every forged trusted-field variant beyond receipt/attempt injection',
    ],
  });

  const stalePort = await startFixtureServer();
  const stale = await disposableWorkspace('stale');
  workspaces.push(stale);
  const staleSpec = httpSpec(`http://127.0.0.1:${stalePort}/health`);
  await writeEvidencePolicy(stale, [staleSpec], { policyId: 'epolicy-docs-stale-v1' });
  const stalePlanId = await createPlan(stale, 'Evidence docs stale policy', [
    { id: 'crit-pob-docs-stale-policy', title: 'Evidence obligation pob-docs-stale-policy' },
  ]);
  await migrateObligations(
    stale,
    stalePlanId,
    [
      obligation({
        id: 'pob-docs-stale-policy',
        planId: stalePlanId,
        spec: staleSpec,
        expected: { status: 'ok' },
        invalidateOn: ['policy_change'],
      }),
    ],
  );
  const staleReadiness = requireSuccess(
    run(stale, ['evidence', 'readiness', '--scope', 'plan', '--id', stalePlanId, '--json']),
  );
  const staleRun = requireSuccess(runReadinessIndex(stale, staleReadiness));
  const stalePolicyChanged = JSON.parse(await readFile(path.join(stale, '.planr', 'evidence.yaml'), 'utf8'));
  stalePolicyChanged.freshness_policy.max_age_seconds = 7200;
  stalePolicyChanged.policy_digest = sha256JsonWithoutField(stalePolicyChanged, 'policy_digest');
  await writeFile(path.join(stale, '.planr', 'evidence.yaml'), `${JSON.stringify(stalePolicyChanged, null, 2)}\n`);
  const staleCoverage = run(stale, ['evidence', 'coverage', '--scope', 'obligation', '--id', 'pob-docs-stale-policy', '--json']);
  cases.push({
    id: 'stale-evidence',
    title: 'Stale evidence is invalidated after repository policy drift',
    command: redactCommand(staleCoverage.command, stale),
    exit_code: staleCoverage.exit_code,
    output: redactVolatileEvidence(redactWorkspace({ run: staleRun.stdout, coverage: staleCoverage.stdout }, stale)),
    stderr: staleCoverage.stderr.replaceAll(stale, '<workspace>'),
    proven_scope: [
      'a valid receipt is created before policy drift',
      'coverage marks the previous receipt stale after policy mutation',
    ],
    not_proven_scope: [
      'the fixture does not refresh the stale receipt',
    ],
  });

  const missing = await disposableWorkspace('missing-capability');
  workspaces.push(missing);
  const missingSpec = queueSpec();
  missingSpec.id = 'verifier-unavailable-queue-v1';
  missingSpec.manifest.id = missingSpec.id;
  missingSpec.manifest.availability_probe.execution.executable = 'definitely-not-a-planr-probe';
  missingSpec.execution.executable = 'definitely-not-a-planr-probe';
  missingSpec.manifest.adapter_digest = processAdapterDigest(missingSpec.execution);
  missingSpec.manifestDigest = sha256Json(missingSpec.manifest);
  await writeEvidencePolicy(missing, [missingSpec], { policyId: 'epolicy-docs-missing-v1' });
  const missingPlanId = await createPlan(missing, 'Evidence docs missing capability', [
    {
      id: 'crit-pob-docs-missing-capability',
      title: 'Evidence obligation pob-docs-missing-capability',
    },
  ]);
  const missingCapabilities = requireSuccess(run(missing, ['evidence', 'capability', 'list', '--json']));
  await migrateObligations(
    missing,
    missingPlanId,
    [
      obligation({
        id: 'pob-docs-missing-capability',
        planId: missingPlanId,
        spec: missingSpec,
        expected: { status: 'drained' },
      }),
    ],
  );
  const missingReadiness = run(missing, [
    'evidence',
    'readiness',
    '--scope',
    'plan',
    '--id',
    missingPlanId,
    '--json',
  ]);
  assert.equal(missingReadiness.exit_code, 3, JSON.stringify(missingReadiness.stdout));
  const missingCoverage = run(missing, [
    'evidence',
    'coverage',
    '--scope',
    'obligation',
    '--id',
    'pob-docs-missing-capability',
    '--json',
  ]);
  cases.push({
    id: 'missing-capability',
    title: 'Missing capability is explicit',
    command: redactCommand(missingReadiness.command, missing),
    exit_code: missingReadiness.exit_code,
    output: redactVolatileEvidence(
      redactWorkspace(
        {
          capabilities: missingCapabilities.stdout,
          readiness: missingReadiness.stdout,
          coverage: missingCoverage.stdout,
        },
        missing,
      ),
    ),
    stderr: missingReadiness.stderr.replaceAll(missing, '<workspace>'),
    proven_scope: [
      'an unavailable repository capability cannot be used to mint a receipt',
      'the binary returns the canonical capability-unavailable rejection',
    ],
    not_proven_scope: [
      'no fallback capability is selected for the obligation',
    ],
  });

  const curlPort = await startFixtureServer();
  const curlBrowser = await disposableWorkspace('curl-browser-negative');
  workspaces.push(curlBrowser);
  const curlHttp = httpSpec(`http://127.0.0.1:${curlPort}/health`);
  const curlBrowserSpec = await writeBrowserCdpSpec(
    curlBrowser,
    await freePort(),
    await freePort(),
    chromePath,
  );
  await writeEvidencePolicy(curlBrowser, [curlHttp, curlBrowserSpec], {
    policyId: 'epolicy-docs-curl-browser-v1',
  });
  const curlPlanId = await createPlan(curlBrowser, 'Evidence docs curl versus browser', [
    { id: 'crit-pob-docs-curl-http', title: 'Evidence obligation pob-docs-curl-http' },
    {
      id: 'crit-pob-docs-browser-rendered',
      title: 'Evidence obligation pob-docs-browser-rendered',
    },
  ]);
  const curlCapabilities = requireSuccess(run(curlBrowser, ['evidence', 'capability', 'list', '--json']));
  const curlHttpInstance = capabilityInstance(curlCapabilities, curlHttp.id);
  await migrateObligations(
    curlBrowser,
    curlPlanId,
    [
      obligation({
        id: 'pob-docs-curl-http',
        planId: curlPlanId,
        spec: curlHttp,
        expected: { status: 'ok' },
      }),
      obligation({
        id: 'pob-docs-browser-rendered',
        planId: curlPlanId,
        spec: curlBrowserSpec,
        expected: { visible: true },
      }),
    ],
  );
  const curlHttpReadiness = requireSuccess(
    run(curlBrowser, ['evidence', 'readiness', '--scope', 'obligation', '--id', 'pob-docs-curl-http', '--json']),
  );
  const curlHttpRun = requireSuccess(runReadinessIndex(curlBrowser, curlHttpReadiness));
  const curlBrowserReadiness = requireSuccess(
    run(curlBrowser, [
      'evidence',
      'readiness',
      '--scope',
      'obligation',
      '--id',
      'pob-docs-browser-rendered',
      '--json',
    ]),
  );
  const curlAgainstBrowser = await runMutatedReadinessIndex(curlBrowser, curlBrowserReadiness, (runIndex) => {
    runIndex.runs[0].capability = {
      instance_id: curlHttpInstance.id,
      manifest_id: curlHttpInstance.manifest_id,
      manifest_digest: curlHttpInstance.manifest_digest,
      manifest_version: curlHttpInstance.manifest_version,
    };
    runIndex.runs[0].input.capability_instance_id = curlHttpInstance.id;
    runIndex.runs[0].input.environment = curlHttpInstance.capability.environment;
    runIndex.runs[0].input.execution_contract = curlHttp.execution;
  });
  const browserSurfaces = hostMatrix.surfaces.filter((surface) => surface.observation_types.some((kind) => kind.includes('browser') || kind.includes('chrome')));
  cases.push({
    id: 'curl-http-not-browser',
    title: 'Curl can prove HTTP-only scope, not browser-rendered obligations',
    command: redactCommand(curlAgainstBrowser.command, curlBrowser),
    exit_code: curlAgainstBrowser.exit_code,
    output: {
      host_matrix_digest: hostMatrixDigest,
      browser_surface_count: browserSurfaces.length,
      http_run: redactVolatileEvidence(redactWorkspace(curlHttpRun.stdout, curlBrowser)),
      browser_readiness: redactVolatileEvidence(redactWorkspace(curlBrowserReadiness.stdout, curlBrowser)),
      browser_rejection: redactVolatileEvidence(redactWorkspace(curlAgainstBrowser.stdout, curlBrowser)),
    },
    proven_scope: [
      'curl-backed HTTP evidence satisfies the HTTP obligation',
      'the same curl capability is rejected for a browser-rendered obligation',
    ],
    not_proven_scope: [
      'HTTP-only proof is not browser-rendered proof',
    ],
  });

  const generated = {
    schema_version: 'planr.evidence_docs_examples.v1',
    generated_by: 'apps/docs/scripts/generate-evidence-examples.mjs',
    candidate_binary: displayCandidateBinary(),
    evidence_schema_digest: evidenceSchemaDigest,
    host_matrix_digest: hostMatrixDigest,
    cases,
  };

  await mkdir(fixtureRoot, { recursive: true });
  const bytes = `${JSON.stringify(generated, null, 2)}\n`;
  if (checkOnly) {
    const current = await readFile(outputPath, 'utf8').catch(() => '');
    assert.equal(current, bytes, 'Evidence docs examples are stale. Run `pnpm docs:evidence-examples:generate`.');
    console.log(`evidence_docs_examples_check=passed cases=${cases.length} host_matrix=${hostMatrixDigest}`);
  } else {
    await writeFile(outputPath, bytes);
    console.log(`evidence_docs_examples_generated=${outputPath} cases=${cases.length} host_matrix=${hostMatrixDigest}`);
  }
} finally {
  await Promise.all(workspaces.map((workspace) => rm(workspace, { recursive: true, force: true })));
  await closeServers();
}
