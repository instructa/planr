import assert from 'node:assert/strict';
import { access, mkdtemp, readFile, rm } from 'node:fs/promises';
import { constants } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const planrBin = process.env.PLANR_BIN
  ? path.resolve(process.cwd(), process.env.PLANR_BIN)
  : path.join(repositoryRoot, 'target', 'debug', 'planr');
const workspace = await mkdtemp(path.join(tmpdir(), 'planr-docs-reference-'));
const assertions = [];

await access(planrBin, constants.X_OK);

function check(condition, message) {
  assert.ok(condition, message);
  assertions.push(message);
}

function run(args, input) {
  const result = spawnSync(planrBin, args, { cwd: workspace, encoding: 'utf8', input });
  assert.equal(result.status, 0, `${planrBin} ${args.join(' ')}\n${result.stderr}`);
  return result.stdout;
}

const HTTP_ROUTE_MANIFEST = [
  ['GET exact:/v1/events/stream', '/v1/events/stream'],
  ['GET exact:/review', '/review'], ['GET exact:/review/', '/review'],
  ['GET exact:/v1/review-workspace', '/v1/review-workspace'],
  ['GET exact:/v1/projects', '/v1/projects'], ['POST exact:/v1/projects', '/v1/projects'],
  ['GET ends_with:/map', '/v1/projects/{project_id}/map'],
  ['GET ends_with:/map/status', '/v1/projects/{project_id}/map/status'],
  ['GET ends_with:/map/lookahead', '/v1/projects/{project_id}/map/lookahead'],
  ['GET ends_with:/items', '/v1/projects/{project_id}/items'], ['POST ends_with:/items', '/v1/projects/{project_id}/items'],
  ['GET ends_with:/unlocks', '/v1/items/{id}/unlocks'], ['GET ends_with:/preview-close', '/v1/items/{id}/preview-close'],
  ['POST exact:/v1/recover/sweep', '/v1/recover/sweep'], ['POST exact:/v1/policy/admit', '/v1/policy/admit'],
  ['POST ends_with:/insert', '/v1/items/{id}/insert'], ['POST ends_with:/amend', '/v1/items/{id}/amend'], ['POST ends_with:/replan', '/v1/items/{id}/replan'],
  ['POST ends_with:/heartbeat', '/v1/items/{id}/heartbeat'], ['POST ends_with:/progress', '/v1/items/{id}/progress'],
  ['POST ends_with:/pause', '/v1/items/{id}/pause'], ['POST ends_with:/resume', '/v1/items/{id}/resume'],
  ['POST ends_with:/approval/request', '/v1/items/{id}/approval/request'], ['POST ends_with:/approval/approve', '/v1/items/{id}/approval/approve'],
  ['POST ends_with:/approval/deny', '/v1/items/{id}/approval/deny'], ['GET exact:/v1/approvals', '/v1/approvals'],
  ['POST exact:/v1/pick', '/v1/pick'], ['POST ends_with:/log', '/v1/items/{id}/log'],
  ['POST starts_with:/v1/reviews/&ends_with:/close', '/v1/reviews/{id}/close'],
  ['GET starts_with:/v1/reviews/&ends_with:/artifact', '/v1/reviews/{id}/artifact'],
  ['POST starts_with:/v1/reviews/&ends_with:/artifact', '/v1/reviews/{id}/artifact'],
  ['POST ends_with:/close', '/v1/items/{id}/close'], ['POST ends_with:/reviews', '/v1/items/{id}/reviews'],
  ['POST ends_with:/review-annotations', '/v1/items/{id}/review-annotations'],
  ['GET ends_with:/review-evidence', '/v1/items/{id}/review-evidence'], ['POST ends_with:/review-evidence', '/v1/items/{id}/review-evidence'],
  ['POST ends_with:/review-feedback', '/v1/items/{id}/review-feedback'], ['POST exact:/v1/contexts', '/v1/contexts'],
  ['POST exact:/v1/artifacts', '/v1/artifacts'], ['GET exact:/v1/artifacts', '/v1/artifacts'],
  ['GET starts_with:/v1/artifacts/', '/v1/artifacts/{id}'], ['GET exact:/v1/events', '/v1/events'],
  ['GET exact:/v1/debug/bundle', '/v1/debug/bundle'], ['GET exact:/v1/search', '/v1/search'],
  ['ANY exact:/health', '/health'],
];

