import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import {
  agentClientIds,
  agentRecipeList,
  agentRecipes,
  setupReceiptFields,
} from '../lib/agent-recipes.ts';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const planrBin = process.env.PLANR_BIN
  ? path.resolve(process.cwd(), process.env.PLANR_BIN)
  : path.join(repositoryRoot, 'target', 'debug', 'planr');
const expectedClients = ['codex', 'claude', 'cursor'];
const forbiddenDefaultGoal = /\/goal Use \$planr(?!-loop)/;

assert.deepEqual(agentClientIds, expectedClients, 'the first-party client union must stay exhaustive');
assert.deepEqual(Object.keys(agentRecipes), expectedClients, 'every client id needs exactly one recipe');
assert.deepEqual(agentRecipeList.map(({ id }) => id), expectedClients, 'ordered UI recipes must match the typed record');

for (const client of agentClientIds) {
  const recipe = agentRecipes[client];
  assert.equal(recipe.id, client);
  assert.equal(recipe.projectInstallerCommand, `planr install ${client}`);
  assert.ok(recipe.integrationUrl.startsWith('/docs/integrations/'));
  assert.ok(recipe.expectedArtifacts.length >= 3, `${client} must declare its complete artifact handoff`);
  assert.deepEqual(recipe.successReceiptFields, setupReceiptFields);
  assert.ok(recipe.reloadGuidance.length > 30, `${client} must state actionable reload guidance`);

  for (const required of [
    'planr --version',
    'continue with these self-contained steps',
    'planr project show --json',
    'planr project init "<repository-name>"',
    'without a client flag',
    `planr install ${client} --dry-run`,
    `planr install ${client}`,
    `planr doctor --client ${client} --json`,
    'Perform setup only; do not implement product work.',
    'Never reinitialize an existing project.',
    'without my explicit approval',
    'Re-running setup must be idempotent.',
    'Exact next prompt after setup: Use $planr.',
  ]) {
    assert.ok(recipe.setupPrompt.includes(required), `${client} setup prompt is missing: ${required}`);
  }
  for (const pluginStep of recipe.pluginSetup) {
    assert.ok(recipe.setupPrompt.includes(pluginStep), `${client} setup prompt omits plugin command: ${pluginStep}`);
  }
  for (const receiptField of setupReceiptFields) {
    assert.ok(recipe.setupPrompt.includes(receiptField), `${client} setup prompt omits receipt field: ${receiptField}`);
  }
  const safety = recipe.safetyRequirements.join(' ').toLowerCase();
  assert.ok(safety.includes('project-scoped by default'), `${client} must default to project scope`);
  assert.ok(safety.includes('never use --force or overwrite without explicit approval'), `${client} must gate overwrites`);
  assert.ok(safety.includes('never apply global mcp configuration'), `${client} must gate global configuration`);
  assert.ok(!forbiddenDefaultGoal.test(recipe.setupPrompt), `${client} setup prompt exposes an invalid /goal driver`);
  assert.ok(!recipe.setupPrompt.includes('claude mcp add'), `${client} setup prompt must not install user-global MCP`);
  assert.ok(!recipe.setupPrompt.includes('cursor://'), `${client} setup prompt must not use the user-level deeplink`);
  assert.ok(recipe.setupPrompt.includes('never use `--force`'), `${client} must gate overwrite behavior`);
  assert.ok(recipe.nextPrompts.first.startsWith('Use $planr.'), `${client} must default to the public router`);
  assert.ok(!forbiddenDefaultGoal.test(JSON.stringify(recipe.nextPrompts)));

  const integrationSlug = recipe.integrationUrl.split('/').at(-1);
  const integration = await readFile(
    path.join(docsRoot, 'content', 'docs', 'integrations', `${integrationSlug}.mdx`),
    'utf8',
  );
  assert.ok(integration.includes(`<AgentRecipe client="${client}" />`), `${client} integration must render the canonical recipe`);
  assert.ok(!integration.includes(`planr install ${client} --dry-run`), `${client} integration must not duplicate canonical setup commands`);
}

const landing = await readFile(path.join(docsRoot, 'app', 'page.tsx'), 'utf8');
assert.ok(landing.includes("import { agentRecipeList } from '@/lib/agent-recipes'"));
assert.ok(landing.includes('agentRecipeList.map((recipe) =>'), 'landing client cards must consume the canonical recipes');

