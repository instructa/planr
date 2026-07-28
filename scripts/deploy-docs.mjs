#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

export function deploymentCommands({ receipt, input, head = 'HEAD', url = 'https://planr.so' }) {
  if (!receipt || !input) throw new Error('docs deployment requires --receipt and --input');
  return [
    {
      label: 'reviewed receipt',
      executable: process.execPath,
      args: ['scripts/verification-runner.mjs', 'verify', '--receipt', receipt, '--input', input, '--head', head],
    },
    {
      label: 'Alchemy production deployment',
      executable: 'pnpm',
      args: ['--filter', '@planr/docs', 'exec', 'alchemy', 'deploy', '--stage', 'prod', '--yes'],
      env: { PLANR_DOCS_RECEIPT_VALIDATED: '1' },
    },
    {
      label: 'bounded live oracle',
      executable: process.execPath,
      args: ['apps/docs/scripts/verify-live-deployment.mjs', '--url', url],
    },
  ];
}

export function deployDocs(options, execute = executeCommand) {
  const completed = [];
  for (const command of deploymentCommands(options)) {
    const result = execute(command.executable, command.args, {
      cwd: repoRoot,
      stdio: 'inherit',
      env: { ...process.env, ...command.env },
    });
    if (result.status !== 0) throw new Error(`${command.label} failed with exit code ${result.status ?? 'unknown'}`);
    completed.push(command.label);
  }
  return completed;
}

function executeCommand(executable, args, options) {
  return spawnSync(executable, args, options);
}

function valueAfter(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const completed = deployDocs({
      receipt: valueAfter('--receipt'),
      input: valueAfter('--input'),
      head: valueAfter('--head') ?? 'HEAD',
      url: valueAfter('--url') ?? 'https://planr.so',
    });
    process.stdout.write(`${JSON.stringify({ verdict: 'pass', completed })}\n`);
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}
