import { existsSync, readFileSync, readdirSync, writeFileSync, renameSync, rmSync, statSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { join, resolve, relative, dirname } from 'node:path';

export const statuses = ['open', 'in-progress', 'review', 'blocked', 'decision', 'done', 'dropped'];
export const states = ['draft', 'approved', 'running', 'done', 'review'];
export const compare = (a, b) =>
  a.localeCompare(b, 'en', { numeric: true }) || (a < b ? -1 : a > b ? 1 : 0);
export const json = value => JSON.stringify(value, null, 2) + '\n';

export function config(root, defaults = JSON.parse(readFileSync(
  new URL('../../assets/templates/planr.config.json', import.meta.url)
))) {
  const file = join(root, 'planr.config.json');
  let c;
  try {
    c = { ...defaults, ...(existsSync(file) ? JSON.parse(readFileSync(file, 'utf8')) : {}) };
  } catch (e) {
    throw Error(`planr.config.json: ${e.message}`);
  }
  for (const k of ['tasksDir', 'goalsDir', 'boardOut']) {
    const path = resolve(root, c[k]);
    if (relative(root, path).startsWith('..') || path === resolve(root)) {
      throw Error(`invalid ${k}: must be inside planning root`);
    }
  }
  return c;
}

export function findRoot(start) {
  let root = resolve(start);
  while (!existsSync(join(root, 'tasks')) && !existsSync(join(root, 'planr.config.json'))) {
    const parent = dirname(root);
    if (parent === root) {
      throw Error('planning root not found; use --root');
    }
    root = parent;
  }
  return root;
}

// Tokenize only outside quotes: comments and inline-list commas retain quoted values.
function tokens(text, delimiter) {
  let quote = '';
  const out = [''];
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (quote && ch === '\\' && quote === '"') {
      out[out.length - 1] += ch + (text[++i] ?? '');
      continue;
    }
    if (ch === quote) {
      if (quote === "'" && text[i + 1] === "'") {
        out[out.length - 1] += "''";
        i++;
        continue;
      }
      quote = '';
    } else if (
      !quote &&
      (ch === '"' || ch === "'") &&
      (!out.at(-1).trim() || /[\[,]\s*$/.test(out.at(-1)))
    ) {
      quote = ch;
    } else if (!quote && ch === '#' && (!i || /\s/.test(text[i - 1]))) {
      break;
    } else if (!quote && ch === delimiter) {
      out.push('');
      continue;
    }
    out[out.length - 1] += ch;
  }
  if (quote) {
    throw Error('unclosed frontmatter quote');
  }
  return out.map(s => s.trim());
}

const scalar = s =>
  s.startsWith('"')
    ? JSON.parse(s)
    : s.startsWith("'")
      ? s.slice(1, -1).replace(/''/g, "'")
      : ['null', '~'].includes(s)
        ? null
        : s;

export function frontmatter(text) {
  text = text.replace(/^\uFEFF/, '');
  const m = text.match(/^---\r?\n([\s\S]*?)^---[ \t]*(?:\r?\n|$)/m);
  if (!m || m.index !== 0) {
    return null;
  }
  const out = Object.create(null);
  let key;
  for (const line of m[1].split(/\r?\n/)) {
    if (!line.trim() || /^\s*#/.test(line)) {
      continue;
    }
    const item = line.match(/^\s*-\s+(.*)$/);
    if (item && key && out[key] === '') {
      out[key] = [];
    }
    if (item && Array.isArray(out[key])) {
      out[key].push(scalar(tokens(item[1])[0]));
      continue;
    }
    const kv = line.match(/^([\w-]+):\s*(.*)$/);
    if (!kv) {
      throw Error(`bad frontmatter line: ${line}`);
    }
    key = kv[1];
    if (Object.hasOwn(out, key)) {
      throw Error(`duplicate frontmatter key ${key}`);
    }
    const v = tokens(kv[2])[0];
    out[key] = v.startsWith('[') && v.endsWith(']')
      ? tokens(v.slice(1, -1), ',').filter(Boolean).map(scalar)
      : scalar(v);
  }
  return { ...out };
}

export const taskFile = file => file.endsWith('.md') && file !== 'README.md' && !file.includes('/');

export function taskStatus(text) {
  const status = frontmatter(text)?.status;
  if (!statuses.includes(status)) {
    throw Error('missing or unknown status');
  }
  return status;
}

export function nextWork(index, goals, stream) {
  const priority = (a, b) => b.unlocks - a.unlocks || compare(a.id, b.id);
  const select = key => {
    const ids = new Set(goals.flatMap(g => g[key]));
    return index.tasks.filter(t => ids.has(t.id) && (!t.owner || t.owner === stream)).sort(priority);
  };
  return { ready: select('ready'), waiting: select('waiting') };
}

export function buildIndex(root, c = config(root), read = file => readFileSync(file, 'utf8')) {
  const tasks = [];
  const goals = [];
  const errors = [];
  const warnings = [];
  const files = dir =>
    existsSync(join(root, dir)) ? readdirSync(join(root, dir)).sort(compare) : [];
  for (const f of files(c.tasksDir).filter(taskFile)) {
    try {
      const text = read(join(root, c.tasksDir, f));
      const t = frontmatter(text);
      if (!t) {
        throw Error('missing frontmatter');
      }
      const invalid = ['id', 'title', 'status', 'track'].filter(k => typeof t[k] !== 'string' || !t[k]);
      errors.push(...invalid.map(k => `${f}: missing or invalid ${k}`));
      if (t.id !== f.slice(0, -3)) {
        errors.push(`${f}: id ${t.id} does not match file name`);
      }
      if (!statuses.includes(t.status)) {
        errors.push(`${f}: unknown status ${t.status}`);
      }
      if (invalid.length) {
        continue;
      }
      if (
        t.depends != null &&
        (!Array.isArray(t.depends) || t.depends.some(d => typeof d !== 'string'))
      ) {
        throw Error('depends must be a list of IDs');
      }
      tasks.push({
        id: t.id,
        title: t.title,
        status: t.status,
        track: t.track,
        depends: [...new Set(t.depends ?? [])],
        owner: t.owner || null,
        file: join(c.tasksDir, f),
        ...(Object.hasOwn(t, 'refs') ? { refs: t.refs } : {}),
      });
    } catch (e) {
      errors.push(`${f}: ${e.message}`);
    }
  }
  tasks.sort((a, b) => compare(a.id ?? '', b.id ?? ''));
  const byId = new Map(tasks.map(t => [t.id, t]));
  for (const t of tasks) {
    for (const d of t.depends) {
      if (!byId.has(d)) {
        errors.push(`${t.id}: unknown dependency ${d}`);
      } else if (byId.get(d).status === 'dropped') {
        warnings.push(`${t.id}: dependency ${d} is dropped; remove the edge`);
      }
    }
    t.openDeps = t.depends.filter(d => !['review', 'done'].includes(byId.get(d)?.status));
    t.ready = t.status === 'open' && !t.openDeps.length;
    t.waitingOn = tasks
      .filter(x => !['done', 'dropped'].includes(x.status) && x.depends.includes(t.id))
      .map(x => x.id);
  }
  for (const t of tasks) {
    t.unlocks = ['review', 'done', 'dropped'].includes(t.status)
      ? 0
      : tasks.filter(x =>
          x.status === 'open' && x.openDeps.length === 1 && x.openDeps[0] === t.id
        ).length;
  }
  const visited = new Set();
  const active = [];
  function visit(id) {
    if (active.includes(id)) {
      errors.push(`cycle: ${[...active.slice(active.indexOf(id)), id].join(' -> ')}`);
      return;
    }
    if (visited.has(id)) {
      return;
    }
    visited.add(id);
    active.push(id);
    for (const d of byId.get(id)?.depends ?? []) {
      if (byId.has(d)) {
        visit(d);
      }
    }
    active.pop();
  }
  for (const t of tasks) {
    visit(t.id);
  }
  for (const f of files(c.goalsDir).filter(f => f.endsWith(c.goalSuffix))) {
    const file = join(c.goalsDir, f);
    const name = f.slice(0, -c.goalSuffix.length);
    try {
      const g = frontmatter(read(join(root, file)));
      if (!g) {
        warnings.push(`${file}: no frontmatter, listed as legacy`);
      }
      if (g && g.state == null) {
        warnings.push(`${file}: missing state`);
      }
      if (g?.state != null && !states.includes(g.state)) {
        errors.push(`${file}: unknown goal state ${g.state}`);
      }
      if (
        g?.scope != null &&
        (!Array.isArray(g.scope) || g.scope.some(id => typeof id !== 'string'))
      ) {
        throw Error('scope must be a list of IDs');
      }
      const scope = g?.state == null ? [] : g.scope ?? [];
      for (const id of scope) {
        if (!byId.has(id)) {
          errors.push(`${file}: unknown scope task ${id}`);
        }
      }
      goals.push({
        name,
        file,
        title: g?.title ?? name,
        state: g?.state ?? 'legacy',
        stream: g?.stream || null,
        budget: g?.budget || null,
        thread: g?.thread || null,
        scope,
        done: scope.filter(id => byId.get(id)?.status === 'done').length,
        review: scope.filter(id => byId.get(id)?.status === 'review').length,
        total: scope.length,
        ready: scope.filter(id => byId.get(id)?.ready),
        waiting: scope.filter(id => ['decision', 'blocked'].includes(byId.get(id)?.status)),
      });
    } catch (e) {
      errors.push(`${file}: ${e.message}`);
    }
  }
  const byName = [...goals].sort((a, b) => compare(a.name, b.name));
  for (const t of tasks) {
    const running = byName.filter(g => g.state === 'running' && g.scope.includes(t.id));
    if (running.length > 1) {
      warnings.push(`${t.id}: in multiple running goals: ${running.map(g => g.name).join(', ')}`);
    }
    t.stream = t.owner || byName.find(g =>
      ['running', 'approved'].includes(g.state) && g.scope.includes(t.id)
    )?.stream || null;
  }
  warnings.unshift(...tasks
    .filter(t => t.status === 'in-progress' && t.stream === null)
    .map(t => `${t.id}: ${t.status} without owner`)
  );
  return {
    index: {
      counts: Object.fromEntries(statuses.map(s => [s, tasks.filter(t => t.status === s).length])),
      tasks,
      goals,
      warnings,
    },
    errors,
  };
}

function encodeScalar(value) {
  const reserved = /^(?:null|~|true|false|yes|no|on|off|[+-]?\.inf|\.nan)$/i;
  const number = /^[+-]?(?:0[xob][\da-f_]+|\d[\d_]*(?:\.[\d_]*)?(?:e[+-]?\d+)?|\.\d+(?:e[+-]?\d+)?)$/i;
  const date = /^\d{4}-\d{2}-\d{2}(?:$|[Tt \t])/;
  const unsafe = /[\x00-\x1f\x7f]|:\s|\s#|:$|^[!&*{}\[\],#|>@`"'%]|^[?:-](?:\s|$)/;
  return !value || value.trim() !== value || unsafe.test(value) ||
    reserved.test(value) || number.test(value) || date.test(value) || ['---', '...'].includes(value)
    ? JSON.stringify(value)
    : value;
}

export function rewrite(file, fields) {
  const text = readFileSync(file, 'utf8');
  const parsed = frontmatter(text);
  if (!parsed) {
    throw Error(`${file}: missing frontmatter`);
  }
  const match = text.match(/^\uFEFF?---(\r?\n)([\s\S]*?)^---[ \t]*(?:\r?\n|$)/m);
  const newline = match[1];
  let head = match[2];
  for (const [key, value] of Object.entries(fields)) {
    if (Array.isArray(parsed[key])) {
      throw Error(`${file}: ${key} must be a scalar to rewrite`);
    }
    const line = `${key}: ${encodeScalar(value)}`;
    const re = new RegExp(`^${key}:[^\\r\\n]*`, 'm');
    head = re.test(head) ? head.replace(re, () => line) : head + line + newline;
  }
  const offset = text.indexOf('\n') + 1;
  const temp = `${file}.${randomUUID()}.tmp`;
  try {
    writeFileSync(temp, text.slice(0, offset) + head + text.slice(offset + match[2].length), {
      flag: 'wx',
      mode: statSync(file).mode,
    });
    if (readFileSync(file, 'utf8') !== text) {
      throw Error(`${file}: changed during rewrite; retry after reviewing the edit`);
    }
    renameSync(temp, file);
  } finally {
    rmSync(temp, { force: true });
  }
}
