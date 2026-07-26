import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { verifyLinuxPortabilityContract } from './linux-portability-contract.mjs';

const docsRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = path.dirname(path.dirname(docsRoot));
const contract = JSON.parse(await readFile(path.join(repositoryRoot, 'docs', 'contracts', 'LINUX_RELEASE_PORTABILITY.json'), 'utf8'));
const packageManifest = JSON.parse(await readFile(path.join(repositoryRoot, 'package.json'), 'utf8'));
const documents = {
  README: await readFile(path.join(repositoryRoot, 'README.md'), 'utf8'),
  installation: await readFile(path.join(docsRoot, 'content', 'docs', 'getting-started', 'installation.mdx'), 'utf8'),
  support: await readFile(path.join(docsRoot, 'content', 'docs', 'reference', 'support-matrix.mdx'), 'utf8'),
  release: await readFile(path.join(docsRoot, 'content', 'docs', 'operations', 'release.mdx'), 'utf8'),
  maintainerRelease: await readFile(path.join(repositoryRoot, 'docs', 'RELEASE.md'), 'utf8'),
  changelog: await readFile(path.join(repositoryRoot, 'CHANGELOG.md'), 'utf8'),
};

function clone(value) {
  return structuredClone(value);
}

function expectRejected(label, input, expected) {
  assert.throws(() => verifyLinuxPortabilityContract(input), expected, label);
}

verifyLinuxPortabilityContract({ packageVersion: packageManifest.version, contract, documents });

expectRejected(
  'package and affectedThrough bump with unchanged v1.7.2 docs',
  {
    packageVersion: '1.7.3',
    contract: { ...clone(contract), affectedReleases: [...contract.affectedReleases, '1.7.3'], affectedThrough: '1.7.3' },
    documents,
  },
  /must derive its affected Linux version from affectedThrough=1\.7\.3/u,
);

expectRejected(
  'contract-only affectedThrough bump',
  {
    packageVersion: packageManifest.version,
    contract: { ...clone(contract), affectedReleases: [...contract.affectedReleases, '1.7.3'], affectedThrough: '1.7.3' },
    documents,
  },
  /must derive its affected Linux version from affectedThrough=1\.7\.3/u,
);

const docsOnlyBump = Object.fromEntries(
  Object.entries(documents).map(([label, source]) => [label, source.replaceAll(`v${contract.affectedThrough}`, 'v1.7.3')]),
);
expectRejected(
  'docs-only affected version bump',
  { packageVersion: packageManifest.version, contract, documents: docsOnlyBump },
  new RegExp(`must derive its affected Linux version from affectedThrough=${contract.affectedThrough}`, 'u'),
);

expectRejected(
  'package-only bump beyond pending boundary',
  { packageVersion: '1.7.3', contract, documents },
  /moved beyond pending affected boundary/u,
);

expectRejected(
  'invalid correctedFrom ordering',
  {
    packageVersion: '1.7.3',
    contract: { ...clone(contract), status: 'corrected', correctedFrom: contract.affectedThrough },
    documents,
  },
  /correctedFrom must be newer than affectedThrough/u,
);

expectRejected(
  'premature static-current claim',
  {
    packageVersion: packageManifest.version,
    contract,
    documents: {
      ...documents,
      support: `${documents.support}\nPublished Linux binaries are genuinely static musl executables.`,
    },
  },
  /must not be presented as a current published artifact/u,
);

const correctedFrom = '1.7.3';
const correctedDocuments = Object.fromEntries(
  Object.entries(documents).map(([label, source]) => [
    label,
    `${source
      .replaceAll(/no corrected release is published yet/giu, 'the corrected release boundary is published')
      .replaceAll(/wait for a corrective release/giu, 'use the corrected release')}
Starting with v${correctedFrom}, current Linux release and npm artifacts are static musl executables and do not require glibc.`,
  ]),
);
verifyLinuxPortabilityContract({
  packageVersion: correctedFrom,
  contract: { ...clone(contract), status: 'corrected', correctedFrom },
  documents: correctedDocuments,
});

console.log('linux_portability_contract_regression=passed adversarial_cases=6 corrected_transition_fixture=true dynamic_version_claims=true');
