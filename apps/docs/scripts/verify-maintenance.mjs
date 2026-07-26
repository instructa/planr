import { readFile, readdir } from 'node:fs/promises';
import { dirname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { legacyRedirects } from '../redirects.mjs';
import { verifyTwoPhaseReleaseContract } from './release-contract.mjs';

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
  'Alchemy v2', 'direct Cloudflare assets', 'planr.so', 'pnpm docs:deploy',
  '/api/search', 'PLANR_DOCS_URL=https://planr.so pnpm docs:verify-shell',
]);

const rollback = await readPage('operations', 'rollback');
requireMarkers(rollback, 'rollback runbook', [
  'last known-good commit', 'pnpm docs:deploy', '/api/search', 'custom 404',
  'pnpm docs:destroy', 'does not touch user `.planr` data', 'Do not assume a binary downgrade is safe',
]);

const governance = await readPage('operations', 'documentation-governance');
requireMarkers(governance, 'governance runbook', [
  '`docs/documentation/COVERAGE.md`', '`docs/documentation/CONTRACT.md`',
  'Generated pages are never hand-edited', 'Freshness triggers', 'At each release', 'failure recovery',
  '`apps/docs/redirects.mjs`', 'permanent Cloudflare edge redirect', 'duplicate sources',
]);

const release = await readPage('operations', 'release');
requireMarkers(release, 'release runbook', [
  'four archives', '`SHA256SUMS`', 'npm under the `alpha` dist-tag', 'Do not silently replace',
]);
verifyTwoPhaseReleaseContract({
  releasePage: release,
  prepareScript: await read('scripts/prepare-release-candidate.sh'),
  publishScript: await read('scripts/release.sh'),
});

const packageJson = JSON.parse(await read('apps/docs/package.json'));
for (const [name, version] of Object.entries({ ...packageJson.dependencies, ...packageJson.devDependencies })) {
  assert(!version.startsWith('^') && !version.startsWith('~'), `apps/docs dependency ${name} must use an exact version`);
}
assert(packageJson.engines.node === '>=22', 'apps/docs must require Node.js 22 or newer');
assert(
  packageJson.scripts.deploy === 'alchemy deploy --stage prod --yes',
  'apps/docs deploy must target the Alchemy prod stage',
);
assert(packageJson.scripts.destroy === 'alchemy destroy --stage prod', 'apps/docs destroy must target the Alchemy prod stage explicitly');
assert(packageJson.scripts.build === 'next build && node scripts/prepare-static-assets.mjs', 'docs build must prepare the deployable static artifact');
assert(packageJson.scripts.start === 'wrangler dev --config wrangler.jsonc --port 3000 --local', 'docs start must emulate Cloudflare static routing');
assert(packageJson.scripts['verify:deployment'].includes('wrangler deploy --config wrangler.jsonc --dry-run'), 'deployment verification must run Wrangler without deploying');
assert(packageJson.scripts['verify:deployment'].includes('verify-static-deployment.mjs'), 'deployment verification must inspect the complete static artifact');
assert(packageJson.devDependencies.alchemy === '2.0.0-beta.63', 'Alchemy v2 must stay exactly pinned');
assert(packageJson.devDependencies.effect === '4.0.0-beta.98', 'Effect v4 must stay exactly pinned');
assert(packageJson.devDependencies['@effect/platform-node'] === '4.0.0-beta.98', 'Effect Node platform must stay exactly pinned');
assert(packageJson.devDependencies['@effect/platform-bun'] === '4.0.0-beta.98', 'Effect Bun platform must stay exactly pinned');
assert(!('@opennextjs/cloudflare' in packageJson.devDependencies), 'static docs must not retain the OpenNext Worker runtime');

const pages = await collectPages(contentRoot);
const routeMap = new Map(pages.map((page) => [page.route, page]));
assert(routeMap.size > 0, 'expected at least one explicit MDX route');

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
requireMarkers(nextConfig, 'Next.js static export wiring', ["output: 'export'", 'createMDX()', 'withMDX(config)']);

