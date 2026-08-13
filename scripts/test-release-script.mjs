#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { routeSelection } from "./ci-router.mjs";
import { classifyChanges } from "./verification-policy.mjs";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const prepareSource = fs.readFileSync(path.join(repo, "scripts/prepare-release-candidate.sh"), "utf8");
const releaseSource = fs.readFileSync(path.join(repo, "scripts/release.sh"), "utf8");
const promotionSource = fs.readFileSync(path.join(repo, "scripts/verify-release-promotion.mjs"), "utf8");
const ciRouterSource = fs.readFileSync(path.join(repo, "scripts/ci-router.mjs"), "utf8");
const verificationPolicySource = fs.readFileSync(path.join(repo, "scripts/verification-policy.mjs"), "utf8");
const changelogLinksSource = fs.readFileSync(path.join(repo, "scripts/verify-changelog-release-links.sh"), "utf8");
const repositoryVersion = JSON.parse(fs.readFileSync(path.join(repo, "package.json"), "utf8")).version;
const version = "9.9.9";
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "planr-release-contract-"));

function write(file, content, mode) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, content);
  if (mode) fs.chmodSync(file, mode);
}

function fixture(name, prepared, changelogPrepared = prepared) {
  const root = path.join(tmp, name);
  const bin = path.join(root, "bin");
  const log = path.join(root, "commands.log");
  const initial = prepared ? version : "1.7.2";
  write(path.join(root, "scripts/prepare-release-candidate.sh"), prepareSource, 0o755);
  write(path.join(root, "scripts/release.sh"), releaseSource, 0o755);
  write(path.join(root, "scripts/verify-changelog-release-links.sh"), changelogLinksSource, 0o755);
  write(path.join(root, "scripts/security-local.sh"), "#!/bin/sh\nset -eu\nprintf 'security-local\\n' >> \"$COMMAND_LOG\"\n", 0o755);
  write(path.join(root, "scripts/verify-release-eval-receipt.mjs"), "");
  write(path.join(root, "Cargo.toml"), `[package]\nname = "planr"\nversion = "${initial}"\n`);
  write(path.join(root, "Cargo.lock"), `[[package]]\nname = "planr"\nversion = "${initial}"\n`);
  for (const file of [
    "package.json",
    "plugins/planr/.codex-plugin/plugin.json",
    "plugins/planr/.claude-plugin/plugin.json",
    ".cursor-plugin/plugin.json",
  ]) write(path.join(root, file), `${JSON.stringify({ name: "planr", version: initial }, null, 2)}\n`);
  const changelog = changelogPrepared
    ? [
      "## [Unreleased]",
      "",
      `## [${version}]`,
      "",
      "## [1.7.2]",
      "",
      `[Unreleased]: https://github.com/instructa/planr/compare/v${version}...HEAD`,
      `[${version}]: https://github.com/instructa/planr/compare/v1.7.2...v${version}`,
      "",
    ]
    : [
      "## [Unreleased]",
      "",
      "## [1.7.2]",
      "",
      "[Unreleased]: https://github.com/instructa/planr/compare/v1.7.2...HEAD",
      "",
    ];
  write(path.join(root, "CHANGELOG.md"), changelog.join("\n"));
  write(path.join(root, "pnpm-lock.yaml"), "lockfileVersion: '9.0'\n");
  write(path.join(root, "pnpm-lock.baseline"), "lockfileVersion: '9.0'\n");
  write(path.join(root, "apps/docs/content/docs/reference/cli-generated.mdx"), `cli ${initial}\n`);
  write(path.join(root, "apps/docs/content/docs/reference/mcp-schemas-generated.mdx"), `mcp ${initial}\n`);
  write(log, "");

  write(path.join(bin, "git"), `#!/bin/sh
set -eu
case "$1 \${2:-}" in
  "rev-parse --abbrev-ref") echo main ;;
  "rev-parse v${version}") exit 1 ;;
  "status --porcelain") ;;
  "diff --quiet")
    printf 'git %s\\n' "$*" >> "$COMMAND_LOG"
    cmp -s pnpm-lock.yaml pnpm-lock.baseline
    ;;
  *) printf 'git %s\\n' "$*" >> "$COMMAND_LOG" ;;
esac
`, 0o755);
  write(path.join(bin, "cargo"), `#!/bin/sh
set -eu
printf 'cargo %s\\n' "$*" >> "$COMMAND_LOG"
if [ "\${1:-} \${2:-}" = "build --quiet" ]; then
  candidate=$(sed -n 's/^version = "\\([^"]*\\)"/\\1/p' Cargo.toml | head -n 1)
  sed "s/^version = \".*\"/version = \"$candidate\"/" Cargo.lock > Cargo.lock.tmp
  mv Cargo.lock.tmp Cargo.lock
fi
`, 0o755);
  write(path.join(bin, "pnpm"), `#!/bin/sh
set -eu
printf 'pnpm %s\\n' "$*" >> "$COMMAND_LOG"
candidate=$(sed -n 's/.*"version": "\\([^"]*\\)".*/\\1/p' package.json | head -n 1)
case "$*" in
  "install --frozen-lockfile")
    if [ "\${MUTATE_LOCKFILE:-0}" = 1 ]; then printf '# drift\\n' >> pnpm-lock.yaml; fi
    ;;
  *"reference:generate")
    printf 'cli %s\\n' "$candidate" > apps/docs/content/docs/reference/cli-generated.mdx
    printf 'mcp %s\\n' "$candidate" > apps/docs/content/docs/reference/mcp-schemas-generated.mdx
    ;;
  *"reference:check")
    if [ "\${FAIL_REFERENCE_CHECK:-0}" = 1 ]; then exit 42; fi
    test "$(cat apps/docs/content/docs/reference/cli-generated.mdx)" = "cli $candidate"
    test "$(cat apps/docs/content/docs/reference/mcp-schemas-generated.mdx)" = "mcp $candidate"
    ;;
esac
`, 0o755);
  for (const command of ["node", "npm"]) {
    write(path.join(bin, command), `#!/bin/sh\nset -eu\nprintf '${command} %s\\n' "$*" >> "$COMMAND_LOG"\n`, 0o755);
  }
  return { root, bin, log };
}

