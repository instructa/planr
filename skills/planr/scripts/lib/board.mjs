import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { basename, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runs, git } from './runs.mjs';

const assets = fileURLToPath(new URL('../../assets/board/', import.meta.url));

export function boardData(root, index, c) {
  const generatedAt = new Date().toISOString();
  const history = runs(root, index, c, generatedAt);
  const order = ['running', 'approved', 'draft', 'review', 'legacy', 'done'];
  return {
    project: basename(root),
    commit: git(root, ['log', '-1', '--format=%h · %cI']),
    dirty: !!git(root, ['status', '--porcelain', '--', c.tasksDir, c.goalsDir]),
    generatedAt,
    tasks: history.tasks,
    goals: [...index.goals].sort((a, b) => order.indexOf(a.state) - order.indexOf(b.state)),
    runs: history.runs,
    warnings: [...index.warnings, ...history.warnings],
  };
}

export function render(data) {
  const payload = JSON.stringify(data).replace(/</g, '\\u003c');
  return readFileSync(resolve(assets, 'template.html'), 'utf8')
    .replace(/\/\*FONT:([\w.-]+)\*\//g, (_, f) =>
      readFileSync(resolve(assets, 'fonts', f)).toString('base64')
    )
    .replace('/*DATA*/null', () => payload);
}

export function writeBoard(root, index, c, out = c.boardOut) {
  const file = resolve(root, out);
  mkdirSync(dirname(file), { recursive: true });
  const data = boardData(root, index, c);
  writeFileSync(file, render(data));
  return { file, warnings: data.warnings };
}