function extractHttpRouteSignatures(source) {
  const start = source.indexOf('let body = match (method, path)');
  const end = source.indexOf('(m, p) => bail!', start);
  assert.ok(start >= 0 && end > start, 'Could not locate authoritative HTTP route match');
  const signatures = ['GET exact:/v1/events/stream'];
  for (const raw of source.slice(start, end).split('\n')) {
    const line = raw.trim();
    if (!line.includes('=>')) continue;
    if (line.includes(', p) if')) {
      const method = line.match(/^\("([A-Z]+)", p\)/)?.[1];
      const predicates = [...line.matchAll(/p\.(starts_with|ends_with)\("([^"]+)"\)/g)]
        .map((match) => `${match[1]}:${match[2]}`)
        .join('&');
      assert.ok(method && predicates, `Unparsed dynamic HTTP route: ${line}`);
      signatures.push(`${method} ${predicates}`);
    } else {
      for (const match of line.matchAll(/\(("[A-Z]+"|_), "([^"]+)"\)/g)) {
        signatures.push(`${match[1] === '_' ? 'ANY' : match[1].slice(1, -1)} exact:${match[2]}`);
      }
    }
  }
  return signatures;
}

try {
  run(['project', 'init', 'Reference verification', '--json']);
  const requests = [
    ['initialize', {}],
    ['tools/list', {}],
    ['resources/list', {}],
    ['prompts/list', {}],
  ].map(([method, params], index) => JSON.stringify({ jsonrpc: '2.0', id: index + 1, method, params })).join('\n');
  const responses = run(['mcp'], `${requests}\n`).trim().split('\n').map(JSON.parse);
  const tools = responses[1].result.tools;
  const resources = responses[2].result.resources;
  const prompts = responses[3].result.prompts;
  const mcpPage = await readFile(path.join(docsRoot, 'content', 'docs', 'reference', 'mcp.mdx'), 'utf8');

  for (const tool of tools) {
    const line = mcpPage.split('\n').find((candidate) => candidate.includes(`\`${tool.name}\``));
    check(Boolean(line), `MCP reference covers tool ${tool.name}`);
    for (const required of tool.inputSchema.required ?? []) {
      check(line.includes(`\`${required}\``), `MCP reference records required field ${tool.name}.${required}`);
    }
  }
  for (const resource of resources) check(mcpPage.includes(`\`${resource.uri}\``), `MCP reference covers resource ${resource.uri}`);
  for (const prompt of prompts) check(mcpPage.includes(`\`${prompt.name}\``), `MCP reference covers prompt ${prompt.name}`);
  check(responses[0].result.protocolVersion === '2025-03-26', 'MCP protocol version matches documented 2025-03-26');

  const httpSource = await readFile(path.join(repositoryRoot, 'src', 'app', 'http.rs'), 'utf8');
  const httpPage = await readFile(path.join(docsRoot, 'content', 'docs', 'reference', 'http-api.mdx'), 'utf8');
  const extractedHttpRoutes = extractHttpRouteSignatures(httpSource).sort();
  const manifestedHttpRoutes = HTTP_ROUTE_MANIFEST.map(([signature]) => signature).sort();
  assert.deepEqual(extractedHttpRoutes, manifestedHttpRoutes, 'HTTP router and checked public route manifest drifted');
  assertions.push(`HTTP route manifest exactly matches all ${manifestedHttpRoutes.length} authoritative router arms`);
  for (const [signature, route] of HTTP_ROUTE_MANIFEST) check(httpPage.includes(route), `HTTP reference covers ${signature} as ${route}`);

  const configPage = await readFile(path.join(docsRoot, 'content', 'docs', 'reference', 'configuration-and-storage.mdx'), 'utf8');
  const envNames = ['PLANR_DB', 'PLANR_WORKER_ID', 'PLANR_SESSION_ID', 'PLANR_PROFILE', 'PLANR_NATIVE_BIN', 'PLANR_BIN', 'PLANR_DOWNLOAD', 'PLANR_REPO', 'PLANR_VERSION', 'PLANR_TARGET', 'PLANR_RELEASE_BASE_URL', 'PLANR_SKIP_CHECKSUM'];
  for (const name of envNames) check(configPage.includes(`\`${name}\``), `configuration reference covers ${name}`);

  const schemaSource = await readFile(path.join(repositoryRoot, 'src', 'storage', 'schema.rs'), 'utf8');
  const modelSource = await readFile(path.join(repositoryRoot, 'src', 'model.rs'), 'utf8');
  const utilSource = await readFile(path.join(repositoryRoot, 'src', 'util.rs'), 'utf8');
  const dataPage = await readFile(path.join(docsRoot, 'content', 'docs', 'reference', 'data-and-status.mdx'), 'utf8');
  const errorPage = await readFile(path.join(docsRoot, 'content', 'docs', 'reference', 'outputs-and-errors.mdx'), 'utf8');
  const tables = [...schemaSource.matchAll(/CREATE (?:VIRTUAL )?TABLE IF NOT EXISTS ([a-z_]+)/g)].map((match) => match[1]);
  for (const table of tables) check(dataPage.includes(`\`${table}\``), `data reference covers storage table ${table}`);
  const statuses = ['pending', 'ready', 'picked', 'running', 'in_review', 'blocked', 'closed', 'closed_partial', 'failed', 'cancelled'];
  for (const status of statuses) {
    check(modelSource.includes(`"${status}"`), `model source contains status ${status}`);
    check(dataPage.includes(`\`${status}\``), `data reference covers status ${status}`);
  }
  for (const link of ['blocks', 'hands_to', 'reviews', 'relates_to']) check(dataPage.includes(`\`${link}\``), `data reference covers link ${link}`);
  for (const code of ['not_found', 'invalid_transition', 'already_closed', 'bad_request', 'locked', 'parse_error', 'internal_error']) {
    check(utilSource.includes(`"${code}"`), `error source contains ${code}`);
    check(errorPage.includes(`\`${code}\``), `error reference covers ${code}`);
  }

  const supportPage = await readFile(path.join(docsRoot, 'content', 'docs', 'reference', 'support-matrix.mdx'), 'utf8');
  for (const required of ['macOS arm64', 'macOS x86_64', 'Linux x86_64', 'Linux arm64', 'Native Windows', 'Codex', 'Claude Code', 'Cursor', 'Generic MCP', 'CLI-only/CI', '127.0.0.1']) {
    check(supportPage.includes(required), `support matrix covers ${required}`);
  }

  console.log(JSON.stringify({
    ok: true,
    planrBinary: planrBin,
    coverage: { tools: tools.length, resources: resources.length, prompts: prompts.length, httpRoutes: HTTP_ROUTE_MANIFEST.length, tables: tables.length },
    assertions: assertions.length,
  }, null, 2));
} finally {
  await rm(workspace, { recursive: true, force: true });
}
