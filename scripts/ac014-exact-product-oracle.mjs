#!/usr/bin/env node
import { createHash } from "node:crypto";
import { constants, accessSync, existsSync, readFileSync, realpathSync, statSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const BROWSER_CRITERIA = Object.freeze(Array.from({ length: 12 }, (_, index) => `AC-${String(index + 1).padStart(3, "0")}`));
const BROWSER_REQUIREMENTS = new Set(BROWSER_CRITERIA.map((criterion) => `obs-${criterion}-browser`));
const REQUIRED_REQUIREMENTS = new Set([...BROWSER_REQUIREMENTS, "obs-AC-012-build"]);

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const result = runExactProductOracle({
    planrBin: process.env.PLANR_AC014_CANDIDATE_BIN ?? process.env.PLANR_BIN,
    candidateSha256: process.env.PLANR_AC014_CANDIDATE_SHA256,
    planId: process.env.PLANR_AC014_ORACLE_PLAN_ID,
    sourceRevision: process.env.PLANR_AC014_EVIDENCE_SOURCE_REVISION,
    cwd: process.cwd(),
  });
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

export function runExactProductOracle({ planrBin, candidateSha256, planId, sourceRevision, cwd }) {
  const root = realpathSync(path.resolve(required(cwd, "cwd")));
  const binary = realpathSync(path.resolve(required(planrBin, "PLANR_AC014_CANDIDATE_BIN")));
  const expectedDigest = requiredDigest(candidateSha256, "PLANR_AC014_CANDIDATE_SHA256");
  const targetPlan = required(planId, "PLANR_AC014_ORACLE_PLAN_ID");
  const expectedSource = requiredGitRevision(sourceRevision, "PLANR_AC014_EVIDENCE_SOURCE_REVISION");
  assertExecutable(binary);
  const before = binaryIdentity(binary);
  if (before.sha256 !== expectedDigest) throw new Error("AC-014 oracle candidate binary digest mismatch");

  const readiness = runJson(binary, ["evidence", "readiness", "--scope", "plan", "--id", targetPlan, "--json"], root);
  if (readiness.object?.status !== "passed") throw new Error(`dogfood Evidence readiness is not passed: ${readiness.object?.status}`);
  const runIndex = readiness.object?.run_index;
  const sealedRuns = validateRunIndex(runIndex, root, expectedSource);

  const execution = runJson(binary, ["evidence", "run", "--input", runIndex.repository_path, "--json"], root);
  validateExecution(execution.object, expectedSource, sealedRuns);

  const coverage = runJson(binary, ["evidence", "coverage", "--scope", "plan", "--id", targetPlan, "--json"], root);
  validateCoverage(coverage.object);

  const audit = runJson(binary, ["plan", "audit", targetPlan, "--json"], root);
  if (audit.holds !== true) throw new Error("dogfood plan audit does not hold after complete browser Evidence");

  const after = binaryIdentity(binary);
  if (JSON.stringify(after) !== JSON.stringify(before)) throw new Error("AC-014 oracle candidate binary changed during execution");
  return {
    status: "passed",
    oracle_id: "sparziele-exact-product-flow-v1",
    plan_id: targetPlan,
    candidate_binary_sha256: before.sha256,
    readiness_run_index_digest: runIndex.run_index_digest,
    evidence_status: execution.object.status,
    coverage: coverage.object.status,
    browser_requirements_covered: BROWSER_REQUIREMENTS.size,
    build_requirements_covered: 1,
    waivers: 0,
    plan_audit_holds: true,
  };
}

function validateRunIndex(runIndex, root, expectedSource) {
  if (!runIndex || runIndex.schema_version !== "planr.evidence.run-index.v1") throw new Error("AC-014 oracle requires a sealed Evidence run index");
  if (!/^sha256:[0-9a-f]{64}$/.test(runIndex.run_index_digest ?? "")) throw new Error("AC-014 oracle run index digest is missing");
  if (runIndex.source?.dirty !== false || runIndex.source?.revision !== expectedSource) throw new Error("AC-014 oracle sealed run index source mismatch");
  const indexPath = path.resolve(root, required(runIndex.repository_path, "run_index.repository_path"));
  if (!isInside(indexPath, root) || !existsSync(indexPath)) throw new Error("AC-014 oracle run index must be a repository-local file");
  const targets = new Set();
  const obligations = new Set();
  const indexes = new Set();
  const sealedRuns = new Map();
  const runs = runIndex.runs ?? [];
  if (runs.length !== REQUIRED_REQUIREMENTS.size) throw new Error("AC-014 oracle run index must contain exactly 12 browser runs and one build run");
  for (const run of runs) {
    const target = run.input?.target;
    const obligationId = required(run.input?.obligation_id, "sealed run obligation_id");
    const capabilityInstanceId = required(run.input?.capability_instance_id, "sealed run capability_instance_id");
    if (!Number.isInteger(run.index) || run.index < 0 || run.index >= REQUIRED_REQUIREMENTS.size || indexes.has(run.index)) {
      throw new Error("AC-014 oracle sealed run indexes must be distinct and contiguous");
    }
    if (obligations.has(obligationId)) throw new Error("AC-014 oracle sealed run obligations must be distinct");
    indexes.add(run.index);
    obligations.add(obligationId);
    let requirementId;
    if (target?.kind === "browser" && /^http:\/\/127\.0\.0\.1:3000\/#AC-\d{3}$/.test(target.uri ?? "")) {
      const criterion = target.uri.slice(-6);
      targets.add(criterion);
      requirementId = `obs-${criterion}-browser`;
    }
    if (target?.kind === "process" && target.uri === "local://pnpm-build") {
      targets.add("BUILD-001");
      requirementId = "obs-AC-012-build";
    }
    if (!requirementId) throw new Error("AC-014 oracle sealed run has an unexpected target");
    sealedRuns.set(obligationId, { index: run.index, obligationId, capabilityInstanceId, requirementId, target: structuredClone(target) });
  }
  for (const criterion of BROWSER_CRITERIA) if (!targets.has(criterion)) throw new Error(`AC-014 oracle run index is missing browser target ${criterion}`);
  if (!targets.has("BUILD-001")) throw new Error("AC-014 oracle run index is missing the production build target");
  if (targets.size !== REQUIRED_REQUIREMENTS.size) throw new Error("AC-014 oracle run index contains an incomplete or ambiguous target set");
  return sealedRuns;
}

function validateExecution(object, expectedSource, sealedRuns) {
  if (object?.status !== "passed" || object?.verdict !== "passed") throw new Error("AC-014 browser Evidence execution did not pass");
  const results = object.results ?? [];
  if (results.length !== REQUIRED_REQUIREMENTS.size) throw new Error("AC-014 browser Evidence execution must contain exactly 13 results");
  const covered = new Set();
  const seenObligations = new Set();
  const seenReceiptIds = new Set();
  const seenReceiptDigests = new Set();
  const seenAttemptIds = new Set();
  for (const result of results) {
    const receipt = result.receipt;
    if (result.verdict !== "passed" || result.receipt?.receipt_status !== "trusted" || (result.receipt?.proof_gaps ?? []).length !== 0) {
      throw new Error("AC-014 browser Evidence result is not trusted and gap-free");
    }
    if (result.reused !== false) throw new Error("AC-014 browser Evidence may not reuse a cross-run receipt");
    if (receipt?.source?.dirty !== false || receipt?.source?.revision !== expectedSource) throw new Error("AC-014 browser Evidence receipt source mismatch");
    const sealed = sealedRuns.get(receipt?.obligation_id);
    if (!sealed || result.attempt?.obligation_id !== sealed.obligationId) throw new Error("AC-014 browser Evidence result does not bind one sealed obligation");
    if (seenObligations.has(sealed.obligationId)) throw new Error("AC-014 browser Evidence has a duplicate or ambiguous sealed-run result");
    if (result.attempt?.capability_instance_id !== sealed.capabilityInstanceId || receipt.capability?.instance_id !== sealed.capabilityInstanceId) {
      throw new Error("AC-014 browser Evidence result capability does not match its sealed run");
    }
    if (!sameTarget(receipt.target, sealed.target)) throw new Error("AC-014 browser Evidence receipt target does not match its sealed run");
    const observations = receipt.observations ?? [];
    if (observations.length !== 1) throw new Error("AC-014 browser Evidence receipt must contain exactly one run-bound observation");
    const observation = observations[0];
    if (observation.outcome !== "passed" || observation.requirement_id !== sealed.requirementId) {
      throw new Error("AC-014 browser Evidence observation does not match its sealed run requirement");
    }
    const attemptIds = receipt.attempt_ids ?? [];
    if (attemptIds.length !== 1 || attemptIds[0] !== result.attempt?.id) throw new Error("AC-014 browser Evidence receipt must bind exactly its result attempt");
    for (const [value, seen, label] of [
      [receipt.id, seenReceiptIds, "receipt id"],
      [receipt.receipt_digest, seenReceiptDigests, "receipt digest"],
      [result.attempt.id, seenAttemptIds, "attempt id"],
    ]) {
      if (typeof value !== "string" || value.length === 0 || seen.has(value)) throw new Error(`AC-014 browser Evidence has cross-run reuse of ${label}`);
      seen.add(value);
    }
    seenObligations.add(sealed.obligationId);
    covered.add(observation.requirement_id);
  }
  if (seenObligations.size !== sealedRuns.size) throw new Error("AC-014 browser Evidence execution is missing a sealed-run result");
  assertExactRequirements(covered, "execution");
}

function sameTarget(left, right) {
  return left?.kind === right?.kind && left?.uri === right?.uri;
}

function validateCoverage(object) {
  const projection = object?.canonical_projection;
  if (object?.status !== "satisfied" || object?.verdict !== "satisfied" || projection?.pass !== true) throw new Error("dogfood Evidence coverage is not satisfied");
  if ((object.waiver_digests ?? []).length || (projection.waiver_refs ?? []).length || (projection.waiver_digests ?? []).length) {
    throw new Error("AC-014 browser Evidence may not be waived");
  }
  const covered = new Set((projection.observations ?? []).filter((entry) => entry.status === "covered").map((entry) => entry.requirement_id));
  assertExactRequirements(covered, "coverage");
  const details = object.coverage?.validation_details ?? object.validation_details;
  for (const dimension of ["completion", "fixture", "freshness", "provenance", "schema", "target", "trust"]) {
    if (details?.[dimension]?.status !== "passed") throw new Error(`AC-014 browser Evidence ${dimension} validation is stale or incomplete`);
  }
}

function assertExactRequirements(actual, stage) {
  for (const requirement of REQUIRED_REQUIREMENTS) if (!actual.has(requirement)) throw new Error(`AC-014 ${stage} is missing ${requirement}`);
  if (actual.size !== REQUIRED_REQUIREMENTS.size) throw new Error(`AC-014 ${stage} requirement set is ambiguous`);
}

function runJson(binary, args, cwd) {
  const result = spawnSync(binary, args, { cwd, encoding: "utf8", env: process.env });
  if (result.status !== 0) throw new Error(`${path.basename(binary)} ${args.slice(0, 2).join(" ")} failed: ${String(result.stderr || result.stdout).trim()}`);
  try { return JSON.parse(result.stdout); } catch { throw new Error(`${path.basename(binary)} ${args.slice(0, 2).join(" ")} did not emit JSON`); }
}

function binaryIdentity(binary) {
  const stat = statSync(binary);
  return { realpath: binary, size: stat.size, mode: stat.mode & 0o777, sha256: `sha256:${createHash("sha256").update(readFileSync(binary)).digest("hex")}` };
}

function assertExecutable(binary) {
  accessSync(binary, constants.X_OK);
  if (!statSync(binary).isFile()) throw new Error("AC-014 oracle candidate binary must be a file");
}

function required(value, label) {
  if (typeof value !== "string" || value.length === 0) throw new Error(`${label} is required`);
  return value;
}

function requiredDigest(value, label) {
  if (!/^sha256:[0-9a-f]{64}$/.test(value ?? "")) throw new Error(`${label} must be an exact sha256 digest`);
  return value;
}

function requiredGitRevision(value, label) {
  if (!/^[0-9a-f]{40}$/.test(value ?? "")) throw new Error(`${label} must be an exact git revision`);
  return value;
}

function isInside(candidate, root) {
  const relative = path.relative(root, candidate);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}
