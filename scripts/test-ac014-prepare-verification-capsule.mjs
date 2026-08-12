import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { prepareVerificationCapsule } from "./ac014-prepare-verification-capsule.mjs";

const sourceRoot = path.dirname(path.dirname(new URL(import.meta.url).pathname));
const scratch = mkdtempSync(path.join(tmpdir(), "planr-ac014-capsule-"));
process.on("exit", () => rmSync(scratch, { recursive: true, force: true }));
const accepted = {
  attempt: "attempt-8262a5a5", number: 11,
  revision: "ab18029c431857ea0bccc76a2b8da0fc7d80b29b",
  freeze: "freeze-19241224", digest: `sha256:${"1".repeat(64)}`,
};

function fixture(name, { runner = true } = {}) {
  const root = path.join(scratch, name);
  mkdirSync(path.join(root, "scripts"), { recursive: true });
  writeFileSync(path.join(root, ".gitignore"), ".planr/\n");
  cpSync(path.join(sourceRoot, "scripts/ac014-exact-product-oracle.mjs"), path.join(root, "scripts/ac014-exact-product-oracle.mjs"));
  if (runner) cpSync(path.join(sourceRoot, "scripts/ac014-fresh-arm-runner.mjs"), path.join(root, "scripts/ac014-fresh-arm-runner.mjs"));
  execFileSync("git", ["init"], { cwd: root, stdio: "ignore" });
  execFileSync("git", ["config", "user.email", "planr@example.invalid"], { cwd: root });
  execFileSync("git", ["config", "user.name", "Planr Test"], { cwd: root });
  execFileSync("git", ["add", "."], { cwd: root });
  execFileSync("git", ["commit", "-m", "fixture"], { cwd: root, stdio: "ignore" });
  const revision = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
  const candidateDir = path.join(scratch, `${name}-candidates`, revision);
  mkdirSync(candidateDir, { recursive: true });
  const binary = path.join(candidateDir, "planr");
  writeFileSync(binary, "#!/bin/sh\nexit 99\n");
  chmodSync(binary, 0o555);
  return {
    root,
    request: {
      schema_version: "planr.ac014.capsule_preparation_request.v1",
      repo_root: root, planr_binary: binary, planr_binary_sha256: shaFile(binary), review_required: true,
      accepted_review_attempt_id: accepted.attempt, accepted_review_attempt_number: accepted.number,
      accepted_source_freeze_id: accepted.freeze, accepted_source_digest: accepted.digest,
      accepted_source_revision: accepted.revision, accepted_source_tree: "3".repeat(40),
      superseded_obligation_id: "pob-planr2-ac014-482d219-v2",
      baseline_root: "/Users/kregenrek/projects/planr-dogfood/outcome-batching-ac014-alpha4-baseline-final",
      fresh_root: path.join(scratch, `${name}-fresh`), result_path: path.join(scratch, `${name}-result.json`),
      artifact_dir: `.planr/artifacts/planr2-ac014/run-${revision.slice(0, 12)}`,
      codex_home: path.join(scratch, `${name}-codex-home`),
    },
    injected: dependencies(),
  };
}

function dependencies() {
  return {
    trace: {
      proof: { attempts: [], receipts: [] },
      execution_state: {
        owner: { worker_id: "cumulative-recovery-maker", role: "maker" },
        feature_run: { id: "frun-0e9e5adc", phase: "implementation" },
        review_gate: { id: "gate-149bc71f", status: "accepted", latest_attempt: accepted.number },
        review_source_binding: { gate_id: "gate-149bc71f", freeze_id: accepted.freeze, source_revision: accepted.revision, source_digest: accepted.digest },
        review_attempts: [{ id: accepted.attempt, attempt_number: accepted.number, source_revision: accepted.revision, verdict: "accepted" }],
      },
    },
    attempts: { object: { attempts: [] } },
    receipts: { object: { receipts: [] } },
    claims: [],
    acceptedSourceTree: "3".repeat(40),
    migrationPreview: { ok: true, object: { dry_run: true, summary: { create: 1 } } },
    migrationApplied: { ok: true, object: { dry_run: false, summary: { create: 1 } } },
    capabilityList: (config) => ({ object: { instances: [{
      id: "capinst-ac014-capsule-test", manifest_id: config.manifest.id,
      manifest_digest: config.manifestDigest, availability_status: "available",
      capability: { availability: { status: "available" }, environment: { kind: "local", id: "planr-local", digest: `sha256:${"2".repeat(64)}` } },
    }] } }),
  };
}

{
  const { root, request, injected } = fixture("positive");
  const result = prepareVerificationCapsule(request, injected);
  assert.equal(result.status, "prepared_pending_review");
  const handoff = JSON.parse(readFileSync(path.join(root, result.verifier_handoff), "utf8"));
  const runnerInput = JSON.parse(readFileSync(path.join(root, result.runner_input), "utf8"));
  const runInput = JSON.parse(readFileSync(path.join(root, result.run_input), "utf8"));
  assert.equal(handoff.review_required, true);
  assert.equal(handoff.accepted_preparation_review.attempt_id, accepted.attempt);
  assert.equal(handoff.candidate.source_revision, result.source_revision);
  assert.deepEqual(handoff.commands.runner.argv.slice(-4), ["--input", path.join(root, result.runner_input), "--result", request.result_path]);
  assert.deepEqual(handoff.commands.evidence_run.argv, ["evidence", "run", "--input", path.join(root, result.run_input), "--json"]);
  assert.equal(runnerInput.control_handoff.review_required, true);
  assert.equal(runInput.obligation_id, result.obligation_id);
  assert.equal(existsSync(request.fresh_root), false);
  assert.equal(existsSync(request.result_path), false);
  assert.equal(existsSync(request.codex_home), true);
  assert.equal(JSON.parse(readFileSync(path.join(root, ".planr/evidence.yaml"), "utf8")).completion_policy.require_review_evidence, true);
}

{
  const { request, injected } = fixture("stale-gate");
  injected.trace.execution_state.review_gate.latest_attempt = 12;
  assert.throws(() => prepareVerificationCapsule(request, injected), /stale accepted ReviewGate attempt number/);
  assert.equal(existsSync(request.codex_home), false);
}

{
  const { request, injected } = fixture("false-review");
  request.review_required = false;
  assert.throws(() => prepareVerificationCapsule(request, injected), /review_required must be true/);
  assert.equal(existsSync(request.codex_home), false);
}

{
  const { request, injected } = fixture("reused-path");
  mkdirSync(request.fresh_root);
  assert.throws(() => prepareVerificationCapsule(request, injected), /fresh_root must be a fresh never-used path/);
  assert.equal(existsSync(request.codex_home), false);
}

{
  const { request, injected } = fixture("missing-runner", { runner: false });
  assert.throws(() => prepareVerificationCapsule(request, injected), /runner must be an existing canonical regular file/);
  assert.equal(existsSync(request.codex_home), false);
}

process.stdout.write("AC-014 capsule preparation tests passed\n");

function shaFile(file) {
  return `sha256:${createHash("sha256").update(readFileSync(file)).digest("hex")}`;
}
