import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { access, chmod, copyFile, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const verifierPath = fileURLToPath(import.meta.url);
const configuredPlanrBin = process.env.PLANR_BIN;
const sourcePlanrBin = configuredPlanrBin
  ? path.resolve(process.cwd(), configuredPlanrBin)
  : path.join(repositoryRoot, 'target', 'debug', 'planr');
const repositoryPackage = JSON.parse(await readFile(path.join(repositoryRoot, 'package.json'), 'utf8'));
const cargoManifest = await readFile(path.join(repositoryRoot, 'Cargo.toml'), 'utf8');
const cargoVersion = cargoManifest.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

assert.ok(cargoVersion, 'Could not read the Planr package version from Cargo.toml');
assert.equal(repositoryPackage.version, cargoVersion, 'package.json and Cargo.toml must declare the same Planr version');
assert.ok(path.isAbsolute(sourcePlanrBin), 'Onboarding replay must execute an absolute Planr binary path, never an ambient `planr` from PATH');
await access(
  sourcePlanrBin,
  constants.X_OK,
).catch(() => {
  throw new Error(`Configured Planr binary is not executable at ${sourcePlanrBin}. Run \`cargo build --bin planr\` or set PLANR_BIN to an explicit matching repository build; ambient PATH lookup is intentionally disabled.`);
});

const binaryGuardAssertions = [];
if (process.env.PLANR_SKIP_BINARY_GUARD_FIXTURES !== '1') {
  const fixtureRoot = await mkdtemp(path.join(tmpdir(), 'planr-binary-guard-'));
  try {
    const fixtureEnvironment = {
      ...process.env,
      PLANR_SKIP_BINARY_GUARD_FIXTURES: '1',
    };
    const missingPath = path.join(fixtureRoot, 'missing-planr');
    const missing = spawnSync(process.execPath, [verifierPath], {
      cwd: repositoryRoot,
      encoding: 'utf8',
      env: { ...fixtureEnvironment, PLANR_BIN: missingPath },
    });
    assert.notEqual(missing.status, 0, 'a missing explicit Planr binary must fail onboarding verification');
    assert.match(
      `${missing.stderr}\n${missing.stdout}`,
      /Configured Planr binary is not executable.*ambient PATH lookup is intentionally disabled/s,
      'missing binary failure must explain how to build or explicitly select the repository binary',
    );
    binaryGuardAssertions.push('missing explicit binary fails without falling back to ambient PATH');

    const stalePath = path.join(fixtureRoot, 'stale-planr');
    await writeFile(stalePath, "#!/usr/bin/env node\nconsole.log('planr 0.0.0');\n");
    await chmod(stalePath, 0o755);
    const stale = spawnSync(process.execPath, [verifierPath], {
      cwd: repositoryRoot,
      encoding: 'utf8',
      env: { ...fixtureEnvironment, PLANR_BIN: stalePath },
    });
    assert.notEqual(stale.status, 0, 'a stale explicit Planr binary must fail onboarding verification');
    assert.match(
      `${stale.stderr}\n${stale.stdout}`,
      /does not match this repository release/,
      'stale binary failure must identify the repository-version mismatch',
    );
    binaryGuardAssertions.push('stale explicit binary fails with an actionable repository-version mismatch');

    const setupFailureParent = path.join(fixtureRoot, 'setup-failure');
    await mkdir(setupFailureParent);
    const setupFailure = spawnSync(process.execPath, [verifierPath], {
      cwd: repositoryRoot,
      encoding: 'utf8',
      env: {
        ...fixtureEnvironment,
        PLANR_BIN: '/bin',
        TMPDIR: setupFailureParent,
      },
    });
    assert.notEqual(setupFailure.status, 0, 'a configured directory must fail private binary copy');
    assert.deepEqual(
      await readdir(setupFailureParent),
      [],
      'early private binary setup failure must remove its exact captured onboarding root',
    );
    binaryGuardAssertions.push('early private binary setup failure removes its exact captured fixture root');
  } finally {
    await rm(fixtureRoot, { recursive: true, force: true });
  }
}

const onboardingRoot = await mkdtemp(path.join(tmpdir(), 'planr-docs-onboarding-'));
const workspace = path.join(onboardingRoot, 'project');
const planrBin = path.join(onboardingRoot, 'bin', 'planr');
const report = {
  workspace,
  planrBinary: planrBin,
  expectedVersion: repositoryPackage.version,
  commands: [],
  assertions: [
    'replay binary path is absolute and cannot resolve an ambient planr from PATH',
    'replay uses a byte-identical read-only private binary with exactly one filesystem link',
    ...binaryGuardAssertions,
  ],
};

function initializeRepository() {
  const gitEnvironment = {
    ...process.env,
    GIT_AUTHOR_NAME: 'Planr Onboarding Fixture',
    GIT_AUTHOR_EMAIL: 'onboarding@example.invalid',
    GIT_AUTHOR_DATE: '2000-01-01T00:00:00Z',
    GIT_COMMITTER_NAME: 'Planr Onboarding Fixture',
    GIT_COMMITTER_EMAIL: 'onboarding@example.invalid',
    GIT_COMMITTER_DATE: '2000-01-01T00:00:00Z',
  };
  for (const args of [
    ['init', '--quiet', '--initial-branch=main'],
    [
      '-c', 'commit.gpgSign=false',
      '-c', 'core.hooksPath=/dev/null',
      'commit', '--quiet', '--allow-empty', '--message', 'Initialize onboarding fixture',
    ],
  ]) {
    const result = spawnSync('git', args, {
      cwd: workspace,
      encoding: 'utf8',
      env: gitEnvironment,
    });
    assert.equal(result.status, 0, `git ${args.join(' ')}\n${result.stderr || result.stdout}`);
  }
  report.assertions.push('temporary onboarding project starts from a deterministic local Git commit');
}

function run(args, { worker, json = true } = {}) {
  const command = `planr ${args.join(' ')}`;
  const result = spawnSync(planrBin, args, {
    cwd: workspace,
    encoding: 'utf8',
    env: worker ? { ...process.env, PLANR_WORKER_ID: worker } : process.env,
  });

  report.commands.push({ command, exitCode: result.status });
  assert.equal(result.status, 0, `${command}\n${result.stderr || result.stdout}`);
  return json ? JSON.parse(result.stdout) : result.stdout;
}

function check(condition, message) {
  assert.ok(condition, message);
  report.assertions.push(message);
}

try {
  await mkdir(path.dirname(planrBin), { recursive: true });
  await mkdir(workspace);
  await copyFile(sourcePlanrBin, planrBin, constants.COPYFILE_EXCL);
  await chmod(planrBin, 0o555);
  const sourceDigest = createHash('sha256').update(await readFile(sourcePlanrBin)).digest('hex');
  const privateDigest = createHash('sha256').update(await readFile(planrBin)).digest('hex');
  assert.equal(privateDigest, sourceDigest, 'Private onboarding binary must exactly match the configured repository build');
  assert.equal((await stat(planrBin)).nlink, 1, 'Private onboarding binary must have exactly one filesystem link');

  initializeRepository();
  const version = run(['--version'], { json: false });
  assert.equal(
    version.trim(),
    `planr ${repositoryPackage.version}`,
    `Replay binary ${planrBin} does not match this repository release`,
  );
  report.assertions.push('replay binary version exactly matches this repository release');

  const initialized = run(['project', 'init', 'Hello Planr', '--json']);
  check(initialized.project.name === 'Hello Planr', 'project init creates the named local project');
  const shownProject = run(['project', 'show', '--json']);
  check(shownProject.project.id === initialized.project.id, 'project show returns the initialized project');

  const product = run(['plan', 'new', 'Ship hello command', '--platform', 'cli', '--json']).plan;
  const productDir = product.path;
  await writeFile(
    path.join(productDir, 'PRODUCT_SPEC.md'),
    `# Product Specification\n\n## Problem\n\nNew users need a tiny command they can implement and verify.\n\n## Requirements\n\n- Add an executable hello command.\n- Verify its exact output.\n\n## Success Criteria\n\nRunning \`./hello.sh\` prints exactly \`hello from planr\`.\n`,
  );
  await writeFile(
    path.join(productDir, 'TASKS.md'),
    `# Tasks\n\n### TASK-001: Add the hello command\n\nCreate an executable \`hello.sh\` script.\n\n### TASK-002: Verify the hello command\n\nRun the script and record its exact output.\n`,
  );

  const productCheck = run(['plan', 'check', product.id, '--json']);
  check(productCheck.ok, 'product plan passes planr plan check');

  const build = run(['plan', 'split', product.id, '--slice', 'CLI MVP', '--json']).plan;
  await writeFile(
    build.path,
    `---\nname: ship-hello-command-cli-mvp\noverview: "Build and verify the smallest executable Planr example."\ntodos:\n  - id: add-command\n    content: "Add the hello command"\n    status: pending\n  - id: verify-command\n    content: "Verify the hello command"\n    status: pending\nisProject: false\nstage: build\nsource_plan: ${product.id}\nslice: "CLI MVP"\n---\n\n# Ship hello command - CLI MVP\n\n## Scope Decision\n\nShip a two-step shell-script example that demonstrates implementation and verification.\n\n## Ownership Target\n\nThe repository root owns the example script and its verification evidence.\n\n## Existing Leverage\n\nUse POSIX shell and Planr's standard map workflow.\n\n## Phase 1\n\n### TASK-001: Add the hello command\n\nCreate an executable \`hello.sh\` script that prints \`hello from planr\`.\n\n### TASK-002: Verify the hello command\n\nRun \`./hello.sh\` and record the successful output.\n\n## Out Of Scope\n\nPackaging, argument parsing, and platform-specific installers.\n\n## Verification\n\nRun \`./hello.sh\` and confirm the exact output \`hello from planr\`.\n\n## Acceptance Criteria\n\n- \`hello.sh\` is executable.\n- Running it prints exactly \`hello from planr\`.\n`,
  );

  const buildCheck = run(['plan', 'check', build.id, '--json']);
  check(buildCheck.ok, 'build plan passes planr plan check');

  const builtMap = run(['map', 'build', '--from', build.id, '--json']);
  check(builtMap.created.length === 2, 'map build creates two executable items');
  check(builtMap.links.length === 1 && builtMap.links[0].kind === 'blocks', 'map build preserves task order with a blocks link');

  const first = run(['pick', '--plan', build.id, '--work-type', 'code', '--json'], { worker: 'onboarding-builder' }).item;
  const scriptPath = path.join(workspace, 'hello.sh');
  await writeFile(scriptPath, "#!/bin/sh\nprintf '%s\\n' 'hello from planr'\n");
  await chmod(scriptPath, 0o755);
  run(
    ['done', first.id, '--summary', 'Added an executable hello command.', '--files', 'hello.sh', '--cmd', 'chmod +x hello.sh', '--tests', './hello.sh => hello from planr', '--json'],
    { worker: 'onboarding-builder' },
  );

  const second = run(['pick', '--plan', build.id, '--work-type', 'code', '--json'], { worker: 'onboarding-builder' }).item;
  const hello = spawnSync('./hello.sh', [], { cwd: workspace, encoding: 'utf8' });
  assert.equal(hello.status, 0);
  check(hello.stdout === 'hello from planr\n', 'documented hello command prints the exact expected output');
  const submitted = run(
    ['done', second.id, '--summary', 'Verified the documented command and exact output.', '--files', 'hello.sh', '--cmd', './hello.sh', '--tests', 'stdout equals hello from planr', '--escalate', 'user-requested', '--escalation-ref', 'onboarding-independent-review', '--escalation-explanation', 'The onboarding flow demonstrates an explicit independent ReviewGate.', '--json'],
    { worker: 'onboarding-builder' },
  );
  check(submitted.item.status === 'closed' && submitted.work_packet.transition === 'review_gate', 'canonical settlement closes the outcome and opens a ReviewGate');

  const recovery = run(['recover', 'sweep', '--json']);
  check(recovery.mode === 'preview' && recovery.stale.length === 0, 'recovery preview reports no stale work');

  await mkdir(path.join(workspace, '.planr', 'artifacts'), { recursive: true });
  const exported = run(['export', '--out', '.planr/artifacts/onboarding-example', '--include-plans', '--include-logs', '--json']);
  check(exported.out === '.planr/artifacts/onboarding-example', 'export packages plans and logs at the documented path');

  const reviewPacket = run(['pick', '--plan', build.id, '--work-type', 'review', '--json'], { worker: 'onboarding-reviewer' }).work_packet;
  check(reviewPacket.kind === 'review_gate', 'review pick returns the canonical typed ReviewGate packet');
  const review = reviewPacket.execution_state.review_gate;
  const reviewed = run(
    ['review', 'close', review.id, '--verdict', 'complete', '--reviewer', 'onboarding-reviewer', '--json'],
    { worker: 'onboarding-reviewer' },
  );
  check(reviewed.execution_state.review_gate.status === 'accepted', 'independent review accepts the durable ReviewGate');

  const finalMap = run(['map', 'show', '--plan', build.id, '--json']);
  check(finalMap.counts.closed === 2 && finalMap.settled === finalMap.total, 'the tutorial ends with two settled outcome items and no review map item');

  for (const client of ['codex', 'claude', 'cursor']) {
    const preview = run(['install', client, '--dry-run'], { json: false });
    check(preview.includes('planr'), `${client} dry-run prints a Planr MCP setup preview`);
  }
  const doctor = run(['doctor', '--client', 'all', '--json']);
  check(
    doctor.db_status === 'pass' &&
      doctor.clients.map(({ client }) => client).join(',') === 'codex,claude,cursor' &&
      doctor.clients.every(({ status }) => status === 'pass' || status === 'not_installed'),
    'doctor validates the project and reports every supported first-party client',
  );

  const exportedManifest = await readFile(path.join(workspace, '.planr', 'artifacts', 'onboarding-example'), 'utf8');
  check(exportedManifest.includes(product.id), 'export manifest references the tutorial product plan');

  console.log(JSON.stringify({ ok: true, ...report }, null, 2));
} finally {
  await rm(onboardingRoot, { recursive: true, force: true });
}
