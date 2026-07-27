#!/usr/bin/env node
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readlinkSync,
  readdirSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { classifyChanges, parseGitNameStatus, POLICY_DIGEST, POLICY_VERSION } from "./verification-policy.mjs";

export const RECEIPT_SCHEMA = "planr.verification-receipt.v2";
export const LINUX_TARGET_RECEIPT_SCHEMA = "planr.linux-target-receipt.v1";
export const RUNNER_VERSION = "1.1.0";

const LINUX_TARGETS = deepFreeze({
  "linux-x86_64": { cargoTarget: "x86_64-unknown-linux-musl", hostArchitecture: "x64" },
  "linux-arm64": { cargoTarget: "aarch64-unknown-linux-musl", hostArchitecture: "arm64" },
});

const GATE_COMMANDS = deepFreeze({
  "docs-content": [["pnpm", "--filter", "@planr/docs", "content"]],
  "docs-typecheck": [["pnpm", "--filter", "@planr/docs", "typecheck"]],
  "docs-lint": [["pnpm", "--filter", "@planr/docs", "lint"]],
  "docs-build": [["pnpm", "--filter", "@planr/docs", "build"]],
  "docs-artifact": [["pnpm", "--filter", "@planr/docs", "verify:artifact"]],
  "rust-fmt": [["cargo", "fmt", "--all", "--", "--check"]],
  "rust-clippy": [["cargo", "clippy", "--all-targets", "--all-features", "--", "-D", "warnings"]],
  "rust-test": [["cargo", "test", "--all-features"]],
  "generated-reference": [["pnpm", "--filter", "@planr/docs", "reference:check"]],
  "github-actions": [["npm", "run", "verify:github-actions"]],
  "release-contract": [
    ["npm", "run", "verify:release-script"],
    ["npm", "run", "pack:check"],
  ],
  security: [["npm", "run", "security:check"]],
  "linux-portability": [[
    "sh",
    "-c",
    "test \"$(find dist -maxdepth 1 -name 'planr-linux-*.tar.gz' -type f | wc -l)\" -eq 2 && cd dist && sha256sum planr-linux-arm64.tar.gz planr-linux-x86_64.tar.gz > SHA256SUMS && sha256sum -c SHA256SUMS",
  ]],
  "release-evaluation": [["npm", "run", "verify:release-eval-gate"]],
});

const LINUX_ARTIFACTS = Object.freeze([
  "dist/planr-linux-x86_64.tar.gz",
  "dist/planr-linux-arm64.tar.gz",
  "dist/SHA256SUMS",
]);
const DEFAULT_ARTIFACTS = deepFreeze({
  "docs-artifact": ["apps/docs/out"],
  "linux-portability": LINUX_ARTIFACTS,
});
const RECEIPT_OUTPUT_DIRECTORY = ".planr/receipts";
const TOP_LEVEL_KEYS = [
  "artifacts", "changedFiles", "commands", "durationMs", "finishedAt", "linuxTargets", "policy", "runner",
  "schemaVersion", "selection", "source", "startedAt", "verdict",
];
const FORBIDDEN_KEY = /(^|_)(?:authorization|completion|credential|environment|env|password|prompt|secret|session|token)(_|$)/iu;
const SECRET_VALUE = /(?:-----BEGIN [A-Z ]*PRIVATE KEY-----|(?:api|access|auth|secret)[_-]?key\s*[=:]|(?:gh[oprsu]|sk|xai)-[A-Za-z0-9_-]{16,})/iu;
const PRIVATE_PATH = /(?:^|\/)(?:\.codex|\.claude|\.cursor|\.planr\/(?:eval|private)|examples\/eval)(?:\/|$)/u;

export function commandPlanFor(selection) {
  assertSelection(selection);
  const plan = [];
  const seen = new Set();
  for (const gate of selection.selectedGates) {
    const commands = GATE_COMMANDS[gate];
    if (!commands) throw new Error(`unknown verification gate: ${gate}`);
    for (const [executable, ...args] of commands) {
      const identity = JSON.stringify([executable, args]);
      if (seen.has(identity)) continue;
      seen.add(identity);
      plan.push({ gate, executable, args });
    }
  }
  return plan;
}

