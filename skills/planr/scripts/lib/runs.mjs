import { execFileSync } from 'node:child_process';
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

export function runs(root, index, c, now = new Date().toISOString()) {
  const mode = git(root, ['rev-parse', '--is-shallow-repository']);
  const warnings = mode === 'false' ? [] : [
    mode ? 'history: shallow clone, runs incomplete'
      : 'history: not a git repository, runs unavailable',
  ];
  const prefix = git(root, ['rev-parse', '--show-prefix']) +
    posix.normalize(c.tasksDir).replace(/\/$/, '') + '/';
  const log = mode ? git(root, [
    'log', '--first-parent', '--reverse', '--format=%x00PLANR%x09%H%x09%cI%x09%s',
    '--raw', '-z', '--no-abbrev', '--no-renames', '--root', '--diff-merges=first-parent', '--', c.tasksDir,
  ]) : '';
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
  const batch = hashes.length
    ? git(root, ['cat-file', '--batch'], hashes.join('\n') + '\n')
    : Buffer.alloc(0);
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
  let previous = new Map();
  const changesBetween = current => [...new Set([...previous.keys(), ...current.keys()])]
    .sort(compare)
    .filter(id => current.get(id) !== previous.get(id))
    .map(id => ({ id, from: previous.get(id) ?? null, to: current.get(id) ?? 'removed' }));
  const append = (s, current) => {
    const changes = changesBetween(current);
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
    previous = current;
  };
  for (const record of records) {
    const current = new Map(previous);
    for (const f of record.files) {
      const id = f.file.slice(prefix.length, -3);
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
    append(record, current);
  }
  if (mode) {
    append({ commit: null, date: now, subject: 'Uncommitted changes' },
      new Map(index.tasks.map(t => [t.id, t.status])));
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
