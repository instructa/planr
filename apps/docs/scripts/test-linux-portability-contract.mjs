import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  renderLinuxPortabilityNotice,
  replaceLinuxPortabilityNotice,
  verifyLinuxPortabilityContract,
} from './linux-portability-contract.mjs';

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
  /Linux portability notice body is stale or mutated/u,
);

expectRejected(
  'contract-only affectedThrough bump',
  {
    packageVersion: packageManifest.version,
    contract: { ...clone(contract), affectedReleases: [...contract.affectedReleases, '1.7.3'], affectedThrough: '1.7.3' },
    documents,
  },
  /Linux portability notice body is stale or mutated/u,
);

const docsOnlyBump = Object.fromEntries(
  Object.entries(documents).map(([label, source]) => [label, source.replaceAll(`v${contract.affectedThrough}`, 'v1.7.3')]),
);
expectRejected(
  'docs-only affected version bump',
  { packageVersion: packageManifest.version, contract, documents: docsOnlyBump },
  /Linux portability notice body is stale or mutated/u,
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

const missingSurfaceDocuments = clone(documents);
delete missingSurfaceDocuments.installation;
expectRejected(
  'missing required public surface',
  { packageVersion: packageManifest.version, contract, documents: missingSurfaceDocuments },
  /public surface inventory drifted/u,
);

expectRejected(
  'duplicate canonical notice',
  {
    packageVersion: packageManifest.version,
    contract,
    documents: {
      ...documents,
      support: `${documents.support}\n${renderLinuxPortabilityNotice(contract, 'support')}`,
    },
  },
  /support must contain exactly two raw Linux portability reserved tokens/u,
);

const canonicalInstallationNotice = renderLinuxPortabilityNotice(contract, 'installation');
expectRejected(
  'mutated canonical notice body',
  {
    packageVersion: packageManifest.version,
    contract,
    documents: {
      ...documents,
      installation: documents.installation.replace(
        canonicalInstallationNotice,
        canonicalInstallationNotice.replace('Candidate artifacts remain CI-only evidence', 'Candidate artifacts are published evidence'),
      ),
    },
  },
  /installation Linux portability notice body is stale or mutated/u,
);

expectRejected(
  'edited canonical notice marker',
  {
    packageVersion: packageManifest.version,
    contract,
    documents: {
      ...documents,
      installation: documents.installation.replace('surface=installation schema=1', 'surface=support schema=1'),
    },
  },
  /installation Linux portability start marker is edited or assigned to the wrong surface/u,
);

expectRejected(
  'notice assigned to the wrong surface',
  {
    packageVersion: packageManifest.version,
    contract,
    documents: {
      ...documents,
      installation: documents.installation.replace(
        renderLinuxPortabilityNotice(contract, 'installation'),
        renderLinuxPortabilityNotice(contract, 'README'),
      ),
    },
  },
  /installation Linux portability start marker is edited or assigned to the wrong surface/u,
);

const installationNoticeLines = canonicalInstallationNotice.split('\n');
const installationStart = installationNoticeLines[0];
const installationEnd = installationNoticeLines.at(-1);
const wrongInstallationStart = installationStart.replace('surface=installation schema=1', 'surface=wrong schema=999');
const wrongInstallationEnd = installationEnd.replace('surface=installation schema=1', 'surface=wrong schema=999');
const invalidInstallationMarker = installationStart.replace(':start', '-start');
const malformedMarkdownStart = '<!-- planr:linux-release-portability:start surface=installation schema=1 -- >';
const malformedMarkdownEnd = '<!-- planr:linux-release-portability:end surface=installation schema=1 -- >';
const whitespaceMutatedMdxStart = '{ /* planr:linux-release-portability:start surface=installation schema=1 */}';
const whitespaceMutatedMdxEnd = '{ /* planr:linux-release-portability:end surface=installation schema=1 */}';
const rawReservedStart = 'planr:linux-release-portability:start surface=installation schema=1';
const canonicalReadmeNotice = renderLinuxPortabilityNotice(contract, 'README');
const readmeNoticeLines = canonicalReadmeNotice.split('\n');
const readmeStart = readmeNoticeLines[0];
const readmeEnd = readmeNoticeLines.at(-1);

expectRejected(
  'edited wrong start immediately before canonical notice',
  {
    packageVersion: packageManifest.version,
    contract,
    documents: { ...documents, installation: `${wrongInstallationStart}\n${documents.installation}` },
  },
  /installation must contain exactly two raw Linux portability reserved tokens/u,
);

expectRejected(
  'edited wrong end immediately after canonical notice',
  {
    packageVersion: packageManifest.version,
    contract,
    documents: { ...documents, installation: `${documents.installation}\n${wrongInstallationEnd}` },
  },
  /installation must contain exactly two raw Linux portability reserved tokens/u,
);

expectRejected(
  'extra unpaired canonical start after notice',
  {
    packageVersion: packageManifest.version,
    contract,
    documents: { ...documents, installation: `${documents.installation}\n${installationStart}` },
  },
  /installation must contain exactly two raw Linux portability reserved tokens/u,
);

expectRejected(
  'edited notice-like marker syntax beside canonical notice',
  {
    packageVersion: packageManifest.version,
    contract,
    documents: { ...documents, installation: `${invalidInstallationMarker}\n${documents.installation}` },
  },
  /installation must contain exactly two raw Linux portability reserved tokens/u,
);

expectRejected(
  'extra unpaired canonical end before notice',
  {
    packageVersion: packageManifest.version,
    contract,
    documents: { ...documents, installation: `${installationEnd}\n${documents.installation}` },
  },
  /installation must contain exactly two raw Linux portability reserved tokens/u,
);

for (const [label, extraToken] of [
  ['malformed Markdown close on start beside canonical notice', malformedMarkdownStart],
  ['malformed Markdown close on end beside canonical notice', malformedMarkdownEnd],
  ['whitespace-mutated MDX start wrapper beside canonical notice', whitespaceMutatedMdxStart],
  ['whitespace-mutated MDX end wrapper beside canonical notice', whitespaceMutatedMdxEnd],
  ['raw reserved start token beside canonical notice', rawReservedStart],
]) {
  expectRejected(
    label,
    {
      packageVersion: packageManifest.version,
      contract,
      documents: { ...documents, installation: `${extraToken}\n${documents.installation}` },
    },
    /installation must contain exactly two raw Linux portability reserved tokens/u,
  );
}

for (const [label, surface, canonicalNotice, malformedNotice] of [
  [
    'malformed Markdown close on canonical start',
    'README',
    canonicalReadmeNotice,
    canonicalReadmeNotice.replace(readmeStart, readmeStart.replace('-->', '-- >')),
  ],
  [
    'malformed Markdown close on canonical end',
    'README',
    canonicalReadmeNotice,
    canonicalReadmeNotice.replace(readmeEnd, readmeEnd.replace('-->', '-- >')),
  ],
  [
    'whitespace-mutated MDX canonical start wrapper',
    'installation',
    canonicalInstallationNotice,
    canonicalInstallationNotice.replace(installationStart, installationStart.replace('{/*', '{ /*')),
  ],
  [
    'whitespace-mutated MDX canonical end wrapper',
    'installation',
    canonicalInstallationNotice,
    canonicalInstallationNotice.replace(installationEnd, installationEnd.replace('{/*', '{ /*')),
  ],
]) {
  expectRejected(
    label,
    {
      packageVersion: packageManifest.version,
      contract,
      documents: {
        ...documents,
        [surface]: documents[surface].replace(canonicalNotice, malformedNotice),
      },
    },
    new RegExp(`${surface} Linux portability reserved tokens must use valid comment wrappers`, 'u'),
  );
}

expectRejected(
  'canonical markers in reverse order',
  {
    packageVersion: packageManifest.version,
    contract,
    documents: {
      ...documents,
      installation: documents.installation.replace(
        canonicalInstallationNotice,
        [installationEnd, ...installationNoticeLines.slice(1, -1), installationStart].join('\n'),
      ),
    },
  },
  /installation Linux portability markers are out of order/u,
);

const correctedFrom = '1.7.3';
const correctedContract = { ...clone(contract), status: 'corrected', correctedFrom };
expectRejected(
  'stale pending notice after corrected state transition',
  { packageVersion: correctedFrom, contract: correctedContract, documents },
  /Linux portability notice body is stale or mutated/u,
);

for (const [index, bypassClaim] of [
  'Linux release binaries use musl alongside candidate npm artifacts tested in CI.',
  'Linux release binaries use musl next to future installer assets.',
].entries()) {
  expectRejected(
    `cross-artifact prose cannot replace canonical notice ${index + 1}`,
    {
      packageVersion: packageManifest.version,
      contract,
      documents: {
        ...documents,
        installation: `${documents.installation.replace(renderLinuxPortabilityNotice(contract, 'installation'), '')}\n${bypassClaim}`,
      },
    },
    /installation must contain exactly two raw Linux portability reserved tokens/u,
  );
}

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
  'Musl powers the Linux release binaries, although future candidates remain under CI.',
  'The Linux npm artifact is glibc-free, with candidate builds tested in CI.',
  'Installer bytes require no glibc — future workflow candidates are also verified.',
  'Although future candidates remain under CI, musl powers the Linux release binaries.',
  'Future workflow candidates are verified; the Linux npm artifact is glibc-free.',
  'The Linux installer bytes require no glibc (future candidates are verified in CI).',
  'Linux release binaries use musl and future CI candidates are also tested.',
  '| Linux npm artifact | glibc-free | future CI candidates verified |',
  'Linux release binaries use musl because future CI candidates are tested.',
  'Linux release binaries use musl even as future CI candidates are tested.',
  'Linux release binaries use musl / future CI candidates are verified.',
  'Because future CI candidates are tested, Linux release binaries use musl.',
  'Linux installer artifacts are glibc-free because planned workflow candidates are verified.',
  '| Linux release binaries use musl | because future CI candidates are tested |',
  'Linux release binaries use musl: future CI candidates are tested.',
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
  'The latest CI candidate artifacts for Linux will be static musl once published.',
  'Current CI candidate Linux binaries are static musl before upload.',
  'Before upload, current CI candidate Linux binaries are static musl.',
  '| Linux CI candidate binaries | static musl before upload |',
  'Latest Linux candidate binaries built by CI will be static musl once published.',
  'Linux candidate binaries currently verified in CI are static musl before upload.',
  'Once published, Linux CI candidate binaries will be glibc-free.',
  'Linux binaries planned as CI candidates will be static musl after they are published.',
  '| Latest Linux candidate binaries built by CI | static musl once published |',
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

const correctedDocuments = Object.fromEntries(
  Object.entries(documents).map(([label, source]) => [
    label,
    replaceLinuxPortabilityNotice(`${source
      .replaceAll(/no corrected release is published yet/giu, 'the corrected release boundary is published')
      .replaceAll(/wait for a corrective release/giu, 'use the corrected release')}`,
    correctedContract,
    label),
  ]),
);
verifyLinuxPortabilityContract({
  packageVersion: correctedFrom,
  contract: correctedContract,
  documents: correctedDocuments,
});

console.log('linux_portability_contract_regression=passed adversarial_cases=35 allowed_pending_scopes=13 structural_notice_cases=23 corrected_transition_fixture=true dynamic_version_claims=true');