export function linuxTargetCommandPlan(target) {
  const config = LINUX_TARGETS[target];
  if (!config) throw new Error(`unknown Linux release target: ${target}`);
  const bindings = [`PLANR_TARGET=${target}`, `PLANR_CARGO_TARGET=${config.cargoTarget}`];
  return [
    { gate: "linux-portability", executable: "env", args: [...bindings, "sh", "scripts/build-linux-release.sh"] },
    { gate: "linux-portability", executable: "env", args: [...bindings, "sh", "scripts/verify-linux-release-artifact.sh"] },
  ];
}

export function runLinuxTargetVerification({
  selection,
  target,
  repoRoot = process.cwd(),
  execute = executeCommand,
  host = { platform: process.platform, architecture: process.arch },
  now = () => new Date(),
  monotonicNow = () => performance.now(),
}) {
  assertSelection(selection);
  const config = assertCompatibleLinuxHost(target, host);
  const root = canonicalRoot(repoRoot);
  const source = currentSourceIdentity(root, selection, { allowedDirtyPaths: LINUX_ARTIFACTS });
  const startedAt = now().toISOString();
  const startedTick = monotonicNow();
  const commands = executePlan(linuxTargetCommandPlan(target), execute, root, monotonicNow);
  const artifact = artifactRecord(root, "linux-portability", `dist/planr-${target}.tar.gz`);
  const sourceAfter = currentSourceIdentity(root, selection, { allowedDirtyPaths: LINUX_ARTIFACTS });
  assertEqual(sourceAfter.revision, source.revision, "source revision changed during Linux target verification");
  assertEqual(sourceAfter.stateDigest, source.stateDigest, "source inputs changed during Linux target verification");
  const receipt = {
    schemaVersion: LINUX_TARGET_RECEIPT_SCHEMA,
    runner: { version: RUNNER_VERSION, digest: runnerDigest() },
    source,
    target,
    cargoTarget: config.cargoTarget,
    host,
    commands,
    artifact,
    startedAt,
    finishedAt: now().toISOString(),
    durationMs: elapsedMilliseconds(startedTick, monotonicNow()),
    verdict: commands.every(({ status }) => status === "passed") && artifact.present ? "pass" : "fail",
  };
  validateLinuxTargetReceipt(receipt, { root, selection });
  return receipt;
}

export function verifyLinuxTargetReceipt(receipt, { selection, repoRoot = process.cwd() } = {}) {
  assertSelection(selection);
  const root = canonicalRoot(repoRoot);
  validateLinuxTargetReceipt(receipt, { root, selection });
  return { verdict: "pass", target: receipt.target, sourceRevision: receipt.source.revision, artifactDigest: receipt.artifact.digest };
}

export function receiptDigest(receipt) {
  validateReceiptShape(receipt);
  return digest(receipt);
}

export function runVerification({
  selection,
  repoRoot = process.cwd(),
  receiptPath,
  artifactPaths = DEFAULT_ARTIFACTS,
  linuxTargetReceipts = [],
  execute = executeCommand,
  now = () => new Date(),
  monotonicNow = () => performance.now(),
}) {
  assertSelection(selection);
  const root = canonicalRoot(repoRoot);
  const startedAt = now().toISOString();
  const startedTick = monotonicNow();
  const outputPaths = selection.selectedGates.flatMap((gate) => artifactPaths[gate] ?? []);
  const source = currentSourceIdentity(root, selection, { allowedDirtyPaths: outputPaths });
  const commandResults = [];
  let priorFailure = false;

  const linuxTargets = selection.selectedGates.includes("linux-portability")
    ? validateLinuxTargetReceipts(linuxTargetReceipts, { root, selection, source })
    : [];
  commandResults.push(...executePlan(commandPlanFor(selection), execute, root, monotonicNow));
  priorFailure ||= commandResults.some(({ status }) => status !== "passed");

  const artifacts = [];
  for (const gate of selection.selectedGates) {
    for (const artifactPath of artifactPaths[gate] ?? []) {
      const record = artifactRecord(root, gate, artifactPath);
      artifacts.push(record);
      if (!record.present) priorFailure = true;
    }
  }

  const sourceAfterExecution = currentSourceIdentity(root, selection, { allowedDirtyPaths: outputPaths });
  assertEqual(sourceAfterExecution.revision, source.revision, "source revision changed during verification");
  assertEqual(sourceAfterExecution.stateDigest, source.stateDigest, "source inputs changed during verification");

  const finishedAt = now().toISOString();
  const durationMs = elapsedMilliseconds(startedTick, monotonicNow());
  const receipt = {
    schemaVersion: RECEIPT_SCHEMA,
    runner: { version: RUNNER_VERSION, digest: runnerDigest() },
    policy: { version: selection.policyVersion, digest: selection.policyDigest },
    source,
    changedFiles: { digest: selection.changedFilesDigest, changes: selection.changes },
    selection: {
      profile: selection.profile,
      escalatedToFull: selection.escalatedToFull,
      matchedPathClasses: selection.matchedPathClasses,
      selectedGates: selection.selectedGates,
    },
    commands: commandResults,
    artifacts,
    linuxTargets,
    startedAt,
    finishedAt,
    durationMs,
    verdict: priorFailure ? "fail" : "pass",
  };
  validateReceiptShape(receipt);
  if (receiptPath) writeReceipt(root, receiptPath, receipt);
  return receipt;
}

