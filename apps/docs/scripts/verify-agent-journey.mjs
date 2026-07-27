import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { agentPrompts, piAgentPrompts } from '../lib/agent-recipes.ts';

const docsRoot = path.resolve(import.meta.dirname, '..');
const contentRoot = path.join(docsRoot, 'content', 'docs');
const forbiddenUnscopedDriver = /\/goal Use \$planr(?!-loop)/;

async function read(relativePath) {
  return readFile(path.join(contentRoot, relativePath), 'utf8');
}

async function collectMdx(directory = contentRoot) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = await Promise.all(
    entries.map(async (entry) => {
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) return collectMdx(absolute);
      return entry.isFile() && entry.name.endsWith('.mdx') ? [absolute] : [];
    }),
  );
  return files.flat();
}

const rootMeta = JSON.parse(await read('meta.json'));
const agentMeta = JSON.parse(await read('agents/meta.json'));
assert.deepEqual(agentMeta.pages, ['index', 'quickstart', 'prompt-recipes', 'skills']);
assert.equal(rootMeta.pages[rootMeta.pages.indexOf('getting-started') + 1], 'agents');
for (const preserved of ['getting-started', 'integrations', 'concepts', 'guides', 'reference']) {
  assert(rootMeta.pages.includes(preserved), `human navigation lost ${preserved}`);
}

assert.match(agentPrompts.first, /^Use \$planr\./);
assert.match(agentPrompts.planOnly, /do not implement/i);
assert.match(agentPrompts.autonomousPreparation, /Do not implement during preparation/);
assert.match(agentPrompts.generatedLoopHandoff, /^\/goal Use \$planr-loop on plan <plan-id>\./);
assert.match(agentPrompts.generatedLoopHandoff, /goal-contract/);
assert.match(agentPrompts.generatedLoopHandoff, /iteration budget/);
assert.match(agentPrompts.portableLoopHandoff, /^Use \$planr-loop on plan <plan-id>\./);
assert.match(agentPrompts.status, /do not implement anything/);
assert.match(agentPrompts.recovery, /report the safest verified next step before changing state/);
assert.match(agentPrompts.advancedPlan, /^Use \$planr-plan\./);
assert.match(agentPrompts.advancedGoal, /^Use \$planr-goal\./);
assert(!forbiddenUnscopedDriver.test(JSON.stringify(agentPrompts)));
assert.match(piAgentPrompts.first, /^Use \/skill:planr\./);
assert.match(piAgentPrompts.planOnly, /do not implement/i);
assert.match(piAgentPrompts.generatedLoopHandoff, /^Use \/skill:planr-loop on plan <plan-id>\./);
assert.match(piAgentPrompts.advancedGoal, /^Use \/skill:planr-goal\./);
assert(!JSON.stringify(piAgentPrompts).includes('/goal '));

const agentIndex = await read('agents/index.mdx');
for (const route of ['/docs/agents/quickstart', '/docs/agents/prompt-recipes', '/docs/agents/skills']) {
  assert(agentIndex.includes(route), `For Agents index omits ${route}`);
}

const quickstart = await read('agents/quickstart.mdx');
for (const client of ['codex', 'claude', 'cursor', 'pi']) {
  assert(quickstart.includes(`<AgentRecipe client="${client}" />`));
}
assert(quickstart.includes('<PromptBlock prompt="first"'));
assert.match(quickstart, /Start with the right entry/);
assert.match(quickstart, /planr doctor --client <client> --json/);

const recipes = await read('agents/prompt-recipes.mdx');
for (const prompt of Object.keys(agentPrompts)) {
  assert(recipes.includes(`prompt="${prompt}"`), `prompt recipes omit typed prompt ${prompt}`);
}
for (const marker of [
  'Preparation compiles durable state; it does not implement.',
  'Use the real plan ID returned by preparation.',
  '`$planr-plan` creates or refines product/build plans',
  '`$planr-goal` is preparation-only.',
  '`$planr-loop` is the execution protocol',
]) {
  assert(recipes.includes(marker), `prompt recipes omit boundary: ${marker}`);
}

const skills = await read('agents/skills.mdx');
assert.match(skills, /\$planr` is the public router/);
for (const skill of ['$planr-plan', '$planr-goal', '$planr-loop', '$planr-status']) {
  assert(skills.includes(skill), `skills page omits ${skill}`);
}

for (const page of ['codex', 'claude-code', 'cursor', 'pi']) {
  const integration = await read(`integrations/${page}.mdx`);
  assert(integration.includes('/docs/agents/prompt-recipes'), `${page} lacks the canonical prompt link`);
}
const troubleshooting = await read('troubleshooting.mdx');
for (const route of ['/docs/agents/quickstart', '/docs/agents/skills', '/docs/agents/prompt-recipes']) {
  assert(troubleshooting.includes(route), `troubleshooting omits ${route}`);
}

for (const file of await collectMdx()) {
  const content = await readFile(file, 'utf8');
  assert(!forbiddenUnscopedDriver.test(content), `${path.relative(contentRoot, file)} recommends an unscoped host driver`);
}

console.log(`agent_journey_contract=passed prompts=${Object.keys(agentPrompts).length} pages=4 clients=4`);
