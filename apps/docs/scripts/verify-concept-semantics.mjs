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
const workspace = await mkdtemp(path.join(tmpdir(), 'planr-docs-concepts-'));
const assertions = [];

await access(planrBin, constants.X_OK).catch(() => {
  throw new Error(`Planr binary is not executable at ${planrBin}. Run \`cargo build --bin planr\` or set PLANR_BIN.`);
});

function run(args, worker = 'concept-replay') {
  const result = spawnSync(planrBin, args, {
    cwd: workspace,
    encoding: 'utf8',
    env: { ...process.env, PLANR_WORKER_ID: worker },
  });
  assert.equal(result.status, 0, `${planrBin} ${args.join(' ')}\n${result.stderr || result.stdout}`);
  return JSON.parse(result.stdout);
}

function check(condition, message) {
  assert.ok(condition, message);
  assertions.push(message);
}

function itemStatus(map, id) {
  return map.items.find((item) => item.id === id)?.status;
}

try {
  const modelSource = await readFile(path.join(repositoryRoot, 'src', 'model.rs'), 'utf8');
  const repositorySource = await readFile(path.join(repositoryRoot, 'src', 'app', 'repository.rs'), 'utf8');
  const conceptPage = await readFile(path.join(docsRoot, 'content', 'docs', 'concepts', 'graph-and-readiness.mdx'), 'utf8');

  check(
    /matches!\(self, Self::Blocks \| Self::HandsTo\)/.test(modelSource),
    'source declares both blocks and hands_to as readiness-blocking',
  );
  check(
    /l\.kind IN \('blocks','hands_to'\).*upstream\.status NOT IN \('closed','closed_partial'\)/s.test(repositorySource),
    'repository readiness accepts only closed or closed_partial upstream items',
  );
  for (const phrase of [
    'Both link kinds block readiness',
    'A `cancelled` item counts as settled in aggregate progress, but it does **not** satisfy either readiness link',
    'Evidence-backed `planr done` without `--review` closes directly',
    '`in_review` → `closed` after the review gate completes',
  ]) {
    check(conceptPage.includes(phrase), `concept page preserves semantic contract: ${phrase}`);
  }

  run(['project', 'init', 'Concept semantics', '--json']);

  const blocksUpstream = run(['item', 'create', 'Blocks upstream', '--description', 'blocks source', '--json']).item;
  const blocksDownstream = run(['item', 'create', 'Blocks downstream', '--description', 'blocks target', '--json']).item;
  run(['link', 'add', blocksUpstream.id, blocksDownstream.id, '--type', 'blocks', '--json']);
  let map = run(['map', 'show', '--json']);
  check(itemStatus(map, blocksDownstream.id) === 'pending', 'blocks keeps downstream pending while upstream is open');
  run(['item', 'cancel', blocksUpstream.id, '--reason', 'semantic replay', '--confirm', '--json']);
  map = run(['map', 'show', '--json']);
  check(itemStatus(map, blocksUpstream.id) === 'cancelled', 'blocks fixture upstream is cancelled');
  check(itemStatus(map, blocksDownstream.id) === 'pending', 'cancelled blocks upstream does not unlock downstream');

  const handsUpstream = run(['item', 'create', 'Hands upstream', '--description', 'handoff source', '--json']).item;
  const handsDownstream = run(['item', 'create', 'Hands downstream', '--description', 'handoff target', '--json']).item;
  run(['link', 'add', handsUpstream.id, handsDownstream.id, '--type', 'hands_to', '--json']);
  map = run(['map', 'show', '--json']);
  check(itemStatus(map, handsDownstream.id) === 'pending', 'hands_to blocks downstream readiness while upstream is open');
  const handsDone = run(['done', handsUpstream.id, '--summary', 'Completed handoff source.', '--cmd', 'semantic replay', '--json']);
  check(handsDone.item.status === 'closed' && handsDone.review === null, 'done without review closes the upstream directly');
  map = run(['map', 'show', '--json']);
  check(itemStatus(map, handsDownstream.id) === 'ready', 'closed hands_to upstream unlocks downstream');

  const cancelledHandsUpstream = run(['item', 'create', 'Cancelled hands upstream', '--description', 'cancelled handoff source', '--json']).item;
  const cancelledHandsDownstream = run(['item', 'create', 'Cancelled hands downstream', '--description', 'cancelled handoff target', '--json']).item;
  run(['link', 'add', cancelledHandsUpstream.id, cancelledHandsDownstream.id, '--type', 'hands_to', '--json']);
  run(['item', 'cancel', cancelledHandsUpstream.id, '--reason', 'semantic replay', '--confirm', '--json']);
  map = run(['map', 'show', '--json']);
  check(itemStatus(map, cancelledHandsDownstream.id) === 'pending', 'cancelled hands_to upstream does not unlock downstream');

  const reviewTarget = run(['item', 'create', 'Optional review target', '--description', 'review branch', '--json']).item;
  const submitted = run([
    'done', reviewTarget.id,
    '--summary', 'Submitted optional review branch.',
    '--cmd', 'semantic replay',
    '--review',
    '--json',
  ]);
  check(submitted.item.status === 'in_review', 'done with review moves target to in_review');
  check(submitted.review?.status === 'ready', 'done with review creates a ready review item');

  console.log(JSON.stringify({ ok: true, planrBinary: planrBin, assertions }, null, 2));
} finally {
  await rm(workspace, { recursive: true, force: true });
}
