import assert from 'node:assert/strict';
import { access, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const outputPath = path.join(docsRoot, 'content', 'docs', 'reference', 'mcp-schemas-generated.mdx');
const planrBin = process.env.PLANR_BIN
  ? path.resolve(process.cwd(), process.env.PLANR_BIN)
  : path.join(repositoryRoot, 'target', 'debug', 'planr');
const checkOnly = process.argv.includes('--check');
const workspace = await mkdtemp(path.join(tmpdir(), 'planr-mcp-reference-'));

await access(planrBin, constants.X_OK);

try {
  const initialized = spawnSync(planrBin, ['project', 'init', 'MCP schema generation', '--json'], { cwd: workspace, encoding: 'utf8' });
  assert.equal(initialized.status, 0, initialized.stderr);
  const request = `${JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list', params: {} })}\n`;
  const result = spawnSync(planrBin, ['mcp'], { cwd: workspace, encoding: 'utf8', input: request });
  assert.equal(result.status, 0, result.stderr);
  const tools = JSON.parse(result.stdout).result.tools;
  const packageJson = JSON.parse(await readFile(path.join(repositoryRoot, 'package.json'), 'utf8'));
  const body = [
    '---',
    'title: Generated MCP Tool Schemas',
    `description: Complete Planr ${packageJson.version} tools/list inventory with every property, type, description, required field, enum/default declaration, and additionalProperties rule.`,
    '---',
    '',
    '<Callout type="info" title="Generated runtime contract">',
    '  This page is generated from live `tools/list`. Optional fields are every property absent from `required`; defaults and enums appear exactly where the runtime schema declares them. CI compares the complete schema JSON byte-for-byte.',
    '</Callout>',
    '',
  ];
  for (const tool of tools) {
    body.push(`## \`${tool.name}\``, '', tool.description, '', '```json', JSON.stringify(tool.inputSchema, null, 2), '```', '');
  }
  const generated = `${body.join('\n').replace(/[ \t]+$/gm, '').trimEnd()}\n`;
  if (checkOnly) {
    const current = await readFile(outputPath, 'utf8').catch(() => '');
    assert.equal(current, generated, 'Generated MCP schema reference is stale. Run `pnpm docs:reference:generate`.');
    console.log(`mcp_schema_reference_check=passed tools=${tools.length} version=${packageJson.version}`);
  } else {
    await writeFile(outputPath, generated);
    console.log(`mcp_schema_reference_generated=${outputPath} tools=${tools.length} version=${packageJson.version}`);
  }
} finally {
  await rm(workspace, { recursive: true, force: true });
}