export function verifyReceipt(receipt, { selection, repoRoot = process.cwd(), artifactPaths = DEFAULT_ARTIFACTS, receiptPath } = {}) {
  validateReceiptShape(receipt);
  assertSelection(selection);
  const root = canonicalRoot(repoRoot);
  if (receiptPath) assertReceiptPathBinding(root, receiptPath, receipt);
  const allowedDirtyPaths = [
    ...selection.selectedGates.flatMap((gate) => artifactPaths[gate] ?? []),
    ...(receiptPath ? [receiptPath] : []),
  ];
  const expectedSource = currentSourceIdentity(root, selection, { allowedDirtyPaths });
  assertEqual(receipt.policy.version, POLICY_VERSION, "receipt policy version is stale");
  assertEqual(receipt.policy.digest, POLICY_DIGEST, "receipt policy digest is stale");
  assertEqual(receipt.policy.version, selection.policyVersion, "selection policy version mismatch");
  assertEqual(receipt.policy.digest, selection.policyDigest, "selection policy digest mismatch");
  assertEqual(receipt.source.revision, expectedSource.revision, "receipt source revision is stale");
  assertEqual(receipt.source.stateDigest, expectedSource.stateDigest, "receipt source inputs were altered");
  assertEqual(receipt.changedFiles.digest, selection.changedFilesDigest, "changed-file set digest mismatch");
  assertEqual(canonicalJson(receipt.changedFiles.changes), canonicalJson(selection.changes), "changed-file set mismatch");
  assertEqual(canonicalJson(receipt.selection.selectedGates), canonicalJson(selection.selectedGates), "required gate set mismatch");
  assertEqual(receipt.selection.profile, selection.profile, "verification profile mismatch");
  assertEqual(receipt.selection.escalatedToFull, selection.escalatedToFull, "selection escalation mismatch");
  assertEqual(canonicalJson(receipt.selection.matchedPathClasses), canonicalJson(selection.matchedPathClasses), "matched path classes mismatch");
  assertEqual(receipt.runner.version, RUNNER_VERSION, "runner version mismatch");
  assertEqual(receipt.runner.digest, runnerDigest(), "runner implementation changed");
  assertEqual(receipt.verdict, "pass", "verification receipt is not green");
  const expectedLinuxTargets = selection.selectedGates.includes("linux-portability")
    ? validateLinuxTargetReceipts(receipt.linuxTargets, { root, selection, source: receipt.source })
    : [];
  assertEqual(canonicalJson(receipt.linuxTargets), canonicalJson(expectedLinuxTargets), "Linux target receipt set mismatch");

  const expectedCommands = commandPlanFor(selection);
  assertEqual(receipt.commands.length, expectedCommands.length, "receipt command count mismatch");
  for (const [index, expected] of expectedCommands.entries()) {
    const actual = receipt.commands[index];
    assertEqual(canonicalJson(pick(actual, ["gate", "executable", "args"])), canonicalJson(expected), `command ${index} mismatch`);
    assertEqual(actual.status, "passed", `command ${index} did not pass`);
    assertEqual(actual.exitCode, 0, `command ${index} exit code is not zero`);
  }
  const expectedArtifacts = selection.selectedGates.flatMap((gate) => (artifactPaths[gate] ?? []).map((artifactPath) => ({ gate, path: artifactPath })));
  assertEqual(
    canonicalJson(receipt.artifacts.map(({ gate, path: artifactPath }) => ({ gate, path: artifactPath }))),
    canonicalJson(expectedArtifacts),
    "receipt artifact set mismatch",
  );
  for (const artifact of receipt.artifacts) {
    const current = artifactRecord(root, artifact.gate, artifact.path);
    assertEqual(current.present, true, `artifact is missing: ${artifact.path}`);
    assertEqual(current.digest, artifact.digest, `artifact changed: ${artifact.path}`);
    assertEqual(current.files, artifact.files, `artifact file count changed: ${artifact.path}`);
    assertEqual(current.bytes, artifact.bytes, `artifact byte count changed: ${artifact.path}`);
  }
  return { verdict: "pass", sourceRevision: receipt.source.revision, gates: receipt.selection.selectedGates.length };
}

