import {
  mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, openSync, closeSync, existsSync, realpathSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';

// Everything the site shows of planr at work comes from the synthetic sample in fixtures/:
// a throwaway planning repo replays its runs as commits, the real CLI runs the landing's session
// in it, and the board is the file that session's `planr board --out` wrote.

export const FONTS = ['MonaSans-Variable', 'MonaspaceNeon-400', 'MonaspaceNeon-500', 'MonaspaceNeon-600'];

// The renderer inlines the fonts; the site serves the same files once, so the board shares the cache.
export function shareFonts(root, html) {
  for (const font of FONTS) {
    const file = `${root}/skills/planr/assets/board/fonts/${font}.woff2`;
    const inlined = `data:font/woff2;base64,${readFileSync(file).toString('base64')}`;
    if (!html.includes(inlined)) {
      throw Error(`board: font ${font} is not inlined where expected`);
    }
    html = html.replaceAll(inlined, `/fonts/${font}.woff2`);
  }
  return html;
}

// A fixed identity and no user or system config, so the replayed hashes are the same on every machine.
const GIT_ENV = {
  GIT_AUTHOR_NAME: 'planr sample', GIT_AUTHOR_EMAIL: 'sample@planr.invalid',
  GIT_COMMITTER_NAME: 'planr sample', GIT_COMMITTER_EMAIL: 'sample@planr.invalid',
  GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1',
};

// Replays the sample's runs as commits, so the CLI sees a planning repo with real history.
// A task whose first change starts from null is created in that run; a goal file is committed with the
// first run in which all of its scope tasks exist.
export function replay(root, dir, bodies = {}) {
  const data = JSON.parse(readFileSync(`${root}/fixtures/board-data.json`, 'utf8'));
  mkdirSync(join(dir, 'tasks'), { recursive: true });
  mkdirSync(join(dir, 'plans'), { recursive: true });
  const env = { ...process.env, ...GIT_ENV };
  const git = (args, extra = {}) =>
    execFileSync('git', args, { cwd: dir, env: { ...env, ...extra }, stdio: ['ignore', 'pipe', 'pipe'] });
  const status = new Map(data.tasks.map(t => [t.id, t.history.length ? t.history[0].from : t.status]));
  const list = items => `[${items.join(', ')}]`;
  const task = t => [
    '---',
    `id: ${t.id}`,
    `title: ${t.title}`,
    `status: ${status.get(t.id)}`,
    `track: ${t.track}`,
    ...(t.depends.length ? [`depends: ${list(t.depends)}`] : []),
    ...(t.owner ? [`owner: ${t.owner}`] : []),
    '---',
    '',
    bodies[t.id] ?? `## Goal\n${t.title}.`,
    '',
  ].join('\n');
  git(['init', '-q', '-b', 'main']);
  writeFileSync(join(dir, 'planr.config.json'),
    readFileSync(`${root}/skills/planr/assets/templates/planr.config.json`));
  const goal = g => {
    writeFileSync(join(dir, g.file), [
      '---',
      `title: ${g.title}`,
      `state: ${g.state}`,
      ...(g.stream ? [`stream: ${g.stream}`] : []),
      ...(g.budget ? [`budget: ${g.budget}`] : []),
      ...(g.thread ? [`thread: ${g.thread}`] : []),
      `scope: ${list(g.scope)}`,
      '---',
      '',
      `${g.title}.`,
      '',
    ].join('\n'));
  };
  const written = new Set();
  data.runs.forEach((run, i) => {
    for (const t of data.tasks) {
      for (const h of t.history) {
        if (h.run === i) {
          status.set(t.id, h.to);
        }
      }
    }
    for (const t of data.tasks.filter(t => status.get(t.id))) {
      writeFileSync(join(dir, 'tasks', `${t.id}.md`), task(t));
    }
    for (const g of data.goals.filter(g => !written.has(g.name) && g.scope.every(id => status.get(id)))) {
      goal(g);
      written.add(g.name);
    }
    git(['add', '-A']);
    git(['commit', '-q', '--allow-empty', '--no-gpg-sign', '-m', run.subject],
      { GIT_AUTHOR_DATE: run.date, GIT_COMMITTER_DATE: run.date });
  });
  return data;
}

// Splits a displayed command line into argv; double quotes group words, nothing else is special.
const argv = line => [...line.matchAll(/"([^"]*)"|(\S+)/g)].map(m => m[1] ?? m[2]);

// Runs a session in a planning repo replayed from the sample: `planr` is the real CLI from this
// repository, `git` and `cat` run as they are. Commands run in order in one working tree, and
// stdout and stderr stay interleaved. Temporary paths print as ~, like a home directory.
// `fresh` starts from an empty Git repository instead of the sample.
export function session(root, commands, { fresh = false, name = 'planning', date, bodies } = {}) {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'planr-site-')));
  const repo = join(dir, name);
  const env = {
    ...process.env, ...GIT_ENV,
    ...(date ? { GIT_AUTHOR_DATE: date, GIT_COMMITTER_DATE: date } : {}),
  };
  try {
    if (fresh) {
      mkdirSync(repo);
      execFileSync('git', ['init', '-q', '-b', 'main'], { cwd: repo, env });
    } else {
      replay(root, repo, bodies);
    }
    const steps = commands.map(line => {
      const [cmd, ...args] = argv(line);
      if (!['planr', 'git', 'cat'].includes(cmd)) {
        throw Error(`sample: unsupported command ${line}`);
      }
      const [file, list] = cmd === 'planr'
        ? [process.execPath, [`${root}/skills/planr/scripts/planr.mjs`, ...args]]
        : [cmd, cmd === 'git' ? ['-c', 'commit.gpgsign=false', ...args] : args];
      const log = join(dir, 'out.txt');
      const fd = openSync(log, 'w');
      const result = spawnSync(file, list, { cwd: repo, stdio: ['ignore', fd, fd], env });
      closeSync(fd);
      const output = readFileSync(log, 'utf8').trimEnd().replaceAll(dir, '~');
      if (result.status !== 0) {
        throw Error(`sample: "${line}" exited ${result.status}:\n${output}`);
      }
      return { command: line, output: output ? output.split('\n') : [] };
    });
    const board = existsSync(join(repo, '.planr/board.html'))
      ? readFileSync(join(repo, '.planr/board.html'), 'utf8') : null;
    return { steps, board };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}