function run(name, script, prepared, env = {}) {
  const test = fixture(name, prepared, script === "release.sh");
  const args = script === "release.sh" ? [version, "contract test"] : [version];
  const result = spawnSync("sh", [`scripts/${script}`, ...args], {
    cwd: test.root,
    encoding: "utf8",
    env: {
      ...process.env,
      ...env,
      PATH: `${test.bin}:${process.env.PATH}`,
      COMMAND_LOG: test.log,
      PLANR_RELEASE_EVAL_RECEIPT: path.join(test.root, "receipt.json"),
      PLANR_RELEASE_EVAL_SUITE: path.join(test.root, "suite.json"),
      PLANR_RELEASE_EVAL_DB: path.join(test.root, "eval.sqlite"),
      PLANR_RELEASE_PLANR_BIN: path.join(test.root, "planr"),
      PLANR_RELEASE_CI_RECEIPT: path.join(test.root, "ci-receipt.json"),
      PLANR_RELEASE_APPROVAL: path.join(test.root, "approval.json"),
    },
  });
  const calls = fs.readFileSync(test.log, "utf8").trim().split("\n").filter(Boolean);
  return { ...test, result, calls };
}

assert.doesNotMatch(prepareSource, /git (add|commit|tag|push)/u, "candidate preparation must not mutate Git history");
assert.doesNotMatch(releaseSource, /git (add|commit)/u, "publication must not change the reviewed candidate commit");
assert.doesNotMatch(releaseSource, /^replace\(\)|^replace /mu, "publication must not rewrite manifests");
assert.doesNotMatch(
  prepareSource,
  /verify-changelog-release-links\.sh/u,
  "candidate preparation must run before the target changelog section is authored",
);
assert.match(releaseSource, /verify-changelog-release-links\.sh/u, "publication must verify changelog comparison links");

