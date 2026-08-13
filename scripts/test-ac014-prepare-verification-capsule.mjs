import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { prepareReviewDraft, sealLaunchCapsule } from "./ac014-prepare-verification-capsule.mjs";

const sourceRoot = path.dirname(path.dirname(new URL(import.meta.url).pathname));
const scratch = mkdtempSync(path.join(tmpdir(), "planr-ac014-capsule-"));
process.on("exit", () => rmSync(scratch, { recursive: true, force: true }));
const accepted = { attempt: "attempt-a1b2c3d4", number: 13, freeze: "freeze-a1b2c3d4", digest: `sha256:${"1".repeat(64)}` };

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
  const revision = git(root, "HEAD");
  const tree = git(root, "HEAD^{tree}");
  const candidateDir = path.join(scratch, `${name}-candidates`, revision);
  mkdirSync(candidateDir, { recursive: true });
  const binary = path.join(candidateDir, "planr");
  writeFileSync(binary, "#!/bin/sh\nexit 99\n");
  chmodSync(binary, 0o555);
  const request = {
    schema_version: "planr.ac014.review_draft_request.v1",
    repo_root: root,
    planr_binary: binary,
    planr_binary_sha256: shaFile(binary),
    review_required: true,
    superseded_obligation_id: "pob-planr2-ac014-obsolete-v1",
    baseline_root: "/Users/kregenrek/projects/planr-dogfood/outcome-batching-ac014-alpha4-baseline-final",
    fresh_root: path.join(scratch, `${name}-fresh`),
    result_path: path.join(scratch, `${name}-result.json`),
    artifact_dir: `.planr/artifacts/planr2-ac014/run-${revision.slice(0, 12)}`,
    codex_home: path.join(scratch, `${name}-codex-home`),
  };
  const injected = {
    trace: draftTrace(),
    attempts: { object: { attempts: [] } },
    receipts: { object: { receipts: [] } },
    claims: [],
    migrationPreview: { ok: true, object: { dry_run: true, summary: { create: 1 } } },
    migrationApplied: { ok: true, object: { dry_run: false, summary: { create: 1 } } },
  };
  return { root, revision, tree, request, injected };
}

function draftTrace() {
  return {
    item: { id: "i-execute-exactly-one-fresh-isolat-bde0", status: "ready", worker_id: null },
    proof: { attempts: [], receipts: [] },
    execution_state: {
      owner: { worker_id: "cumulative-recovery-maker", role: "maker", lease_generation: 11 },
      feature_run: { id: "frun-0e9e5adc", phase: "implementation" },
      review_gate: { id: "gate-149bc71f", status: "changes_requested", latest_attempt: 12 },
    },
  };
}

function acceptedTrace(revision) {
  return {
    item: { id: "i-execute-exactly-one-fresh-isolat-bde0", status: "picked", worker_id: "exact-verifier" },
    proof: { attempts: [], receipts: [] },
    execution_state: {
      owner: { worker_id: "exact-verifier", role: "verifier", lease_generation: 3 },
      feature_run: { id: "frun-0e9e5adc", phase: "verification" },
      review_gate: { id: "gate-149bc71f", status: "accepted", latest_attempt: accepted.number },
      review_source_binding: { gate_id: "gate-149bc71f", freeze_id: accepted.freeze, source_revision: revision, source_digest: accepted.digest },
      review_attempts: [{ id: accepted.attempt, attempt_number: accepted.number, source_revision: revision, verdict: "accepted", reviewer_mode: "independent", reviewer_worker_id: "independent-reviewer" }],
    },
  };
}

function createDraft(name, options) {
  const value = fixture(name, options);
  value.result = prepareReviewDraft(value.request, value.injected);
  value.draftPath = path.join(value.root, value.result.review_draft);
  return value;
}

function sealDependencies(value) {
  return {
    trace: acceptedTrace(value.revision),
    attempts: { object: { attempts: [] } },
    receipts: { object: { receipts: [] } },
    claims: [],
    capabilityList: (draft) => ({ object: { instances: [{
      id: "capinst-ac014-capsule-test",
      manifest_id: draft.evidence.manifest_id,
      availability_status: "available",
      capability: { availability: { status: "available" }, environment: { kind: "local", id: "planr-local", digest: `sha256:${"2".repeat(64)}` } },
    }] } }),
  };
}

{
  const value = createDraft("draft-positive");
  const draft = JSON.parse(readFileSync(value.draftPath, "utf8"));
  assert.equal(value.result.status, "pending_exact_source_review");
  assert.equal(value.result.launch_allowed, false);
  assert.equal(draft.launch_guard.launch_allowed, false);
  assert.equal(draft.launch_guard.authorization, null);
  assert.equal(draft.candidate.source_revision, value.revision);
  assert.equal("accepted_review_attempt_id" in draft, false);
  assert.equal("source_freeze" in draft, false);
  assert.equal(existsSync(value.request.fresh_root), false);
  assert.equal(existsSync(value.request.result_path), false);
  assert.equal(existsSync(value.request.codex_home), false);
  assert.equal(statSync(value.draftPath).mode & 0o222, 0);
  assertGeneratedSchemaIdentity(value);
}

