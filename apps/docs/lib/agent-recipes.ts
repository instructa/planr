export const agentClientIds = ['codex', 'claude', 'cursor'] as const;

export type AgentClientId = (typeof agentClientIds)[number];

export type AgentArtifact = {
  path: string;
  owner: 'plugin' | 'planr install';
  purpose: string;
};

export type AgentPrompts = {
  first: string;
  planOnly: string;
  autonomousPreparation: string;
  generatedLoopHandoff: string;
  portableLoopHandoff: string;
  status: string;
  recovery: string;
  advancedPlan: string;
  advancedGoal: string;
};

export type AgentRecipe = {
  id: AgentClientId;
  displayName: string;
  logoPath: string;
  logoAlt: string;
  cardSummary: string;
  invocationLabel: string;
  pluginRequired: boolean;
  pluginSetup: readonly string[];
  projectInstallerCommand: `planr install ${AgentClientId}`;
  expectedArtifacts: readonly AgentArtifact[];
  reloadGuidance: string;
  integrationUrl: `/docs/integrations/${string}`;
  safetyRequirements: readonly string[];
  successReceiptFields: readonly string[];
  setupPrompt: string;
  nextPrompts: AgentPrompts;
};

export const setupReceiptFields = [
  'Detected client and repository root',
  'Resolved Planr binary path and version',
  'Existing or newly initialized Planr project',
  'Project integration paths changed or left unchanged',
  'Plugin and workflow-skill status',
  'Structured doctor result',
  'Reload or trust action still required',
  'Exact next $planr prompt',
] as const;

const safetyRequirements = [
  'Keep MCP, roles, skills, and hooks project-scoped by default.',
  'Preview the integration before applying it and explain every repository path.',
  'Preserve hand-edited files; never use --force or overwrite without explicit approval.',
  'Never apply global MCP configuration or a user-level deeplink without explicit approval.',
  'Initialize Planr only when project inspection proves that no project exists.',
  'Repeat setup safely: an existing project and current integration must remain unchanged.',
  'Run structured diagnostics before reporting success.',
] as const;

export const agentPrompts: AgentPrompts = {
  first: 'Use $planr. Inspect this repository and tell me the verified next step.',
  planOnly:
    'Use $planr. Create and check the smallest build plan for my request, build its map, and do not implement yet.',
  autonomousPreparation:
    'Use $planr-goal to prepare an autonomous goal for my request. Capture the outcome and observable proof, create and check the plan and map, store the goal contract, and return the plan-bound loop handoff. Do not implement during preparation.',
  generatedLoopHandoff:
    '/goal Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract). Continue until the contract holds or the iteration budget is exhausted.',
  portableLoopHandoff:
    'Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract). Continue until the contract holds or the iteration budget is exhausted.',
  status:
    'Use $planr. Report what is complete, ready, blocked, and in review for this repository. Read live Planr state and do not implement anything.',
  recovery:
    'Use $planr. Recover the interrupted Planr work in this repository. Inspect the map, active leases, reviews, approvals, logs, and recovery preview; report the safest verified next step before changing state.',
  advancedPlan:
    'Use $planr-plan. Create and check the smallest implementation-ready plan for my request, then name the next map command. Do not implement.',
  advancedGoal:
    'Use $planr-goal. Prepare my request as a durable autonomous goal with observable proof, a checked plan, a linked map, a stored goal contract, and a plan-bound loop handoff. Do not implement during preparation.',
};

function setupPrompt(input: {
  displayName: string;
  client: AgentClientId;
  pluginSteps: readonly string[];
  reloadGuidance: string;
}) {
  const pluginInstruction = input.pluginSteps.length
    ? `Install or verify the official Planr plugin using only these host commands:\n${input.pluginSteps.map((step) => `   - ${step}`).join('\n')}`
    : 'This client needs no separate plugin step; the project installer owns its Planr agents and skills.';

  return `Set up Planr for ${input.displayName} in this repository. Perform setup only; do not implement product work.

1. Detect the current host, repository root, OS, available package manager, Planr binary path/version, existing Planr project, and existing client configuration. If network access exists, consult the stable Planr Agent Bootstrap page; if it does not, continue with these self-contained steps.
2. If Planr is missing, install the CLI only through a trusted supported method (prefer \`brew install instructa/tap/planr\` when Homebrew is available, otherwise the official GitHub Release installer), then run \`planr --version\` and report the resolved binary path.
3. ${pluginInstruction}
4. Run \`planr project show --json\`. If and only if no Planr project exists, infer a concise name from the repository and run \`planr project init "<repository-name>"\` once, without a client flag. This creates Planr project state without writing client integration files. Never reinitialize an existing project.
5. Run \`planr install ${input.client} --dry-run\`. Explain every project path that would be reconciled. Then run \`planr install ${input.client}\` only for project-scoped, non-conflicting changes. Preserve hand edits; never use \`--force\`, global MCP configuration, or a user-level install/deeplink without my explicit approval. Re-running setup must be idempotent.
6. Run \`planr project show --json\` and \`planr doctor --client ${input.client} --json\`. Do not claim success unless the project is readable and diagnostics are reported honestly.
7. Return a compact setup receipt with: ${setupReceiptFields.join('; ')}.

Reload guidance: ${input.reloadGuidance}
Exact next prompt after setup: ${agentPrompts.first}`;
}