const repositoryChangelogLinks = spawnSync(
  "sh",
  ["scripts/verify-changelog-release-links.sh", repositoryVersion],
  { cwd: repo, encoding: "utf8" },
);
assert.equal(repositoryChangelogLinks.status, 0, repositoryChangelogLinks.stderr);

function changelogLinkCheck(name, content) {
  const root = path.join(tmp, name);
  write(path.join(root, "scripts/verify-changelog-release-links.sh"), changelogLinksSource, 0o755);
  write(path.join(root, "CHANGELOG.md"), content);
  return spawnSync("sh", ["scripts/verify-changelog-release-links.sh", version], {
    cwd: root,
    encoding: "utf8",
  });
}

const validChangelog = [
  "## [Unreleased]",
  `## [${version}]`,
  "## [1.7.2]",
  `[Unreleased]: https://github.com/instructa/planr/compare/v${version}...HEAD`,
  `[${version}]: https://github.com/instructa/planr/compare/v1.7.2...v${version}`,
  "",
].join("\n");
assert.equal(changelogLinkCheck("changelog-valid", validChangelog).status, 0);
const staleUnreleased = changelogLinkCheck(
  "changelog-stale-unreleased",
  validChangelog.replace(`compare/v${version}...HEAD`, "compare/v1.7.2...HEAD"),
);
assert.equal(staleUnreleased.status, 1, "stale Unreleased comparison must fail");
assert.match(staleUnreleased.stderr, /\[Unreleased\] comparison must start/u);
const missingReleaseLink = changelogLinkCheck(
  "changelog-missing-release-link",
  validChangelog.replace(/^\[9\.9\.9\]:.*\n/mu, ""),
);
assert.equal(missingReleaseLink.status, 1, "missing release comparison must fail");
assert.match(missingReleaseLink.stderr, /\[9\.9\.9\] comparison must span/u);

const prepared = run("prepare-pass", "prepare-release-candidate.sh", false);
assert.equal(prepared.result.status, 0, prepared.result.stderr);
const preparedChangelog = fs.readFileSync(path.join(prepared.root, "CHANGELOG.md"), "utf8");
assert.doesNotMatch(preparedChangelog, new RegExp(`^## \\[${version.replaceAll(".", "\\.")}\\]`, "mu"));
assert.match(preparedChangelog, /\[Unreleased\]: https:\/\/github\.com\/instructa\/planr\/compare\/v1\.7\.2\.\.\.HEAD/u);
for (const file of [
  "Cargo.toml",
  "Cargo.lock",
  "package.json",
  "plugins/planr/.codex-plugin/plugin.json",
  "plugins/planr/.claude-plugin/plugin.json",
  ".cursor-plugin/plugin.json",
  "apps/docs/content/docs/reference/cli-generated.mdx",
  "apps/docs/content/docs/reference/mcp-schemas-generated.mdx",
]) assert.match(fs.readFileSync(path.join(prepared.root, file), "utf8"), new RegExp(version.replaceAll(".", "\\.")), `${file} was not prepared`);
assert.deepEqual(
  prepared.calls,
  [
    "pnpm install --frozen-lockfile",
    "git diff --quiet -- pnpm-lock.yaml",
    "cargo build --quiet",
    "pnpm --filter @planr/docs reference:generate",
    "pnpm --filter @planr/docs reference:check",
  ],
  "candidate preparation order drifted",
);

const lockDrift = run("prepare-lock-drift", "prepare-release-candidate.sh", false, { MUTATE_LOCKFILE: "1" });
assert.equal(lockDrift.result.status, 1, "lockfile drift must fail candidate preparation");
assert.ok(!lockDrift.calls.some((call) => call.startsWith("cargo ")), "lockfile drift must stop before build");

