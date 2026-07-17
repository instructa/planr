import { readFile, readdir } from 'node:fs/promises';
import { dirname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { legacyRedirects } from '../redirects.mjs';

const docsRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = dirname(dirname(docsRoot));
const contentRoot = join(docsRoot, 'content', 'docs');

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function read(path) {
  return readFile(join(repositoryRoot, path), 'utf8');
}

async function readPage(section, slug) {
  const content = await readFile(join(contentRoot, section, `${slug}.mdx`), 'utf8');
  const frontmatter = content.startsWith('---\n') ? content.split('\n---\n', 1)[0] : '';
  assert(/^---\ntitle: .+$/m.test(frontmatter) && /^description: .+$/m.test(frontmatter), `${section}/${slug}.mdx needs title and description frontmatter`);
  return content;
}

function requireMarkers(content, label, markers) {
  for (const marker of markers) assert(content.includes(marker), `${label} is missing contract marker: ${marker}`);
}

function routeFor(file) {
  const slug = relative(contentRoot, file).split(sep).join('/').replace(/\.mdx$/, '');
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
    const path = join(directory, entry.name);
    if (entry.isDirectory()) pages.push(...await collectPages(path));
    else if (entry.name.endsWith('.mdx')) {
      const content = await readFile(path, 'utf8');
      const anchors = new Set([...content.matchAll(/^#{2,6}\s+(.+)$/gm)].map((match) => headingSlug(match[1])));
      pages.push({ path, route: routeFor(path), anchors });
    }
  }
  return pages;
}

const expected = {
  contributing: ['index', 'repository-setup', 'architecture', 'docs-authoring', 'testing', 'security-and-privacy'],
  operations: ['index', 'release', 'versioning-and-migrations', 'docs-deployment', 'health-and-diagnostics', 'rollback', 'documentation-governance'],
};

let pageCount = 0;
for (const [section, pages] of Object.entries(expected)) {
  const meta = JSON.parse(await readFile(join(contentRoot, section, 'meta.json'), 'utf8'));
  assert(JSON.stringify(meta.pages) === JSON.stringify(pages), `${section}/meta.json must declare the reviewed page order explicitly`);
  for (const page of pages) {
    await readPage(section, page);
    pageCount += 1;
  }
}

const authoring = await readPage('contributing', 'docs-authoring');
requireMarkers(authoring, 'docs authoring guide', [
  'title` and `description` frontmatter', '`meta.json` `pages` array', '<CommandBlock', '<PathCard>',
  'pnpm docs:content', 'pnpm docs:dev',
  'Never edit `cli-generated.mdx` or `mcp-schemas-generated.mdx` by hand', 'accessibility',
]);

const testing = await readPage('contributing', 'testing');
requireMarkers(testing, 'testing guide', [
  'pnpm docs:build', 'pnpm docs:verify-onboarding', 'pnpm docs:verify-concepts',
  'pnpm docs:verify-reference', 'pnpm docs:verify-maintenance', 'pnpm docs:verify-shell',
  'pnpm docs:verify-deployment',
  'scripts/ci-local.sh', 'scripts/security-local.sh',
]);

const security = await readPage('contributing', 'security-and-privacy');
requireMarkers(security, 'security guide', [
  'local SQLite database', 'no content telemetry by default', '`127.0.0.1`', 'preview-first',
  '[REDACTED]', 'no analytics', 'scripts/security-local.sh',
]);

const deployment = await readPage('operations', 'docs-deployment');
requireMarkers(deployment, 'deployment runbook', [
  'Node.js 22', 'pnpm install --frozen-lockfile', 'NEXT_PUBLIC_SITE_URL',
  'Alchemy v2', 'OpenNext', 'planr.so', 'pnpm docs:deploy',
  '/api/search?query=installation', 'PLANR_DOCS_URL=https://planr.so pnpm docs:verify-shell',
]);

const rollback = await readPage('operations', 'rollback');
requireMarkers(rollback, 'rollback runbook', [
  'last known-good commit', 'pnpm docs:deploy', '/api/search?query=installation', 'custom 404',
  'pnpm docs:destroy', 'does not touch user `.planr` data', 'Do not assume a binary downgrade is safe',
]);

const governance = await readPage('operations', 'documentation-governance');
requireMarkers(governance, 'governance runbook', [
  '`docs/documentation/COVERAGE.md`', '`docs/documentation/CONTRACT.md`',
  'Generated pages are never hand-edited', 'Freshness triggers', 'At each release', 'failure recovery',
  '`apps/docs/redirects.mjs`', 'permanent Next.js redirect', 'duplicate sources',
]);

const release = await readPage('operations', 'release');
requireMarkers(release, 'release runbook', [
  '`scripts/release.sh <version> "summary"` is the only supported release entry point',
  'four archives', '`SHA256SUMS`', 'npm under the `alpha` dist-tag', 'Do not silently replace',
]);

const packageJson = JSON.parse(await read('apps/docs/package.json'));
for (const [name, version] of Object.entries({ ...packageJson.dependencies, ...packageJson.devDependencies })) {
  assert(!version.startsWith('^') && !version.startsWith('~'), `apps/docs dependency ${name} must use an exact version`);
}
assert(packageJson.engines.node === '>=22', 'apps/docs must require Node.js 22 or newer');
assert(packageJson.scripts.deploy === 'alchemy deploy --stage prod', 'apps/docs deploy must target the Alchemy prod stage explicitly');
assert(packageJson.scripts.destroy === 'alchemy destroy --stage prod', 'apps/docs destroy must target the Alchemy prod stage explicitly');
assert(packageJson.scripts['build:worker'] === 'opennextjs-cloudflare build --skipWranglerConfigCheck', 'worker build must use OpenNext');
assert(packageJson.scripts['bundle:worker'] === 'wrangler deploy --dry-run --outdir .alchemy-worker', 'worker bundle must use Wrangler without deploying');
assert(packageJson.scripts['build:deploy'] === 'pnpm run build:worker && pnpm run bundle:worker', 'deployment build must produce the OpenNext and bundled Worker artifacts');
assert(packageJson.scripts['verify:deployment'] === 'pnpm run build:deploy', 'deployment verification must build the deployable Worker artifact');
assert(packageJson.devDependencies.alchemy === '2.0.0-beta.63', 'Alchemy v2 must stay exactly pinned');
assert(packageJson.devDependencies.effect === '4.0.0-beta.98', 'Effect v4 must stay exactly pinned');
assert(packageJson.devDependencies['@effect/platform-node'] === '4.0.0-beta.98', 'Effect Node platform must stay exactly pinned');
assert(packageJson.devDependencies['@effect/platform-bun'] === '4.0.0-beta.98', 'Effect Bun platform must stay exactly pinned');
assert(packageJson.devDependencies['@opennextjs/cloudflare'] === '1.20.1', 'OpenNext must stay exactly pinned');

const pages = await collectPages(contentRoot);
const routeMap = new Map(pages.map((page) => [page.route, page]));
assert(routeMap.size === 55, `expected 55 unique MDX routes, found ${routeMap.size}`);

const rootMeta = JSON.parse(await readFile(join(contentRoot, 'meta.json'), 'utf8'));
const declaredRoutes = new Set();
for (const slug of rootMeta.pages) {
  if (slug === 'index') {
    declaredRoutes.add('/docs');
    continue;
  }
  try {
    const sectionMeta = JSON.parse(await readFile(join(contentRoot, slug, 'meta.json'), 'utf8'));
    for (const page of sectionMeta.pages) declaredRoutes.add(page === 'index' ? `/docs/${slug}` : `/docs/${slug}/${page}`);
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
    declaredRoutes.add(`/docs/${slug}`);
  }
}
assert(declaredRoutes.size === routeMap.size, `meta.json declares ${declaredRoutes.size} routes but MDX provides ${routeMap.size}`);
for (const route of routeMap.keys()) assert(declaredRoutes.has(route), `${route} exists but is absent from explicit meta.json navigation`);
for (const route of declaredRoutes) assert(routeMap.has(route), `meta.json declares missing page ${route}`);

const coverage = await read('docs/documentation/COVERAGE.md');
const coverageTargets = [...coverage.matchAll(/`(\/docs(?:\/[a-z0-9][a-z0-9/-]*)?(?:#[a-z0-9][a-z0-9-]*)?)`/g)].map((match) => match[1]);
assert(coverageTargets.length > 0, 'COVERAGE.md has no checked route targets');
for (const target of coverageTargets) {
  const [route, anchor] = target.split('#');
  const page = routeMap.get(route);
  assert(page, `COVERAGE.md target does not resolve: ${target}`);
  if (anchor) assert(page.anchors.has(anchor), `COVERAGE.md anchor does not resolve: ${target}`);
}
const coveredRoutes = new Set(coverageTargets.map((target) => target.split('#')[0]));
for (const route of routeMap.keys()) assert(coveredRoutes.has(route), `published route is missing from COVERAGE.md inventory: ${route}`);

const informationArchitecture = await read('docs/documentation/INFORMATION_ARCHITECTURE.md');
const redirectSources = new Set();
for (const redirect of legacyRedirects) {
  assert(redirect.permanent === true, `redirect must be permanent: ${redirect.source}`);
  assert(redirect.source.startsWith('/docs/'), `redirect source must be a docs route: ${redirect.source}`);
  assert(!redirectSources.has(redirect.source), `duplicate redirect source: ${redirect.source}`);
  assert(!routeMap.has(redirect.source), `redirect source collides with a current page: ${redirect.source}`);
  assert(routeMap.has(redirect.destination), `redirect destination does not resolve: ${redirect.source} -> ${redirect.destination}`);
  assert(
    informationArchitecture.includes(`| \`${redirect.source}\` | \`${redirect.destination}\` |`),
    `redirect is missing from INFORMATION_ARCHITECTURE.md: ${redirect.source}`,
  );
  redirectSources.add(redirect.source);
}

const nextConfig = await read('apps/docs/next.config.mjs');
requireMarkers(nextConfig, 'Next.js redirect wiring', [
  "import { initOpenNextCloudflareForDev } from '@opennextjs/cloudflare'",
  "import { legacyRedirects } from './redirects.mjs'", 'redirects: async () => legacyRedirects',
  'initOpenNextCloudflareForDev()',
]);

const alchemyConfig = await read('apps/docs/alchemy.run.ts');
requireMarkers(alchemyConfig, 'Alchemy deployment wiring', [
  'Alchemy.Stack(', 'Cloudflare.providers()', 'Cloudflare.state()',
  'Cloudflare.Website.StaticSite(', 'planr-docs-${stage}',
  'stage === "prod"', 'planr.so', 'AdoptPolicy.adopt(true)',
  'main: ".alchemy-worker/worker.js"', 'bundle: false', 'NEXT_PUBLIC_SITE_URL',
]);
requireMarkers(await read('apps/docs/open-next.config.ts'), 'OpenNext configuration', [
  'defineCloudflareConfig', 'export default',
]);

const releaseScript = await read('scripts/release.sh');
assert(
  releaseScript.includes('git tag -a "v$version" -m "planr v$version: $summary"'),
  'release script must create an annotated tag carrying the release summary',
);
assert(!/^git tag "v\$version"$/m.test(releaseScript), 'release script still contains a lightweight tag invocation');

const sourceChecks = [
  ['apps/docs/README.md', ['Node.js 22', 'pnpm install --frozen-lockfile', 'NEXT_PUBLIC_SITE_URL', 'planr.so', 'pnpm docs:deploy', 'Alchemy v2']],
  ['apps/docs/.env.example', ['NEXT_PUBLIC_SITE_URL=https://planr.so']],
  ['apps/docs/alchemy.run.ts', ['planr.so', 'Cloudflare.Website.StaticSite', 'AdoptPolicy.adopt(true)', 'bundle: false']],
  ['apps/docs/app/page.tsx', ['Works with your coding agent', '/agents/codex.svg', '/agents/claude.svg', '/agents/cursor.svg']],
  ['apps/docs/public/agents/README.md', ['developers.openai.com/assets/OpenAI-black-monoblossom.svg', 'anthropic.com/press-kit', 'cursor.com/brand']],
  ['apps/docs/public/agents/codex.svg', ['<svg', 'fill="black"']],
  ['apps/docs/public/agents/claude.svg', ['<svg', 'fill="#D97757"']],
  ['apps/docs/public/agents/cursor.svg', ['<svg', 'fill: #26251e']],
  ['apps/docs/open-next.config.ts', ['defineCloudflareConfig']],
  ['.github/workflows/ci.yml', ['Build Cloudflare Worker deployment artifact', 'pnpm docs:verify-deployment']],
  ['scripts/release.sh', ['The only supported release path', 'cargo test', 'scripts/security-local.sh', 'git tag -a']],
  ['docs/RELEASE.md', ['only supported release path', 'annotated `vx.y.z` tag']],
  ['apps/docs/redirects.mjs', ['legacyRedirects', 'permanent: true']],
  ['src/storage/schema.rs', ['const SCHEMA_VERSION', 'ensure_column', "'schema_version'"]],
];
for (const [path, markers] of sourceChecks) requireMarkers(await read(path), path, markers);

console.log('maintenance_docs_verification=passed');
console.log(`pages=${pageCount} sections=${Object.keys(expected).length}`);
console.log(`published_routes=${routeMap.size} coverage_targets=${coverageTargets.length}`);
console.log(`redirects=${legacyRedirects.length} redirect_destinations=${new Set(legacyRedirects.map(({ destination }) => destination)).size}`);
console.log(`exact_dependencies=${Object.keys(packageJson.dependencies).length + Object.keys(packageJson.devDependencies).length}`);
console.log(`source_contracts=${sourceChecks.length}`);
