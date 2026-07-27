#!/usr/bin/env node

import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const evaluatedSubjectPaths = [
  "plugins/planr/skills/planr-goal/SKILL.md",
  "plugins/planr/skills/planr-loop/SKILL.md",
  "plugins/planr/skills/planr-loop/references/host-dispatch.md",
  "plugins/planr/skills/planr-loop/references/recovery-and-verification.md",
  "plugins/planr/skills/planr-task-graph/SKILL.md",
];

function candidateRevision() {
  const inventory = spawnSync(
    "git",
    ["ls-files", "-z", "--cached", "--others", "--exclude-standard"],
    { cwd: repo, encoding: "utf8" },
  );
  assert.equal(inventory.status, 0, "candidate source inventory failed");
  const candidatePaths = inventory.stdout.split("\0").filter(Boolean).sort();
  assert.ok(candidatePaths.length > 0, "candidate source inventory is empty");

  const hash = crypto.createHash("sha256");
  hash.update("planr.release-source.v1\0");
  for (const relative of candidatePaths) {
    const absolute = path.join(repo, relative);
    let stat;
    try {
      stat = fs.lstatSync(absolute);
    } catch (error) {
      if (error?.code === "ENOENT") continue;
      throw error;
    }
    const mode = stat.isSymbolicLink()
      ? "120000"
      : stat.isFile()
        ? (stat.mode & 0o111) === 0 ? "100644" : "100755"
        : null;
    assert.ok(mode, `unsupported candidate source file type: ${relative}`);
    const content = stat.isSymbolicLink()
      ? Buffer.from(fs.readlinkSync(absolute))
      : fs.readFileSync(absolute);
    hash.update(relative);
    hash.update("\0");
    hash.update(mode);
    hash.update("\0");
    hash.update(content);
    hash.update("\0");
  }
  return `sha256:${hash.digest("hex")}`;
}

function evaluatedSubjectRevision() {
  const hash = crypto.createHash("sha256");
  for (const relative of evaluatedSubjectPaths) {
    hash.update(relative);
    hash.update("\0");
    hash.update(fs.readFileSync(path.join(repo, relative)));
    hash.update("\0");
  }
  return `sha256:${hash.digest("hex")}`;
}

if (process.argv.includes("--print-candidate-revision")) {
  console.log(candidateRevision());
  process.exit(0);
}
if (process.argv.includes("--print-evaluated-subject-revision")) {
  console.log(evaluatedSubjectRevision());
  process.exit(0);
}

function option(name) {
  const index = process.argv.indexOf(name);
  assert.ok(index >= 0 && process.argv[index + 1], `missing ${name}`);
  return process.argv[index + 1];
}

const receiptPath = option("--receipt");
const dbPath = option("--db");
const suitePath = option("--suite");
const planrBin = option("--planr-bin");
const receiptPathResolved = path.resolve(repo, receiptPath);
const dbPathResolved = path.resolve(repo, dbPath);
const suitePathResolved = path.resolve(repo, suitePath);
const planrBinPath = path.resolve(repo, planrBin);
let receipt;
try {
  receipt = JSON.parse(fs.readFileSync(receiptPathResolved, "utf8"));
} catch {
  throw new Error("release eval receipt is unreadable or invalid JSON");
}

const allowedKeys = [
  "schema_version",
  "comparison_id",
  "candidate_run_id",
  "suite_digest",
  "candidate_revision",
  "evaluated_subject_revision",
  "created_at",
  "expires_at",
];
assert.deepEqual(Object.keys(receipt).sort(), allowedKeys.sort(), "release eval receipt fields are not allowlisted");
assert.equal(receipt.schema_version, "planr.release-eval-receipt.v2", "unsupported release eval receipt schema");
for (const key of ["suite_digest", "candidate_revision", "evaluated_subject_revision"]) {
  assert.match(receipt[key], /^sha256:[0-9a-f]{64}$/u, `invalid ${key}`);
}
for (const key of ["comparison_id", "candidate_run_id"]) {
  assert.match(receipt[key], /^[A-Za-z0-9][A-Za-z0-9._-]{2,127}$/u, `invalid ${key}`);
}
const createdAt = Date.parse(receipt.created_at);
const expiresAt = Date.parse(receipt.expires_at);
assert.ok(Number.isFinite(createdAt) && Number.isFinite(expiresAt), "receipt timestamps must be RFC 3339");
assert.ok(createdAt <= Date.now() && Date.now() < expiresAt, "release eval receipt is stale");
assert.ok(expiresAt - createdAt <= 7 * 24 * 60 * 60 * 1000, "release eval receipt freshness window exceeds seven days");

assert.equal(receipt.candidate_revision, candidateRevision(), "release eval receipt does not bind the current candidate");
assert.equal(receipt.evaluated_subject_revision, evaluatedSubjectRevision(), "release eval receipt does not bind the current evaluated subject");

