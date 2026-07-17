import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const temporaryRoot = await mkdtemp(path.join(tmpdir(), 'planr-docs-clean-install-'));
const installRoot = path.join(temporaryRoot, 'install');
const outputDir = path.join(repositoryRoot, '.planr', 'artifacts', 'docs-release-readiness');
const packageJson = JSON.parse(await readFile(path.join(repositoryRoot, 'package.json'), 'utf8'));

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    encoding: 'utf8',
    env: process.env,
    ...options,
  });
  assert.equal(result.status, 0, `${command} ${args.join(' ')}\n${result.stderr || result.stdout}`);
  return result;
}

try {
  await mkdir(installRoot, { recursive: true });
  run('cargo', ['install', '--path', repositoryRoot, '--locked', '--root', installRoot]);
  const planrBin = path.join(installRoot, 'bin', 'planr');
  const version = run(planrBin, ['--version']).stdout.trim();
  assert.equal(version, `planr ${packageJson.version}`, 'cleanly installed binary version does not match the repository release');

  const replay = run(process.execPath, [path.join(docsRoot, 'scripts', 'verify-onboarding.mjs')], {
    env: { ...process.env, PLANR_BIN: planrBin },
  });
  const report = JSON.parse(replay.stdout);
  assert.equal(report.ok, true);
  assert.ok(report.commands.length >= 20 && report.assertions.length >= 18, 'clean install replay did not exercise the full lifecycle');

  await mkdir(outputDir, { recursive: true });
  const reportPath = path.join(outputDir, 'clean-install-report.json');
  await writeFile(reportPath, `${JSON.stringify({
    ok: true,
    version,
    install: 'cargo install --path <repository> --locked --root <temporary-prefix>',
    commands: report.commands,
    assertions: report.assertions,
  }, null, 2)}\n`);

  console.log('clean_install_verification=passed');
  console.log(`version=${version} lifecycle_commands=${report.commands.length} assertions=${report.assertions.length}`);
  console.log('install=cargo install --path <repository> --locked --root <temporary-prefix>');
  console.log(`report=${reportPath}`);
} finally {
  await rm(temporaryRoot, { recursive: true, force: true });
}
