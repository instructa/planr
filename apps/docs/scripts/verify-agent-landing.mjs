import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { agentClientIds, agentRecipes } from '../lib/agent-recipes.ts';

const docsRoot = path.resolve(import.meta.dirname, '..');
const landing = await readFile(path.join(docsRoot, 'app', 'page.tsx'), 'utf8');
const panel = await readFile(path.join(docsRoot, 'components', 'agent-setup-panel.tsx'), 'utf8');

for (const marker of [
  "href=\"#agent-setup\"",
  'Set up with your agent',
  'Install manually',
  '<AgentSetupPanel />',
  'agentRecipeList.map((recipe) =>',
  'href="/docs/getting-started/quickstart"',
  'href="/docs/guides"',
  'href="/docs/integrations"',
]) {
  assert(landing.includes(marker), `landing page is missing ${marker}`);
}

for (const marker of [
  'agentClientIds.map((client) =>',
  'data-agent-copy={client}',
  'const prompt = agentRecipes[client].setupPrompt',
  "status: 'idle' | 'copied' | 'error'",
  "document.execCommand('copy')",
  "if (!copied) throw new Error",
  'role="status"',
  'aria-live="polite"',
  '/docs/agents/quickstart.md',
  '<noscript>',
  '/docs/integrations/grok-build',
  '/docs/getting-started/installation',
]) {
  assert(panel.includes(marker), `agent setup panel is missing ${marker}`);
}

for (const forbidden of [
  'role="tablist"',
  'role="tab"',
  'role="tabpanel"',
  '<CommandBlock',
  'Safety boundary',
  'Required success receipt',
  'recipe.safetyRequirements',
  'recipe.successReceiptFields',
  'Set up Planr for Codex in this repository.',
]) {
  assert(!panel.includes(forbidden), `compact agent setup panel must not render ${forbidden}`);
}
assert.equal(
  panel.match(/setupPrompt/g)?.length,
  1,
  'the canonical setupPrompt must be consumed only once by the clipboard path',
);
assert(
  panel.includes('const prompt = agentRecipes[client].setupPrompt;'),
  'the clipboard path must read setupPrompt directly from the typed recipe',
);
assert.deepEqual(agentClientIds, ['codex', 'claude', 'cursor', 'grok']);
for (const client of agentClientIds) {
  assert(agentRecipes[client].setupPrompt.length > 1_000, `${client} setup prompt is not self-contained`);
  assert(agentRecipes[client].successReceiptFields.length >= 8, `${client} receipt is incomplete`);
  assert(agentRecipes[client].safetyRequirements.length >= 7, `${client} safety contract is incomplete`);
}

console.log(`agent_landing_contract=passed compact_actions=${agentClientIds.length} canonical_prompts=${agentClientIds.length} visible_prompt_bodies=0 visible_contract_blocks=0`);
