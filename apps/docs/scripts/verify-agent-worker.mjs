import assert from 'node:assert/strict';
import { access, readdir, readFile } from 'node:fs/promises';
import path from 'node:path';

const appRoot = path.resolve(import.meta.dirname, '..');
const workerPath = path.join(appRoot, '.open-next', 'worker.js');
const bundlePath = path.join(appRoot, '.alchemy-worker', 'worker.js');
const manifestPath = path.join(
  appRoot,
  '.open-next',
  'server-functions',
  'default',
  'apps',
  'docs',
  '.next',
  'server',
  'app-paths-manifest.json',
);
const cachePath = path.join(appRoot, '.open-next', 'cache');
const contentPath = path.join(appRoot, 'content', 'docs');

await access(workerPath);
await access(bundlePath);

const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
for (const route of [
  '/llms.txt/route',
  '/llms-full.txt/route',
  '/api/markdown/[...slug]/route',
]) {
  assert(route in manifest, `OpenNext app manifest is missing ${route}`);
}

const cacheFiles = await readdir(cachePath, { recursive: true });
const contentFiles = await readdir(contentPath, { recursive: true });
assert(cacheFiles.some((file) => file.endsWith('/llms.txt.cache')));
assert(cacheFiles.some((file) => file.endsWith('/llms-full.txt.cache')));
assert.equal(
  cacheFiles.filter((file) => file.includes('/api/markdown/') && file.endsWith('.cache')).length,
  contentFiles.filter((file) => file.endsWith('.mdx')).length,
  'OpenNext did not prerender every Markdown page',
);

console.log('Verified agent-readable routes in the OpenNext worker and Wrangler dry-run bundle.');