const referenceDrift = run("prepare-reference-drift", "prepare-release-candidate.sh", false, { FAIL_REFERENCE_CHECK: "1" });
assert.equal(referenceDrift.result.status, 42, "reference drift must preserve checker failure");

const unprepared = run("release-unprepared", "release.sh", false);
assert.equal(unprepared.result.status, 1, "publication must reject an unprepared version");
assert.match(unprepared.result.stderr, /not prepared candidate/u);
assert.ok(!unprepared.calls.some((call) => call.startsWith("git tag") || call.startsWith("git push")));

const released = run("release-pass", "release.sh", true);
assert.equal(released.result.status, 0, released.result.stderr);
const expected = [
  "node scripts/verify-release-promotion.mjs",
  `git tag -a v${version}`,
  `git push origin HEAD v${version}`,
];
let cursor = -1;
for (const prefix of expected) {
  const index = released.calls.findIndex((call, candidate) => candidate > cursor && call.startsWith(prefix));
  assert.ok(index > cursor, `missing or reordered publication command: ${prefix}`);
  cursor = index;
}
for (const repeatedGate of ["pnpm install", "cargo build", "cargo test", "npm pack", "security-local"]) {
  assert.ok(!released.calls.some((call) => call.startsWith(repeatedGate)), `publication must not replay ${repeatedGate}`);
}

