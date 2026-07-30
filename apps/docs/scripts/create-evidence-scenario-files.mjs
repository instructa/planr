#!/usr/bin/env node
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  SCENARIOS,
  SCENARIO_IDS,
  obligation,
  scenarioRunInput,
  scenarioSpec,
  writeEvidencePolicy,
  writeJson,
} from './evidence-fixture-builder.mjs';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const planrBin = process.env.PLANR_BIN
  ? path.resolve(process.cwd(), process.env.PLANR_BIN)
  : path.join(repositoryRoot, 'target', 'debug', 'planr');

const scenario = valueFor('--scenario');
const apiUrl = valueFor('--api-url') ?? 'http://127.0.0.1:3000/health';
const exampleInvocations = ['--scenario api-only', '--scenario repository-custom-extension'];

if (!SCENARIO_IDS.includes(scenario)) {
  throw new Error(
    `usage: node apps/docs/scripts/create-evidence-scenario-files.mjs ${exampleInvocations.join(' or ')} [--api-url http://127.0.0.1:3000/health]`,
  );
}

function valueFor(flag) {
  const index = process.argv.indexOf(flag);
  return index === -1 ? null : process.argv[index + 1];
}

function run(args) {
  const result = spawnSync(planrBin, args, {
    cwd: process.cwd(),
    encoding: 'utf8',
    env: {
      ...process.env,
      PLANR_WORKER_ID: process.env.PLANR_WORKER_ID ?? 'docs-scenario-helper',
      PLANR_PROFILE: process.env.PLANR_PROFILE ?? 'docs-scenario',
    },
  });
  assert.equal(result.status, 0, `${planrBin} ${args.join(' ')}\n${result.stderr}\n${result.stdout}`);
  return result.stdout.trim() ? JSON.parse(result.stdout) : null;
}

function runGit(args) {
  return spawnSync('git', args, {
    cwd: process.cwd(),
    encoding: 'utf8',
    env: {
      ...process.env,
      GIT_AUTHOR_DATE: '2026-07-29T00:00:00Z',
      GIT_COMMITTER_DATE: '2026-07-29T00:00:00Z',
    },
  });
}

function ensureGitRepository() {
  const existing = runGit(['rev-parse', '--is-inside-work-tree']);
  if (existing.status === 0) return;
  for (const args of [
    ['init', '--quiet'],
    ['add', '.'],
    ['-c', 'user.name=Planr Docs', '-c', 'user.email=docs@planr.invalid', 'commit', '--quiet', '-m', 'fixture baseline'],
  ]) {
    const result = runGit(args);
    assert.equal(result.status, 0, `git ${args.join(' ')}\n${result.stderr}\n${result.stdout}`);
  }
}

function capabilityInstance(manifestId) {
  const capabilities = run(['evidence', 'capability', 'list', '--json']);
  const capabilityObject = capabilities.object ?? capabilities;
  const probe = capabilityObject.registry.probes.find((entry) => entry.manifest_id === manifestId);
  const instance = capabilityObject.instances.find((entry) => entry.id === probe?.instance_id);
  assert.ok(instance, `missing capability instance for ${manifestId}`);
  return instance;
}

const definition = SCENARIOS[scenario];
const spec = scenarioSpec(scenario, { apiUrl });
ensureGitRepository();
const policyDigest = await writeEvidencePolicy(process.cwd(), [spec], { policyId: definition.policyId });
const plan = run(['plan', 'new', definition.planTitle, '--json']);
const instance = capabilityInstance(spec.id);
const obligationPayload = obligation({
  id: definition.obligationId,
  planId: plan.plan.id,
  policyDigest,
  spec,
  expected: definition.expected,
  environment: instance.capability.environment,
  configDigest: definition.configDigest,
});

await writeJson(definition.files[3], obligationPayload);
run(['evidence', 'obligation', 'add', '--input', definition.files[3], '--json']);
await writeJson(
  definition.files[4],
  scenarioRunInput({
    obligationId: definition.obligationId,
    capabilityInstanceId: instance.id,
    target: spec.target,
  }),
);

console.log(
  JSON.stringify(
    {
      scenario,
      files: definition.files,
      commands: [
        `planr evidence run --input ${definition.files[4]} --json`,
        `planr evidence coverage --scope obligation --id ${definition.obligationId} --json`,
      ],
    },
    null,
    2,
  ),
);
