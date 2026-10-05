import { test } from 'node:test';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { runs } from '../skills/planr/scripts/lib/runs.mjs';
import fs from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  buildIndex, json, frontmatter, rewrite, nextWork, config,
} from '../skills/planr/scripts/lib/graph.mjs';

function fixture(fn) {
  const root = mkdtempSync(join(tmpdir(), 'planr-graph-'));
  mkdirSync(join(root, 'tasks'));
  mkdirSync(join(root, 'plans'));
  const task = (id, status = 'open', depends = [], extra = '') => {
    const header = [
      '---',
      `id: ${id}`,
      `title: ${id}`,
      `status: ${status}`,
      'track: any',
      `depends: [${depends.join(', ')}]`,
      '',
    ].join('\n');
    writeFileSync(join(root, 'tasks', `${id}.md`), header + extra + '---\nBody\n');
  };
  try {
    fn(root, task);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test('readiness and unlocks: review satisfies dependencies; dropped does not', () => fixture((root, task) => {
  task('T1');
  task('T2', 'open', ['T1']);
  task('T3', 'open', ['T1', 'T4', 'T8']);
  task('T4', 'done');
  task('T5', 'dropped');
  task('T6', 'open', ['T5']);
  task('T7', 'dropped', ['T1']);
  task('T8', 'review');
  task('T9', 'open', ['T8']);
  const { index, errors } = buildIndex(root);
  const byId = Object.fromEntries(index.tasks.map(t => [t.id, t]));
  assert.deepEqual(errors, []);
  assert.equal(byId.T1.ready, true);
  assert.equal(byId.T1.unlocks, 2);
  assert.deepEqual(byId.T1.waitingOn, ['T2', 'T3']);
  assert.deepEqual(byId.T3.openDeps, ['T1']);
  assert.equal(byId.T6.ready, false);
  assert.deepEqual(byId.T6.openDeps, ['T5']);
  assert.equal(byId.T5.unlocks, 0);
  assert.equal(byId.T4.unlocks, 0);
  assert.equal(byId.T8.unlocks, 0);
  assert.equal(byId.T9.ready, true);
  assert.deepEqual(byId.T9.openDeps, []);
  assert.match(index.warnings[0], /T6: dependency T5 is dropped/);
}));

test('cycles report a path and unknown dependencies are errors', () => fixture((root, task) => {
  task('A', 'open', ['B']);
  task('B', 'open', ['C']);
  task('C', 'open', ['A', 'UNKNOWN']);
  assert.deepEqual(buildIndex(root).errors, ['C: unknown dependency UNKNOWN', 'cycle: A -> B -> C -> A']);
}));

test('owners and legacy goals warn; explicit goals derive their scope', () => fixture((root, task) => {
  task('A', 'in-progress');
  task('B', 'review');
  task('C', 'done');
  task('D', 'open', ['B']);
  task('E', 'blocked');
  task('F', 'decision');
  writeFileSync(join(root, 'plans', 'old-goal.md'), '# Old goal mentions A\n');
  writeFileSync(join(root, 'plans', 'new-goal.md'), [
    '---',
    'title: "Goal # one"',
    'state: approved # comment',
    'scope:',
    '  - B',
    '  - C',
    '  - D',
    '  - E',
    '  - F',
    '---',
    '',
  ].join('\n'));
  const { index, errors } = buildIndex(root);
  assert.deepEqual(errors, []);
  assert.deepEqual(index.warnings, [
    'A: in-progress without owner',
    'plans/old-goal.md: no frontmatter, listed as legacy',
  ]);
  assert.equal(index.goals[0].done, 1);
  assert.equal(index.goals[0].review, 1);
  assert.equal(index.goals[1].review, 0);
  assert.deepEqual(index.goals[0].ready, ['D']);
  assert.deepEqual(index.goals[0].waiting, ['E', 'F']);
  assert.equal(index.goals[1].state, 'legacy');
  assert.deepEqual(index.goals[1].scope, []);
}));

test('stream uses owner or the first active goal by name; only unassigned active tasks warn', () =>
  fixture((root, task) => {
    task('A', 'in-progress', [], 'owner: thr_owner\n');
    task('B', 'in-progress');
    task('C', 'review');
    task('D', 'review');
    task('E', 'in-progress');
    task('F');
    task('G');
    task('H', 'review');
    for (const [name, state, stream, scope] of [
      ['a-1', 'running', 'S2', 'B'],
      ['a', 'approved', 'S1', 'A, B, F'],
      ['d', 'approved', 'S4', 'D'],
      ['e', 'running', '', 'E'],
      ['h', 'running', 'S5', 'H'],
      ['draft', 'draft', 'S3', 'C'],
      ['review', 'review', 'S3', 'C'],
      ['done', 'done', 'S3', 'C'],
    ]) {
      writeFileSync(join(root, 'plans', `${name}-goal.md`), [
        '---',
        `state: ${state}`,
        `stream: ${stream}`,
        `scope: [${scope}]`,
        '---',
        '',
      ].join('\n'));
    }
    const { index, errors } = buildIndex(root);
    assert.deepEqual(errors, []);
    assert.deepEqual(index.tasks.map(t => [t.id, t.stream]), [
      ['A', 'thr_owner'],
      ['B', 'S1'],
      ['C', null],
      ['D', 'S4'],
      ['E', null],
      ['F', 'S1'],
      ['G', null],
      ['H', 'S5'],
    ]);
    assert.deepEqual(index.warnings, ['E: in-progress without owner']);
  })
);

test('data validates required fields, filename, status and goal scope', () => fixture((root, task) => {
  task('A', 'unexpected', ['missing']);
  writeFileSync(join(root, 'tasks', 'wrong.md'), '---\nid: other\nstatus: open\n---\n');
  writeFileSync(join(root, 'plans', 'bad-goal.md'), '---\nstate: draft\nscope: [missing]\n---\n');
  const errors = buildIndex(root).errors.join('\n');
  for (const pattern of [
    /unknown status/,
    /missing or invalid title/,
    /missing or invalid track/,
    /does not match file name/,
    /unknown dependency/,
    /unknown scope/,
  ]) {
    assert.match(errors, pattern);
  }
}));

test('deterministic numeric-aware data and scalar/list subset', () => fixture((root, task) => {
  task('T10');
  task('T2');
  task('T1');
  const titleFile = join(root, 'tasks', 'T1.md');
  writeFileSync(titleFile, readFileSync(titleFile, 'utf8').replace('title: T1', 'title: "Task title"'));
  assert.equal(buildIndex(root).index.tasks[0].title, 'Task title');
  task('T2', 'open', [], 'refs: [doc#a, doc:b]\n');
  assert.deepEqual(buildIndex(root).index.tasks[1].refs, ['doc#a', 'doc:b']);
  const text = json(buildIndex(root).index);
  assert.equal(text, json(buildIndex(root).index));
  assert.deepEqual(buildIndex(root).index.tasks.map(t => t.id), ['T1', 'T2', 'T10']);
  assert.ok(text.endsWith('\n'));
  assert.throws(
    () => frontmatter('---\nstatus: open\nstatus: done\n---\n'),
    /duplicate frontmatter key status/
  );
  assert.equal(frontmatter("---\ntitle: Agent's plan\n---\n").title, "Agent's plan");
  assert.deepEqual(
    frontmatter('---\r\ntitle: \'It\'\'s # fine\'\r\nrefs: ["a,b", x#y]\r\n---\r\n'),
    { title: "It's # fine", refs: ['a,b', 'x#y'] }
  );
}));

test('status/owner edit leaves other frontmatter and body byte-identical', () => fixture((root, task) => {
  task('A', 'open', [], 'owner: old\nrefs: [x, y]\n');
  const file = join(root, 'tasks', 'A.md');
  const before = readFileSync(file, 'utf8');
  rewrite(file, { status: 'review', owner: 'S9' });
  assert.equal(
    readFileSync(file, 'utf8'),
    before.replace('status: open', 'status: review').replace('owner: old', 'owner: S9')
  );
  writeFileSync(file, before.replace('owner: old\n', '').replaceAll('\n', '\r\n'));
  rewrite(file, { status: 'review', owner: 'S9' });
  assert.equal(
    readFileSync(file, 'utf8'),
    before.replace('owner: old\n', '')
      .replace('status: open', 'status: review')
      .replace('\n---\nBody', '\nowner: S9\n---\nBody')
      .replaceAll('\n', '\r\n')
  );
  for (const owner of [' S9 ', 'S9: thread', 'S9 # note', '[S9]', '"S9"', 'null', 'true', '12']) {
    writeFileSync(file, before);
    rewrite(file, { owner });
    const after = readFileSync(file, 'utf8');
    assert.equal(after, before.replace('owner: old', `owner: ${JSON.stringify(owner)}`));
    assert.equal(frontmatter(after).owner, owner);
  }
  for (const field of ['status', 'owner']) {
    const duplicate = before.replace('\n---\n', `\n${field}: repeated\n---\n`);
    writeFileSync(file, duplicate);
    assert.throws(() => rewrite(file, { [field]: 'new' }), /duplicate frontmatter key/);
    assert.equal(readFileSync(file, 'utf8'), duplicate);
    const list = before.replace(/^owner:.*$/m, 'owner:\n  - S9');
    writeFileSync(file, list);
    assert.throws(() => rewrite(file, { owner: 'S1' }), /owner must be a scalar/);
    assert.equal(readFileSync(file, 'utf8'), list);
  }
  const originalWrite = fs.writeFileSync;
  for (const failure of ['write', 'concurrent']) {
    originalWrite(file, before);
    fs.writeFileSync = (path, ...args) => {
      if (failure === 'write') {
        originalWrite(path, 'partial');
        throw Error('injected write failure');
      }
      originalWrite(file, before + 'Concurrent edit\n');
      return originalWrite(path, ...args);
    };
    syncBuiltinESMExports();
    try {
      assert.throws(() => rewrite(file, { status: 'done' }), /injected write failure|changed during rewrite/);
      assert.equal(readFileSync(file, 'utf8'), before + (failure === 'write' ? '' : 'Concurrent edit\n'));
      assert.deepEqual(fs.readdirSync(join(root, 'tasks')), ['A.md']);
    } finally {
      fs.writeFileSync = originalWrite;
      syncBuiltinESMExports();
    }
  }
}));

const entry = fileURLToPath(new URL('../skills/planr/scripts/planr.mjs', import.meta.url));
const cli = (root, args) => spawnSync(process.execPath, [entry, '--root', root, ...args], {
  encoding: 'utf8',
});
const commit = (root, subject, date = '2026-01-01T12:00:00Z') => {
  const options = {
    cwd: root,
    encoding: 'utf8',
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: 'Test',
      GIT_AUTHOR_EMAIL: 'test@example.invalid',
      GIT_COMMITTER_NAME: 'Test',
      GIT_COMMITTER_EMAIL: 'test@example.invalid',
      GIT_AUTHOR_DATE: date,
      GIT_COMMITTER_DATE: date,
    },
  };
  execFileSync('git', ['add', '-A'], options);
  execFileSync('git', ['commit', '-qm', subject], options);
  return execFileSync('git', ['rev-parse', '--short', 'HEAD'], options).trim();
};

test('check validates without writes and ignores a legacy index; strict treats warnings as errors', () =>
  fixture((root, task) => {
    execFileSync('git', ['init', '-q', root]);
    task('A', 'in-progress');
    const placeholder = join(root, 'tasks/index.json');
    writeFileSync(placeholder, 'not JSON: ignored');
    const before = readFileSync(placeholder, 'utf8');
    const result = cli(root, ['check', '--json']);
    assert.equal(result.status, 0);
    assert.deepEqual(JSON.parse(result.stdout), {
      ok: true,
      errors: [],
      warnings: ['A: in-progress without owner'],
    });
    assert.equal(cli(root, ['check', '--strict']).status, 1);
    const alias = cli(root, ['index', '--json']);
    assert.equal(alias.status, 0);
    assert.equal(alias.stderr, 'planr: index is deprecated, use check\n');
    assert.equal(readFileSync(placeholder, 'utf8'), before);
    assert.deepEqual(fs.readdirSync(root).sort(), ['.git', 'plans', 'tasks']);
    task('A', 'invalid');
    assert.equal(cli(root, ['check']).status, 1);
  })
);

test('next excludes other owners for stream and goal; overlapping running scopes warn once', () =>
  fixture((root, task) => {
    task('A');
    task('B', 'open', [], 'owner: S2\n');
    task('C', 'blocked', [], 'owner: S1\n');
    task('D', 'decision', [], 'owner: S2\n');
    for (const name of ['one', 'two']) {
      writeFileSync(join(root, 'plans', `${name}-goal.md`),
        '---\nstate: running\nstream: S1\nscope: [A, B, C, D]\n---\n');
    }
    const { index } = buildIndex(root);
    assert.deepEqual(index.warnings, ['A', 'B', 'C', 'D'].map(id =>
      `${id}: in multiple running goals: one, two`
    ));
    assert.deepEqual(nextWork(index, index.goals, 'S1').ready.map(t => t.id), ['A']);
    for (const args of [['--stream', 'S1'], ['--goal', 'one']]) {
      const result = cli(root, ['next', ...args, '--json']);
      assert.equal(result.status, 0);
      const data = JSON.parse(result.stdout);
      assert.deepEqual(data.ready.map(t => t.id), ['A']);
      assert.deepEqual(data.waiting.map(t => t.id), ['C']);
    }
  })
);

test('task history derives status runs, removals, counts, last touch and a working-tree run', () =>
  fixture((root, task) => {
    execFileSync('git', ['init', '-q', root]);
    task('A');
    commit(root, 'add A');
    const file = join(root, 'tasks/A.md');
    writeFileSync(file, readFileSync(file, 'utf8') + 'Body edit\n');
    commit(root, 'body only', '2026-01-02T12:00:00Z');
    task('A', 'review');
    task('B');
    commit(root, 'review A, add B', '2026-01-03T12:00:00Z');
    writeFileSync(file, '---\nstatus: "unclosed\n---\n');
    const unreadable = commit(root, 'broken snapshot', '2026-01-04T12:00:00Z');
    task('A', 'review');
    rmSync(join(root, 'tasks/B.md'));
    commit(root, 'repair body, remove B', '2026-01-05T12:00:00Z');
    const c = config(root);
    const derive = () => runs(root, buildIndex(root).index, c, '2026-01-06T11:00:00Z');
    const data = derive();
    assert.deepEqual(data.runs.map(r => r.subject), ['add A', 'review A, add B', 'repair body, remove B']);
    assert.deepEqual(data.runs.map(r => r.counts), [{ open: 1 }, { review: 1, open: 1 }, { review: 1 }]);
    assert.deepEqual(data.runs[1].changes, [
      { id: 'A', from: 'open', to: 'review' },
      { id: 'B', from: null, to: 'open' },
    ]);
    assert.deepEqual(data.runs[2].changes, [{ id: 'B', from: 'open', to: 'removed' }]);
    assert.deepEqual(data.warnings, [`history: ${unreadable} tasks/A.md unreadable, skipped`]);
    assert.equal(data.tasks[0].lastRun, 1);
    assert.deepEqual(data.tasks[0].history, [{ run: 1, from: 'open', to: 'review' }]);
    task('A', 'done');
    const working = derive().runs.at(-1);
    assert.equal(working.commit, null);
    assert.deepEqual(working.changes, [{ id: 'A', from: 'review', to: 'done' }]);
    assert.deepEqual(working.counts, { done: 1 });
  })
);

test('first-parent merges reconstruct HEAD; shallow and non-git histories report incomplete runs', () =>
  fixture((root, task) => {
    execFileSync('git', ['init', '-q', root]);
    task('A', 'review', [], 'owner: S1\n');
    commit(root, 'add A');
    const git = args => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
    const branch = git(['symbolic-ref', '--short', 'HEAD']);
    git(['checkout', '-qb', 'side']);
    task('A', 'done', [], 'owner: S1\n');
    commit(root, 'side status', '2026-01-02T12:00:00Z');
    git(['checkout', '-q', branch]);
    const file = join(root, 'tasks/A.md');
    writeFileSync(file, readFileSync(file, 'utf8') + 'Body edit\n');
    commit(root, 'main body', '2026-01-03T12:00:00Z');
    execFileSync('git', [
      '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid',
      'merge', '--no-ff', 'side', '-m', 'merge',
    ], {
      cwd: root,
      env: {
        ...process.env,
        GIT_AUTHOR_DATE: '2026-01-04T12:00:00Z',
        GIT_COMMITTER_DATE: '2026-01-04T12:00:00Z',
      },
    });
    const now = '2026-01-10T12:00:00Z';
    const derive = dir => runs(dir, buildIndex(dir).index, config(dir), now);
    assert.deepEqual(derive(root).runs.map(r => r.subject), ['add A', 'merge']);
    assert.deepEqual(derive(root).runs[1].changes, [{ id: 'A', from: 'review', to: 'done' }]);
    task('A', 'review', [], 'owner: S1\n');
    const shallow = join(root, 'shallow');
    execFileSync('git', ['clone', '-q', '--depth=1', '--no-local', root, shallow]);
    rewrite(join(shallow, 'tasks/A.md'), { status: 'review' });
    assert.deepEqual(derive(shallow).warnings, ['history: shallow clone, runs incomplete']);
    rmSync(join(root, '.git'), { recursive: true, force: true });
    assert.deepEqual(derive(root).runs, []);
    assert.deepEqual(derive(root).warnings, ['history: not a git repository, runs unavailable']);
  })
);

test('mutations report relevant warnings and summarize others in human and JSON output', () =>
  fixture((root, task) => {
    execFileSync('git', ['init', '-q', root]);
    task('T0');
    for (let i = 1; i <= 37; i++) {
      task(`T${i}`, 'in-progress');
    }
    const set = cli(root, ['set', 'T0', 'review', '--json']);
    assert.equal(set.status, 0);
    assert.equal(set.stderr, '');
    assert.deepEqual(JSON.parse(set.stdout), {
      id: 'T0', from: 'open', to: 'review', ok: true,
      errors: [], warnings: [], otherWarnings: 37,
    });
    const human = cli(root, ['set', 'T0', 'in-progress']);
    assert.equal(human.status, 0);
    assert.deepEqual(human.stdout.trim().split('\n'), [
      'planr: T0: review -> in-progress',
      'planr: T0: in-progress without owner',
      'planr: 37 other warnings (run check)',
    ]);
    writeFileSync(join(root, 'plans/g-goal.md'), '---\nstate: draft\nstream: S1\nscope: [T0]\n---\n');
    const goal = cli(root, ['goal', 'g', 'approved', '--json']);
    assert.equal(goal.status, 0);
    assert.equal(goal.stderr, '');
    const data = JSON.parse(goal.stdout);
    assert.deepEqual([data.errors, data.warnings, data.otherWarnings], [[], [], 37]);
    assert.equal(data.state, 'approved');
    const review = cli(root, ['goal', 'g', 'review']);
    assert.equal(review.status, 0);
    assert.deepEqual(review.stdout.trim().split('\n'), [
      'planr: g review 0/1 thread=-',
      'planr: T0: in-progress without owner',
      'planr: 37 other warnings (run check)',
    ]);
    assert.equal(JSON.parse(cli(root, ['check', '--json']).stdout).warnings.length, 38);
  })
);