const promotionRoot = path.join(tmp, "promotion-verifier");
const promotionBin = path.join(promotionRoot, "bin");
const sourceSha = "a".repeat(40);
const baseSha = "d".repeat(40);
write(path.join(promotionRoot, "scripts/verify-release-promotion.mjs"), promotionSource);
write(path.join(promotionRoot, "scripts/ci-router.mjs"), ciRouterSource);
write(path.join(promotionRoot, "scripts/verification-policy.mjs"), verificationPolicySource);
write(path.join(promotionBin, "git"), `#!/bin/sh
set -eu
case "$1 \${2:-}" in
  "rev-parse --verify")
    case "\${3:-}" in HEAD*) printf '%s\\n' "$SOURCE_SHA" ;; *) printf '%s\\n' "$BASE_SHA" ;; esac
    ;;
  "merge-base --is-ancestor") ;;
  "describe --tags") printf 'v9.9.8\\n' ;;
  "diff --name-only") printf '%s\\n' "\${DIFF_PATHS:-README.md}" ;;
  "diff --name-status") printf 'M\\0%s\\0' "\${DIFF_PATH:-README.md}" ;;
  *) echo "unexpected git command: $*" >&2; exit 2 ;;
esac
`, 0o755);
write(path.join(promotionBin, "gh"), `#!/bin/sh
set -eu
if [ "$1" = "api" ]; then
  printf '{"id":123,"run_attempt":1,"name":"CI","event":"push","head_branch":"main","head_sha":"%s","conclusion":"%s","repository":{"full_name":"instructa/planr"}}\\n' "$SOURCE_SHA" "\${GH_CONCLUSION:-success}"
  exit 0
fi
if [ "$1 $2" = "run download" ]; then
  destination=""
  while [ "$#" -gt 0 ]; do
    if [ "$1" = "--dir" ]; then destination="$2"; shift 2; continue; fi
    shift
  done
  cp "$AUTHENTIC_RECEIPT" "$destination/promotion-receipt.json"
  exit 0
fi
echo "unexpected gh command: $*" >&2
exit 2
`, 0o755);
const fixtureSelection = classifyChanges([{ status: "M", path: "README.md" }], {
  baseRevision: baseSha,
  headRevision: sourceSha,
});
const fixtureRouting = routeSelection(fixtureSelection);
const ciReceipt = {
  schema_version: "planr.ci-promotion-receipt.v1",
  repository: "instructa/planr",
  workflow: "CI",
  run_id: "123",
  run_attempt: "1",
  event: "push",
  source_ref: "refs/heads/main",
  source_base_sha: baseSha,
  source_sha: sourceSha,
  conclusion: "success",
  policy: {
    profile: fixtureSelection.profile,
    version: fixtureSelection.policyVersion,
    digest: fixtureSelection.policyDigest,
    changed_files_digest: fixtureSelection.changedFilesDigest,
  },
  jobs: Object.fromEntries(Object.entries(fixtureRouting)
    .filter(([key]) => ["docs", "quality", "release", "linux_portability"].includes(key))
    .map(([key, selected]) => [key, selected === "true" ? "success" : "skipped"])),
};
const approval = {
  schema_version: "planr.release-approval.v1",
  approval_id: "approval-123",
  source_sha: sourceSha,
  version,
  decision: "approved",
  approved_by: "maintainer@example.invalid",
  approved_at: new Date(Date.now() - 60_000).toISOString(),
};
write(path.join(promotionRoot, "ci.json"), `${JSON.stringify(ciReceipt)}\n`);
write(path.join(promotionRoot, "approval.json"), `${JSON.stringify(approval)}\n`);
function verifyPromotion({
  receipt = "ci.json",
  authenticReceipt = "ci.json",
  approvalFile = "approval.json",
  requestedVersion = version,
  env = {},
} = {}) {
  return spawnSync(process.execPath, [
    "scripts/verify-release-promotion.mjs",
    "--version", requestedVersion,
    "--ci-receipt", receipt,
    "--approval", approvalFile,
  ], {
    cwd: promotionRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      ...env,
      PATH: `${promotionBin}:${process.env.PATH}`,
      SOURCE_SHA: sourceSha,
      BASE_SHA: baseSha,
      AUTHENTIC_RECEIPT: path.join(promotionRoot, authenticReceipt),
    },
  });
}
const promoted = verifyPromotion();
assert.equal(promoted.status, 0, promoted.stderr);
assert.equal(JSON.parse(promoted.stdout).evaluation, "not_required");
const failedCi = verifyPromotion({ env: { GH_CONCLUSION: "failure" } });
assert.notEqual(failedCi.status, 0, "authoritative failed CI must block promotion");
const evalRequired = verifyPromotion({ env: { DIFF_PATHS: "plugins/planr/skills/planr-loop/SKILL.md" } });
assert.notEqual(evalRequired.status, 0, "evaluated-subject changes must require external eval evidence");
const prereleaseVersion = `${version}-alpha.1`;
write(path.join(promotionRoot, "prerelease-approval.json"), `${JSON.stringify({
  ...approval,
  approval_id: "approval-prerelease-123",
  version: prereleaseVersion,
})}\n`);
const prereleasePromotion = verifyPromotion({
  approvalFile: "prerelease-approval.json",
  requestedVersion: prereleaseVersion,
  env: { DIFF_PATHS: "plugins/planr/skills/planr-loop/SKILL.md" },
});
assert.equal(prereleasePromotion.status, 0, prereleasePromotion.stderr);
assert.equal(JSON.parse(prereleasePromotion.stdout).release_channel, "alpha");
assert.equal(JSON.parse(prereleasePromotion.stdout).evaluation, "not_required_before_prerelease");

function rejectBoundMutation(name, mutate, errorPattern, message) {
  const candidate = mutate(structuredClone(ciReceipt));
  const file = `${name}.json`;
  write(path.join(promotionRoot, file), `${JSON.stringify(candidate)}\n`);
  const result = verifyPromotion({ receipt: file, authenticReceipt: file });
  assert.notEqual(result.status, 0, message);
  assert.match(result.stderr, errorPattern, `${name} must fail for its binding check`);
}
rejectBoundMutation("forged-policy", (receipt) => {
  receipt.policy.digest = `sha256:${"b".repeat(64)}`;
  return receipt;
}, /policy digest is stale/u, "forged current-run policy digest must fail closed");
rejectBoundMutation("forged-policy-version", (receipt) => {
  receipt.policy.version = "0.0.0";
  return receipt;
}, /policy version is stale/u, "forged current-run policy version must fail closed");
rejectBoundMutation("forged-changes", (receipt) => {
  receipt.policy.changed_files_digest = `sha256:${"c".repeat(64)}`;
  return receipt;
}, /changed-files digest mismatch/u, "forged current-run changed-files digest must fail closed");
rejectBoundMutation("forged-profile", (receipt) => {
  receipt.policy.profile = "core";
  return receipt;
}, /profile mismatch/u, "forged current-run profile must fail closed");
rejectBoundMutation("forged-jobs", (receipt) => {
  receipt.jobs.docs = "skipped";
  return receipt;
}, /does not match the current policy selection/u, "selected job recorded as skipped must fail closed");
rejectBoundMutation("copied-receipt", (receipt) => {
  receipt.source_sha = "f".repeat(40);
  return receipt;
}, /does not bind the current candidate SHA/u, "receipt copied from another source SHA must fail closed");

