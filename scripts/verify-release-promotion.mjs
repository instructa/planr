#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { routeSelection } from "./ci-router.mjs";
import { classifyChanges, parseGitNameStatus, POLICY_DIGEST, POLICY_VERSION } from "./verification-policy.mjs";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const EVALUATED_SUBJECT_PATHS = [
  "plugins/planr/skills/planr-goal/SKILL.md",
  "plugins/planr/skills/planr-loop/SKILL.md",
  "plugins/planr/skills/planr-loop/references/host-dispatch.md",
  "plugins/planr/skills/planr-loop/references/recovery-and-verification.md",
  "plugins/planr/skills/planr-task-graph/SKILL.md",
];
const EVALUATION_POLICY_PATHS = [
  "docs/contracts/EVAL_CONTRACT_V1.md",
  "scripts/test-release-eval-gate.mjs",
  "scripts/verify-release-eval-receipt.mjs",
  "scripts/verify-release-promotion.mjs",
];

function option(name, { required = false } = {}) {
  const index = process.argv.indexOf(name);
  const value = index >= 0 ? process.argv[index + 1] : undefined;
  if (required) assert.ok(value, `missing ${name}`);
  return value;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: repo, encoding: "utf8", ...options });
  assert.equal(result.status, 0, `${command} ${args.join(" ")} failed: ${result.stderr.trim()}`);
  return result.stdout.trim();
}

function jsonFile(input, label) {
  try {
    return JSON.parse(fs.readFileSync(path.resolve(repo, input), "utf8"));
  } catch {
    throw new Error(`${label} is unreadable or invalid JSON`);
  }
}

function exactKeys(value, keys, label) {
  assert.ok(value && typeof value === "object" && !Array.isArray(value), `${label} must be an object`);
  assert.deepEqual(Object.keys(value).sort(), [...keys].sort(), `${label} fields are not allowlisted`);
}

const version = option("--version", { required: true });
const ciReceipt = jsonFile(option("--ci-receipt", { required: true }), "CI promotion receipt");
const approval = jsonFile(option("--approval", { required: true }), "release approval");
const head = run("git", ["rev-parse", "--verify", "HEAD^{commit}"]);

exactKeys(ciReceipt, [
  "schema_version", "repository", "workflow", "run_id", "run_attempt", "event", "source_ref",
  "source_base_sha", "source_sha", "conclusion", "policy", "jobs",
], "CI promotion receipt");
exactKeys(ciReceipt.policy, ["profile", "version", "digest", "changed_files_digest"], "CI receipt policy");
exactKeys(ciReceipt.jobs, ["docs", "quality", "release", "linux_portability"], "CI receipt jobs");
assert.equal(ciReceipt.schema_version, "planr.ci-promotion-receipt.v1", "unsupported CI promotion receipt schema");
assert.match(ciReceipt.repository, /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u, "invalid CI repository identity");
assert.equal(ciReceipt.workflow, "CI", "promotion requires the canonical CI workflow");
assert.match(ciReceipt.run_id, /^[1-9][0-9]*$/u, "invalid CI run identity");
assert.match(ciReceipt.run_attempt, /^[1-9][0-9]*$/u, "invalid CI run attempt");
assert.equal(ciReceipt.event, "push", "promotion requires CI from a push event");
assert.equal(ciReceipt.source_ref, "refs/heads/main", "promotion requires CI from main");
assert.match(ciReceipt.source_base_sha, /^[0-9a-f]{40}$/u, "invalid CI base SHA");
assert.equal(ciReceipt.source_sha, head, "CI receipt does not bind the current candidate SHA");
assert.equal(ciReceipt.conclusion, "success", "CI receipt is not green");
assert.match(ciReceipt.policy.digest, /^sha256:[0-9a-f]{64}$/u, "invalid CI policy digest");
assert.match(ciReceipt.policy.changed_files_digest, /^sha256:[0-9a-f]{64}$/u, "invalid CI changed-files digest");
for (const [job, result] of Object.entries(ciReceipt.jobs)) {
  assert.match(result, /^(?:success|skipped)$/u, `CI job ${job} is not settled`);
}

const runRecord = JSON.parse(run("gh", ["api", `repos/${ciReceipt.repository}/actions/runs/${ciReceipt.run_id}`]));
assert.equal(String(runRecord.id), ciReceipt.run_id, "GitHub CI run identity mismatch");
assert.equal(String(runRecord.run_attempt), ciReceipt.run_attempt, "GitHub CI run attempt mismatch");
assert.equal(runRecord.name, ciReceipt.workflow, "GitHub workflow identity mismatch");
assert.equal(runRecord.event, ciReceipt.event, "GitHub CI event mismatch");
assert.equal(runRecord.head_branch, "main", "GitHub CI run is not for main");
assert.equal(runRecord.head_sha, head, "GitHub CI run does not bind the current candidate SHA");
assert.equal(runRecord.conclusion, "success", "GitHub CI run is not green");
assert.equal(runRecord.repository?.full_name, ciReceipt.repository, "GitHub CI repository identity mismatch");