function executePlan(plan, execute, root, monotonicNow) {
  const results = [];
  let priorFailure = false;
  for (const command of plan) {
    if (priorFailure) {
      results.push({ ...command, durationMs: null, exitCode: null, status: "not_run_after_failure" });
      continue;
    }
    const commandStarted = monotonicNow();
    const result = execute(command.executable, command.args, { cwd: root });
    const durationMs = elapsedMilliseconds(commandStarted, monotonicNow());
    const exitCode = Number.isInteger(result?.status) ? result.status : 1;
    const status = exitCode === 0 ? "passed" : "failed";
    results.push({ ...command, durationMs, exitCode, status });
    priorFailure ||= exitCode !== 0;
  }
  return results;
}

function assertCompatibleLinuxHost(target, host) {
  const config = LINUX_TARGETS[target];
  if (!config) throw new Error(`unknown Linux release target: ${target}`);
  if (host?.platform !== "linux" || host?.architecture !== config.hostArchitecture) {
    throw new Error(`${target} requires native linux/${config.hostArchitecture}, received ${host?.platform ?? "unknown"}/${host?.architecture ?? "unknown"}`);
  }
  return config;
}

function validateLinuxTargetReceipts(receipts, { root, selection, source }) {
  if (!Array.isArray(receipts) || receipts.length !== Object.keys(LINUX_TARGETS).length) {
    throw new Error("both independent Linux target receipts are required");
  }
  const byTarget = new Map(receipts.map((receipt) => [receipt?.target, receipt]));
  if (byTarget.size !== receipts.length) throw new Error("Linux target receipts must be unique");
  return Object.keys(LINUX_TARGETS).map((target) => {
    const receipt = byTarget.get(target);
    if (!receipt) throw new Error(`missing Linux target receipt: ${target}`);
    validateLinuxTargetReceipt(receipt, { root, selection });
    assertEqual(receipt.source.revision, source.revision, `${target} source revision mismatch`);
    assertEqual(receipt.source.stateDigest, source.stateDigest, `${target} source inputs mismatch`);
    return receipt;
  });
}

