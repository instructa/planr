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

const falseCurrentClaims = [
  'Linux x86_64 and arm64 are supported with static musl release binaries; glibc is not required.',
  'The two Linux binaries are the exact checksum-verified static musl bytes from the corresponding GitHub Release tarballs.',
  'Linux x86_64 | Supported | Static musl release/npm binary; installer; source',
  'Published Linux binaries are genuinely static musl executables.',
  'Current installer artifacts for Linux are glibc-free.',
  'Static / musl binaries are supported today through npm on Linux.',
  'Linux release assets use musl and do not require glibc.',
  'npm and installer bytes for arm64 are no-glibc artifacts.',
  '| Linux x86_64 | npm + installer | glibc-free; static-musl |',
  'Without a glibc dependency, the Linux tarballs are available from the current release.',
  'Current Linux release binaries are static musl, although future candidates remain under CI.',
  'Linux release binaries use musl.',
  'The Linux npm artifacts are statically linked.',
  'Installer assets require no glibc.',
  'Current Linux binaries are free of glibc.',
];
for (const [index, claim] of falseCurrentClaims.entries()) {
  expectRejected(
    `pending current-artifact phrase ${index + 1}`,
    {
      packageVersion: packageManifest.version,
      contract,
      documents: {
        ...documents,
        installation: `${documents.installation}\n${claim}`,
      },
    },
    /presents pending static-musl\/glibc-free behavior as a current artifact claim/u,
  );
}

for (const allowedPendingClaim of [
  'Future corrective release candidate artifacts for Linux are static musl and do not require glibc once published.',
  'CI verifies static-musl Linux candidate binaries before upload.',
  'The future Linux release binaries will use musl once published.',
  `Historically, v${contract.affectedThrough} Linux binaries were not static musl and were affected by GLIBC_2.39.`,
]) {
  verifyLinuxPortabilityContract({
    packageVersion: packageManifest.version,
    contract,
    documents: {
      ...documents,
      installation: `${documents.installation}\n${allowedPendingClaim}`,
    },
  });
}

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

console.log('linux_portability_contract_regression=passed adversarial_cases=20 allowed_pending_scopes=4 corrected_transition_fixture=true dynamic_version_claims=true');
