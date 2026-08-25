import { spawnSync } from 'node:child_process';
import path from 'node:path';

export function resolvePlanrBinary(repositoryRoot) {
  if (process.env.PLANR_BIN) {
    return path.resolve(process.cwd(), process.env.PLANR_BIN);
  }

  const result = spawnSync(
    'cargo',
    ['metadata', '--no-deps', '--format-version', '1'],
    { cwd: repositoryRoot, encoding: 'utf8' },
  );
  if (result.status !== 0) {
    throw new Error(`Could not resolve Cargo target directory: ${result.stderr || result.stdout}`);
  }

  let metadata;
  try {
    metadata = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`Cargo metadata returned invalid JSON: ${error.message}`);
  }
  if (typeof metadata.target_directory !== 'string' || metadata.target_directory.length === 0) {
    throw new Error('Cargo metadata did not return target_directory');
  }

  const executable = process.platform === 'win32' ? 'planr.exe' : 'planr';
  return path.resolve(metadata.target_directory, 'debug', executable);
}