function planrJson(args, acceptedStatuses = [0], cwd = repo) {
  const result = spawnSync(planrBinPath, ["--db", dbPathResolved, "--json", ...args], {
    cwd,
    encoding: "utf8",
    env: process.env,
  });
  assert.ok(acceptedStatuses.includes(result.status), `Planr ${args.join(" ")} rejected release evidence`);
  try {
    return JSON.parse(result.stdout);
  } catch {
    throw new Error(`Planr ${args.join(" ")} returned invalid JSON`);
  }
}

// Fixture paths belong to the external suite workspace, never to the Planr
// product checkout. Other eval lookups stay rooted in the candidate checkout.
const suiteEnvelope = planrJson(
  ["eval", "suite-check", "--input", suitePathResolved],
  [0],
  path.dirname(suitePathResolved),
);
assert.equal(suiteEnvelope.ok, true, "current suite canonicalization failed");
assert.equal(suiteEnvelope.object?.suite?.digest, receipt.suite_digest, "canonical current suite digest mismatch");

const comparisonEnvelope = planrJson(["eval", "show", "comparison", receipt.comparison_id]);
assert.equal(comparisonEnvelope.ok, true, "comparison lookup failed");
const comparison = comparisonEnvelope.object?.comparison;
assert.equal(comparison?.id, receipt.comparison_id, "comparison identity mismatch");
assert.equal(comparison?.candidate_run_id, receipt.candidate_run_id, "comparison candidate mismatch");
assert.ok(typeof comparison?.baseline_run_id === "string" && comparison.baseline_run_id.length > 0, "comparison baseline identity missing");
assert.ok(typeof comparison?.policy_digest === "string" && comparison.policy_digest.length > 0, "comparison policy identity missing");

const runEnvelope = planrJson(["eval", "show", "run", receipt.candidate_run_id]);
assert.equal(runEnvelope.ok, true, "candidate run lookup failed");
const run = runEnvelope.object?.run;
assert.equal(run?.id, receipt.candidate_run_id, "candidate run identity mismatch");
assert.equal(run?.suite_digest, receipt.suite_digest, "candidate suite mismatch");
assert.equal(run?.subject_revision, receipt.evaluated_subject_revision, "evaluated subject revision mismatch");
assert.equal(run?.status, "success", "candidate run is not successful");
assert.equal(run?.invalidated_by ?? null, null, "candidate run is invalidated");

const attempts = (run.cases ?? []).flatMap((testCase) => testCase.attempts ?? []).filter((attempt) => attempt.countable !== false);
assert.ok(attempts.length > 0, "candidate run has no countable attempts");
for (const attempt of attempts) {
  const validation = attempt.route_observation_validation;
  assert.equal(validation?.source, "planr.route_audit.v1", "candidate treatment provenance was not validated by Planr");
  assert.equal(validation?.status, "verified", "candidate effective treatment lacks verified route provenance");
  for (const field of [
    "client",
    "provider",
    "runtime",
    "model",
    "effort",
    "profile_id",
    "profile_config_digest",
    "runner_harness_version",
    "agent_type",
  ]) {
    assert.ok(typeof validation.effective?.[field] === "string" && validation.effective[field].length > 0, `candidate validated treatment is missing: ${field}`);
  }
}

const recomputeEnvelope = planrJson([
  "eval",
  "compare",
  comparison.baseline_run_id,
  receipt.candidate_run_id,
  "--policy-digest",
  comparison.policy_digest,
  "--recompute-of",
  receipt.comparison_id,
]);
assert.equal(recomputeEnvelope.ok, true, "release-time comparison recomputation failed");
const recomputed = recomputeEnvelope.object?.comparison;
assert.ok(typeof recomputed?.id === "string" && recomputed.id.length > 0, "recomputed comparison identity missing");
assert.equal(recomputed?.baseline_run_id, comparison.baseline_run_id, "recomputed comparison baseline mismatch");
assert.equal(recomputed?.candidate_run_id, receipt.candidate_run_id, "recomputed comparison candidate mismatch");
assert.equal(recomputed?.policy_digest, comparison.policy_digest, "recomputed comparison policy mismatch");
assert.equal(recomputed?.recompute_of, receipt.comparison_id, "recomputed comparison provenance mismatch");
assert.equal(recomputed?.verdict, "improved", "release comparison did not prove material improvement");

const gate = planrJson(["eval", "gate", recomputed.id]);
assert.equal(gate.ok, true, "existing Planr eval gate did not pass");
assert.equal(gate.object?.verdict, "improved", "release gate accepted a non-improving comparison");

console.log(JSON.stringify({
  verdict: "pass",
  comparison_id: receipt.comparison_id,
  recomputed_comparison_id: recomputed.id,
  candidate_run_id: receipt.candidate_run_id,
  suite_digest: receipt.suite_digest,
  candidate_revision: receipt.candidate_revision,
  countable_attempts: attempts.length,
  raw_prompt_or_completion_retained: false,
}, null, 2));
