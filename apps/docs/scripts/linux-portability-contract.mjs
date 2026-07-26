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

export const LINUX_PORTABILITY_NOTICE_SURFACES = Object.freeze([
  'README',
  'installation',
  'support',
  'release',
  'maintainerRelease',
  'changelog',
]);

const mdxNoticeSurfaces = new Set(['installation', 'support', 'release']);
const noticeReservedPrefix = 'planr:linux-release-portability';
const noticeMarkerPatterns = [
  /<!--\s*(planr:linux-release-portability[^>]*)-->/giu,
  /\{\/\*\s*(planr:linux-release-portability[^*]*)\*\/\}/giu,
];

function noticeMarker(surface, boundary, schema) {
  const marker = `planr:linux-release-portability:${boundary} surface=${surface} schema=${schema}`;
  return mdxNoticeSurfaces.has(surface) ? `{/* ${marker} */}` : `<!-- ${marker} -->`;
}

function noticeMarkers(source) {
  return noticeMarkerPatterns
    .flatMap((pattern) => [...source.matchAll(pattern)].map((match) => ({
      boundary: /^planr:linux-release-portability:([a-z-]+)\b/iu.exec(match[1].trim())?.[1].toLowerCase() ?? 'invalid',
      index: match.index,
      reservedIndex: match.index + match[0].toLowerCase().indexOf(noticeReservedPrefix),
      raw: match[0],
    })))
    .sort((left, right) => left.index - right.index);
}

function inspectNoticeStructure(source, contract, surface, requireExact) {
  const reservedIndexes = [...source.matchAll(/planr:linux-release-portability/gu)].map((match) => match.index);
  assert.equal(reservedIndexes.length, 2, `${surface} must contain exactly two raw Linux portability reserved tokens`);
  const markers = noticeMarkers(source);
  assert.equal(markers.length, 2, `${surface} Linux portability reserved tokens must use valid comment wrappers`);
  assert.deepEqual(
    markers.map(({ reservedIndex }) => reservedIndex),
    reservedIndexes,
    `${surface} Linux portability reserved tokens must be the parsed comment markers`,
  );
  const starts = markers.filter(({ boundary }) => boundary === 'start');
  const ends = markers.filter(({ boundary }) => boundary === 'end');
  assert.equal(starts.length, 1, `${surface} must contain exactly one Linux portability start marker`);
  assert.equal(ends.length, 1, `${surface} must contain exactly one Linux portability end marker`);
  const expectedStart = noticeMarker(surface, 'start', contract.noticeSchema);
  const expectedEnd = noticeMarker(surface, 'end', contract.noticeSchema);
  assert.equal(starts[0].raw, expectedStart, `${surface} Linux portability start marker is edited or assigned to the wrong surface`);
  assert.equal(ends[0].raw, expectedEnd, `${surface} Linux portability end marker is edited or assigned to the wrong surface`);
  assert.ok(starts[0].index < ends[0].index, `${surface} Linux portability markers are out of order`);
  const endIndex = ends[0].index + ends[0].raw.length;
  if (requireExact) {
    assert.equal(
      source.slice(starts[0].index, endIndex),
      renderLinuxPortabilityNotice(contract, surface),
      `${surface} Linux portability notice body is stale or mutated`,
    );
  }
  return { startIndex: starts[0].index, endIndex };
}

export function renderLinuxPortabilityNotice(contract, surface) {
  assert.equal(contract.noticeSchema, 1, 'Linux portability noticeSchema must be 1');
  assert.ok(LINUX_PORTABILITY_NOTICE_SURFACES.includes(surface), `unknown Linux portability notice surface ${surface}`);
  const correctedFrom = contract.correctedFrom === null ? 'unpublished' : `v${contract.correctedFrom}`;
  const lines = [
    noticeMarker(surface, 'start', contract.noticeSchema),
    `> **Linux release portability — ${contract.status}**`,
    '>',
    `> Contract state: \`status=${contract.status}\`; \`affectedThrough=v${contract.affectedThrough}\`; \`correctedFrom=${correctedFrom}\`.`,
    `> Published Linux release, installer, and npm binaries through v${contract.affectedThrough} require GLIBC_2.39; macOS is unaffected.`,
  ];
  if (contract.status === 'pending') {
    lines.push(
      '> On an affected Linux system, build from source on the target distribution or wait for a corrective release.',
      '> Candidate artifacts remain CI-only evidence for a future corrective release; no corrected release is published yet.',
    );
  } else {
    lines.push(
      `> Starting with v${contract.correctedFrom}, current Linux release, installer, and npm artifacts are static-musl executables and do not require glibc.`,
      `> On an affected Linux release, build from source on the target distribution or upgrade to v${contract.correctedFrom}.`,
    );
  }
  lines.push(noticeMarker(surface, 'end', contract.noticeSchema));
  return lines.join('\n');
}

