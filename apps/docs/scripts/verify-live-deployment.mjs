#!/usr/bin/env node
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

export const LIVE_DOCS_ORACLE = Object.freeze([
  { path: '/', type: 'text/html', markers: ['Planr'] },
  { path: '/docs', type: 'text/html', markers: ['Planr Documentation'] },
  { path: '/docs/getting-started/installation', type: 'text/html', markers: ['Installation'] },
  { path: '/docs/getting-started/installation.md', type: 'text/markdown', markers: ['Installation'] },
  { path: '/api/search', type: 'application/json', markers: ['getting-started/installation'] },
]);

export async function verifyLiveDeployment(origin, {
  fetchImpl = fetch,
  timeoutMs = 10_000,
  routes = LIVE_DOCS_ORACLE,
} = {}) {
  const base = new URL(origin);
  assert.equal(base.protocol, 'https:', 'live docs origin must use HTTPS');
  assert.equal(base.pathname, '/', 'live docs origin must not contain a path');
  const observations = [];

  for (const route of routes) {
    const url = new URL(route.path, base);
    const response = await fetchImpl(url, {
      redirect: 'follow',
      signal: AbortSignal.timeout(timeoutMs),
      headers: { 'user-agent': 'planr-docs-promotion-oracle/1' },
    });
    assert.equal(response.status, 200, `${route.path} returned ${response.status}`);
    const contentType = response.headers.get('content-type') ?? '';
    assert.ok(contentType.toLowerCase().startsWith(route.type), `${route.path} returned ${contentType}, expected ${route.type}`);
    const body = await response.text();
    for (const marker of route.markers) assert.ok(body.includes(marker), `${route.path} omits ${marker}`);
    observations.push({ path: route.path, status: response.status, contentType, bytes: Buffer.byteLength(body) });
  }

  return observations;
}

async function main() {
  const urlIndex = process.argv.indexOf('--url');
  const origin = urlIndex >= 0 ? process.argv[urlIndex + 1] : process.env.PLANR_DOCS_URL;
  if (!origin) throw new Error('usage: verify-live-deployment.mjs --url https://HOST');
  const observations = await verifyLiveDeployment(origin);
  console.log(JSON.stringify({ verdict: 'pass', origin, routes: observations }, null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  });
}