function validateLinuxTargetReceipt(receipt, { root, selection }) {
  assertKeys(receipt, [
    "artifact", "cargoTarget", "commands", "durationMs", "finishedAt", "host", "runner", "schemaVersion",
    "source", "startedAt", "target", "verdict",
  ], "Linux target receipt");
  if (receipt.schemaVersion !== LINUX_TARGET_RECEIPT_SCHEMA) throw new Error("unsupported Linux target receipt schema");
  const config = assertCompatibleLinuxHost(receipt.target, receipt.host);
  assertEqual(receipt.cargoTarget, config.cargoTarget, `${receipt.target} cargo target mismatch`);
  assertKeys(receipt.host, ["architecture", "platform"], "Linux target host");
  assertKeys(receipt.runner, ["digest", "version"], "Linux target runner");
  assertKeys(receipt.source, ["revision", "stateDigest"], "Linux target source");
  assertKeys(receipt.artifact, ["bytes", "digest", "files", "gate", "path", "present"], "Linux target artifact");
  assertEqual(receipt.runner.version, RUNNER_VERSION, "Linux target runner version mismatch");
  assertEqual(receipt.runner.digest, runnerDigest(), "Linux target runner implementation changed");
  assertEqual(receipt.verdict, "pass", `${receipt.target} receipt is not green`);
  const expectedSource = currentSourceIdentity(root, selection, { allowedDirtyPaths: LINUX_ARTIFACTS });
  assertEqual(receipt.source.revision, expectedSource.revision, `${receipt.target} source revision is stale`);
  assertEqual(receipt.source.stateDigest, expectedSource.stateDigest, `${receipt.target} source inputs were altered`);
  const expectedCommands = linuxTargetCommandPlan(receipt.target);
  assertEqual(receipt.commands.length, expectedCommands.length, `${receipt.target} command count mismatch`);
  for (const [index, expected] of expectedCommands.entries()) {
    const actual = receipt.commands[index];
    assertKeys(actual, ["args", "durationMs", "executable", "exitCode", "gate", "status"], `Linux target commands[${index}]`);
    assertEqual(canonicalJson(pick(actual, ["gate", "executable", "args"])), canonicalJson(expected), `${receipt.target} command ${index} mismatch`);
    assertEqual(actual.status, "passed", `${receipt.target} command ${index} did not pass`);
    assertEqual(actual.exitCode, 0, `${receipt.target} command ${index} exit code is not zero`);
  }
  const expectedPath = `dist/planr-${receipt.target}.tar.gz`;
  assertEqual(receipt.artifact.path, expectedPath, `${receipt.target} artifact path mismatch`);
  const currentArtifact = artifactRecord(root, "linux-portability", expectedPath);
  assertEqual(currentArtifact.present, true, `artifact is missing: ${expectedPath}`);
  assertEqual(canonicalJson(receipt.artifact), canonicalJson(currentArtifact), `${receipt.target} artifact changed`);
  scanUnsafe(receipt, ["linuxTarget"]);
}

function currentSourceIdentity(root, selection, { allowedDirtyPaths = [] } = {}) {
  const requestedRevision = git(root, ["rev-parse", "--verify", `${selection.headRevision ?? "HEAD"}^{commit}`]).trim();
  const revision = git(root, ["rev-parse", "--verify", "HEAD^{commit}"]).trim();
  if (!/^[0-9a-f]{40,64}$/u.test(revision)) throw new Error("current source revision is not a commit SHA");
  if (requestedRevision !== revision) {
    throw new Error(`selected source revision ${requestedRevision} is not checked out at HEAD ${revision}`);
  }
  const exclusions = allowedDirtyPaths.map((relativePath) => {
    safeRepositoryPath(root, relativePath);
    return `:(literal,exclude)${relativePath}`;
  });
  const worktreeState = git(root, ["status", "--porcelain=v1", "-z", "--untracked-files=all", "--", ".", ...exclusions]);
  if (worktreeState.length > 0) {
    throw new Error("source worktree must be clean, including untracked files");
  }
  const tree = git(root, ["rev-parse", "--verify", "HEAD^{tree}"]).trim();
  const inputs = [];
  for (const change of selection.changes) {
    for (const relativePath of change.paths) {
      inputs.push(sourcePathRecord(root, relativePath));
    }
  }
  return { revision, stateDigest: digest({ tree, inputs }) };
}

function sourcePathRecord(root, relativePath) {
  const resolved = safeRepositoryPath(root, relativePath);
  if (!existsSync(resolved)) return { path: relativePath, kind: "missing", digest: null };
  const stat = lstatSync(resolved);
  if (stat.isSymbolicLink()) return { path: relativePath, kind: "symlink", digest: digest(readlinkSync(resolved)) };
  if (!stat.isFile()) throw new Error(`changed input is not a regular file: ${relativePath}`);
  return { path: relativePath, kind: "file", digest: digest(readFileSync(resolved)) };
}

