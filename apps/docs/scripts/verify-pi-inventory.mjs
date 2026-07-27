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
    'planr project init [--client codex|claude|cursor|grok|pi|all]',
    'planr doctor [--client codex|claude|cursor|grok|pi|all]',
    'planr install codex|claude|cursor|grok|pi',
    'PI_CODING_AGENT=true',
  ]],
  ['.planr/plans/product/planr/README.md', ['explicitly opted-in Grok Build and Pi']],
  ['.planr/plans/product/planr/AI_SPEC.md', ['explicitly opted-in Grok Build and Pi']],
  ['apps/docs/content/docs/index.mdx', ['explicitly opted-in Grok Build and Pi']],
  ['apps/docs/content/docs/getting-started/index.mdx', ['explicitly opted-in Grok Build or Pi']],
  ['apps/docs/content/docs/faq.mdx', ['explicitly opted-in Grok Build and Pi']],
  ['apps/docs/content/docs/getting-started/full-lifecycle.mdx', ['explicitly opted-in Grok Build or Pi']],
  ['apps/docs/content/docs/agents/index.mdx', ['/skill:planr']],
  ['apps/docs/content/docs/agents/quickstart.mdx', [
    '<AgentRecipe client="pi" />',
    'Pi setup is an explicit repository opt-in',
  ]],
  ['apps/docs/content/docs/getting-started/why-planr.mdx', ['explicitly opted-in Grok Build and Pi']],
  ['apps/docs/content/docs/concepts/local-first-model.mdx', ['explicitly opted-in Grok Build and Pi']],
  ['apps/docs/content/docs/integrations/pi.mdx', [
    '<AgentRecipe client="pi" />',
    'not included by `--client all`',
    'no `.pi/settings.json`',
    'PI_CODING_AGENT=true',
    'Use /skill:planr.',
  ]],
  ['apps/docs/content/docs/reference/configuration-and-storage.mdx', [
    'PI_CODING_AGENT',
    '.pi/skills/',
  ]],
  ['apps/docs/content/docs/reference/support-matrix.mdx', [
    '| Pi |',
    'planr doctor --client pi',
  ]],
  ['apps/docs/lib/agent-recipes.ts', [
    "id: 'pi'",
    "projectInstallerCommand: 'planr install pi'",
    "integrationUrl: '/docs/integrations/pi'",
    "invocationLabel: '/skill:planr'",
  ]],
  ['apps/docs/components/agent-setup-panel.tsx', [
    'href="/docs/integrations/pi">Pi</Link>',
  ]],
  ['apps/docs/scripts/verify-release-readiness.mjs', [
    'explicitly opted-in Grok Build and Pi',
    'seven integration routes',
  ]],
]);

for (const [relative, fragments] of required) {
  const body = await source(relative);
  for (const fragment of fragments) {
    assert.ok(body.includes(fragment), `${relative} is missing Pi inventory fragment: ${fragment}`);
  }
}

console.log(`pi_inventory_check=passed surfaces=${required.size}`);
