import { mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { replay } from './sample.mjs';

// Refreshes fixtures/board-data.json from its own source fields. Edit the tasks (id, title, status,
// track, depends, owner, history), goals (name, file, title, state, stream, budget, thread, scope) or
// runs (date, subject), then run `npm run fixture`: the sample is replayed as commits and every derived
// field (ready, unlocks, goal progress, run changes, commits) comes from the real engine.
// `generatedAt` stays as it is, so the file is the same every run.
const root = fileURLToPath(new URL('../../', import.meta.url));
const file = join(root, 'fixtures/board-data.json');
const { buildIndex, config } = await import(`${root}/skills/planr/scripts/lib/graph.mjs`);
const { runs, git } = await import(`${root}/skills/planr/scripts/lib/runs.mjs`);
// The sample's own name is its planning repo's name.
const { project, generatedAt } = JSON.parse(readFileSync(file, 'utf8'));
const dir = realpathSync(mkdtempSync(join(tmpdir(), 'planr-fixture-')));
const repo = join(dir, project);
try {
  replay(root, repo);
  const c = config(repo);
  const { index, errors } = buildIndex(repo, c);
  if (errors.length) {
    throw Error(`fixture: ${errors.join('; ')}`);
  }
  const history = runs(repo, index, c, generatedAt);
  const order = ['running', 'approved', 'draft', 'review', 'legacy', 'done'];
  writeFileSync(file, JSON.stringify({
    project,
    commit: git(repo, ['log', '-1', '--format=%h · %cI']),
    dirty: false,
    generatedAt,
    tasks: history.tasks,
    goals: [...index.goals].sort((a, b) => order.indexOf(a.state) - order.indexOf(b.state)),
    runs: history.runs,
    warnings: [...index.warnings, ...history.warnings],
  }, null, 2) + '\n');
  console.log(`fixture: ${index.tasks.length} tasks, ${index.goals.length} goals, ${history.runs.length} runs`);
} finally {
  rmSync(dir, { recursive: true, force: true });
}
