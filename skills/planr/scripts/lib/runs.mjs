import { execFileSync, spawnSync } from 'node:child_process';
import { posix } from 'node:path';
import { compare, taskFile, taskStatus } from './graph.mjs';

export function git(root, args, input) {
  try {
    const value = execFileSync('git', args, {
      cwd: root,
      encoding: input === undefined ? 'utf8' : undefined,
      input,
      maxBuffer: 64 << 20,
      stdio: ['pipe', 'pipe', 'ignore'],
    });
    return typeof value === 'string' ? value.trimEnd() : value;
  } catch {
    return input === undefined ? '' : Buffer.alloc(0);
  }
}

// History reads must fail loudly: a silently empty or truncated read would rewrite every run.
function strict(root, args, input) {
  const out = spawnSync('git', args, { cwd: root, input, maxBuffer: 1 << 30 });
  if (out.error || out.status !== 0) {
    const reason = out.error?.message ?? String(out.stderr).trim().split('\n')[0];
    throw new Error(`git ${args[0]} failed: ${reason}`);
  }
  return out.stdout;
}

export function runs(root, index, c, now = new Date().toISOString()) {
  const mode = git(root, ['rev-parse', '--is-shallow-repository']);
  const warnings = mode === 'false' ? [] : [
    mode ? 'history: shallow clone, runs incomplete'
      : 'history: not a git repository, runs unavailable',
  ];
  const prefix = git(root, ['rev-parse', '--show-prefix']) +
    posix.normalize(c.tasksDir).replace(/\/$/, '') + '/';
  let log = '';
  let batch = Buffer.alloc(0);
  let failed = false;
  try {
    log = mode && git(root, ['rev-parse', '--verify', '--quiet', 'HEAD']) ? strict(root, [
      'log', '--first-parent', '--reverse', '--format=%x00PLANR%x09%H%x09%cI%x09%s', '--raw', '-z',
      '--no-abbrev', '--no-renames', '--root', '--diff-merges=first-parent', '--', c.tasksDir,
    ]).toString('utf8').trimEnd() : '';
  } catch (e) {
    warnings.push(`history: ${e.message}; runs unavailable`);
    failed = true;
  }
  const records = [];
  const parts = log.split('\0');
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i].replace(/^\n/, '');
    if (part.startsWith('PLANR\t')) {
      const [, full, date, ...subject] = part.split('\t');
      records.push({ commit: full.slice(0, 7), date, subject: subject.join('\t'), files: [] });
    } else if (part.startsWith(':')) {
      const [, , , blob, status] = part.split(' ');
      const file = parts[++i];
      if (file?.startsWith(prefix) && taskFile(file.slice(prefix.length))) {
        records.at(-1).files.push({ file, blob, removed: status === 'D' });
      }
    }
  }
  const hashes = [...new Set(records.flatMap(r =>
    r.files.filter(f => !f.removed).map(f => f.blob)
  ))];
  try {
    batch = hashes.length ? strict(root, ['cat-file', '--batch'], hashes.join('\n') + '\n') : batch;
  } catch (e) {
    warnings.push(`history: ${e.message}; runs unavailable`);
    failed = true;
    records.length = 0;
  }
  const blobs = new Map();
  let cursor = 0;
  while (cursor < batch.length) {
    const end = batch.indexOf('\n', cursor);
    if (end < 0) {
      break;
    }
    const [hash, type, bytes] = batch.subarray(cursor, end).toString().split(' ');
    cursor = end + 1;
    if (type === 'blob') {
      const size = Number(bytes);
      blobs.set(hash, batch.subarray(cursor, cursor + size).toString('utf8'));
      cursor += size + 1;
    }
  }
  const result = [];
  const history = new Map(index.tasks.map(t => [t.id, []]));
  // One status map is updated in place per commit; only the IDs a commit touched are compared and sorted.
  const current = new Map();
  const append = (s, before) => {
    const changes = [...before]
      .filter(([id, from]) => current.get(id) !== from)
      .map(([id]) => id)
      .sort(compare)
      .map(id => ({ id, from: before.get(id) ?? null, to: current.get(id) ?? 'removed' }));
    if (changes.length) {
      const initial = !result.length;
      result.push({
        commit: s.commit,
        date: s.date,
        subject: s.subject,
        counts: [...current.values()].reduce((out, status) => {
          out[status] = (out[status] ?? 0) + 1;
          return out;
        }, {}),
        total: current.size,
        initial,
        changes: initial ? [] : changes,
      });
      for (const change of (initial ? [] : changes)) {
        history.get(change.id)?.push({ run: result.length - 1, from: change.from, to: change.to });
      }
    }
  };
  for (const record of records) {
    const before = new Map();
    for (const f of record.files) {
      const id = f.file.slice(prefix.length, -3);
      if (!before.has(id)) {
        before.set(id, current.get(id));
      }
      if (f.removed) {
        current.delete(id);
      } else {
        try {
          current.set(id, taskStatus(blobs.get(f.blob) ?? ''));
        } catch {
          warnings.push(`history: ${record.commit} ${f.file} unreadable, skipped`);
        }
      }
    }
    append(record, before);
  }
  if (mode && !failed) {
    const before = new Map(current);
    current.clear();
    for (const t of index.tasks) {
      current.set(t.id, t.status);
    }
    for (const id of current.keys()) {
      if (!before.has(id)) {
        before.set(id, undefined);
      }
    }
    append({ commit: null, date: now, subject: 'Uncommitted changes' }, before);
  }
  const tasks = index.tasks.map(t => {
    const h = history.get(t.id);
    return {
      ...t,
      goal: index.goals.filter(g => g.state !== 'done' && g.scope.includes(t.id)).map(g => g.name),
      history: h,
      lastRun: h.at(-1)?.run ?? null,
    };
  });
  return { runs: result, tasks, warnings };
}