const recipes = {
  codex: {
    id: 'codex',
    displayName: 'Codex',
    logoPath: '/agents/codex.svg',
    logoAlt: 'Codex logo',
    cardSummary: 'Plugin skills, project MCP, and hooks',
    invocationLabel: '$planr',
    pluginRequired: true,
    pluginSetup: [
      'codex plugin marketplace add instructa/planr',
      'codex plugin add planr@planr',
    ],
    projectInstallerCommand: 'planr install codex',
    expectedArtifacts: [
      { path: 'Planr plugin: skills/*', owner: 'plugin', purpose: 'All ten workflow skills' },
      { path: '.planr/integrations/codex-mcp.toml', owner: 'planr install', purpose: 'Project MCP snippet' },
      { path: '.codex/hooks.json', owner: 'planr install', purpose: 'Fail-open session hook' },
    ],
    reloadGuidance: 'Start a fresh Codex session after plugin changes, then run /hooks once and trust the Planr hook entries.',
    integrationUrl: '/docs/integrations/codex',
    safetyRequirements,
    successReceiptFields: setupReceiptFields,
    setupPrompt: setupPrompt({
      displayName: 'Codex',
      client: 'codex',
      pluginSteps: [
        'codex plugin marketplace add instructa/planr',
        'codex plugin add planr@planr',
      ],
      reloadGuidance: 'Start a fresh Codex session after plugin changes, then run /hooks once and trust the Planr hook entries.',
    }),
    nextPrompts: agentPrompts,
  },
  claude: {
    id: 'claude',
    displayName: 'Claude Code',
    logoPath: '/agents/claude.svg',
    logoAlt: 'Claude logo',
    cardSummary: 'Plugin skills/agents, project MCP, roles, and hooks',
    invocationLabel: '/planr:planr',
    pluginRequired: true,
    pluginSetup: [
      '/plugin marketplace add instructa/planr',
      '/plugin install planr@planr',
    ],
    projectInstallerCommand: 'planr install claude',
    expectedArtifacts: [
      { path: 'Planr plugin: skills/* and agents/*', owner: 'plugin', purpose: 'Ten skills plus plugin worker/reviewer' },
      { path: '.mcp.json', owner: 'planr install', purpose: 'Project MCP configuration' },
      { path: '.claude/agents/planr-{worker,reviewer}.md', owner: 'planr install', purpose: 'Standalone project roles' },
      { path: '.claude/settings.json', owner: 'planr install', purpose: 'Fail-open SessionStart hook' },
    ],
    reloadGuidance: 'Start a fresh Claude Code session after installing the plugin or changing standalone project roles.',
    integrationUrl: '/docs/integrations/claude-code',
    safetyRequirements,
    successReceiptFields: setupReceiptFields,
    setupPrompt: setupPrompt({
      displayName: 'Claude Code',
      client: 'claude',
      pluginSteps: [
        '/plugin marketplace add instructa/planr',
        '/plugin install planr@planr',
      ],
      reloadGuidance: 'Start a fresh Claude Code session after installing the plugin or changing standalone project roles.',
    }),
    nextPrompts: agentPrompts,
  },
  cursor: {
    id: 'cursor',
    displayName: 'Cursor',
    logoPath: '/agents/cursor.svg',
    logoAlt: 'Cursor logo',
    cardSummary: 'Project MCP, agents, skills, and hooks',
    invocationLabel: '/planr',
    pluginRequired: false,
    pluginSetup: [],
    projectInstallerCommand: 'planr install cursor',
    expectedArtifacts: [
      { path: '.cursor/mcp.json', owner: 'planr install', purpose: 'Project MCP configuration' },
      { path: '.cursor/agents/planr-{worker,reviewer}.md', owner: 'planr install', purpose: 'Project worker/reviewer agents' },
      { path: '.cursor/skills/planr*/SKILL.md', owner: 'planr install', purpose: 'All ten workflow skills' },
      { path: '.cursor/hooks.json', owner: 'planr install', purpose: 'Session and evidence hooks' },
      { path: '.cursor/hooks/planr-evidence-guard.sh', owner: 'planr install', purpose: 'Fail-open evidence guard' },
    ],
    reloadGuidance: 'Reload the Cursor window after setup so newly installed project agents and skills are discovered.',
    integrationUrl: '/docs/integrations/cursor',
    safetyRequirements,
    successReceiptFields: setupReceiptFields,
    setupPrompt: setupPrompt({
      displayName: 'Cursor',
      client: 'cursor',
      pluginSteps: [],
      reloadGuidance: 'Reload the Cursor window after setup so newly installed project agents and skills are discovered.',
    }),
    nextPrompts: agentPrompts,
  },
} satisfies Record<AgentClientId, AgentRecipe>;

export const agentRecipes: Readonly<Record<AgentClientId, AgentRecipe>> = recipes;
export const agentRecipeList = agentClientIds.map((client) => agentRecipes[client]);

export function getAgentRecipe(client: AgentClientId) {
  return agentRecipes[client];
}
