import assert from 'node:assert/strict';
import { access, readFile, writeFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const outputPath = path.join(docsRoot, 'content', 'docs', 'reference', 'cli-generated.mdx');
const planrBin = process.env.PLANR_BIN
  ? path.resolve(process.cwd(), process.env.PLANR_BIN)
  : path.join(repositoryRoot, 'target', 'debug', 'planr');
const checkOnly = process.argv.includes('--check');

await access(planrBin, constants.X_OK).catch(() => {
  throw new Error(`Planr binary is not executable at ${planrBin}. Run \`cargo build --bin planr\` or set PLANR_BIN.`);
});

function help(commandPath) {
  const result = spawnSync(planrBin, [...commandPath, '--help'], { encoding: 'utf8' });
  assert.equal(result.status, 0, `${planrBin} ${commandPath.join(' ')} --help\n${result.stderr}`);
  return result.stdout.trimEnd();
}

function subcommands(output) {
  const section = output.match(/\nCommands:\n([\s\S]*?)(?=\nOptions:|\nArguments:|$)/)?.[1] ?? '';
  return [...section.matchAll(/^  ([a-z][a-z0-9-]*)(?:[ \t]|$)/gm)]
    .map((match) => match[1])
    .filter((name) => name !== 'help');
}

const entries = [];
function visit(commandPath, depth = 0) {
  const output = help(commandPath);
  entries.push({ commandPath, output });
  if (depth >= 3) return;
  for (const child of subcommands(output)) visit([...commandPath, child], depth + 1);
}
visit([]);

const packageJson = JSON.parse(await readFile(path.join(repositoryRoot, 'package.json'), 'utf8'));
const body = [
  '---',
  'title: Generated CLI Reference',
  `description: Complete compiled Planr ${packageJson.version} command and option help, checked against the repository binary.`,
  '---',
  '',
  '<Callout type="info" title="Generated executable contract">',
  '  This page is generated from the current repository binary. Run `pnpm docs:reference:generate` after changing the CLI; CI fails if the committed help is stale.',
  '</Callout>',
  '',
  'Global options apply where shown. Prefer `--json` for automation; human output is intended for terminals.',
  '',
];
for (const { commandPath, output } of entries) {
  const level = Math.min(2 + commandPath.length, 5);
  const label = commandPath.length === 0 ? 'planr' : `planr ${commandPath.join(' ')}`;
  body.push(`${'#'.repeat(level)} \`${label}\``, '', '```text', output, '```', '');
}
const generated = `${body.join('\n').replace(/[ \t]+$/gm, '').trimEnd()}\n`;

if (checkOnly) {
  const current = await readFile(outputPath, 'utf8').catch(() => '');
  assert.equal(current, generated, 'Generated CLI reference is stale. Run `pnpm docs:reference:generate`.');
  console.log(`cli_reference_check=passed commands=${entries.length} version=${packageJson.version}`);
} else {
  await writeFile(outputPath, generated);
  console.log(`cli_reference_generated=${outputPath} commands=${entries.length} version=${packageJson.version}`);
}
