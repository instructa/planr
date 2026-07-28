import assert from 'node:assert/strict';
import { access, readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { legacyRedirects } from '../redirects.mjs';

const appRoot = path.resolve(import.meta.dirname, '..');
const outputRoot = path.join(appRoot, 'out');
const contentRoot = path.join(appRoot, 'content', 'docs');

async function filesUnder(root) {
  return (await readdir(root, { recursive: true })).filter((file) => !file.endsWith('.DS_Store'));
}

for (const file of [
  'index.html',
  'docs.html',
  '404.html',
  'llms.txt',
  'llms-full.txt',
  'sitemap.xml',
  'robots.txt',
  'manifest.webmanifest',
  'api/search',
  'docs/index.md',
  '_headers',
  '_redirects',
]) {
  await access(path.join(outputRoot, file));
}

const contentFiles = (await filesUnder(contentRoot)).filter((file) => file.endsWith('.mdx'));
const outputFiles = await filesUnder(outputRoot);
const apiMarkdown = outputFiles.filter((file) => file.startsWith('api/markdown/') && file.endsWith('.md'));
const publicMarkdown = outputFiles.filter((file) => file.startsWith('docs/') && file.endsWith('.md'));
const docsHtml = outputFiles.filter((file) => file === 'docs.html' || (file.startsWith('docs/') && file.endsWith('.html')));

assert.equal(apiMarkdown.length, contentFiles.length, 'static API Markdown count must match authored pages');
assert.equal(publicMarkdown.length, contentFiles.length, 'public Markdown count must match authored pages');
assert.equal(docsHtml.length, contentFiles.length, 'static docs HTML count must match authored pages');

const agentQuickstart = await readFile(path.join(outputRoot, 'docs', 'agents', 'quickstart.md'), 'utf8');
for (const route of [
  '/docs/integrations/codex',
  '/docs/integrations/claude-code',
  '/docs/integrations/cursor',
  '/docs/integrations/grok-build',
  '/docs/integrations/pi',
]) {
  assert.ok(agentQuickstart.includes(route), `static agent quickstart omits ${route}`);
}
assert.ok(!/planr install (codex|claude|cursor|grok)/.test(agentQuickstart), 'static agent quickstart duplicated a runtime setup recipe');

const grokGuide = await readFile(path.join(outputRoot, 'docs', 'integrations', 'grok-build.md'), 'utf8');
for (const marker of ['Planr 1.8.0', 'planr install grok', 'no xAI credential is required']) {
  assert.ok(grokGuide.includes(marker), `static Grok guide omits ${marker}`);
}

const piGuide = await readFile(path.join(outputRoot, 'docs', 'integrations', 'pi.md'), 'utf8');
for (const marker of ['planr install pi', '/skill:planr', 'PI_CODING_AGENT=true']) {
  assert.ok(piGuide.includes(marker), `static Pi guide omits ${marker}`);
}

const search = JSON.parse(await readFile(path.join(outputRoot, 'api', 'search'), 'utf8'));
assert.equal(search.type, 'advanced', 'search export must be the advanced Orama index');
for (const route of ['/docs/getting-started/installation', '/docs/agents/quickstart', '/docs/reference/mcp-schemas-generated']) {
  assert.ok(JSON.stringify(search).includes(route), `search export omits ${route}`);
}

const headers = await readFile(path.join(outputRoot, '_headers'), 'utf8');
for (const marker of ['/api/search', '/docs/*.md', 'text/markdown', 'application/json']) {
  assert.ok(headers.includes(marker), `static headers omit ${marker}`);
}

const redirects = (await readFile(path.join(outputRoot, '_redirects'), 'utf8')).trim().split('\n');
assert.equal(redirects.length, legacyRedirects.length, 'static redirect count must match canonical inventory');
for (const { source, destination } of legacyRedirects) {
  assert.ok(redirects.includes(`${source} ${destination} 308`), `static redirects omit ${source}`);
}

const wrangler = JSON.parse(await readFile(path.join(appRoot, 'wrangler.jsonc'), 'utf8'));
assert.equal(wrangler.main, 'worker.mjs', 'Wrangler must deploy the redirect and agent MIME worker');
const workerFirst = wrangler.assets.run_worker_first;
const redirectRoutes = legacyRedirects.map(({ source }) => source);
assert.deepEqual(
  workerFirst.slice(0, redirectRoutes.length),
  redirectRoutes,
  'Wrangler worker-first redirect routes must match the canonical inventory in order',
);
assert.deepEqual(
  workerFirst.slice(redirectRoutes.length),
  ['/docs/*.md', '/api/search', '/llms.txt', '/llms-full.txt'],
  'only agent MIME routes may join legacy redirects in worker-first routing',
);

const worker = await readFile(path.join(appRoot, 'worker.mjs'), 'utf8');
for (const marker of [
  "from './redirects.mjs'",
  "status: 308",
  "`${destination}${url.search}`",
  "env.ASSETS.fetch(request)",
  "'text/markdown; charset=utf-8'",
  "'application/json; charset=utf-8'",
]) {
  assert.ok(worker.includes(marker), `edge worker omits ${marker}`);
}

console.log('static_deployment_verification=passed');
console.log(`html_pages=${docsHtml.length} markdown_pages=${publicMarkdown.length} redirects=${redirects.length}`);
console.log(`assets=${outputFiles.length} search_bytes=${(await readFile(path.join(outputRoot, 'api', 'search'))).byteLength}`);