{
  const value = createDraft("seal-positive");
  const sealed = sealLaunchCapsule(value.draftPath, sealDependencies(value));
  const seal = JSON.parse(readFileSync(path.join(value.root, sealed.launch_seal), "utf8"));
  const runnerInput = JSON.parse(readFileSync(path.join(value.root, sealed.runner_input), "utf8"));
  const handoff = JSON.parse(readFileSync(path.join(value.root, sealed.verifier_handoff), "utf8"));
  assert.equal(seal.launch_allowed, true);
  assert.equal(seal.accepted_review_attempt_id, accepted.attempt);
  assert.equal(seal.source_freeze.id, accepted.freeze);
  assert.deepEqual(seal.verifier_lease, { worker_id: "exact-verifier", generation: 3 });
  assert.deepEqual(runnerInput.launch_authorization, sealWithoutPaths(seal));
  assert.equal(handoff.execution_guard.requires_live_revalidation, true);
  assert.equal(existsSync(value.request.codex_home), true);
  assert.equal(existsSync(value.request.fresh_root), false);
  assert.equal(existsSync(value.request.result_path), false);
  assert.throws(() => sealLaunchCapsule(value.draftPath, sealDependencies(value)), /cannot be resealed or overwritten/);
}

{
  const value = createDraft("pending-gate");
  const dependencies = sealDependencies(value);
  dependencies.trace.execution_state.review_gate.status = "pending";
  assert.throws(() => sealLaunchCapsule(value.draftPath, dependencies), /requires the same accepted ReviewGate/);
  assert.equal(existsSync(value.request.codex_home), false);
}

{
  const value = createDraft("stale-attempt");
  const dependencies = sealDependencies(value);
  dependencies.trace.execution_state.review_attempts[0].source_revision = "a".repeat(40);
  assert.throws(() => sealLaunchCapsule(value.draftPath, dependencies), /latest independently accepted attempt is not for the exact draft source/);
  assert.equal(existsSync(value.request.codex_home), false);
}

{
  const value = createDraft("invalidated-freeze");
  const dependencies = sealDependencies(value);
  dependencies.trace.execution_state.review_source_binding = null;
  assert.throws(() => sealLaunchCapsule(value.draftPath, dependencies), /freeze is missing, invalidated, or stale/);
  assert.equal(existsSync(value.request.codex_home), false);
}

{
  const value = createDraft("wrong-verifier");
  const dependencies = sealDependencies(value);
  dependencies.trace.item.worker_id = "different-verifier";
  assert.throws(() => sealLaunchCapsule(value.draftPath, dependencies), /current verifier lease and picked item/);
  assert.equal(existsSync(value.request.codex_home), false);
}

{
  const { request, injected } = fixture("false-review");
  request.review_required = false;
  assert.throws(() => prepareReviewDraft(request, injected), /review_required must be true/);
}

{
  const { request, injected } = fixture("reused-path");
  mkdirSync(request.fresh_root);
  assert.throws(() => prepareReviewDraft(request, injected), /fresh_root must be a fresh never-used path/);
}

{
  const { request, injected } = fixture("missing-runner", { runner: false });
  assert.throws(() => prepareReviewDraft(request, injected), /runner must be an existing canonical regular file/);
}

process.stdout.write("AC-014 two-stage capsule preparation tests passed\n");

function sealWithoutPaths(seal) {
  const { handoff_path: _handoff, runner_input_path: _runner, run_input_path: _run, ...authorization } = seal;
  return authorization;
}

function assertGeneratedSchemaIdentity(value) {
  const short = value.revision.slice(0, 12);
  const type = `com.planr.planr2.ac014.terminal_arm.${short}.v2`;
  const schemaRef = `schema://${type}`;
  const schemaPath = `.planr/evidence/schemas/${type}.schema.json`;
  const policy = JSON.parse(readFileSync(path.join(value.root, ".planr/evidence.yaml"), "utf8"));
  const schema = JSON.parse(readFileSync(path.join(value.root, schemaPath), "utf8"));
  const manifest = JSON.parse(readFileSync(path.join(value.root, `.planr/evidence/adapters/verifier-planr2-ac014-${short}-v2.manifest.json`), "utf8"));
  const migration = JSON.parse(readFileSync(path.join(value.root, `.planr/evidence/obligations/pob-planr2-ac014-${short}-v2.migration.json`), "utf8"));
  const runIndex = JSON.parse(readFileSync(path.join(value.root, `.planr/evidence/obligations/pob-planr2-ac014-${short}-v2.run-index.json`), "utf8"));
  const registration = policy.observation_schema_registrations[0];
  const adapter = policy.adapter_registrations[0];
  const observation = migration.obligations[0].observations[0];
  assert.deepEqual({ type: schema.type, schema_ref: schema.schema_ref }, { type, schema_ref: schemaRef });
  assert.deepEqual({ type: registration.type, schema_ref: registration.schema_ref }, { type, schema_ref: schemaRef });
  assert.deepEqual(policy.named_presets[0].observations[0].type, type);
  assert.deepEqual(adapter.observation_types, [type]);
  assert.deepEqual(adapter.payload_schemas[0], { type, schema_ref: schemaRef, schema_digest: registration.schema_digest });
  assert.deepEqual(manifest.supported_observations[0], adapter.payload_schemas[0]);
  assert.deepEqual(observation.type, type);
  assert.deepEqual(observation.payload_schema, { schema_ref: schemaRef });
  assert.deepEqual({ observation_type: runIndex.observation_type, schema_ref: runIndex.schema_ref, schema_path: runIndex.schema_path }, { observation_type: type, schema_ref: schemaRef, schema_path: schemaPath });
  assert.equal(existsSync(path.join(value.root, `.planr/evidence/schemas/com.planr.planr2.ac014.terminal_arm.${short}.v1.schema.json`)), false);
}

function git(root, revision) {
  return execFileSync("git", ["rev-parse", revision], { cwd: root, encoding: "utf8" }).trim();
}

function shaFile(file) {
  return `sha256:${createHash("sha256").update(readFileSync(file)).digest("hex")}`;
}