const artifactRoot = fs.mkdtempSync(path.join(os.tmpdir(), "planr-release-promotion-"));
try {
  run("gh", [
    "run", "download", ciReceipt.run_id,
    "--repo", ciReceipt.repository,
    "--name", `release-promotion-${head}`,
    "--dir", artifactRoot,
  ]);
  const authenticReceipt = jsonFile(path.join(artifactRoot, "promotion-receipt.json"), "authenticated CI promotion artifact");
  assert.deepEqual(ciReceipt, authenticReceipt, "supplied CI receipt does not match the authenticated run artifact");
} finally {
  fs.rmSync(artifactRoot, { recursive: true, force: true });
}

run("git", ["rev-parse", "--verify", `${ciReceipt.source_base_sha}^{commit}`]);
run("git", ["merge-base", "--is-ancestor", ciReceipt.source_base_sha, head]);
const nameStatus = run("git", [
  "diff", "--name-status", "-z", "--find-renames", ciReceipt.source_base_sha, head,
]);
const selection = classifyChanges(parseGitNameStatus(nameStatus.endsWith("\0") ? nameStatus : `${nameStatus}\0`), {
  baseRevision: ciReceipt.source_base_sha,
  headRevision: head,
});
const routing = routeSelection(selection);
assert.equal(ciReceipt.policy.version, POLICY_VERSION, "CI receipt policy version is stale");
assert.equal(ciReceipt.policy.digest, POLICY_DIGEST, "CI receipt policy digest is stale");
assert.equal(ciReceipt.policy.version, selection.policyVersion, "CI receipt policy version mismatch");
assert.equal(ciReceipt.policy.digest, selection.policyDigest, "CI receipt policy digest mismatch");
assert.equal(ciReceipt.policy.changed_files_digest, selection.changedFilesDigest, "CI receipt changed-files digest mismatch");
assert.equal(ciReceipt.policy.profile, selection.profile, "CI receipt profile mismatch");
for (const [job, result] of Object.entries(ciReceipt.jobs)) {
  const expected = routing[job] === "true" ? "success" : "skipped";
  assert.equal(result, expected, `CI job ${job} does not match the current policy selection`);
}

exactKeys(approval, ["schema_version", "approval_id", "source_sha", "version", "decision", "approved_by", "approved_at"], "release approval");
assert.equal(approval.schema_version, "planr.release-approval.v1", "unsupported release approval schema");
assert.match(approval.approval_id, /^[A-Za-z0-9][A-Za-z0-9._-]{2,127}$/u, "invalid approval identity");
assert.equal(approval.source_sha, head, "approval does not bind the current candidate SHA");
assert.equal(approval.version, version, "approval does not bind the requested version");
assert.equal(approval.decision, "approved", "release is not approved");
assert.match(approval.approved_by, /^[A-Za-z0-9][A-Za-z0-9._@-]{1,127}$/u, "invalid approver identity");
const approvedAt = Date.parse(approval.approved_at);
assert.ok(Number.isFinite(approvedAt) && approvedAt <= Date.now(), "approval timestamp is invalid or in the future");

let baseTag = null;
const base = spawnSync("git", ["describe", "--tags", "--abbrev=0", "HEAD^"], { cwd: repo, encoding: "utf8" });
if (base.status === 0) baseTag = base.stdout.trim();
const changed = baseTag
  ? run("git", ["diff", "--name-only", `${baseTag}..HEAD`]).split("\n").filter(Boolean)
  : [...EVALUATED_SUBJECT_PATHS, ...EVALUATION_POLICY_PATHS];
const evalTriggerPaths = changed.filter((candidate) =>
  EVALUATED_SUBJECT_PATHS.includes(candidate) || EVALUATION_POLICY_PATHS.includes(candidate));
const evaluationRequired = evalTriggerPaths.length > 0;

if (evaluationRequired) {
  const evalReceipt = option("--eval-receipt", { required: true });
  const evalDb = option("--eval-db", { required: true });
  const evalSuite = option("--eval-suite", { required: true });
  const planrBin = option("--planr-bin", { required: true });
  run(process.execPath, [
    "scripts/verify-release-eval-receipt.mjs",
    "--receipt", evalReceipt,
    "--db", evalDb,
    "--suite", evalSuite,
    "--planr-bin", planrBin,
  ], { stdio: ["ignore", "pipe", "pipe"] });
}

console.log(JSON.stringify({
  verdict: "pass",
  source_sha: head,
  ci_run_id: ciReceipt.run_id,
  approval_id: approval.approval_id,
  evaluation: evaluationRequired ? "verified" : "not_required",
  evaluation_trigger_paths: evalTriggerPaths,
}));
