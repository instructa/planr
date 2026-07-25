#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const releasePath = path.join(repo, "scripts/release.sh");
const release = fs.readFileSync(releasePath, "utf8");
const docsPackage = JSON.parse(fs.readFileSync(path.join(repo, "apps/docs/package.json"), "utf8"));
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "planr-release-script-contract-"));
const version = "9.9.9";
const workspaceSyncCommand = "pnpm install --frozen-lockfile";
const lockfileInvariantCommand = "if ! git diff --quiet -- pnpm-lock.yaml; then";
const referenceGenerateCommand = "pnpm --filter @planr/docs reference:generate";
const referenceCheckCommand = "pnpm --filter @planr/docs reference:check";
const laterReleaseCommands = [
  ["local eval receipt gate", "node scripts/verify-release-eval-receipt.mjs"],
  ["cargo test", "cargo test"],
  ["deterministic release-eval gate", "npm run verify:release-eval-gate"],
  ["npm package dry-run", "npm pack --dry-run"],
  ["security gate", "scripts/security-local.sh"],
  ["exact Git staging", "git add Cargo.toml"],
  ["release commit", "git commit -m"],
  ["annotated release tag", "git tag -a"],
  ["release push", "git push origin"],
];

function commandIndex(source, command) {
  const marker = `\n${command}`;
  const index = source.indexOf(marker);
  assert.ok(index >= 0, `release script must contain command line: ${command}`);
  return index + 1;
}

function assertReferenceCheckBeforeEveryLaterCommand(source) {
  const checkIndex = commandIndex(source, referenceCheckCommand);
  for (const [label, marker] of laterReleaseCommands) {
    const markerIndex = commandIndex(source, marker);
    assert.ok(checkIndex < markerIndex, `reference:check must precede ${label}`);
  }
}

function assertWorkspaceSyncContract(source) {
  assert.doesNotMatch(
    source,
    /verifyDepsBeforeRun|verify-deps-before-run|verify_deps_before_run/u,
    "release must never bypass pnpm dependency verification",
  );
  const finalBumpIndex = commandIndex(source, "replace .cursor-plugin/plugin.json");
  const syncIndex = commandIndex(source, workspaceSyncCommand);
  const invariantIndex = commandIndex(source, lockfileInvariantCommand);
  const buildIndex = commandIndex(source, "cargo build --quiet");
  const generateIndex = commandIndex(source, referenceGenerateCommand);
  assert.ok(finalBumpIndex < syncIndex, "workspace synchronization must follow every manifest bump");
  assert.ok(syncIndex < invariantIndex, "lockfile byte verification must follow workspace synchronization");
  assert.ok(invariantIndex < buildIndex, "lockfile byte verification must precede the candidate build");
  assert.ok(invariantIndex < generateIndex, "lockfile byte verification must precede reference generation");
}

function commandBlock(source, marker) {
  const markerIndex = commandIndex(source, marker);
  const start = source.lastIndexOf("\n", markerIndex) + 1;
  let end = source.indexOf("\n", markerIndex);
  assert.ok(end >= 0, `command has no line ending: ${marker}`);
  while (source.slice(start, end).trimEnd().endsWith("\\")) {
    end = source.indexOf("\n", end + 1);
    assert.ok(end >= 0, `continued command has no final line: ${marker}`);
  }
  return source.slice(start, end + 1);
}

function moveCommandBeforeReferenceCheck(source, marker) {
  const block = commandBlock(source, marker);
  const withoutBlock = source.replace(block, "");
  const checkIndex = commandIndex(withoutBlock, referenceCheckCommand);
  const checkLineStart = withoutBlock.lastIndexOf("\n", checkIndex) + 1;
  return `${withoutBlock.slice(0, checkLineStart)}${block}${withoutBlock.slice(checkLineStart)}`;
}

function write(file, content, mode) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, content);
  if (mode) fs.chmodSync(file, mode);
}

