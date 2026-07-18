import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readdir, readFile } from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import process from 'node:process';

const appRoot = path.resolve(import.meta.dirname, '..');
const contentRoot = path.join(appRoot, 'content', 'docs');
const wranglerBin = path.join(appRoot, 'node_modules', 'wrangler', 'bin', 'wrangler.js');

async function collectMdxFiles(directory = contentRoot) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map(async (entry) => {
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) return collectMdxFiles(absolute);
      return entry.isFile() && entry.name.endsWith('.mdx') ? [absolute] : [];
    }),
  );
  return nested.flat();
}

async function availablePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      assert(address && typeof address === 'object');
      server.close((error) => (error ? reject(error) : resolve(address.port)));
    });
  });
}

async function waitForServer(baseUrl, child, output) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    if (child.exitCode !== null) throw new Error(`Wrangler exited early.\n${output.join('')}`);
    try {
      const response = await fetch(`${baseUrl}/llms.txt`);
      if (response.ok) return;
    } catch {
      // The production-shaped static server is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Timed out waiting for Wrangler.\n${output.join('')}`);
}

function pageContract(file) {
  const relative = path.relative(contentRoot, file).replaceAll(path.sep, '/');
  const withoutExtension = relative.slice(0, -'.mdx'.length);
  const slugs = withoutExtension.split('/').filter((part) => part !== 'index');
  const canonicalPath = slugs.length ? `/docs/${slugs.join('/')}` : '/docs';
  const markdownPath = `/docs/${slugs.length ? slugs.join('/') : 'index'}.md`;
  return { canonicalPath, file, markdownPath };
}

async function main() {
  const contracts = (await collectMdxFiles()).map(pageContract);
  assert(contracts.length > 0, 'No MDX pages were discovered.');

  const port = await availablePort();
  const baseUrl = `http://127.0.0.1:${port}`;
  const output = [];
  const child = spawn(process.execPath, [
    wranglerBin,
    'dev',
    '--config',
    'wrangler.jsonc',
    '--port',
    String(port),
    '--local',
  ], {
    cwd: appRoot,
    env: { ...process.env, NEXT_TELEMETRY_DISABLED: '1' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  child.stdout.on('data', (chunk) => output.push(chunk.toString()));
  child.stderr.on('data', (chunk) => output.push(chunk.toString()));

  try {
    await waitForServer(baseUrl, child, output);

    const compactResponse = await fetch(`${baseUrl}/llms.txt`);
    const compact = await compactResponse.text();
    assert.match(compactResponse.headers.get('content-type') ?? '', /^text\/markdown/);
    assert(compact.indexOf('Agent Quickstart') < compact.indexOf('Installation'));
    assert.match(compact, /\/llms-full\.txt/);
    assert(!compact.includes('# Planr complete documentation corpus'));

    const fullResponse = await fetch(`${baseUrl}/llms-full.txt`);
    const full = await fullResponse.text();
    assert.match(fullResponse.headers.get('content-type') ?? '', /^text\/markdown/);
    assert.match(full, /^# Planr complete documentation corpus/);

    for (const contract of contracts) {
      const sourceText = await readFile(contract.file, 'utf8');
      const title = sourceText.match(/^title:\s*(.+)$/m)?.[1]?.trim();
      assert(title, `Missing title in ${contract.file}`);

      const response = await fetch(`${baseUrl}${contract.markdownPath}`);
      const markdown = await response.text();
      assert.equal(response.status, 200, `${contract.markdownPath} did not resolve`);
      assert.match(response.headers.get('content-type') ?? '', /^text\/markdown/);
      assert(markdown.startsWith(`# ${title}\n`), `${contract.markdownPath} lost its title`);
      assert(
        markdown.includes(`Canonical page: https://planr.so${contract.canonicalPath}`),
        `${contract.markdownPath} lost its canonical URL`,
      );
      assert(!markdown.includes('\0'), `${contract.markdownPath} leaked a placeholder`);
      assert(
        !/^<(AgentRecipe|Callout|Cards?|CommandBlock|PathCard|PromptBlock)\b/m.test(markdown),
        `${contract.markdownPath} leaked a raw MDX component`,
      );
      assert(full.includes(`Markdown: https://planr.so${contract.markdownPath}`));
    }

    const quickstart = await (await fetch(`${baseUrl}/docs/getting-started/quickstart.md`)).text();
    assert.match(quickstart, /planr project init "Hello Planr"/);
    assert.match(quickstart, /planr doctor --client all --json/);

    const codex = await (await fetch(`${baseUrl}/docs/integrations/codex.md`)).text();
    assert.match(codex, /Copyable setup prompt/);
    assert.match(codex, /planr install codex --dry-run/);
    assert(!codex.includes('<AgentRecipe'));

    const agentQuickstart = await (await fetch(`${baseUrl}/docs/agents/quickstart.md`)).text();
    assert.match(agentQuickstart, /Use \$planr\. Inspect this repository/);
    assert.match(agentQuickstart, /planr install (codex|claude|cursor) --dry-run/);
    assert(!agentQuickstart.includes('<PromptBlock'));

    const promptRecipes = await (await fetch(`${baseUrl}/docs/agents/prompt-recipes.md`)).text();
    assert.match(promptRecipes, /\/goal Use \$planr-loop on plan <plan-id>/);
    assert.match(promptRecipes, /Use \$planr-goal\. Prepare my request/);
    assert(!promptRecipes.includes('<PromptBlock'));

    const html = await (await fetch(`${baseUrl}/docs/agents/quickstart`)).text();
    assert.match(html, /Copy Markdown/);
    assert.match(html, /\/docs\/agents\/quickstart\.md/);

    console.log(`Verified ${contracts.length} processed Markdown routes, both corpus indexes, and page actions.`);
  } finally {
    child.kill('SIGTERM');
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      new Promise((resolve) => setTimeout(resolve, 2_000)),
    ]);
  }
}

await main();
