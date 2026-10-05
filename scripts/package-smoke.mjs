import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { root, checkVersions } from './versions.mjs';

const temporary = mkdtempSync(join(tmpdir(), 'planr-smoke-'));
const run = (command, args, cwd = root) => {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8' });
  console.log(`$ ${command} ${args.join(' ')} (exit ${result.status})`);
  if (result.stderr) {
    process.stderr.write(result.stderr);
  }
  if (result.error || result.status !== 0) {
    throw Error(result.error?.message ?? result.stdout ?? `${command} failed`);
  }
  return result.stdout;
};

try {
  const expected = checkVersions();
  const [packed] = JSON.parse(run('npm', ['pack', '--json', '--pack-destination', temporary]));
  const manifests = ['plugin.json', '.claude-plugin/plugin.json'];
  const allowed = /^(skills\/planr\/|docs\/|(?:README\.md|LICENSE|CHANGELOG\.md|package\.json)$)/;
  for (const { path } of packed.files) {
    assert(allowed.test(path) || manifests.includes(path), `unexpected package file: ${path}`);
    assert(!path.split('/').some(part => part === '.DS_Store' || part.startsWith('._')), path);
  }
  for (const file of manifests) {
    assert(packed.files.some(entry => entry.path === file), `missing plugin manifest: ${file}`);
  }
  const prefix = join(temporary, 'prefix');
  console.log(run('npm', [
    'install', '--global', '--prefix', prefix, '--offline', '--no-audit', '--no-fund',
    join(temporary, packed.filename),
  ]).trim());
  const cli = join(prefix, 'bin', 'planr');
  const actual = run(cli, ['--version']);
  assert.equal(actual, `planr ${expected}\n`);
  console.log(actual.trim());
  const help = run(cli, ['--help']);
  assert.equal(run(cli, []), help);
  for (const command of ['check', 'index', 'list', 'next', 'set', 'goal', 'board', 'init']) {
    assert(help.includes(command), `help is missing ${command}`);
  }
  console.log(help.trim());
  const planning = join(temporary, 'planning');
  mkdirSync(planning);
  run('git', ['init', '--quiet'], planning);
  run(cli, ['init'], planning);
  console.log(run(cli, ['check'], planning).trim());
  console.log(run(cli, ['next', '--goal', 'example'], planning).trim());
  console.log('planr: package smoke passed');
} catch (error) {
  console.error(`planr: ${error.message}`);
  process.exitCode = 1;
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