function setupCase(name, releaseSource = release) {
  const root = path.join(tmp, name);
  const bin = path.join(root, "test-bin");
  const commandLog = path.join(root, "commands.log");
  fs.mkdirSync(bin, { recursive: true });
  write(path.join(root, "scripts/release.sh"), releaseSource, 0o755);
  write(path.join(root, "scripts/security-local.sh"), `#!/bin/sh\nset -eu\nprintf '%s\\n' 'security-local' >> "$COMMAND_LOG"\n`, 0o755);
  write(path.join(root, "Cargo.toml"), `[package]\nname = "planr"\nversion = "1.7.1"\n`);
  write(path.join(root, "Cargo.lock"), `[[package]]\nname = "planr"\nversion = "1.7.1"\n`);
  for (const file of [
    "package.json",
    "plugins/planr/.codex-plugin/plugin.json",
    "plugins/planr/.claude-plugin/plugin.json",
    ".cursor-plugin/plugin.json",
  ]) write(path.join(root, file), `${JSON.stringify({ name: "planr", version: "1.7.1" }, null, 2)}\n`);
  write(path.join(root, "CHANGELOG.md"), `## [${version}]\n`);
  write(path.join(root, "pnpm-lock.yaml"), "lockfileVersion: '9.0'\n");
  write(path.join(root, "pnpm-lock.baseline"), "lockfileVersion: '9.0'\n");
  write(path.join(root, ".workspace-state-version"), "1.7.1\n");
  write(path.join(root, "apps/docs/content/docs/reference/cli-generated.mdx"), "cli 1.7.1\n");
  write(path.join(root, "apps/docs/content/docs/reference/mcp-schemas-generated.mdx"), "mcp 1.7.1\n");
  write(commandLog, "");

  write(path.join(bin, "git"), `#!/bin/sh
set -eu
case "$1 \${2:-}" in
  "rev-parse --abbrev-ref") echo main ;;
  "status --porcelain") ;;
  "rev-parse v${version}") exit 1 ;;
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
  release_version=$(sed -n 's/^version = "\\([^"]*\\)"/\\1/p' Cargo.toml | head -n 1)
  sed "s/^version = \".*\"/version = \"$release_version\"/" Cargo.lock > Cargo.lock.test-tmp
  mv Cargo.lock.test-tmp Cargo.lock
fi
`, 0o755);
  write(path.join(bin, "pnpm"), `#!/bin/sh
set -eu
printf 'pnpm %s\\n' "$*" >> "$COMMAND_LOG"
release_version=$(sed -n 's/.*"version": "\\([^"]*\\)".*/\\1/p' package.json | head -n 1)
case "$*" in
  "install --frozen-lockfile")
    if [ "\${FAIL_WORKSPACE_SYNC:-0}" = 1 ]; then
      echo 'synthetic workspace sync failure' >&2
      exit 43
    fi
    printf '%s\\n' "$release_version" > .workspace-state-version
    if [ "\${MUTATE_LOCKFILE_ON_SYNC:-0}" = 1 ]; then
      printf '%s\\n' '# synthetic drift' >> pnpm-lock.yaml
    fi
    ;;
  *"reference:generate")
    test "$(cat .workspace-state-version)" = "$release_version"
    printf 'cli %s\\n' "$release_version" > apps/docs/content/docs/reference/cli-generated.mdx
    printf 'mcp %s\\n' "$release_version" > apps/docs/content/docs/reference/mcp-schemas-generated.mdx
    ;;
  *"reference:check")
    if [ "\${FAIL_REFERENCE_CHECK:-0}" = 1 ]; then
      echo 'synthetic reference check failure' >&2
      exit 42
    fi
    test "$(cat apps/docs/content/docs/reference/cli-generated.mdx)" = "cli $release_version"
    test "$(cat apps/docs/content/docs/reference/mcp-schemas-generated.mdx)" = "mcp $release_version"
    ;;
esac
`, 0o755);
  for (const command of ["node", "npm"]) {
    write(path.join(bin, command), `#!/bin/sh\nset -eu\nprintf '${command} %s\\n' "$*" >> "$COMMAND_LOG"\n`, 0o755);
  }
  return { root, bin, commandLog };
}

function runCase(name, extraEnv = {}, releaseSource = release) {
  const fixture = setupCase(name, releaseSource);
  const result = spawnSync("sh", ["scripts/release.sh", version, "contract test"], {
    cwd: fixture.root,
    encoding: "utf8",
    env: {
      ...process.env,
      ...extraEnv,
      PATH: `${fixture.bin}:${process.env.PATH}`,
      COMMAND_LOG: fixture.commandLog,
      PLANR_RELEASE_EVAL_RECEIPT: path.join(fixture.root, "receipt.json"),
      PLANR_RELEASE_EVAL_SUITE: path.join(fixture.root, "suite.json"),
      PLANR_RELEASE_EVAL_DB: path.join(fixture.root, "eval.sqlite"),
    },
  });
  return { ...fixture, result, calls: fs.readFileSync(fixture.commandLog, "utf8").trim().split("\n").filter(Boolean) };
}