export function replaceLinuxPortabilityNotice(source, contract, surface) {
  const { startIndex, endIndex } = inspectNoticeStructure(source, contract, surface, false);
  return `${source.slice(0, startIndex)}${renderLinuxPortabilityNotice(contract, surface)}${source.slice(endIndex)}`;
}

function assertCanonicalNotices(documents, contract) {
  assert.deepEqual(
    Object.keys(documents).sort(),
    [...LINUX_PORTABILITY_NOTICE_SURFACES].sort(),
    'Linux portability public surface inventory drifted',
  );
  for (const surface of LINUX_PORTABILITY_NOTICE_SURFACES) {
    inspectNoticeStructure(documents[surface], contract, surface, true);
  }
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

function claimClauses(unit) {
  const clauses = unit
    .replace(/[()]/gu, ' — ')
    .split(/\s*(?:—|–)\s*|\s+\/\s+(?=(?:future|candidate|CI|workflow|pipeline)\b)|,\s*(?=(?:although|though|but|whereas|while|with|yet|and\s+(?:future|candidate|CI|workflow|pipeline)|future|candidate)\b)|\s+and\s+(?=(?:future|candidate|CI|workflow|pipeline)\b)|\s+(?=(?:although|but|whereas|while|because|even\s+as)\b)/giu)
    .map((clause) => clause.trim())
    .filter(Boolean);

  return clauses.flatMap((clause) => {
    const comma = clause.indexOf(',');
    if (comma < 0) return [clause];
    const prefix = clause.slice(0, comma);
    if (!/\b(?:future|candidate|planned|CI|workflow|pipeline)\b/iu.test(prefix)) return [clause];
    return [prefix, clause.slice(comma + 1)].map((part) => part.trim()).filter(Boolean);
  });
}

function tokenDistance(source, leftEnd, rightStart) {
  return source.slice(leftEnd, rightStart).match(/[0-9A-Za-z_]+(?:-[0-9A-Za-z_]+)*/gu)?.length ?? 0;
}

function isCandidateScopedArtifactClaim(clause, artifact, portableProperty) {
  const artifacts = [...clause.matchAll(new RegExp(artifact.source, `${artifact.flags}g`))];
  const properties = [...clause.matchAll(new RegExp(portableProperty.source, `${portableProperty.flags}g`))];
  const qualifiers = [...clause.matchAll(/\b(?:future|candidates?|planned)\b/giu)];
  const candidateNouns = [...clause.matchAll(/\bcandidates?\b/giu)];
  const auxiliaries = [...clause.matchAll(/\bwill\b/giu)];
  const lifecycle = /\b(?:once published|after (?:it is |they are )?published|not yet published|before upload)\b/iu.test(clause);
  const candidateArtifact = qualifiers.some((qualifier) =>
    artifacts.some((artifactMatch) =>
      qualifier.index < artifactMatch.index
      && tokenDistance(clause, qualifier.index + qualifier[0].length, artifactMatch.index) <= 3));
  const ciCandidateAction = /\b(?:CI|workflow|pipeline)\b/iu.test(clause)
    && /\b(?:prepar(?:e|es|ing)|build(?:s|ing|t)?|verif(?:y|ies|ied|ying)|test(?:s|ed|ing)?)\b/iu.test(clause)
    && candidateNouns.some((candidate) =>
      properties.some((property) => {
        const left = Math.min(candidate.index, property.index);
        const right = Math.max(candidate.index + candidate[0].length, property.index + property[0].length);
        return tokenDistance(clause, left, right) <= 3;
      }));
  const futurePredicate = artifacts.some((artifactMatch) =>
    properties.some((property) =>
      auxiliaries.some((auxiliary) => artifactMatch.index < auxiliary.index && auxiliary.index < property.index)));
  const explicitlyCurrent = /\b(?:current|currently|latest|today|already)\b|^\s*published\b|\bpublished Linux\b/iu.test(clause);
  if (!explicitlyCurrent) return candidateArtifact || ciCandidateAction || futurePredicate || lifecycle;
  const candidateLifecycle = /\b(?:CI|workflow|pipeline)\b/iu.test(clause) || lifecycle || futurePredicate;
  return ciCandidateAction || (candidateArtifact && candidateLifecycle);
}

function assertPendingArtifactClaimsAreFutureScoped(entries, affectedThrough) {
  const artifact = /\b(?:x86_64|arm64|binar(?:y|ies)|artifacts?|assets?|releases?|npm|installer|tarballs?|bytes)\b/iu;
  const linuxContext = /\b(?:linux|x86_64|arm64|musl|glibc)\b/iu;
  const portableProperty = /(?:\bstatic(?:ally)?\b|\bmusl\b|glibc[- ]free|no[- ]glibc|free of glibc|without (?:a )?glibc(?: dependency)?\b|glibc (?:is|are) not required|(?:does|do) not require glibc|requires? no glibc)/iu;
  const affectedToken = new RegExp(`\\bv${affectedThrough.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&')}\\b`, 'iu');
  const historicalNonPortable = /(?:(?:was|were) not static(?:\s*[-/]\s*|\s+)musl|(?:was|were) not glibc[- ]free|did not (?:ship|use|provide) (?:a )?(?:static(?:\s*[-/]\s*|\s+)|musl)|never (?:shipped|used|provided) (?:a )?(?:static(?:\s*[-/]\s*|\s+)|musl))/iu;

  for (const [label, source] of entries) {
    for (const unit of semanticUnits(source)) {
      for (const clause of claimClauses(unit)) {
        if (!linuxContext.test(clause) || !artifact.test(clause) || !portableProperty.test(clause)) continue;
        if (affectedToken.test(clause) && historicalNonPortable.test(clause)) continue;
        if (isCandidateScopedArtifactClaim(clause, artifact, portableProperty)) continue;
        assert.fail(`${label} presents pending static-musl/glibc-free behavior as a current artifact claim: ${clause}`);
      }
    }
  }
}

export function verifyLinuxPortabilityContract({ packageVersion, contract, documents }) {
  assert.deepEqual(
    Object.keys(contract).sort(),
    ['affectedReleases', 'affectedThrough', 'correctedFrom', 'noticeSchema', 'status'],
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

  if (contract.status === 'pending') {
    assert.equal(contract.correctedFrom, null, 'pending portability requires correctedFrom=null');
    assert.ok(
      compareSemver(packageVersion, contract.affectedThrough) <= 0,
      `package ${packageVersion} moved beyond pending affected boundary ${contract.affectedThrough}; transition the contract to corrected with an explicit correctedFrom boundary`,
    );
  } else {
    assert.equal(typeof contract.correctedFrom, 'string', 'corrected portability requires a correctedFrom version');
    parseSemver(contract.correctedFrom, 'correctedFrom');
    assert.ok(compareSemver(contract.correctedFrom, contract.affectedThrough) > 0, 'correctedFrom must be newer than affectedThrough');
    assert.ok(compareSemver(packageVersion, contract.correctedFrom) >= 0, 'correctedFrom must be the current-or-earlier corrective package boundary');
  }

  const entries = Object.entries(documents);
  assert.ok(entries.length > 0, 'public Linux documents are required');
  assertCanonicalNotices(documents, contract);
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
    for (const [label, source] of userDocuments) {
      assert.ok(source.includes('wait for a corrective release'), `${label} must retain the pending wait remediation`);
      assert.ok(source.includes('no corrected release is published yet'), `${label} must state that the corrected release remains unpublished`);
      assert.match(source, /(?:future corrective release|candidate artifacts)/iu, `${label} must scope static-musl behavior to future candidates`);
    }
    assertPendingArtifactClaimsAreFutureScoped(entries, contract.affectedThrough);
  } else {
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
