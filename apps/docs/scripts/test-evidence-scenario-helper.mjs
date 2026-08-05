#!/usr/bin/env node
import assert from 'node:assert/strict';
import { access, mkdtemp, readFile, rm } from 'node:fs/promises';
import { constants } from 'node:fs';
import { spawn, spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  SCENARIOS,
  buildEvidencePolicy,
  obligation,
  scenarioSpec,
} from './evidence-fixture-builder.mjs';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const helperPath = path.join(docsRoot, 'scripts/create-evidence-scenario-files.mjs');
const planrBin = process.env.PLANR_BIN
  ? path.resolve(process.cwd(), process.env.PLANR_BIN)
  : path.join(repositoryRoot, 'target', 'debug', 'planr');

await access(planrBin, constants.X_OK);

function run(command, args, { cwd, env = {} } = {}) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: 'utf8',
    env: {
      ...process.env,
      PLANR_BIN: planrBin,
      PLANR_WORKER_ID: 'docs-scenario-helper-test',
      PLANR_PROFILE: 'docs-scenario-test',
      ...env,
    },
  });
  assert.equal(result.status, 0, `${command} ${args.join(' ')}\n${result.stderr}\n${result.stdout}`);
  return result.stdout.trim() ? JSON.parse(result.stdout) : null;
}

async function withServer(callback) {
  const server = spawn(process.execPath, [
    '-e',
    `
      const { createServer } = require('node:http');
      const server = createServer((_, response) => {
        response.writeHead(200, { 'content-type': 'application/json' });
        response.end('{"status":"ok"}');
      });
      server.listen(0, '127.0.0.1', () => console.log(server.address().port));
    `,
  ], { stdio: ['ignore', 'pipe', 'pipe'] });
  const port = await new Promise((resolve, reject) => {
    let output = '';
    server.stdout.on('data', (chunk) => {
      output += chunk.toString('utf8');
      const match = output.match(/\d+/);
      if (match) resolve(Number(match[0]));
    });
    server.once('exit', (code) => reject(new Error(`fixture server exited before ready: ${code}`)));
  });
  try {
    return await callback(`http://127.0.0.1:${port}/health`);
  } finally {
    await new Promise((resolve) => {
      server.once('exit', resolve);
      server.kill();
    });
  }
}

async function runScenario({ scenario, projectTitle, obligationId, apiUrl }) {
  const workspace = await mkdtemp(path.join(tmpdir(), `planr-docs-helper-${scenario}-`));
  try {
    const definition = SCENARIOS[scenario];
    const spec = scenarioSpec(scenario, { apiUrl });
    run(planrBin, ['project', 'init', projectTitle, '--json'], { cwd: workspace });
    const helperArgs = [helperPath, '--scenario', scenario];
    if (apiUrl) helperArgs.push('--api-url', apiUrl);
    const helper = run(process.execPath, helperArgs, { cwd: workspace });
    assert.deepEqual(
      helper.files.slice(0, definition.files.length),
      definition.files,
      `${scenario} helper must report shared documented files`,
    );

    const schema = JSON.parse(await readFile(path.join(workspace, definition.files[0]), 'utf8'));
    const manifest = JSON.parse(await readFile(path.join(workspace, definition.files[1]), 'utf8'));
    const policy = JSON.parse(await readFile(path.join(workspace, definition.files[2]), 'utf8'));
    const obligationPayload = JSON.parse(await readFile(path.join(workspace, definition.files[3]), 'utf8'));
    const runIndexPath = helper.files.at(-1);
    const runInput = JSON.parse(await readFile(path.join(workspace, runIndexPath), 'utf8'));

    assert.deepEqual(schema, spec.schema, `${scenario} schema must come from shared builder`);
    assert.deepEqual(manifest, spec.manifest, `${scenario} manifest must come from shared builder`);
    assert.deepEqual(
      policy,
      buildEvidencePolicy([spec], { policyId: definition.policyId }),
      `${scenario} policy must come from shared builder`,
    );

    const evidenceRun = run(planrBin, ['evidence', 'run', '--input', runIndexPath, '--json'], {
      cwd: workspace,
    });
    assert.equal(evidenceRun.ok, true, `${scenario} evidence run must succeed`);
    assert.equal(
      evidenceRun.object?.results?.[0]?.receipt?.receipt_status,
      'trusted',
      `${scenario} receipt must be trusted`,
    );

    const expectedObligation = obligation({
      id: obligationId,
      planId: obligationPayload.plan_id,
      spec,
      expected: definition.expected,
    });
    assert.deepEqual(obligationPayload, expectedObligation, `${scenario} obligation must come from shared builder`);
    assert.equal(runInput.schema_version, 'planr.evidence.run-index.v1');
    assert.equal(runInput.runs[0].input.obligation_id, obligationId);

    const coverage = run(
      planrBin,
      ['evidence', 'coverage', '--scope', 'obligation', '--id', obligationId, '--json'],
      { cwd: workspace },
    );
    assert.equal(coverage.ok, true, `${scenario} coverage command must succeed`);
    assert.equal(coverage.object?.status, 'satisfied', `${scenario} coverage must be satisfied`);
  } finally {
    await rm(workspace, { recursive: true, force: true });
  }
}

await withServer((apiUrl) =>
  runScenario({
    scenario: 'api-only',
    projectTitle: 'Evidence API',
    obligationId: 'pob-docs-api-http',
    apiUrl,
  }),
);

await runScenario({
  scenario: 'repository-custom-extension',
  projectTitle: 'Evidence custom adapter',
  obligationId: 'pob-docs-queue-depth',
});

console.log(
  JSON.stringify(
    {
      ok: true,
      scenarios: ['api-only', 'repository-custom-extension'],
    },
    null,
    2,
  ),
);