const buildIndex = release.indexOf("cargo build --quiet");
const generateIndex = release.indexOf(referenceGenerateCommand);
const checkIndex = release.indexOf("pnpm --filter @planr/docs reference:check");
const gateIndex = release.indexOf("node scripts/verify-release-eval-receipt.mjs");
const addIndex = release.indexOf("git add ");
assert.ok(buildIndex < generateIndex, "reference generation must follow the bumped candidate build");
assert.ok(generateIndex < checkIndex, "strict reference verification must follow generation");
assert.ok(checkIndex < gateIndex, "reference verification must precede the local eval gate");
assert.ok(gateIndex < addIndex, "all release gates must precede Git staging");
assertWorkspaceSyncContract(release);
assertReferenceCheckBeforeEveryLaterCommand(release);

const missingSync = release.replace(`${workspaceSyncCommand}\n`, "");
assert.throws(
  () => assertWorkspaceSyncContract(missingSync),
  /release script must contain command line: pnpm install --frozen-lockfile/u,
  "removing workspace synchronization must fail the release contract before execution",
);
const reorderedSync = release.replace(`${workspaceSyncCommand}\n`, "").replace(
  `${referenceGenerateCommand}\n`,
  `${referenceGenerateCommand}\n${workspaceSyncCommand}\n`,
);
assert.throws(
  () => assertWorkspaceSyncContract(reorderedSync),
  /lockfile byte verification must follow workspace synchronization/u,
  "moving workspace synchronization after reference generation must fail the release contract before execution",
);
for (const [label, marker] of laterReleaseCommands) {
  const reordered = moveCommandBeforeReferenceCheck(release, marker);
  assert.ok(
    commandIndex(reordered, marker) < commandIndex(reordered, referenceCheckCommand),
    `seeded mutation must move the actual ${label} command before reference:check`,
  );
  assert.throws(
    () => assertReferenceCheckBeforeEveryLaterCommand(reordered),
    new RegExp(`reference:check must precede ${label.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")}`, "u"),
    `seeded ${label} reorder must fail the release contract`,
  );
}
assert.equal(
  docsPackage.scripts["reference:generate"],
  "node scripts/generate-cli-reference.mjs && node scripts/generate-mcp-reference.mjs",
  "release generation must own exactly both public generated references",
);
assert.equal(
  docsPackage.scripts["reference:check"],
  "node scripts/generate-cli-reference.mjs --check && node scripts/generate-mcp-reference.mjs --check",
  "release verification must strictly check both public generated references",
);

const passing = runCase("passing");
assert.equal(passing.result.status, 0, `release contract fixture must pass: ${passing.result.stderr}`);
const expectedAdd = "git add Cargo.toml Cargo.lock package.json plugins/planr/.codex-plugin/plugin.json plugins/planr/.claude-plugin/plugin.json .cursor-plugin/plugin.json apps/docs/content/docs/reference/cli-generated.mdx apps/docs/content/docs/reference/mcp-schemas-generated.mdx";
assert.equal(passing.calls.filter((call) => call.startsWith("git add ")).length, 1, "release must stage exactly once");
assert.ok(passing.calls.includes(expectedAdd), "release must stage the six version files and exactly both generated references");
const passingCheckIndex = passing.calls.findIndex((call) => call.endsWith("reference:check"));
assert.ok(passingCheckIndex >= 0, "passing release must execute strict reference verification");
const passingSyncIndex = passing.calls.indexOf(workspaceSyncCommand);
const passingInvariantIndex = passing.calls.indexOf("git diff --quiet -- pnpm-lock.yaml");
const passingBuildIndex = passing.calls.indexOf("cargo build --quiet");
const passingGenerateIndex = passing.calls.indexOf(referenceGenerateCommand);
assert.ok(passingSyncIndex >= 0, "passing release must execute frozen workspace synchronization");
assert.ok(passingSyncIndex < passingInvariantIndex, "workspace synchronization must execute before lockfile verification");
assert.ok(passingInvariantIndex < passingBuildIndex, "lockfile verification must execute before the candidate build");
assert.ok(passingBuildIndex < passingGenerateIndex, "candidate build must execute before reference generation");
assert.equal(
  fs.readFileSync(path.join(passing.root, "pnpm-lock.yaml"), "utf8"),
  fs.readFileSync(path.join(passing.root, "pnpm-lock.baseline"), "utf8"),
  "frozen workspace synchronization must preserve every lockfile byte",
);
const laterCallPrefixes = [
  "node scripts/verify-release-eval-receipt.mjs --receipt",
  "cargo test",
  "npm run verify:release-eval-gate",
  "npm pack --dry-run",
  "security-local",
  expectedAdd,
  `git commit -m release ${version}: contract test`,
  `git tag -a v${version} -m planr v${version}: contract test`,
  `git push origin HEAD v${version}`,
];
for (const prefix of laterCallPrefixes) {
  const laterIndex = passing.calls.findIndex((call) => call.startsWith(prefix));
  assert.ok(laterIndex > passingCheckIndex, `reference:check must execute before ${prefix}`);
}
for (const [before, after] of [
  ["cargo build --quiet", "pnpm --filter @planr/docs reference:generate"],
  ["pnpm --filter @planr/docs reference:generate", "pnpm --filter @planr/docs reference:check"],
  ["pnpm --filter @planr/docs reference:check", "node scripts/verify-release-eval-receipt.mjs --receipt"],
  ["security-local", expectedAdd],
  [expectedAdd, `git commit -m release ${version}: contract test`],
  [`git commit -m release ${version}: contract test`, `git tag -a v${version} -m planr v${version}: contract test`],
  [`git tag -a v${version} -m planr v${version}: contract test`, `git push origin HEAD v${version}`],
]) {
  const beforeIndex = passing.calls.findIndex((call) => call.startsWith(before));
  const afterIndex = passing.calls.findIndex((call) => call.startsWith(after));
  assert.ok(beforeIndex >= 0 && beforeIndex < afterIndex, `${before} must precede ${after}`);
}
assert.equal(fs.readFileSync(path.join(passing.root, "apps/docs/content/docs/reference/cli-generated.mdx"), "utf8"), `cli ${version}\n`);
assert.equal(fs.readFileSync(path.join(passing.root, "apps/docs/content/docs/reference/mcp-schemas-generated.mdx"), "utf8"), `mcp ${version}\n`);

