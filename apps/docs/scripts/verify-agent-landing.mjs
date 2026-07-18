import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { agentClientIds, agentRecipes } from '../lib/agent-recipes.ts';

const docsRoot = path.resolve(import.meta.dirname, '..');
const landing = await readFile(path.join(docsRoot, 'app', 'page.tsx'), 'utf8');
const panel = await readFile(path.join(docsRoot, 'components', 'agent-setup-panel.tsx'), 'utf8');
const commandBlock = await readFile(path.join(docsRoot, 'components', 'command-block.tsx'), 'utf8');

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
  'role="tablist"',
  'role="tab"',
  'aria-selected={selected}',
  'role="tabpanel"',
  'id={`agent-panel-${client}`}',
  'aria-labelledby={`agent-tab-${client}`}',
  'data-agent-setup-panel={client}',
  'hidden={!selected}',
  "event.key === 'ArrowRight'",
  "event.key === 'ArrowLeft'",
  "event.key === 'Home'",
  "event.key === 'End'",
  'command={recipe.setupPrompt}',
  'recipe.safetyRequirements',
  'recipe.successReceiptFields',
  '/docs/agents/quickstart.md',
  '<noscript>',
  '/docs/getting-started/installation',
]) {
  assert(panel.includes(marker), `agent setup panel is missing ${marker}`);
}

assert(!panel.includes('Set up Planr for Codex in this repository.'), 'panel must not copy the canonical prompt into JSX');
assert.deepEqual(agentClientIds, ['codex', 'claude', 'cursor']);
for (const client of agentClientIds) {
  assert(agentRecipes[client].setupPrompt.length > 1_000, `${client} setup prompt is not self-contained`);
  assert(agentRecipes[client].successReceiptFields.length >= 8, `${client} receipt is incomplete`);
  assert(agentRecipes[client].safetyRequirements.length >= 7, `${client} safety contract is incomplete`);
}

for (const marker of [
  "'idle' | 'copied' | 'error'",
  "setCopyState('error')",
  "document.execCommand('copy')",
  "if (!copied) throw new Error",
  'role="status"',
  'aria-live="polite"',
  "'Copy failed'",
]) {
  assert(commandBlock.includes(marker), `copy control is missing ${marker}`);
}

console.log(`agent_landing_contract=passed clients=${agentClientIds.length} canonical_prompts=${agentClientIds.length} receipt_fields=${agentRecipes.codex.successReceiptFields.length}`);
