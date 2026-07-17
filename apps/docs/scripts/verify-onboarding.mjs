import assert from 'node:assert/strict';
import { access, chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const configuredPlanrBin = process.env.PLANR_BIN;
const planrBin = configuredPlanrBin
  ? path.resolve(process.cwd(), configuredPlanrBin)
  : path.join(repositoryRoot, 'target', 'debug', 'planr');
const repositoryPackage = JSON.parse(await readFile(path.join(repositoryRoot, 'package.json'), 'utf8'));
const cargoManifest = await readFile(path.join(repositoryRoot, 'Cargo.toml'), 'utf8');
const cargoVersion = cargoManifest.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

assert.ok(cargoVersion, 'Could not read the Planr package version from Cargo.toml');
assert.equal(repositoryPackage.version, cargoVersion, 'package.json and Cargo.toml must declare the same Planr version');
await access(
  planrBin,
  constants.X_OK,
).catch(() => {
  throw new Error(`Repository Planr binary is not executable at ${planrBin}. Run \`cargo build --bin planr\` or set PLANR_BIN to an explicit repository build.`);
});

const workspace = await mkdtemp(path.join(tmpdir(), 'planr-docs-onboarding-'));
const report = {
  workspace,
  planrBinary: planrBin,
  expectedVersion: repositoryPackage.version,
  commands: [],
  assertions: [],
};

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
    ['done', second.id, '--summary', 'Verified the documented command and exact output.', '--files', 'hello.sh', '--cmd', './hello.sh', '--tests', 'stdout equals hello from planr', '--review', '--json'],
    { worker: 'onboarding-builder' },
  );
  check(submitted.item.status === 'in_review', 'review-gated completion moves the target into review');

  const recovery = run(['recover', 'sweep', '--json']);
  check(recovery.mode === 'preview' && recovery.stale.length === 0, 'recovery preview reports no stale work');

  await mkdir(path.join(workspace, '.planr', 'artifacts'), { recursive: true });
  const exported = run(['export', '--out', '.planr/artifacts/onboarding-example', '--include-plans', '--include-logs', '--json']);
  check(exported.out === '.planr/artifacts/onboarding-example', 'export packages plans and logs at the documented path');

  const review = run(['pick', '--plan', build.id, '--work-type', 'review', '--json'], { worker: 'onboarding-reviewer' }).item;
  const reviewed = run(
    ['review', 'close', review.id, '--verdict', 'complete', '--findings', 'Command, output, executable bit, and evidence match the plan.', '--reviewer', 'onboarding-reviewer', '--close-target', '--json'],
    { worker: 'onboarding-reviewer' },
  );
  check(reviewed.closed_target.id === second.id, 'independent review closes the reviewed target');

  const finalMap = run(['map', 'show', '--plan', build.id, '--json']);
  check(finalMap.counts.closed === 3 && finalMap.settled === finalMap.total, 'the tutorial ends with every map item settled and closed');

  for (const client of ['codex', 'claude', 'cursor']) {
    const preview = run(['install', client, '--dry-run'], { json: false });
    check(preview.includes('planr'), `${client} dry-run prints a Planr MCP setup preview`);
  }
  const doctor = run(['doctor', '--client', 'all', '--json']);
  check(doctor.db_status === 'pass' && doctor.clients.every(({ status }) => status === 'pass'), 'doctor validates the project and all supported first-party clients');

  const exportedManifest = await readFile(path.join(workspace, '.planr', 'artifacts', 'onboarding-example'), 'utf8');
  check(exportedManifest.includes(product.id), 'export manifest references the tutorial product plan');

  console.log(JSON.stringify({ ok: true, ...report }, null, 2));
} finally {
  await rm(workspace, { recursive: true, force: true });
}