function artifactRecord(root, gate, relativePath) {
  const resolved = safeRepositoryPath(root, relativePath);
  if (!existsSync(resolved)) return { gate, path: relativePath, present: false, digest: null, files: 0, bytes: 0 };
  const records = [];
  walkArtifact(resolved, relativePath, records);
  if (records.length === 0) return { gate, path: relativePath, present: false, digest: null, files: 0, bytes: 0 };
  return {
    gate,
    path: relativePath,
    present: true,
    digest: digest(records),
    files: records.length,
    bytes: records.reduce((sum, record) => sum + record.bytes, 0),
  };
}

function walkArtifact(absolutePath, relativePath, records) {
  const stat = lstatSync(absolutePath);
  if (stat.isSymbolicLink()) throw new Error(`artifact contains a symlink: ${relativePath}`);
  if (stat.isFile()) {
    records.push({ path: relativePath, bytes: stat.size, digest: digest(readFileSync(absolutePath)) });
    return;
  }
  if (!stat.isDirectory()) throw new Error(`artifact contains an unsupported entry: ${relativePath}`);
  for (const name of readdirSync(absolutePath).sort()) {
    walkArtifact(path.join(absolutePath, name), `${relativePath}/${name}`, records);
  }
}

function validateReceiptShape(receipt) {
  if (!receipt || typeof receipt !== "object" || Array.isArray(receipt)) throw new Error("receipt must be an object");
  assertKeys(receipt, TOP_LEVEL_KEYS, "receipt");
  assertKeys(receipt.runner, ["digest", "version"], "runner");
  assertKeys(receipt.policy, ["digest", "version"], "policy");
  assertKeys(receipt.source, ["revision", "stateDigest"], "source");
  assertKeys(receipt.changedFiles, ["changes", "digest"], "changedFiles");
  assertKeys(receipt.selection, ["escalatedToFull", "matchedPathClasses", "profile", "selectedGates"], "selection");
  requiredArray(receipt.linuxTargets, "linuxTargets");
  for (const [index, command] of requiredArray(receipt.commands, "commands").entries()) {
    assertKeys(command, ["args", "durationMs", "executable", "exitCode", "gate", "status"], `commands[${index}]`);
  }
  for (const [index, artifact] of requiredArray(receipt.artifacts, "artifacts").entries()) {
    assertKeys(artifact, ["bytes", "digest", "files", "gate", "path", "present"], `artifacts[${index}]`);
  }
  if (receipt.schemaVersion !== RECEIPT_SCHEMA) throw new Error("unsupported receipt schema");
  scanUnsafe(receipt, []);
}

function scanUnsafe(value, pathParts) {
  if (Array.isArray(value)) {
    value.forEach((child, index) => scanUnsafe(child, [...pathParts, String(index)]));
    return;
  }
  if (value && typeof value === "object") {
    for (const [key, child] of Object.entries(value)) {
      if (FORBIDDEN_KEY.test(key)) throw new Error(`forbidden receipt field: ${[...pathParts, key].join(".")}`);
      scanUnsafe(child, [...pathParts, key]);
    }
    return;
  }
  if (typeof value !== "string") return;
  if (value.includes("\0") || /[\r\n]/u.test(value)) throw new Error(`unsafe receipt string: ${pathParts.join(".")}`);
  if (SECRET_VALUE.test(value)) throw new Error(`secret-like receipt value: ${pathParts.join(".")}`);
  if (path.isAbsolute(value) || PRIVATE_PATH.test(value)) throw new Error(`private path in receipt: ${pathParts.join(".")}`);
}

function assertSelection(selection) {
  if (!selection || typeof selection !== "object") throw new Error("verification selection is required");
  if (selection.policyVersion !== POLICY_VERSION || selection.policyDigest !== POLICY_DIGEST) throw new Error("selection policy is stale");
  if (!Array.isArray(selection.changes) || !Array.isArray(selection.selectedGates)) throw new Error("selection is incomplete");
  for (const gate of selection.selectedGates) if (!GATE_COMMANDS[gate]) throw new Error(`unknown verification gate: ${gate}`);
}

