#!/usr/bin/env node
import { createHash } from 'node:crypto';
import { mkdtemp, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const repoRoot = path.resolve(import.meta.dirname, '..');
const docsOut = path.join(repoRoot, 'apps/docs/out');

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    env: process.env,
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

async function walkFiles(root, current = root) {
  const entries = await readdir(current, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name));
  const files = [];
  for (const entry of entries) {
    const absolute = path.join(current, entry.name);
    if (entry.isDirectory()) {
      files.push(...await walkFiles(root, absolute));
    } else if (entry.isFile()) {
      files.push(path.relative(root, absolute).split(path.sep).join('/'));
    }
  }
  return files;
}

async function digestOutput() {
  const files = await walkFiles(docsOut);
  const lines = [];
  for (const file of files) {
    const bytes = await readFile(path.join(docsOut, file));
    lines.push(`${createHash('sha256').update(bytes).digest('hex')}  ${file}`);
  }
  const manifest = `${lines.join('\n')}\n`;
  return {
    fileCount: files.length,
    manifest,
    digest: createHash('sha256').update(manifest).digest('hex'),
  };
}

async function ensureOutputExists() {
  const info = await stat(docsOut);
  if (!info.isDirectory()) {
    throw new Error(`${docsOut} is not a directory`);
  }
}

const tempRoot = await mkdtemp(path.join(tmpdir(), 'planr-docs-determinism-'));
try {
  run('pnpm', ['docs:build']);
  await ensureOutputExists();
  const first = await digestOutput();
  await writeTempManifest(tempRoot, 'first.sha256', first.manifest);

  run('pnpm', ['docs:build']);
  await ensureOutputExists();
  const second = await digestOutput();
  await writeTempManifest(tempRoot, 'second.sha256', second.manifest);

  if (first.manifest !== second.manifest) {
    throw new Error(
      `docs artifact is nondeterministic: first=${first.digest} second=${second.digest} manifests=${tempRoot}`,
    );
  }

  console.log(`docs_artifact_determinism=passed files=${first.fileCount} digest=sha256:${first.digest}`);
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
} finally {
  if (process.exitCode !== 1) {
    await rm(tempRoot, { recursive: true, force: true });
  }
}

async function writeTempManifest(root, name, content) {
  await writeFile(path.join(root, name), content);
}
