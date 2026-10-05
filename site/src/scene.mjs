import { mkdtempSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { replay } from './sample.mjs';

// The sample as the landing shows it. `data` is the board's own data (lib/board.mjs) for the replayed
// sample, which the schematics draw from. `graph` is the task-graph scene: the sample's tasks as board
// rows, joined by their `depends` edges. Where each row floats is design; which status it moves to is
// not. The build answers the decision and then asks the real engine (graph.mjs) what a team would do
// next, so the browser only plays back changes the engine allowed.

// side: l or r of the centered text. u: 0 at the clear area around it, 1 at the page edge.
// v: 0 at the top of the band, 1 at its bottom. z: depth, 1 is near (large, sharp), 2.4 is far.
const SLOTS = [
  ['WEB-01', 'l', 0.50, 0.04, 1.9],
  ['OPS-01', 'l', 0.92, 0.16, 1.0],
  ['WEB-02', 'l', 0.22, 0.30, 1.5],
  ['WEB-03', 'l', 0.66, 0.50, 1.15],
  ['OPS-02', 'l', 0.98, 0.66, 1.05],
  ['EXP-01', 'l', 0.04, 0.64, 2.3],
  ['EXP-02', 'l', 0.34, 0.9, 1.4],
  ['MOB-01', 'r', 0.40, 0.0, 1.7],
  ['MOB-02', 'r', 0.92, 0.14, 1.0],
  ['MOB-04', 'r', 0.06, 0.28, 2.1],
  ['MOB-03', 'r', 0.98, 0.38, 1.1],
  ['MOB-05', 'r', 0.30, 0.50, 1.5],
  ['MOB-06', 'r', 0.80, 0.62, 1.05],
  ['MOB-07', 'r', 0.02, 0.66, 1.7],
  ['MOB-08', 'r', 0.40, 0.88, 1.25],
];

// What happens first, then the order of moves the scene cycles through.
const FIRST = [['MOB-07', 'done']];
const MOVES = ['start', 'finish', 'check'];
const LIMIT = 16;

export async function scene(root, { bodies, name = 'planning' } = {}) {
  const { buildIndex, config, compare, rewrite } = await import(`${root}/skills/planr/scripts/lib/graph.mjs`);
  const { boardData } = await import(`${root}/skills/planr/scripts/lib/board.mjs`);
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'planr-scene-')));
  const repo = join(dir, name);
  try {
    replay(root, repo, bodies);
    const c = config(repo);
    const index = () => buildIndex(repo, c).index;
    const shown = new Set(SLOTS.map(([id]) => id));
    const start = index();
    const data = boardData(repo, start, c);
    const byId = new Map(start.tasks.map(t => [t.id, t]));
    const steps = [];
    const apply = (id, to) => {
      const before = index();
      const task = before.tasks.find(t => t.id === id);
      rewrite(join(repo, task.file), { status: to });
      const after = index();
      const was = new Set(before.tasks.filter(t => t.ready).map(t => t.id));
      steps.push({
        id,
        to,
        unlocked: after.tasks.filter(t => t.ready && !was.has(t.id) && shown.has(t.id)).map(t => t.id),
      });
    };
    for (const [id, to] of FIRST) {
      apply(id, to);
    }
    // Start the ready task of an approved goal that unlocks the most (as `planr next` orders it), move
    // work in progress to review, check the longest-waiting review. Each move falls through to the next
    // if it has nothing.
    const candidates = {
      start: (tasks, goals) => tasks.filter(t => t.ready && goals.some(g =>
        ['approved', 'running'].includes(g.state) && g.scope.includes(t.id)))
        .sort((a, b) => b.unlocks - a.unlocks || compare(a.id, b.id)).map(t => [t.id, 'in-progress']),
      finish: tasks => tasks.filter(t => t.status === 'in-progress').map(t => [t.id, 'review']),
      check: tasks => tasks.filter(t => t.status === 'review').map(t => [t.id, 'done']),
    };
    for (let turn = 0; steps.length < LIMIT; turn++) {
      const now = index();
      const tasks = now.tasks.filter(t => shown.has(t.id));
      const move = MOVES.map((_, k) => candidates[MOVES[(turn + k) % MOVES.length]](tasks, now.goals)[0])
        .find(Boolean);
      if (!move) {
        break;
      }
      apply(...move);
    }
    return {
      data,
      graph: {
        cards: SLOTS.map(([id, side, u, v, z]) => {
          const t = byId.get(id);
          if (!t) {
            throw Error(`scene: ${id} is not in the sample`);
          }
          return { id, title: t.title, status: t.status, side, u, v, z };
        }),
        edges: SLOTS.flatMap(([id]) => byId.get(id).depends.filter(d => shown.has(d)).map(d => [d, id])),
        steps,
      },
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}
