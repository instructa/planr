import { renderPlaceholder } from 'fumadocs-core/mdx-plugins/remark-llms.runtime';
import {
  agentClientIds,
  agentPrompts,
  getAgentRecipe,
  type AgentClientId,
} from '@/lib/agent-recipes';
import { source } from '@/lib/source';

const siteOrigin = 'https://planr.so';

type SourcePage = ReturnType<typeof source.getPages>[number];
type MarkdownData = SourcePage['data'] & {
  _markdown?: string;
  _exports?: { _markdown?: string };
};

function attribute(data: { attributes: Record<string, unknown> }, name: string) {
  const value = data.attributes[name];
  if (typeof value === 'string') return value;
  if (!value || typeof value !== 'object' || !('data' in value)) return undefined;

  const expression = value as {
    data?: { estree?: { body?: Array<{ expression?: { value?: unknown } }> } };
  };
  const literal = expression.data?.estree?.body?.[0]?.expression?.value;
  return typeof literal === 'string' ? literal : undefined;
}

function markdownLink(label: string, href: string) {
  return `[${label.replaceAll('[', '\\[').replaceAll(']', '\\]')}](${href})`;
}

const placeholderRenderers = {
  AgentRecipe: (data: { attributes: Record<string, unknown> }) => {
    const client = attribute(data, 'client');
    if (!client || !agentClientIds.includes(client as AgentClientId)) return '';

    const recipe = getAgentRecipe(client as AgentClientId);
    const pluginSteps = recipe.pluginSetup.length
      ? recipe.pluginSetup.map((step) => `- \`${step}\``).join('\n')
      : '- No separate plugin step is required.';
    const artifacts = recipe.expectedArtifacts
      .map((artifact) => `- \`${artifact.path}\` — ${artifact.purpose} (${artifact.owner})`)
      .join('\n');

    return `## ${recipe.displayName} agent setup

${recipe.cardSummary}.

### Install and configure

${pluginSteps}

- Preview: \`${recipe.projectInstallerCommand} --dry-run\`
- Apply: \`${recipe.projectInstallerCommand}\`
- Verify: \`planr doctor --client ${recipe.id} --json\`

### Expected project artifacts

${artifacts}

### Reload guidance

${recipe.reloadGuidance}

### Copyable setup prompt

\`\`\`text
${recipe.setupPrompt}
\`\`\`

### First Planr prompt

\`\`\`text
${recipe.nextPrompts.first}
\`\`\`
`;
  },
  Callout: (data: { attributes: Record<string, unknown>; children: string }) => {
    const title = attribute(data, 'title');
    return `${title ? `> **${title}**\n>\n` : ''}${data.children
      .trim()
      .split('\n')
      .map((line) => `> ${line}`)
      .join('\n')}\n`;
  },
  Card: (data: { attributes: Record<string, unknown>; children: string }) => {
    const title = attribute(data, 'title') ?? data.children.trim();
    const href = attribute(data, 'href');
    const description = attribute(data, 'description');
    if (!href) return data.children;
    return `- ${markdownLink(title, href)}${description ? ` — ${description}` : ''}\n`;
  },
  Cards: (data: { children: string }) => `\n${data.children.trim()}\n`,
  CommandBlock: (data: { attributes: Record<string, unknown> }) => {
    const command = attribute(data, 'command');
    const label = attribute(data, 'label');
    if (!command) return '';
    return `${label ? `**${label}**\n\n` : ''}\`\`\`sh\n${command}\n\`\`\`\n`;
  },
  PathCard: (data: { attributes: Record<string, unknown>; children: string }) => {
    const title = attribute(data, 'title') ?? data.children.trim();
    const href = attribute(data, 'href');
    const description = attribute(data, 'description');
    if (!href) return data.children;
    return `- ${markdownLink(title, href)}${description ? ` — ${description}` : ''}\n`;
  },
  PromptBlock: (data: { attributes: Record<string, unknown> }) => {
    const prompt = attribute(data, 'prompt');
    const label = attribute(data, 'label');
    if (!prompt || !(prompt in agentPrompts)) return '';
    return `${label ? `**${label}**\n\n` : ''}\`\`\`text\n${agentPrompts[prompt as keyof typeof agentPrompts]}\n\`\`\`\n`;
  },
};

export function markdownPathForSlugs(slugs: string[]) {
  return `/docs/${slugs.length ? slugs.join('/') : 'index'}.md`;
}

export function markdownPathForPage(page: SourcePage) {
  return markdownPathForSlugs(page.slugs);
}

export function canonicalPathForPage(page: SourcePage) {
  return page.slugs.length ? `/docs/${page.slugs.join('/')}` : '/docs';
}

export async function getLLMText(page: SourcePage) {
  const data = page.data as MarkdownData;
  const processed = data._markdown ?? data._exports?._markdown;
  if (!processed) throw new Error(`Processed Markdown is missing for ${page.path}`);

  const canonicalUrl = `${siteOrigin}${canonicalPathForPage(page)}`;
  const markdownUrl = `${siteOrigin}${markdownPathForPage(page)}`;
  const body = (await renderPlaceholder(processed, placeholderRenderers)).trim();

  return `# ${page.data.title}

> ${page.data.description}

- Canonical page: ${canonicalUrl}
- Markdown: ${markdownUrl}

${body}
`;
}

export function getLLMIndex() {
  return `# Planr documentation

Planr is a local-first planning and execution coordination tool for coding agents. This compact index points agents to the highest-value entry pages. Use the full corpus only when broader retrieval is necessary.

## Start here

- [Agent Quickstart](${siteOrigin}/docs/agents/quickstart.md) — let Codex, Claude Code, Cursor, Grok Build, or Pi set up Planr safely and reach the first runtime-native Planr prompt.
- [Installation](${siteOrigin}/docs/getting-started/installation.md) — supported installation paths and binary verification.
- [Choose your agent interface](${siteOrigin}/docs/getting-started/choose-your-interface.md) — Codex, Claude Code, Cursor, Grok Build, Pi, generic MCP, or CLI-only.
- [Full lifecycle](${siteOrigin}/docs/getting-started/full-lifecycle.md) — idea through evidence-backed closure.

## Agent integrations

- [Codex](${siteOrigin}/docs/integrations/codex.md)
- [Claude Code](${siteOrigin}/docs/integrations/claude-code.md)
- [Cursor](${siteOrigin}/docs/integrations/cursor.md)
- [Grok Build](${siteOrigin}/docs/integrations/grok-build.md)
- [Pi](${siteOrigin}/docs/integrations/pi.md)
- [Generic MCP](${siteOrigin}/docs/integrations/generic-mcp.md)

## Complete retrieval

- [Full documentation corpus](${siteOrigin}/llms-full.txt)
- Every canonical documentation page offers Copy Markdown and Open Markdown actions.
`;
}

export async function getLLMFullText() {
  const pages = [...source.getPages()].sort((left, right) =>
    canonicalPathForPage(left).localeCompare(canonicalPathForPage(right)),
  );
  const rendered = await Promise.all(pages.map((page) => getLLMText(page)));
  return `# Planr complete documentation corpus\n\n${rendered.join('\n\n---\n\n')}`;
}

export function markdownResponse(body: string) {
  return new Response(body, {
    headers: {
      'Cache-Control': 'public, max-age=0, must-revalidate',
      'Content-Type': 'text/markdown; charset=utf-8',
    },
  });
}