function safeRepositoryPath(root, relativePath) {
  if (typeof relativePath !== "string" || relativePath.length === 0 || path.isAbsolute(relativePath) || relativePath.includes("\\")) {
    throw new Error("receipt paths must be repository-relative");
  }
  const resolved = path.resolve(root, relativePath);
  if (resolved !== root && !resolved.startsWith(`${root}${path.sep}`)) throw new Error("receipt path escapes repository");
  return resolved;
}

function safeReceiptPath(root, relativePath, { mustExist = false } = {}) {
  const resolved = safeRepositoryPath(root, relativePath);
  const segments = relativePath.split("/");
  if (
    segments.length !== 3
    || segments[0] !== ".planr"
    || segments[1] !== "receipts"
    || !/^[A-Za-z0-9][A-Za-z0-9._-]*\.json$/u.test(segments[2])
  ) {
    throw new Error(`receipt path must be a JSON file directly under ${RECEIPT_OUTPUT_DIRECTORY}`);
  }
  for (const candidate of [path.join(root, ".planr"), path.join(root, RECEIPT_OUTPUT_DIRECTORY), resolved]) {
    if (existsSync(candidate) && lstatSync(candidate).isSymbolicLink()) {
      throw new Error("receipt path must not contain symbolic-link aliases");
    }
  }
  const tracked = spawnSync("git", ["ls-files", "--error-unmatch", "--", relativePath], {
    cwd: root,
    encoding: "utf8",
  });
  if (tracked.status === 0) throw new Error("receipt path must not be a tracked source path");
  if (mustExist && (!existsSync(resolved) || !lstatSync(resolved).isFile())) {
    throw new Error("receipt path must identify an existing regular file");
  }
  return resolved;
}

function assertReceiptPathBinding(root, relativePath, receipt) {
  const resolved = safeReceiptPath(root, relativePath, { mustExist: true });
  let storedReceipt;
  try {
    storedReceipt = JSON.parse(readFileSync(resolved, "utf8"));
  } catch {
    throw new Error("receipt path must contain valid receipt JSON");
  }
  assertEqual(canonicalJson(storedReceipt), canonicalJson(receipt), "receipt path content does not match the verified receipt");
}

function canonicalRoot(repoRoot) {
  const root = realpathSync(repoRoot);
  git(root, ["rev-parse", "--is-inside-work-tree"]);
  return root;
}

function executeCommand(executable, args, options) {
  return spawnSync(executable, args, { ...options, stdio: "inherit", env: process.env });
}