const rootLayout = await read('apps/docs/app/layout.tsx');
const plausibleScript = '<script defer data-domain="planr.so" src="https://analytics.int.macherjek.com/js/script.js" />';
assert(rootLayout.includes(plausibleScript), 'root layout must include the exact deferred Plausible script');
assert(
  (rootLayout.match(/https:\/\/analytics\.int\.macherjek\.com\/js\/script\.js/g) ?? []).length === 1,
  'root layout must include the Plausible script URL exactly once',
);

const alchemyConfig = await read('apps/docs/alchemy.run.ts');
requireMarkers(alchemyConfig, 'Alchemy deployment wiring', [
  'Alchemy.Stack(', 'Cloudflare.providers()', 'Cloudflare.state()',
  'Cloudflare.Website.StaticSite(', 'planr-docs-${stage}',
  'stage === "prod"', 'planr.so', 'AdoptPolicy.adopt(true)',
  'command: "pnpm run build"', 'outdir: "out"', 'main: "worker.mjs"',
  'notFoundHandling: "404-page"', 'runWorkerFirst:', 'legacyRedirects.map', 'NEXT_PUBLIC_SITE_URL',
]);
requireMarkers(await read('apps/docs/wrangler.jsonc'), 'Wrangler static asset configuration', [
  '"name": "planr-docs-prod"', '"main": "worker.mjs"', '"directory": "out"',
  '"binding": "ASSETS"', '"not_found_handling": "404-page"', '"run_worker_first": [',
]);
requireMarkers(await read('apps/docs/scripts/prepare-static-assets.mjs'), 'static asset preparation', [
  'legacyRedirects', "path.join(outputRoot, '_headers')", "path.join(outputRoot, '_redirects')",
  'Content-Type: text/markdown',
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
  ['apps/docs/alchemy.run.ts', ['planr.so', 'Cloudflare.Website.StaticSite', 'AdoptPolicy.adopt(true)', 'outdir: "out"']],
  ['apps/docs/app/page.tsx', ['Works with your coding agent', "agentRecipeList.map((recipe) =>"]],
  ['apps/docs/lib/agent-recipes.ts', ['/agents/codex.svg', '/agents/claude.svg', '/agents/cursor.svg', 'satisfies Record<AgentClientId, AgentRecipe>']],
  ['apps/docs/public/agents/README.md', ['developers.openai.com/assets/OpenAI-black-monoblossom.svg', 'anthropic.com/press-kit', 'cursor.com/brand']],
  ['apps/docs/public/agents/codex.svg', ['<svg', 'fill="black"']],
  ['apps/docs/public/agents/claude.svg', ['<svg', 'fill="#D97757"']],
  ['apps/docs/public/agents/cursor.svg', ['<svg', 'fill: #26251e']],
  ['apps/docs/scripts/verify-static-deployment.mjs', ['static_deployment_verification=passed', 'api/markdown/', '_redirects']],
  ['.github/workflows/ci.yml', ['Build Cloudflare static deployment artifact', 'pnpm docs:verify-deployment']],
  ['scripts/prepare-release-candidate.sh', ['Prepare the exact source state', 'pnpm install --frozen-lockfile', 'reference:generate']],
  ['scripts/release.sh', ['Publish an already prepared, reviewed release commit', 'cargo test', 'scripts/security-local.sh', 'git tag -a']],
  ['docs/RELEASE.md', ['only supported publication path', 'annotated `vx.y.z` tag']],
  ['apps/docs/redirects.mjs', ['legacyRedirects', 'permanent: true']],
  ['apps/docs/worker.mjs', ["from './redirects.mjs'", 'status: 308', 'env.ASSETS.fetch(request)']],
  ['src/storage/schema.rs', ['const SCHEMA_VERSION', 'ensure_column', "'schema_version'"]],
];
for (const [path, markers] of sourceChecks) requireMarkers(await read(path), path, markers);

console.log('maintenance_docs_verification=passed');
console.log(`pages=${pageCount} sections=${Object.keys(expected).length}`);
console.log(`published_routes=${routeMap.size} coverage_targets=${coverageTargets.length}`);
console.log(`redirects=${legacyRedirects.length} redirect_destinations=${new Set(legacyRedirects.map(({ destination }) => destination)).size}`);
console.log(`exact_dependencies=${Object.keys(packageJson.dependencies).length + Object.keys(packageJson.devDependencies).length}`);
console.log(`source_contracts=${sourceChecks.length}`);
console.log('plausible_script_contract=passed');