const failing = runCase("failing-reference-check", { FAIL_REFERENCE_CHECK: "1" });
assert.equal(failing.result.status, 42, "reference drift must fail closed with the checker status");
assert.ok(failing.calls.some((call) => call.endsWith("reference:check")), "failing checker must execute");
for (const prefix of laterCallPrefixes) {
  assert.ok(!failing.calls.some((call) => call.startsWith(prefix)), `reference failure must stop before ${prefix}`);
}

const failingSync = runCase("failing-workspace-sync", { FAIL_WORKSPACE_SYNC: "1" });
assert.equal(failingSync.result.status, 43, "workspace synchronization failure must preserve the pnpm status");
assert.ok(failingSync.calls.includes(workspaceSyncCommand), "failing workspace synchronization must execute");
for (const prefix of ["git diff --quiet -- pnpm-lock.yaml", "cargo build --quiet", referenceGenerateCommand, ...laterCallPrefixes]) {
  assert.ok(!failingSync.calls.some((call) => call.startsWith(prefix)), `workspace sync failure must stop before ${prefix}`);
}

const driftingSync = runCase("drifting-workspace-sync", { MUTATE_LOCKFILE_ON_SYNC: "1" });
assert.equal(driftingSync.result.status, 1, "workspace synchronization lockfile drift must fail closed");
assert.ok(driftingSync.calls.includes(workspaceSyncCommand), "drifting workspace synchronization must execute");
assert.ok(driftingSync.calls.includes("git diff --quiet -- pnpm-lock.yaml"), "drifting workspace synchronization must verify lockfile bytes");
for (const prefix of ["cargo build --quiet", referenceGenerateCommand, ...laterCallPrefixes]) {
  assert.ok(!driftingSync.calls.some((call) => call.startsWith(prefix)), `lockfile drift must stop before ${prefix}`);
}

fs.rmSync(tmp, { recursive: true, force: true });
console.log(JSON.stringify({
  verdict: "pass",
  version_transition_owner: "scripts/release.sh",
  order: ["bump", "frozen workspace sync", "lockfile byte check", "build", "generate", "check", "gates", "stage", "commit", "tag", "push"],
  staged_version_files: 6,
  staged_generated_references: 2,
  later_commands_guarded: laterReleaseCommands.length,
  seeded_reorder_cases: laterReleaseCommands.length,
  workspace_sync_adversarial_cases: ["missing", "reordered", "failed", "lockfile drift"],
  lockfile_byte_invariant: true,
  reference_failure_before_git_mutation: true,
}, null, 2));
