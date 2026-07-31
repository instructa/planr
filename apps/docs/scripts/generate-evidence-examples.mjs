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
  scenarioRunInput,
  sha256,
  sha256Json,
  sha256JsonWithoutField,
  writeEvidencePolicy,
  writeJson,
} from './evidence-fixture-builder.mjs';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const fixtureRoot = path.join(repositoryRoot, 'tests', 'fixtures', 'evidence', 'docs', 'v1');
const outputPath = path.join(fixtureRoot, 'examples.generated.json');
const planrBin = process.env.PLANR_BIN
  ? path.resolve(process.cwd(), process.env.PLANR_BIN)
  : path.join(repositoryRoot, 'target', 'debug', 'planr');
const checkOnly = process.argv.includes('--check');
const liveMode = process.argv.includes('--live');

await access(planrBin, constants.X_OK);

async function fileDigest(relative) {
  return sha256(await readFile(path.join(repositoryRoot, relative)));
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
        (key === 'digest' || key.endsWith('_digest')) &&
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
    return value.replace(/http:\/\/127\.0\.0\.1:\d+/g, 'http://127.0.0.1:<port>');
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

async function findChrome() {
  const candidates = [
    process.env.PLANR_TEST_CHROME,
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
    '/usr/bin/chromium-browser',
  ].filter(Boolean);
  for (const candidate of candidates) {
    try {
      await access(candidate, constants.X_OK);
      return candidate;
    } catch {}
  }
  throw new Error(
    'real browser evidence example requires Chrome/Chromium; set PLANR_TEST_CHROME to an executable path',
  );
}

async function createPlan(workspace, title) {
  const result = requireSuccess(run(workspace, ['plan', 'new', title, '--json']));
  return result.stdout.plan.id;
}

async function addObligation(workspace, obligation) {
  const file = path.join(workspace, `${obligation.id}.obligation.json`);
  await writeJson(file, obligation);
  return requireSuccess(run(workspace, ['evidence', 'obligation', 'add', '--input', file, '--json']));
}

async function runEvidence(workspace, id, input) {
  const file = path.join(workspace, `${id}.run.json`);
  await writeJson(file, input);
  return run(workspace, ['evidence', 'run', '--input', file, '--json']);
}

function capabilityInstance(capabilities, manifestId) {
  const probe = capabilities.stdout.object.registry.probes.find((entry) => entry.manifest_id === manifestId);
  const instance = capabilities.stdout.object.instances.find((entry) => entry.id === probe?.instance_id);
  assert.ok(instance, `missing capability instance for ${manifestId}`);
  return instance;
}

function browserCdpObligation({ planId, policyDigest, spec, environment, configDigest }) {
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
      environment,
      runtime_target: spec.runtimeTarget,
    })),
    fixture_policy: { fixtures_allowed: true, mocks_allowed: false, disclosure_required: true },
    freshness_policy: { invalidate_on: ['policy_change', 'adapter_schema_change'] },
    assurance_policy: {},
    policy_digest: policyDigest,
    config_digest: configDigest,
    created_at: '2026-07-29T00:00:00Z',
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
    '__PLANR_CHROME_PATH__',
    chromePath.replaceAll('\\', '\\\\').replaceAll('"', '\\"'),
  );
  await mkdir(path.dirname(helperPath), { recursive: true });
  await writeFile(helperPath, helper, { mode: 0o755 });

  const schema = {
    schema_version: 'evidence.contract.v1',
    type: 'com.example.browser.cdp',
    schema_ref: 'schema://planr.structured_observation_results.v1',
    json_schema: { type: 'object' },
  };
  const observationTypes = [
    'com.example.browser.rendered_visibility',
    'com.example.browser.user_interaction',
    'com.example.browser.navigation',
    'com.example.browser.network',
    'com.example.browser.console',
    'com.example.browser.reload_storage',
  ];
  const payloadSchemas = observationTypes.map((type) => ({
    type,
    schema_ref: schema.schema_ref,
    schema_digest: sha256Json(schema),
  }));
  const target = { kind: 'browser', uri: `http://127.0.0.1:${port}/workflow` };
  const execution = {
    kind: 'process',
    executable: 'node',
    args: [relativeHelper, target.uri, String(port), String(debugPort)],
    working_directory: '.',
    timeout_ms: 20000,
    stdout_limit_bytes: 65536,
    stderr_limit_bytes: 65536,
    payload_schema: payloadSchemas[0],
  };
  const cwd = await pathRealpath(workspace);
  const canonicalHelper = await pathRealpath(helperPath);
  const fileArguments = [
    {
      argument_index: 0,
      argument: relativeHelper,
      resolved_relative_to: 'command_cwd',
      cwd,
      path: canonicalHelper,
      cwd_relative_path: path.relative(cwd, canonicalHelper),
      path_digest: sha256(canonicalHelper),
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
    supported_artifacts: ['stdout', 'planr.structured_observation_results.v1'],
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
  const helperDigest = sha256(helper);
  return {
    id: manifest.id,
    schema,
    payloadSchema: payloadSchemas[0],
    payloadSchemas,
    observationType: observationTypes[0],
    observationTypes,
    execution,
    manifest,
    manifestDigest: sha256Json(manifest),
    runtimeTarget: { kind: 'browser', id: 'chrome-cdp' },
    target,
    fixtureAllowed: true,
    fixtureDisclosure: {
      fixtures_used: true,
      mocks_used: false,
      fixture_refs: [`planr-test-fixture:browser-cdp-live-helper:${helperDigest}`],
    },
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

  const chromePath = await findChrome();
  const apiPort = await startFixtureServer();
  const api = await disposableWorkspace('api-only');
  workspaces.push(api);
  const apiDefinition = SCENARIOS['api-only'];
  const apiSpec = httpSpec(`http://127.0.0.1:${apiPort}/health`);
  const apiPolicyDigest = await writeEvidencePolicy(api, [apiSpec], { policyId: apiDefinition.policyId });
  const apiPlanId = await createPlan(api, apiDefinition.planTitle);
  const apiPolicy = requireSuccess(run(api, ['evidence', 'policy', '--json']));
  const apiCapabilities = requireSuccess(run(api, ['evidence', 'capability', 'list', '--json']));
  const apiInstance = capabilityInstance(apiCapabilities, apiSpec.id);
  await addObligation(
    api,
    obligation({
      id: apiDefinition.obligationId,
      planId: apiPlanId,
      policyDigest: apiPolicyDigest,
      spec: apiSpec,
      expected: apiDefinition.expected,
      environment: apiInstance.capability.environment,
      configDigest: apiDefinition.configDigest,
    }),
  );
  const apiRun = requireSuccess(
    await runEvidence(
      api,
      apiDefinition.obligationId,
      scenarioRunInput({
        obligationId: apiDefinition.obligationId,
        capabilityInstanceId: apiInstance.id,
        target: apiSpec.target,
      }),
    ),
  );
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
  const customPolicyDigest = await writeEvidencePolicy(custom, [customSpec], { policyId: customDefinition.policyId });
  const customPlanId = await createPlan(custom, customDefinition.planTitle);
  const customCapabilities = requireSuccess(run(custom, ['evidence', 'capability', 'list', '--json']));
  const customInstance = capabilityInstance(customCapabilities, customSpec.id);
  await addObligation(
    custom,
    obligation({
      id: customDefinition.obligationId,
      planId: customPlanId,
      policyDigest: customPolicyDigest,
      spec: customSpec,
      expected: customDefinition.expected,
      environment: customInstance.capability.environment,
      configDigest: customDefinition.configDigest,
    }),
  );
  const customRun = requireSuccess(
    await runEvidence(
      custom,
      customDefinition.obligationId,
      scenarioRunInput({
        obligationId: customDefinition.obligationId,
        capabilityInstanceId: customInstance.id,
        target: customSpec.target,
      }),
    ),
  );
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
  const fullPolicyDigest = await writeEvidencePolicy(fullStack, [fullHttp, fullBrowser], {
    policyId: 'epolicy-docs-full-stack-v1',
  });
  const fullPlanId = await createPlan(fullStack, 'Evidence docs full stack');
  const fullCapabilities = requireSuccess(run(fullStack, ['evidence', 'capability', 'list', '--json']));
  const fullHttpInstance = capabilityInstance(fullCapabilities, fullHttp.id);
  const fullBrowserInstance = capabilityInstance(fullCapabilities, fullBrowser.id);
  await addObligation(
    fullStack,
    obligation({
      id: 'pob-docs-full-http',
      planId: fullPlanId,
      policyDigest: fullPolicyDigest,
      spec: fullHttp,
      expected: { status: 'ok' },
      environment: fullHttpInstance.capability.environment,
      configDigest: 'sha256:3030303030303030303030303030303030303030303030303030303030303030',
    }),
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
  await addObligation(
    fullStack,
    browserCdpObligation({
      planId: fullPlanId,
      policyDigest: fullPolicyDigest,
      spec: fullBrowser,
      environment: fullBrowserInstance.capability.environment,
      configDigest: 'sha256:3131313131313131313131313131313131313131313131313131313131313131',
    }),
  );
  const fullHttpRun = requireSuccess(
    await runEvidence(fullStack, 'pob-docs-full-http', {
      obligation_id: 'pob-docs-full-http',
      capability_instance_id: fullHttpInstance.id,
      target: fullHttp.target,
    }),
  );
  const fullBrowserRun = requireSuccess(
    await runEvidence(fullStack, 'pob-browser-cdp', {
      obligation_id: 'pob-browser-cdp',
      capability_instance_id: fullBrowserInstance.id,
      target: fullBrowser.target,
      fixture_disclosure: fullBrowser.fixtureDisclosure,
    }),
  );
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
          runs: [fullHttpRun.stdout, fullBrowserRun.stdout],
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
  const forgedPolicyDigest = await writeEvidencePolicy(forged, [forgedSpec], { policyId: 'epolicy-docs-forged-v1' });
  const forgedPlanId = await createPlan(forged, 'Evidence docs forged input');
  const forgedCapabilities = requireSuccess(run(forged, ['evidence', 'capability', 'list', '--json']));
  const forgedInstance = capabilityInstance(forgedCapabilities, forgedSpec.id);
  await addObligation(
    forged,
    obligation({
      id: 'pob-docs-forged',
      planId: forgedPlanId,
      policyDigest: forgedPolicyDigest,
      spec: forgedSpec,
      expected: { status: 'ok' },
      environment: forgedInstance.capability.environment,
      configDigest: 'sha256:4040404040404040404040404040404040404040404040404040404040404040',
    }),
  );
  const forgedRun = await runEvidence(forged, 'pob-docs-forged', {
    obligation_id: 'pob-docs-forged',
    capability_instance_id: forgedInstance.id,
    target: forgedSpec.target,
    receipt: { id: 'receipt-forged-by-caller', receipt_status: 'trusted' },
    attempt: { id: 'attempt-forged-by-caller', attempt_status: 'passed' },
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
  const stalePolicyDigest = await writeEvidencePolicy(stale, [staleSpec], { policyId: 'epolicy-docs-stale-v1' });
  const stalePlanId = await createPlan(stale, 'Evidence docs stale policy');
  const staleCapabilities = requireSuccess(run(stale, ['evidence', 'capability', 'list', '--json']));
  const staleInstance = capabilityInstance(staleCapabilities, staleSpec.id);
  await addObligation(
    stale,
    obligation({
      id: 'pob-docs-stale-policy',
      planId: stalePlanId,
      policyDigest: stalePolicyDigest,
      spec: staleSpec,
      expected: { status: 'ok' },
      environment: staleInstance.capability.environment,
      configDigest: 'sha256:5050505050505050505050505050505050505050505050505050505050505050',
      invalidateOn: ['policy_change'],
    }),
  );
  const staleRun = requireSuccess(
    await runEvidence(stale, 'pob-docs-stale-policy', {
      obligation_id: 'pob-docs-stale-policy',
      capability_instance_id: staleInstance.id,
      target: staleSpec.target,
    }),
  );
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
  const missingPolicyDigest = await writeEvidencePolicy(missing, [missingSpec], { policyId: 'epolicy-docs-missing-v1' });
  const missingPlanId = await createPlan(missing, 'Evidence docs missing capability');
  const missingCapabilities = requireSuccess(run(missing, ['evidence', 'capability', 'list', '--json']));
  const missingInstance = capabilityInstance(missingCapabilities, missingSpec.id);
  await addObligation(
    missing,
    obligation({
      id: 'pob-docs-missing-capability',
      planId: missingPlanId,
      policyDigest: missingPolicyDigest,
      spec: missingSpec,
      expected: { status: 'drained' },
      environment: missingInstance.capability.environment,
      configDigest: 'sha256:6060606060606060606060606060606060606060606060606060606060606060',
    }),
  );
  const missingRun = await runEvidence(missing, 'pob-docs-missing-capability', {
    obligation_id: 'pob-docs-missing-capability',
    capability_instance_id: missingInstance.id,
    target: missingSpec.target,
  });
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
    command: redactCommand(missingRun.command, missing),
    exit_code: missingRun.exit_code,
    output: redactVolatileEvidence(
      redactWorkspace(
        { capabilities: missingCapabilities.stdout, run: missingRun.stdout, coverage: missingCoverage.stdout },
        missing,
      ),
    ),
    stderr: missingRun.stderr.replaceAll(missing, '<workspace>'),
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
  const curlPolicyDigest = await writeEvidencePolicy(curlBrowser, [curlHttp, curlBrowserSpec], {
    policyId: 'epolicy-docs-curl-browser-v1',
  });
  const curlPlanId = await createPlan(curlBrowser, 'Evidence docs curl versus browser');
  const curlCapabilities = requireSuccess(run(curlBrowser, ['evidence', 'capability', 'list', '--json']));
  const curlHttpInstance = capabilityInstance(curlCapabilities, curlHttp.id);
  const curlBrowserInstance = capabilityInstance(curlCapabilities, curlBrowserSpec.id);
  await addObligation(
    curlBrowser,
    obligation({
      id: 'pob-docs-curl-http',
      planId: curlPlanId,
      policyDigest: curlPolicyDigest,
      spec: curlHttp,
      expected: { status: 'ok' },
      environment: curlHttpInstance.capability.environment,
      configDigest: 'sha256:7070707070707070707070707070707070707070707070707070707070707070',
    }),
  );
  await addObligation(
    curlBrowser,
    obligation({
      id: 'pob-docs-browser-rendered',
      planId: curlPlanId,
      policyDigest: curlPolicyDigest,
      spec: curlBrowserSpec,
      expected: { visible: true },
      environment: curlBrowserInstance.capability.environment,
      configDigest: 'sha256:7171717171717171717171717171717171717171717171717171717171717171',
    }),
  );
  const curlHttpRun = requireSuccess(
    await runEvidence(curlBrowser, 'pob-docs-curl-http', {
      obligation_id: 'pob-docs-curl-http',
      capability_instance_id: curlHttpInstance.id,
      target: curlHttp.target,
    }),
  );
  const curlAgainstBrowser = await runEvidence(curlBrowser, 'pob-docs-browser-rendered', {
    obligation_id: 'pob-docs-browser-rendered',
    capability_instance_id: curlHttpInstance.id,
    target: curlBrowserSpec.target,
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
    candidate_binary: path.relative(repositoryRoot, planrBin),
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
