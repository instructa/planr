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

function write(file, content, mode) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, content);
  if (mode) fs.chmodSync(file, mode);
}

function setupCase(name) {
  const root = path.join(tmp, name);
  const bin = path.join(root, "test-bin");
  const commandLog = path.join(root, "commands.log");
  fs.mkdirSync(bin, { recursive: true });
  write(path.join(root, "scripts/release.sh"), release, 0o755);
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
  write(path.join(root, "apps/docs/content/docs/reference/cli-generated.mdx"), "cli 1.7.1\n");
  write(path.join(root, "apps/docs/content/docs/reference/mcp-schemas-generated.mdx"), "mcp 1.7.1\n");
  write(commandLog, "");

  write(path.join(bin, "git"), `#!/bin/sh
set -eu
case "$1 \${2:-}" in
  "rev-parse --abbrev-ref") echo main ;;
  "status --porcelain") ;;
  "rev-parse v${version}") exit 1 ;;
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
  *"reference:generate")
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

function runCase(name, extraEnv = {}) {
  const fixture = setupCase(name);
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
const generateIndex = release.indexOf("pnpm --filter @planr/docs reference:generate");
const checkIndex = release.indexOf("pnpm --filter @planr/docs reference:check");
const gateIndex = release.indexOf("node scripts/verify-release-eval-receipt.mjs");
const addIndex = release.indexOf("git add ");
assert.ok(buildIndex < generateIndex, "reference generation must follow the bumped candidate build");
assert.ok(generateIndex < checkIndex, "strict reference verification must follow generation");
assert.ok(checkIndex < gateIndex, "reference verification must precede the local eval gate");
assert.ok(gateIndex < addIndex, "all release gates must precede Git staging");
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
assert.ok(!failing.calls.some((call) => /^(git add|git commit|git tag|git push) /u.test(call)), "reference failure must precede every Git mutation");
assert.ok(!failing.calls.some((call) => call.startsWith("node scripts/verify-release-eval-receipt.mjs")), "reference failure must stop before later release gates");

fs.rmSync(tmp, { recursive: true, force: true });
console.log(JSON.stringify({
  verdict: "pass",
  version_transition_owner: "scripts/release.sh",
  order: ["bump", "build", "generate", "check", "gates", "stage", "commit", "tag", "push"],
  staged_version_files: 6,
  staged_generated_references: 2,
  reference_failure_before_git_mutation: true,
}, null, 2));
