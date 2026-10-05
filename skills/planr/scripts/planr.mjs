#!/usr/bin/env node
import { existsSync, readFileSync, writeFileSync, mkdirSync, readdirSync, chmodSync, rmSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'node:http';
import {
  config, findRoot, buildIndex, json, compare, statuses, states, rewrite, nextWork,
} from './lib/graph.mjs';
import { boardData, render, writeBoard } from './lib/board.mjs';
import { git, runs } from './lib/runs.mjs';
import { version } from './lib/version.mjs';

const argv = process.argv.slice(2);
const options = {};
const positional = [];
const flags = ['json', 'strict', 'ready', 'serve', 'in-repo'];
const allowed = {
  check: ['strict'],
  index: ['strict'],
  list: ['status', 'track', 'owner', 'goal', 'ready'],
  next: ['goal', 'stream'],
  set: ['owner'],
  goal: ['thread'],
  board: ['out', 'serve', 'port'],
  init: ['in-repo'],
};
const message = text => console.error(`planr: ${String(text).replace(/[\r\n]+/g, ' ')}`);
const usage = text => {
  const e = Error(text);
  e.code = 2;
  throw e;
};
const output = (value, lines = []) => options.json
  ? console.log(json(value).trimEnd())
  : lines.forEach(line => console.log(`planr: ${String(line).replace(/[\r\n]+/g, ' ')}`));

function checked(root, c) {
  const result = buildIndex(root, c);
  if (result.errors.length) {
    const e = Error(result.errors.join('; '));
    e.code = 1;
    e.errors = result.errors;
    throw e;
  }
  return result.index;
}

function check(root, c, target) {
  const result = buildIndex(root, c);
  const warnings = [...result.index.warnings];
  if (!result.errors.length) {
    warnings.push(...runs(root, result.index, c).warnings);
  }
  const errors = options.strict ? [...result.errors, ...warnings] : result.errors;
  process.exitCode = errors.length ? 1 : 0;
  if (target) {
    const scope = target.id ? [target.id]
      : result.index.goals.find(g => g.name === target.name)?.scope ?? target.scope;
    const files = [target.file, ...result.index.tasks.filter(t => scope.includes(t.id)).map(t => t.file)];
    const relevant = warnings.filter(w =>
      scope.some(id => w.startsWith(`${id}:`) || w.includes(`: dependency ${id} is dropped;`)) ||
      files.some(file => w.startsWith(`${file}:`) ||
        w.endsWith(` ${file} unreadable, skipped`) || w.endsWith(`/${file} unreadable, skipped`)
      )
    );
    return {
      index: result.index,
      ok: !errors.length,
      errors,
      warnings: relevant,
      otherWarnings: warnings.length - relevant.length,
    };
  }
  output(
    { ok: !errors.length, errors, warnings },
    [...warnings, ...result.errors, errors.length ? 'check failed' : 'check passed']
  );
}

function mutationOutput(value, line, validation) {
  const { index, ...diagnostics } = validation;
  output({ ...value, ...diagnostics }, [
    line,
    ...diagnostics.errors,
    ...diagnostics.warnings,
    ...(diagnostics.otherWarnings ? [`${diagnostics.otherWarnings} other warnings (run check)`] : []),
  ]);
}

function init(root) {
  const planning = options['in-repo'] ? join(root, '.planr') : root;
  const source = fileURLToPath(new URL('../assets/templates/', import.meta.url));
  const create = (file, content, mode) => {
    if (existsSync(file)) {
      return;
    }
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, content);
    if (mode) {
      chmodSync(file, mode);
    }
    message(`created ${file}`);
  };
  for (const [from, to] of [
    ['tasks-README.md', 'tasks/README.md'],
    ['goal.md', 'plans/example-goal.md'],
  ]) {
    create(join(planning, to), readFileSync(join(source, from)));
  }
  const configTemplate = options['in-repo'] ? 'in-repo.config.json' : 'planr.config.json';
  create(join(root, 'planr.config.json'), readFileSync(join(source, configTemplate)));
  const ignore = join(root, '.gitignore');
  const previous = existsSync(ignore) ? readFileSync(ignore, 'utf8') : '';
  const entry = '.planr/board.html';
  const lines = (previous.match(/[^\n]*\n|[^\n]+$/g) ?? []).filter(s =>
    !['.planr/', '/.planr/', '.planr'].includes(s.trim())
  );
  let content = lines.join('');
  if (!lines.some(s => [entry, '/' + entry, entry.replace(/\/$/, '')].includes(s.trim()))) {
    content += (content && !content.endsWith('\n') ? '\n' : '') + entry + '\n';
  }
  if (content !== previous) {
    writeFileSync(ignore, content);
  }
  const hooks = git(root, ['rev-parse', '--git-path', 'hooks']);
  if (hooks) {
    for (const hook of ['post-commit', 'post-merge']) {
      const file = resolve(root, hooks, hook);
      const old = existsSync(file) ? readFileSync(file, 'utf8') : '';
      const generated = old.startsWith('#!/bin/sh\nentry="') && old.split('\n').length === 5 &&
        old.endsWith('node "$entry" board >/dev/null 2>&1 || true\n');
      if (old === '#!/bin/sh\nplanr board\n' || generated) {
        rmSync(file);
        message(`removed ${file}`);
      }
    }
    create(
      resolve(root, hooks, 'pre-commit'),
      readFileSync(join(source, 'hook.sh'), 'utf8').replace('/*ENTRY*/', () =>
        fileURLToPath(import.meta.url).replace(/["\\$`]/g, ch => '\\' + ch)
      ),
      0o755
    );
  }
  message('initialized');
}

function serve(root, c, port) {
  let html;
  let version = 0;
  let signature;
  const scan = () => {
    const data = [git(root, ['rev-parse', 'HEAD']), git(root, ['rev-parse', '--git-path', 'HEAD'])];
    for (const dir of [c.tasksDir, c.goalsDir]) {
      if (existsSync(join(root, dir))) {
        for (const f of readdirSync(join(root, dir), { recursive: true }).sort()) {
          try {
            data.push(f, readFileSync(join(root, dir, f), 'utf8'));
          } catch {
            /* directories */
          }
        }
      }
    }
    const head = data[1];
    if (head && existsSync(resolve(root, head))) {
      data.push(readFileSync(resolve(root, head), 'utf8'));
    }
    const next = JSON.stringify(data);
    if (next === signature) {
      return;
    }
    signature = next;
    const page = render(boardData(root, checked(root, c), c));
    version++;
    html = page.replace('</body>', () => `<script>
setInterval(async()=>{
  try {
    if ((await (await fetch('/version',{cache:'no-store'})).text()) !== '${version}') {
      location.reload();
    }
  } catch {}
},700);
</script></body>`);
  };
  scan();
  const timer = setInterval(() => {
    try {
      scan();
    } catch (e) {
      message(e.message);
    }
  }, 500);
  const server = createServer((req, res) => {
    res.setHeader('Cache-Control', 'no-store');
    res.setHeader('Content-Type', req.url === '/version' ? 'text/plain' : 'text/html; charset=utf-8');
    if (!['/', '/version'].includes(req.url)) {
      res.writeHead(404);
      res.end();
      return;
    }
    res.end(req.url === '/version' ? String(version) : html);
  });
  server.on('error', e => {
    clearInterval(timer);
    message(e.message);
    process.exitCode = 1;
  });
  server.listen(port, '127.0.0.1', () => message(`serving http://127.0.0.1:${server.address().port}`));
  const stop = () => {
    clearInterval(timer);
    server.close();
  };
  process.on('SIGINT', stop);
  process.on('SIGTERM', stop);
}

try {
  if (argv.length === 1 && argv[0] === '--version') {
    console.log(`planr ${version()}`);
    process.exit(0);
  }
  if (!argv.length || (argv.length === 1 && argv[0] === '--help')) {
    console.log(`Usage: planr <command> [options]

Commands:
  check [--strict]             Validate tasks and goals (index is a deprecated alias)
  list [--ready]               List tasks; filter by --status, --track, --owner or --goal
  next --goal <name>           Show ready and waiting tasks (--stream <S> also works)
  set <id> <status>            Change task status [--owner <stream>]
  goal <name> [<state>]        Show or change a goal [--thread <id>]
  board [--out <file>]         Render the board [--serve] [--port <n>]
  init [--in-repo]             Scaffold a planning repository

Options: --root <dir>, --json, --help, --version`);
    process.exit(0);
  }
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i].startsWith('--')) {
      positional.push(argv[i]);
      continue;
    }
    const key = argv[i].slice(2);
    if (key in options) {
      usage(`duplicate --${key}`);
    }
    if (flags.includes(key)) {
      options[key] = true;
    } else {
      if (!argv[i + 1] || argv[i + 1].startsWith('--')) {
        usage(`--${key} requires a value`);
      }
      options[key] = argv[++i];
    }
  }
  const [command, id, value, ...extra] = positional;
  if (!allowed[command]) {
    usage('usage: planr check | list | next | set | goal | board | init');
  }
  for (const key of Object.keys(options)) {
    if (!['root', 'json', ...allowed[command]].includes(key)) {
      usage(`unknown option --${key}`);
    }
  }
  const limit = ['set', 'goal'].includes(command) ? 3 : 1;
  if (positional.length > limit || extra.length) {
    usage('too many arguments');
  }
  if (command === 'set' && (!id || !statuses.includes(value))) {
    usage('set requires a task ID and a known status');
  }
  if (command === 'goal' && (!id || (value && !states.includes(value)))) {
    usage('goal requires a name and an optional known state');
  }
  if (command === 'next' && !!options.goal === !!options.stream) {
    usage('next requires exactly one of --goal or --stream');
  }
  const root = options.root
    ? resolve(options.root)
    : command === 'init'
      ? process.cwd()
      : findRoot(process.cwd());
  if (command === 'init') {
    init(root);
  } else {
    const c = config(root);
    if (['check', 'index'].includes(command)) {
      if (command === 'index') {
        message('index is deprecated, use check');
      }
      check(root, c);
    } else {
      const data = checked(root, c);
      const goal = name => {
        const g = data.goals.find(g => g.name === name);
        if (!g) {
          usage(`unknown goal ${name}`);
        }
        return g;
      };
      const lines = tasks => tasks.map(t =>
        `${t.id} ${t.status} ${t.track} ${t.title} (unlocks ${t.unlocks})`
      );
      if (command === 'list') {
        const order = ['blocked', 'decision', 'in-progress', 'review', 'open', 'done', 'dropped'];
        const scope = options.goal ? goal(options.goal).scope : null;
        const tasks = data.tasks
          .filter(t =>
            (!options.status || options.status.split(',').includes(t.status)) &&
            (!options.track || t.track === options.track) &&
            (!options.owner || t.owner === options.owner) &&
            (!scope || scope.includes(t.id)) &&
            (!options.ready || t.ready)
          )
          .sort((a, b) =>
            order.indexOf(a.status) - order.indexOf(b.status) ||
            b.waitingOn.length - a.waitingOn.length ||
            compare(a.id, b.id)
          );
        output(tasks, lines(tasks));
      } else if (command === 'next') {
        const goals = options.goal
          ? [goal(options.goal)]
          : data.goals.filter(g =>
              g.stream === options.stream && ['approved', 'running'].includes(g.state)
            );
        const stream = options.stream || goals[0].stream;
        const { ready, waiting } = nextWork(data, goals, stream);
        output(
          { [options.goal ? 'goal' : 'stream']: options.goal || options.stream, ready, waiting },
          ['ready', ...lines(ready), 'waiting', ...lines(waiting)]
        );
      } else if (command === 'set') {
        const task = data.tasks.find(t => t.id === id);
        if (!task) {
          usage(`unknown task ${id}`);
        }
        rewrite(join(root, task.file), {
          status: value,
          ...(options.owner ? { owner: options.owner } : {}),
        });
        const validation = check(root, c, task);
        mutationOutput(
          { id, from: task.status, to: value }, `${id}: ${task.status} -> ${value}`, validation
        );
      } else if (command === 'goal') {
        const g = goal(id);
        let validation;
        if (value || options.thread) {
          rewrite(join(root, g.file), {
            ...(value ? { state: value } : {}),
            ...(options.thread ? { thread: options.thread } : {}),
          });
          validation = check(root, c, g);
        }
        const updated = (validation?.index ?? data).goals.find(g => g.name === id) ?? g;
        const line = `${updated.name} ${updated.state} ${updated.done}/${updated.total}` +
          ` thread=${updated.thread ?? '-'}`;
        if (validation) {
          mutationOutput(updated, line, validation);
        } else {
          output(updated, [line]);
        }
      } else if (command === 'board') {
        if (options.port && (!/^\d+$/.test(options.port) || +options.port > 65535)) {
          usage('port must be 0..65535');
        }
        message(`wrote ${writeBoard(root, data, c, options.out).file}`);
        if (options.serve) {
          serve(root, c, +(options.port ?? 4173));
        }
      }
    }
  }
} catch (e) {
  if (options.json) {
    console.log(json({
      ok: false,
      errors: e.errors ?? [e.message],
      warnings: [],
    }).trimEnd());
  } else {
    message(e.message);
  }
  process.exitCode = typeof e.code === 'number' ? e.code : 1;
}
