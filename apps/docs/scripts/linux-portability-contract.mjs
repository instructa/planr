import assert from 'node:assert/strict';

function parseSemver(value, label) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/u.exec(value);
  assert.ok(match, `${label} must be a semantic version`);
  return {
    core: match.slice(1, 4).map(Number),
    prerelease: match[4]?.split('.') ?? [],
  };
}

function compareSemver(leftValue, rightValue) {
  const left = parseSemver(leftValue, 'left version');
  const right = parseSemver(rightValue, 'right version');
  for (let index = 0; index < left.core.length; index += 1) {
    if (left.core[index] !== right.core[index]) return left.core[index] - right.core[index];
  }
  if (left.prerelease.length === 0 || right.prerelease.length === 0) {
    return right.prerelease.length - left.prerelease.length;
  }
  const length = Math.max(left.prerelease.length, right.prerelease.length);
  for (let index = 0; index < length; index += 1) {
    const leftPart = left.prerelease[index];
    const rightPart = right.prerelease[index];
    if (leftPart === undefined) return -1;
    if (rightPart === undefined) return 1;
    if (leftPart === rightPart) continue;
    const leftNumber = /^\d+$/u.test(leftPart) ? Number(leftPart) : null;
    const rightNumber = /^\d+$/u.test(rightPart) ? Number(rightPart) : null;
    if (leftNumber !== null && rightNumber !== null) return leftNumber - rightNumber;
    if (leftNumber !== null) return -1;
    if (rightNumber !== null) return 1;
    return leftPart.localeCompare(rightPart, 'en');
  }
  return 0;
}

function prose(source) {
  return source.replace(/^>\s?/gmu, '').replace(/\s+/gu, ' ');
}

