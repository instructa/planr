import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));

async function source(relative) {
  return readFile(path.join(repositoryRoot, relative), 'utf8');
}

const required = new Map([
  ['.planr/plans/product/planr/API_AND_DATA_MODEL.md', [
    'planr project init [--client codex|claude|cursor|grok|all]',
    'planr doctor [--client codex|claude|cursor|grok|all]',
    'planr install codex|claude|cursor|grok',
    'planr prompt cli|mcp|http [--client codex|claude|cursor|grok|all]',
  ]],
  ['.planr/plans/product/planr/README.md', ['explicitly opted-in Grok Build']],
  ['.planr/plans/product/planr/AI_SPEC.md', ['explicitly opted-in Grok Build']],
  ['apps/docs/content/docs/index.mdx', ['explicitly opted-in Grok Build']],
  ['apps/docs/content/docs/getting-started/index.mdx', ['explicitly opted-in Grok Build']],
  ['apps/docs/content/docs/faq.mdx', ['explicitly opted-in Grok Build']],
  ['apps/docs/content/docs/getting-started/full-lifecycle.mdx', ['explicitly opted-in Grok Build']],
  ['apps/docs/content/docs/agents/index.mdx', ['explicitly opted-in Grok Build']],
  ['apps/docs/content/docs/getting-started/why-planr.mdx', ['explicitly opted-in Grok Build']],
  ['apps/docs/content/docs/concepts/local-first-model.mdx', ['explicitly opted-in Grok Build']],
  ['apps/docs/content/docs/integrations/grok-build.mdx', [
    'not included by `--client all`',
    'Grok has no Planr hooks in v1',
  ]],
  ['apps/docs/scripts/verify-release-readiness.mjs', [
    'explicitly opted-in Grok Build',
    'six integration routes',
  ]],
]);

for (const [relative, fragments] of required) {
  const body = await source(relative);
  for (const fragment of fragments) {
    assert.ok(body.includes(fragment), `${relative} is missing Grok inventory fragment: ${fragment}`);
  }
}

console.log(`grok_inventory_check=passed surfaces=${required.size}`);
