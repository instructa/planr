import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { verifyTwoPhaseReleaseContract } from './release-contract.mjs';

const docsRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = dirname(dirname(docsRoot));
const releasePage = await readFile(join(docsRoot, 'content/docs/operations/release.mdx'), 'utf8');
const prepareScript = await readFile(join(repositoryRoot, 'scripts/prepare-release-candidate.sh'), 'utf8');
const publishScript = await readFile(join(repositoryRoot, 'scripts/release.sh'), 'utf8');

const verify = (overrides = {}) => verifyTwoPhaseReleaseContract({
  releasePage,
  prepareScript,
  publishScript,
  ...overrides,
});
const rejects = (overrides, pattern) => assert.throws(() => verify(overrides), pattern);

verify();
verify({ releasePage: `${releasePage}\nMaintainers may clarify this prose without changing the commands or boundaries.\n` });

rejects({ releasePage: releasePage.replaceAll('`scripts/prepare-release-candidate.sh <version>`', '`prepare <version>`') }, /missing candidate entry point/u);
rejects({ releasePage: releasePage.replaceAll('`scripts/release.sh <version> "summary"`', '`publish <version>`') }, /missing publication entry point/u);
rejects({
  releasePage: releasePage
    .replace('`scripts/prepare-release-candidate.sh <version>`', '__PUBLISH__')
    .replace('`scripts/release.sh <version> "summary"`', '`scripts/prepare-release-candidate.sh <version>`')
    .replace('__PUBLISH__', '`scripts/release.sh <version> "summary"`'),
}, /before publication/u);
rejects({ releasePage: `${releasePage}\nThis is the only supported release path.\n` }, /one-step source transition/u);
rejects({ prepareScript: `${prepareScript}\ngit commit -am "release"\n` }, /must not stage, commit, tag, or push/u);
rejects({ publishScript: `${publishScript}\nprintf '# drift\\n' >> Cargo.toml\n` }, /must not write release-owned source files/u);
rejects({ publishScript: `${publishScript}\nreplace Cargo.toml\n` }, /must not rewrite prepared manifests/u);

console.log('two_phase_release_contract_regressions=passed checks=9');