await access(planrBin, constants.X_OK).catch(() => {
  throw new Error(`Repository Planr binary is not executable at ${planrBin}; build it before recipe verification`);
});
assert.ok(path.isAbsolute(planrBin), 'recipe verification must use an absolute repository binary, never ambient PATH');
const integrationSentinels = {
  codex: ['.planr/integrations/codex-mcp.toml', '.codex/hooks.json'],
  claude: ['.mcp.json', '.claude/agents/planr-worker.md', '.claude/agents/planr-reviewer.md', '.claude/settings.json'],
  cursor: [
    '.cursor/mcp.json',
    '.cursor/agents/planr-worker.md',
    '.cursor/agents/planr-reviewer.md',
    '.cursor/skills/planr/SKILL.md',
    '.cursor/skills/planr-goal/SKILL.md',
    '.cursor/skills/planr-loop/SKILL.md',
    '.cursor/skills/planr-verify-web/SKILL.md',
    '.cursor/skills/planr-task-graph/SKILL.md',
    '.cursor/skills/planr-plan/SKILL.md',
    '.cursor/skills/planr-work/SKILL.md',
    '.cursor/skills/planr-review/SKILL.md',
    '.cursor/skills/planr-status/SKILL.md',
    '.cursor/skills/planr-summary/SKILL.md',
    '.cursor/hooks.json',
    '.cursor/hooks/planr-evidence-guard.sh',
  ],
};
const hookPaths = {
  codex: '.codex/hooks.json',
  claude: '.claude/settings.json',
  cursor: '.cursor/hooks.json',
};

function run(workspace, client, args, { expectSuccess = true } = {}) {
  const result = spawnSync(planrBin, args, { cwd: workspace, encoding: 'utf8' });
  if (expectSuccess) {
    assert.equal(result.status, 0, `${client}: ${planrBin} ${args.join(' ')}\n${result.stderr || result.stdout}`);
  } else {
    assert.notEqual(result.status, 0, `${client}: ${args.join(' ')} unexpectedly succeeded`);
  }
  return result;
}

for (const client of agentClientIds) {
  const workspace = await mkdtemp(path.join(tmpdir(), `planr-recipe-${client}-`));
  const conflictingWorkspace = await mkdtemp(path.join(tmpdir(), `planr-recipe-conflict-${client}-`));
  try {
    run(workspace, client, ['project', 'show', '--json'], { expectSuccess: false });
    const initialized = JSON.parse(run(workspace, client, ['project', 'init', 'Recipe Contract', '--json']).stdout);

    const preview = run(workspace, client, ['install', client, '--dry-run']).stdout;
    for (const sentinel of integrationSentinels[client]) {
      assert.ok(preview.includes(sentinel), `${client}: dry-run must preview ${sentinel}`);
      await assert.rejects(
        access(path.join(workspace, sentinel)),
        `${client}: ${sentinel} must not exist before or after the dry-run`,
      );
    }

    run(workspace, client, ['install', client]);
    const firstInstall = new Map();
    for (const sentinel of integrationSentinels[client]) {
      firstInstall.set(sentinel, await readFile(path.join(workspace, sentinel)));
    }

    run(workspace, client, ['install', client]);
    for (const [sentinel, firstContent] of firstInstall) {
      assert.deepEqual(
        await readFile(path.join(workspace, sentinel)),
        firstContent,
        `${client}: re-running setup must leave current ${sentinel} unchanged`,
      );
    }

    const existing = JSON.parse(run(workspace, client, ['project', 'show', '--json']).stdout);
    assert.equal(existing.project.id, initialized.project.id, `${client}: existing setup must preserve the initialized project`);
    const doctor = JSON.parse(run(workspace, client, ['doctor', '--client', client, '--json']).stdout);
    assert.equal(doctor.db_status, 'pass', `${client}: doctor must validate the initialized project database`);
    assert.deepEqual(doctor.clients.map(({ client: reported }) => reported), [client]);
    assert.ok(
      doctor.clients.every(({ status }) => status === 'pass' || status === 'not_installed'),
      `${client}: doctor must report an honest supported status`,
    );

    run(conflictingWorkspace, client, ['project', 'init', 'Conflicting Recipe Contract', '--json']);
    const hookPath = path.join(conflictingWorkspace, hookPaths[client]);
    const conflictingContent = `{ user-owned-${client}-configuration\n`;
    await mkdir(path.dirname(hookPath), { recursive: true });
    await writeFile(hookPath, conflictingContent);
    const conflict = run(conflictingWorkspace, client, ['install', client]);
    assert.match(
      conflict.stdout,
      /exists but is not a JSON object planr can merge into; hooks skipped/,
      `${client}: conflicting hand-edited hooks must produce actionable preservation output`,
    );
    assert.equal(
      await readFile(hookPath, 'utf8'),
      conflictingContent,
      `${client}: conflicting hand-edited hooks must be preserved byte-for-byte`,
    );
  } finally {
    await Promise.all([
      rm(workspace, { recursive: true, force: true }),
      rm(conflictingWorkspace, { recursive: true, force: true }),
    ]);
  }
}

const normalizedSnapshot = JSON.stringify(agentRecipes);
const snapshotHash = createHash('sha256').update(normalizedSnapshot).digest('hex');
assert.equal(
  snapshotHash,
  'e0f84e111a8690904b11336937aba0c4a156a242d4c25adfe0880aed09588759',
  `agent recipe snapshot changed (${snapshotHash}); inspect the full client contract before accepting it`,
);

console.log(`agent_recipe_contract=passed clients=${agentClientIds.length} fresh=${agentClientIds.length} existing=${agentClientIds.length} conflicts_preserved=${agentClientIds.length} doctors=${agentClientIds.length} snapshot=${snapshotHash}`);