const unauthenticatedReceipt = structuredClone(ciReceipt);
unauthenticatedReceipt.policy.digest = `sha256:${"e".repeat(64)}`;
write(path.join(promotionRoot, "unauthenticated-local.json"), `${JSON.stringify(unauthenticatedReceipt)}\n`);
const unauthenticated = verifyPromotion({ receipt: "unauthenticated-local.json", authenticReceipt: "ci.json" });
assert.notEqual(unauthenticated.status, 0, "locally forged receipt must not replace the run artifact");
assert.match(unauthenticated.stderr, /does not match the authenticated run artifact/u);

const writtenReceipt = path.join(tmp, "written-promotion-receipt.json");
const eventPath = path.join(tmp, "push-event.json");
write(eventPath, `${JSON.stringify({ before: baseSha })}\n`);
const writeReceiptResult = spawnSync(process.execPath, ["scripts/write-ci-promotion-receipt.mjs", writtenReceipt], {
  cwd: repo,
  encoding: "utf8",
  env: {
    ...process.env,
    GITHUB_REPOSITORY: "instructa/planr",
    GITHUB_WORKFLOW: "CI",
    GITHUB_RUN_ID: "123",
    GITHUB_RUN_ATTEMPT: "1",
    GITHUB_EVENT_NAME: "push",
    GITHUB_REF: "refs/heads/main",
    GITHUB_SHA: sourceSha,
    GITHUB_EVENT_PATH: eventPath,
    PLANR_PROFILE: fixtureSelection.profile,
    PLANR_POLICY_VERSION: fixtureSelection.policyVersion,
    PLANR_POLICY_DIGEST: fixtureSelection.policyDigest,
    PLANR_CHANGED_FILES_DIGEST: fixtureSelection.changedFilesDigest,
    PLANR_DOCS_RESULT: ciReceipt.jobs.docs,
    PLANR_QUALITY_RESULT: ciReceipt.jobs.quality,
    PLANR_RELEASE_RESULT: ciReceipt.jobs.release,
    PLANR_LINUX_RESULT: ciReceipt.jobs.linux_portability,
  },
});
assert.equal(writeReceiptResult.status, 0, writeReceiptResult.stderr);
assert.equal(JSON.parse(fs.readFileSync(writtenReceipt, "utf8")).source_sha, sourceSha);
assert.equal(JSON.parse(fs.readFileSync(writtenReceipt, "utf8")).source_base_sha, baseSha);

fs.rmSync(tmp, { recursive: true, force: true });
console.log(JSON.stringify({
  verdict: "pass",
  candidate_transition_owner: "scripts/prepare-release-candidate.sh",
  publication_owner: "scripts/release.sh",
  reviewed_source_mutation_during_publication: false,
  candidate_git_mutation: false,
  fail_closed_cases: ["lockfile drift", "reference drift", "stale changelog comparison", "missing changelog comparison", "unprepared version"],
  exact_sha_promotion: true,
  repeated_publication_gates: 0,
  stable_external_evaluation: true,
  prerelease_prepublication_evaluation: false,
  rejected_receipt_binding_regressions: 7,
}, null, 2));