function writeReceipt(root, relativePath, receipt) {
  const output = safeReceiptPath(root, relativePath);
  mkdirSync(path.dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
}

function runnerDigest() {
  return digest(readFileSync(fileURLToPath(import.meta.url)));
}

function git(root, args) {
  const result = spawnSync("git", args, { cwd: root, encoding: "utf8" });
  if (result.status !== 0) throw new Error(`git ${args[0]} failed`);
  return result.stdout;
}

function digest(value) {
  const input = Buffer.isBuffer(value) ? value : Buffer.from(canonicalJson(value));
  return `sha256:${createHash("sha256").update(input).digest("hex")}`;
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function assertKeys(value, keys, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object`);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (canonicalJson(actual) !== canonicalJson(expected)) throw new Error(`${label} fields are not allowlisted`);
}

function requiredArray(value, label) {
  if (!Array.isArray(value)) throw new Error(`${label} must be an array`);
  return value;
}

function assertEqual(actual, expected, message) {
  if (actual !== expected) throw new Error(message);
}

function elapsedMilliseconds(start, end) {
  const elapsed = Math.max(0, end - start);
  return Math.round(elapsed * 1000) / 1000;
}

function pick(value, keys) {
  return Object.fromEntries(keys.map((key) => [key, value[key]]));
}

function deepFreeze(value) {
  if (!value || typeof value !== "object" || Object.isFrozen(value)) return value;
  for (const child of Object.values(value)) deepFreeze(child);
  return Object.freeze(value);
}

function parseArgs(argv) {
  const args = [...argv];
  const command = args.shift();
  const values = new Map();
  while (args.length > 0) {
    const name = args.shift();
    if (!name?.startsWith("--") || args.length === 0) throw new Error(`invalid argument: ${name ?? "<missing>"}`);
    values.set(name, args.shift());
  }
  return { command, values };
}

function selectionFromCli(values, root) {
  const inputPath = values.get("--input");
  let base = values.get("--base");
  const head = values.get("--head") ?? "HEAD";
  const explicitProfile = values.get("--profile");
  let changes;
  if (inputPath) {
    const input = JSON.parse(readFileSync(safeRepositoryPath(root, inputPath), "utf8"));
    changes = Array.isArray(input) ? input : input.changes;
  } else if (base || explicitProfile) {
    base ??= `${head}^`;
    changes = parseGitNameStatus(git(root, ["diff", "--name-status", "-z", "--find-renames", base, head]));
  } else {
    throw new Error("--input, --base, or --profile is required");
  }
  const selection = classifyChanges(changes, { baseRevision: base ?? null, headRevision: head });
  if (explicitProfile && explicitProfile !== selection.profile) {
    throw new Error(`explicit profile ${explicitProfile} does not match classified profile ${selection.profile}`);
  }
  return selection;
}

function main() {
  const { command, values } = parseArgs(process.argv.slice(2));
  const root = canonicalRoot(process.cwd());
  if (command === "run-linux-target") {
    const receiptPath = values.get("--receipt");
    const target = values.get("--target");
    if (!receiptPath || !target) throw new Error("run-linux-target requires --target and --receipt");
    const selection = selectionFromCli(values, root);
    if (!selection.selectedGates.includes("linux-portability")) throw new Error("selection does not require Linux portability");
    const receipt = runLinuxTargetVerification({ selection, target, repoRoot: root });
    writeReceipt(root, receiptPath, receipt);
    process.stdout.write(`${JSON.stringify({ verdict: receipt.verdict, target, receipt: receiptPath, receiptDigest: digest(receipt), sourceRevision: receipt.source.revision })}\n`);
    return;
  }
  if (command === "verify-linux-target") {
    const receiptPath = values.get("--receipt");
    if (!receiptPath) throw new Error("verify-linux-target requires --receipt");
    const selection = selectionFromCli(values, root);
    const receipt = JSON.parse(readFileSync(safeReceiptPath(root, receiptPath, { mustExist: true }), "utf8"));
    process.stdout.write(`${JSON.stringify(verifyLinuxTargetReceipt(receipt, { selection, repoRoot: root }))}\n`);
    return;
  }
  if (command === "run") {
    const receiptPath = values.get("--receipt");
    if (!receiptPath) throw new Error("run requires --receipt");
    const selection = selectionFromCli(values, root);
    const linuxTargetReceipts = selection.selectedGates.includes("linux-portability")
      ? Object.keys(LINUX_TARGETS).map((target) => {
        const targetReceiptPath = values.get(`--${target}-receipt`);
        if (!targetReceiptPath) throw new Error(`run requires --${target}-receipt for Linux portability`);
        return JSON.parse(readFileSync(safeReceiptPath(root, targetReceiptPath, { mustExist: true }), "utf8"));
      })
      : [];
    const receipt = runVerification({ selection, repoRoot: root, receiptPath, linuxTargetReceipts });
    process.stdout.write(`${JSON.stringify({ verdict: receipt.verdict, receipt: receiptPath, receiptDigest: receiptDigest(receipt), sourceRevision: receipt.source.revision })}\n`);
    if (receipt.verdict !== "pass") process.exitCode = 1;
    return;
  }
  if (command === "verify") {
    const receiptPath = values.get("--receipt");
    if (!receiptPath) throw new Error("verify requires --receipt");
    const selection = selectionFromCli(values, root);
    const receipt = JSON.parse(readFileSync(safeReceiptPath(root, receiptPath, { mustExist: true }), "utf8"));
    process.stdout.write(`${JSON.stringify({ ...verifyReceipt(receipt, { selection, repoRoot: root, receiptPath }), receiptDigest: receiptDigest(receipt) })}\n`);
    return;
  }
  throw new Error(`usage: verification-runner.mjs <run|run-linux-target|verify-linux-target|verify> --receipt ${RECEIPT_OUTPUT_DIRECTORY}/NAME.json (--input PATH | --base REV | --profile PROFILE) [--head REV]`);
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}
