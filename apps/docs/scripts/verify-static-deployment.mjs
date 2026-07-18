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

console.log('static_deployment_verification=passed');
console.log(`html_pages=${docsHtml.length} markdown_pages=${publicMarkdown.length} redirects=${redirects.length}`);
console.log(`assets=${outputFiles.length} search_bytes=${(await readFile(path.join(outputRoot, 'api', 'search'))).byteLength}`);
