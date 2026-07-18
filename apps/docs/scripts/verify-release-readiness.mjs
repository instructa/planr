import assert from 'node:assert/strict';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { legacyRedirects } from '../redirects.mjs';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const contentRoot = path.join(docsRoot, 'content', 'docs');
const live = process.argv.includes('--live');
const baseUrl = process.env.PLANR_DOCS_URL ?? 'http://localhost:3000';
const outputDir = path.join(repositoryRoot, '.planr', 'artifacts', 'docs-release-readiness');

function routeFor(file) {
  const slug = path.relative(contentRoot, file).split(path.sep).join('/').replace(/\.mdx$/, '');
  if (slug === 'index') return '/docs';
  return `/docs/${slug.replace(/\/index$/, '')}`;
}

function headingSlug(heading) {
  return heading
    .replace(/<[^>]+>/g, '')
    .replace(/[`*_~]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, '')
    .trim()
    .replace(/\s+/g, '-')
    .replace(/-+/g, '-');
}

async function collectPages(directory) {
  const pages = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) pages.push(...await collectPages(file));
    else if (entry.name.endsWith('.mdx')) {
      const content = await readFile(file, 'utf8');
      const frontmatterEnd = content.indexOf('\n---\n', 4);
      assert.ok(content.startsWith('---\n') && frontmatterEnd > 0, `${routeFor(file)} has malformed frontmatter`);
      const frontmatter = content.slice(0, frontmatterEnd);
      const title = frontmatter.match(/^title:\s*(.+)$/m)?.[1];
      const description = frontmatter.match(/^description:\s*(.+)$/m)?.[1];
      assert.ok(title && description, `${routeFor(file)} needs title and description`);
      const slugCounts = new Map();
      const anchorList = [...content.matchAll(/^#{2,6}\s+(.+)$/gm)].map((match) => {
        const base = headingSlug(match[1]);
        const count = slugCounts.get(base) ?? 0;
        slugCounts.set(base, count + 1);
        return count === 0 ? base : `${base}-${count}`;
      });
      const explicitAnchors = [...content.matchAll(/<a\s+id=["']([^"']+)["']/g)].map((match) => match[1]);
      const allAnchors = [...anchorList, ...explicitAnchors];
      assert.equal(new Set(allAnchors).size, allAnchors.length, `${routeFor(file)} has colliding generated or explicit anchors`);
      pages.push({ file, route: routeFor(file), title, content, anchors: new Set(allAnchors) });
    }
  }
  return pages;
}

async function declaredRoutes() {
  const routes = new Set();
  const root = JSON.parse(await readFile(path.join(contentRoot, 'meta.json'), 'utf8'));
  for (const slug of root.pages) {
    if (slug === 'index') {
      routes.add('/docs');
      continue;
    }
    try {
      const section = JSON.parse(await readFile(path.join(contentRoot, slug, 'meta.json'), 'utf8'));
      for (const page of section.pages) routes.add(page === 'index' ? `/docs/${slug}` : `/docs/${slug}/${page}`);
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
      routes.add(`/docs/${slug}`);
    }
  }
  return routes;
}

function internalLinks(page) {
  const links = [];
  for (const match of page.content.matchAll(/\]\(((?:\/docs|#)[^)\s]+)\)/g)) links.push(match[1]);
  for (const match of page.content.matchAll(/(?:href|url)=["'](\/docs[^"']*)["']/g)) links.push(match[1]);
  return links;
}

const pages = await collectPages(contentRoot);
const routeMap = new Map(pages.map((page) => [page.route, page]));
assert.equal(routeMap.size, 59, 'release inventory must contain exactly 59 unique MDX routes');
const navigation = await declaredRoutes();
assert.deepEqual([...navigation].sort(), [...routeMap.keys()].sort(), 'explicit navigation and MDX route inventory drifted');

let linkCount = 0;
for (const page of pages) {
  assert.doesNotMatch(page.content, /\b(?:TODO|TBD|FIXME)\b|lorem ipsum|under construction|coming soon/i, `${page.route} contains unfinished copy`);
  for (const target of internalLinks(page)) {
    linkCount += 1;
    const [targetRoute, anchor] = target.startsWith('#') ? [page.route, target.slice(1)] : target.split('#');
    const destination = routeMap.get(targetRoute);
    assert.ok(destination, `${page.route} links to missing or retired route ${target}`);
    if (anchor) assert.ok(destination.anchors.has(anchor), `${page.route} links to missing anchor ${target}`);
  }
}

const redirectSources = new Set(legacyRedirects.map(({ source }) => source));
assert.equal(redirectSources.size, legacyRedirects.length, 'redirect sources must be unique');
for (const { source, destination, permanent } of legacyRedirects) {
  assert.equal(permanent, true, `${source} is not permanent`);
  assert.ok(!routeMap.has(source), `${source} collides with a current page`);
  assert.ok(routeMap.has(destination), `${source} targets missing page ${destination}`);
}

const coverage = await readFile(path.join(repositoryRoot, 'docs', 'documentation', 'COVERAGE.md'), 'utf8');
assert.doesNotMatch(coverage, /\b(?:TODO|TBD|FIXME)\b|lorem ipsum|under construction|coming soon|future `apps\/docs`/i, 'coverage matrix contains unfinished or stale language');
for (const route of routeMap.keys()) assert.ok(coverage.includes(`\`${route}\``), `coverage matrix omits ${route}`);

const rootReadme = await readFile(path.join(repositoryRoot, 'README.md'), 'utf8');
assert.ok(rootReadme.includes('[**Documentation →**](https://planr.so/docs)'), 'root README lacks an actionable docs entry point');

const requirementAudit = [
  { id: 1, requirement: 'Latest stable compatible Fumadocs app, pinned and integrated', evidence: 'exact dependency gate, lockfile install, CONTRACT stack decision, CI scripts' },
  { id: 2, requirement: 'Clear English task-oriented content and copyable commands', evidence: '59 frontmatter-valid pages, authoring contract, CommandBlock browser interaction' },
  { id: 3, requirement: 'Landing, installation, quickstart, and complete lifecycle', evidence: 'published route inventory plus clean-install onboarding replay' },
  { id: 4, requirement: 'Core concepts and canonical ownership', evidence: 'concept page tree plus semantic graph replay' },
  { id: 5, requirement: 'Codex, Claude Code, Cursor, CLI-only, and generic MCP', evidence: 'five integration routes plus client dry-run diagnostics' },
  { id: 6, requirement: 'Exhaustive CLI, MCP, configuration, data, and support reference', evidence: 'generated reference drift checks and 232 coverage assertions' },
  { id: 7, requirement: 'Recipes, troubleshooting, FAQ, contributor, migration, and operations', evidence: 'explicit navigation and coverage inventory' },
  { id: 8, requirement: 'Polished responsive, themed, searchable, accessible shell', evidence: 'production Chrome CDP flow, screenshots, keyboard/focus/zoom/mobile/theme, axe' },
  { id: 9, requirement: 'Content, build, link, orphan, semantic, browser, and accessibility guardrails', evidence: 'CI plus release, maintenance, reference, onboarding, concepts, and shell verifiers' },
  { id: 10, requirement: 'Obvious repository documentation entry points', evidence: 'root README and apps/docs README checks' },
  { id: 11, requirement: 'Official Fumadocs and AgentRig research with recorded decisions', evidence: 'CONTRACT sources and adopted/rejected ADRs' },
  { id: 12, requirement: 'Audited public-surface coverage with no unexplained gaps', evidence: '59-route COVERAGE inventory with agent and human journeys, link/orphan checks, drift verifiers' },
];

const report = {
  ok: true,
  mode: live ? 'production-live' : 'static',
  routes: routeMap.size,
  navigationRoutes: navigation.size,
  internalLinks: linkCount,
  redirects: legacyRedirects.length,
  duplicateAnchors: 0,
  unfinishedMarkers: 0,
  requirementAudit,
  live: null,
};

if (live) {
  const checkedRoutes = [];
  for (const page of pages) {
    const response = await fetch(`${baseUrl}${page.route}`);
    const body = await response.text();
    assert.equal(response.status, 200, `${page.route} returned ${response.status}`);
    assert.ok(body.includes('id="nd-page"') && body.includes('<h1'), `${page.route} did not render the docs shell`);
    checkedRoutes.push(page.route);
  }

  for (const { source, destination } of legacyRedirects) {
    const response = await fetch(`${baseUrl}${source}`, { redirect: 'manual' });
    assert.equal(response.status, 308, `${source} did not return a permanent redirect`);
    assert.equal(response.headers.get('location'), destination, `${source} returned the wrong redirect destination`);
  }

  const searchCases = [
    ['installation', '/docs/getting-started/installation'],
    ['local first', '/docs/concepts/local-first-model'],
    ['MCP schemas', '/docs/reference/mcp-schemas-generated'],
    ['documentation authoring', '/docs/contributing/docs-authoring'],
    ['rollback', '/docs/operations/rollback'],
  ];
  for (const [query, expectedRoute] of searchCases) {
    const response = await fetch(`${baseUrl}/api/search?query=${encodeURIComponent(query)}`);
    const body = await response.text();
    assert.equal(response.status, 200, `search failed for ${query}`);
    assert.ok(body.includes(expectedRoute), `search for ${query} omitted ${expectedRoute}`);
  }

  const missing = await fetch(`${baseUrl}/release-readiness-missing-route`);
  const missingBody = await missing.text();
  assert.equal(missing.status, 404, 'unknown route did not return 404');
  assert.ok(missingBody.includes('This page left the map.'), 'unknown route omitted the custom recovery state');

  const sitemap = await (await fetch(`${baseUrl}/sitemap.xml`)).text();
  for (const route of routeMap.keys()) assert.ok(sitemap.includes(route), `sitemap omits ${route}`);

  report.live = {
    baseUrl,
    renderedRoutes: checkedRoutes.length,
    redirects: legacyRedirects.length,
    searchCases: searchCases.length,
    custom404: true,
    sitemapRoutes: routeMap.size,
  };
}

await mkdir(outputDir, { recursive: true });
const reportPath = path.join(outputDir, 'report.json');
await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.log('release_readiness_verification=passed');
console.log(`mode=${report.mode} routes=${report.routes} internal_links=${report.internalLinks} redirects=${report.redirects}`);
console.log(`requirements=${report.requirementAudit.length} unfinished_markers=${report.unfinishedMarkers} duplicate_anchors=${report.duplicateAnchors}`);
if (report.live) console.log(`rendered=${report.live.renderedRoutes} search_cases=${report.live.searchCases} sitemap_routes=${report.live.sitemapRoutes}`);
console.log(`report=${reportPath}`);
