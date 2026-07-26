import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  LINUX_PORTABILITY_NOTICE_SURFACES,
  replaceLinuxPortabilityNotice,
} from './linux-portability-contract.mjs';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const check = process.argv.includes('--check');
const files = {
  README: path.join(repositoryRoot, 'README.md'),
  installation: path.join(docsRoot, 'content', 'docs', 'getting-started', 'installation.mdx'),
  support: path.join(docsRoot, 'content', 'docs', 'reference', 'support-matrix.mdx'),
  release: path.join(docsRoot, 'content', 'docs', 'operations', 'release.mdx'),
  maintainerRelease: path.join(repositoryRoot, 'docs', 'RELEASE.md'),
  changelog: path.join(repositoryRoot, 'CHANGELOG.md'),
};

assert.deepEqual(Object.keys(files).sort(), [...LINUX_PORTABILITY_NOTICE_SURFACES].sort(), 'notice file inventory drifted');
const contract = JSON.parse(await readFile(path.join(repositoryRoot, 'docs', 'contracts', 'LINUX_RELEASE_PORTABILITY.json'), 'utf8'));
let updated = 0;

for (const surface of LINUX_PORTABILITY_NOTICE_SURFACES) {
  const file = files[surface];
  const source = await readFile(file, 'utf8');
  const synchronized = replaceLinuxPortabilityNotice(source, contract, surface);
  if (check) {
    assert.equal(source, synchronized, `${path.relative(repositoryRoot, file)} has a stale Linux portability notice`);
  } else if (source !== synchronized) {
    await writeFile(file, synchronized);
    updated += 1;
  }
}

console.log(`linux_portability_notice_sync=${check ? 'checked' : 'synchronized'} surfaces=${LINUX_PORTABILITY_NOTICE_SURFACES.length} updated=${updated} schema=${contract.noticeSchema} status=${contract.status}`);
