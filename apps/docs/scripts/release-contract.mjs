const PREPARE_COMMAND = '`scripts/prepare-release-candidate.sh <version>`';
const PUBLISH_COMMAND = '`scripts/release.sh <version> "summary"`';
const PREPARE_BOUNDARY = 'without staging, committing, tagging, pushing, or publishing';
const PUBLISH_BOUNDARY = 'requires clean `main`';

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function executableLines(source) {
  return source
    .split('\n')
    .filter((line) => !line.trimStart().startsWith('#'))
    .join('\n');
}

export function verifyTwoPhaseReleaseContract({ releasePage, prepareScript, publishScript }) {
  const prepareIndex = releasePage.indexOf(PREPARE_COMMAND);
  const publishIndex = releasePage.indexOf(PUBLISH_COMMAND);
  assert(prepareIndex >= 0, `release runbook is missing candidate entry point: ${PREPARE_COMMAND}`);
  assert(publishIndex >= 0, `release runbook is missing publication entry point: ${PUBLISH_COMMAND}`);
  assert(prepareIndex < publishIndex, 'release runbook must present candidate preparation before publication');
  assert(releasePage.includes(PREPARE_BOUNDARY), `release runbook is missing preparation boundary: ${PREPARE_BOUNDARY}`);
  assert(releasePage.includes(PUBLISH_BOUNDARY), `release runbook is missing publication boundary: ${PUBLISH_BOUNDARY}`);
  assert(
    !/only supported release (entry point|path)|commits?, tags?, and pushes? in one step/iu.test(releasePage),
    'release runbook must not describe publication as a one-step source transition',
  );

  const prepareCommands = executableLines(prepareScript);
  assert(
    !/^\s*git\s+(add|commit|tag|push)\b/mu.test(prepareCommands),
    'candidate preparation must not stage, commit, tag, or push',
  );

  const publishCommands = executableLines(publishScript);
  assert(publishScript.includes('branch="$(git rev-parse --abbrev-ref HEAD)"'), 'publication must inspect the exact branch');
  assert(publishScript.includes('if [ "$branch" != "main" ]; then'), 'publication must require exact main');
  assert(publishScript.includes('git status --porcelain'), 'publication must require a clean prepared commit');
  assert(!/^\s*git\s+(add|commit)\b/mu.test(publishCommands), 'publication must not stage or commit source');
  assert(!/^\s*replace(?:\(\))?\b/mu.test(publishCommands), 'publication must not rewrite prepared manifests');
  assert(
    !/(?:>|>>)\s*(?:Cargo\.toml|Cargo\.lock|package\.json|CHANGELOG\.md|plugins\/|\.cursor-plugin\/)/u.test(publishCommands),
    'publication must not write release-owned source files',
  );
}