function semanticUnits(source) {
  const cleaned = source
    .replace(/^>\s?/gmu, '')
    .replace(/[`*~]/gu, '')
    .replace(/<[^>]+>/gu, ' ');
  const lines = cleaned.split('\n');
  const tableCells = lines
    .filter((line) => line.includes('|'))
    .flatMap((line) => {
      const cells = line.split('|').map((cell) => cell.trim()).filter(Boolean);
      const context = [...line.matchAll(/\b(?:linux|x86_64|arm64|binar(?:y|ies)|artifacts?|assets?|releases?|npm|installer|tarballs?|bytes)\b/giu)]
        .map((match) => match[0])
        .join(' ');
      return cells.map((cell) => `${context} ${cell}`.trim());
    });
  const proseParagraphs = lines
    .map((line) => (line.includes('|') ? '' : line))
    .join('\n')
    .split(/\n\s*\n/gu)
    .map((paragraph) => paragraph.replace(/\s+/gu, ' ').trim())
    .filter(Boolean);
  const sentences = proseParagraphs.flatMap((paragraph) => paragraph.split(/(?<=[.!?])\s+|;\s+/gu));
  return [...tableCells, ...sentences].map((unit) => unit.trim()).filter(Boolean);
}

function claimClauses(unit, futureScope, ciCandidateScope) {
  const clauses = unit
    .replace(/[()]/gu, ' — ')
    .split(/\s*(?:—|–)\s*|,\s*(?=(?:although|though|but|whereas|while|with|yet|and\s+(?:future|candidate|CI|workflow|pipeline)|future|candidate)\b)|\s+and\s+(?=(?:future|candidate|CI|workflow|pipeline)\b)|\s+(?=(?:although|but|whereas|while)\b)/giu)
    .map((clause) => clause.trim())
    .filter(Boolean);

  return clauses.flatMap((clause) => {
    const comma = clause.indexOf(',');
    if (comma < 0) return [clause];
    const prefix = clause.slice(0, comma);
    if (!futureScope.test(prefix) && !ciCandidateScope.test(prefix)) return [clause];
    return [prefix, clause.slice(comma + 1)].map((part) => part.trim()).filter(Boolean);
  });
}

function assertPendingArtifactClaimsAreFutureScoped(entries, affectedThrough) {
  const artifact = /\b(?:x86_64|arm64|binar(?:y|ies)|artifacts?|assets?|releases?|npm|installer|tarballs?|bytes)\b/iu;
  const linuxContext = /\b(?:linux|x86_64|arm64|musl|glibc)\b/iu;
  const portableProperty = /(?:\bstatic(?:ally)?\b|\bmusl\b|glibc[- ]free|no[- ]glibc|free of glibc|without (?:a )?glibc(?: dependency)?\b|glibc (?:is|are) not required|(?:does|do) not require glibc|requires? no glibc)/iu;
  const futureScope = /\b(?:future|candidate|planned|will|once published|after (?:it is |they are )?published|not yet published|before upload)\b/iu;
  const ciCandidateScope = /\b(?:CI|workflow|pipeline)\b.{0,100}\b(?:candidate|prepar(?:e|es|ing)|build(?:s|ing)?|verif(?:y|ies|ying))\b/iu;
  const presentScope = /\b(?:current|currently|supported|today|latest|already)\b|^\s*published\b|\bpublished Linux\b/iu;
  const presentCandidateScope = /\b(?:current|currently|latest)\b.{0,80}\b(?:CI|workflow|pipeline)\b.{0,80}\bcandidate\b|\b(?:CI|workflow|pipeline)\b.{0,80}\b(?:current|currently|latest)\b.{0,80}\bcandidate\b/iu;
  const affectedToken = new RegExp(`\\bv${affectedThrough.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&')}\\b`, 'iu');
  const historicalNonPortable = /(?:(?:was|were) not static(?:\s*[-/]\s*|\s+)musl|(?:was|were) not glibc[- ]free|did not (?:ship|use|provide) (?:a )?(?:static(?:\s*[-/]\s*|\s+)|musl)|never (?:shipped|used|provided) (?:a )?(?:static(?:\s*[-/]\s*|\s+)|musl))/iu;

  for (const [label, source] of entries) {
    for (const unit of semanticUnits(source)) {
      for (const clause of claimClauses(unit, futureScope, ciCandidateScope)) {
        if (!linuxContext.test(clause) || !artifact.test(clause) || !portableProperty.test(clause)) continue;
        if (affectedToken.test(clause) && historicalNonPortable.test(clause)) continue;
        const candidateScoped = futureScope.test(clause) || ciCandidateScope.test(clause);
        const unqualifiedPresent = presentScope.test(clause) && !presentCandidateScope.test(clause);
        if (candidateScoped && !unqualifiedPresent) continue;
        assert.fail(`${label} presents pending static-musl/glibc-free behavior as a current artifact claim: ${clause}`);
      }
    }
  }
}

export function verifyLinuxPortabilityContract({ packageVersion, contract, documents }) {
  assert.deepEqual(
    Object.keys(contract).sort(),
    ['affectedReleases', 'affectedThrough', 'correctedFrom', 'status'],
    'Linux release portability contract has an unexpected shape',
  );
  assert.ok(['pending', 'corrected'].includes(contract.status), 'Linux release portability status must be pending or corrected');
  assert.ok(Array.isArray(contract.affectedReleases) && contract.affectedReleases.length > 0, 'affectedReleases must record at least one published affected release');
  for (const version of contract.affectedReleases) parseSemver(version, 'affected release');
  parseSemver(contract.affectedThrough, 'affectedThrough');
  parseSemver(packageVersion, 'package version');
  const orderedAffected = [...contract.affectedReleases].sort(compareSemver);
  assert.deepEqual(contract.affectedReleases, orderedAffected, 'affectedReleases must be strictly ordered');
  assert.equal(new Set(contract.affectedReleases).size, contract.affectedReleases.length, 'affectedReleases must not contain duplicates');
  assert.equal(orderedAffected.at(-1), contract.affectedThrough, 'affectedThrough must be the last explicitly recorded affected release');

  const entries = Object.entries(documents);
  assert.ok(entries.length > 0, 'public Linux documents are required');
  const affectedClaim = /v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?).{0,240}?GLIBC_2\.39/giu;
  for (const [label, source] of entries) {
    const versions = [...prose(source).matchAll(affectedClaim)].map((match) => match[1]);
    assert.ok(versions.length > 0, `${label} must retain the affected Linux/GLIBC claim`);
    assert.ok(
      versions.every((version) => version === contract.affectedThrough),
      `${label} must derive its affected Linux version from affectedThrough=${contract.affectedThrough}`,
    );
  }

  const userDocuments = ['README', 'installation', 'support'].map((label) => [label, prose(documents[label] ?? '')]);
  for (const [label, source] of userDocuments) {
    assert.ok(source.includes('macOS is unaffected'), `${label} must state that macOS is unaffected`);
    assert.ok(source.includes('build from source on the target distribution'), `${label} must offer the safe source-build remediation`);
  }

  if (contract.status === 'pending') {
    assert.equal(contract.correctedFrom, null, 'pending portability requires correctedFrom=null');
    assert.ok(
      compareSemver(packageVersion, contract.affectedThrough) <= 0,
      `package ${packageVersion} moved beyond pending affected boundary ${contract.affectedThrough}; transition the contract to corrected with an explicit correctedFrom boundary`,
    );
    for (const [label, source] of userDocuments) {
      assert.ok(source.includes('wait for a corrective release'), `${label} must retain the pending wait remediation`);
      assert.ok(source.includes('no corrected release is published yet'), `${label} must state that the corrected release remains unpublished`);
      assert.match(source, /(?:future corrective release|candidate artifacts)/iu, `${label} must scope static-musl behavior to future candidates`);
    }
    assertPendingArtifactClaimsAreFutureScoped(entries, contract.affectedThrough);
  } else {
    assert.equal(typeof contract.correctedFrom, 'string', 'corrected portability requires a correctedFrom version');
    parseSemver(contract.correctedFrom, 'correctedFrom');
    assert.ok(compareSemver(contract.correctedFrom, contract.affectedThrough) > 0, 'correctedFrom must be newer than affectedThrough');
    assert.ok(compareSemver(packageVersion, contract.correctedFrom) >= 0, 'correctedFrom must be the current-or-earlier corrective package boundary');
    const correctedClaim = new RegExp(
      `v${contract.correctedFrom.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&')}.{0,240}static[- ]musl.{0,160}(?:does not|do not) require glibc`,
      'iu',
    );
    for (const [label, source] of entries.map(([label, source]) => [label, prose(source)])) {
      assert.match(source, correctedClaim, `${label} must derive its corrected current-artifact claim from correctedFrom=${contract.correctedFrom}`);
      assert.doesNotMatch(source, /no corrected release is published yet/iu, `${label} must remove the pending unpublished claim after correction`);
      assert.doesNotMatch(source, /wait for a corrective release/iu, `${label} must remove the pending wait remediation after correction`);
    }
  }

  return {
    packageVersion,
    status: contract.status,
    affectedThrough: contract.affectedThrough,
    correctedFrom: contract.correctedFrom,
  };
}
