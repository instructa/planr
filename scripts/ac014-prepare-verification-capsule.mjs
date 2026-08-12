#!/usr/bin/env node
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { DatabaseSync } from "node:sqlite";

const FIXED = Object.freeze({
  plan_id: "pln-8bd38ca8",
  feature_run_id: "frun-0e9e5adc",
  preflight_item_id: "i-build-hash-freeze-and-independen-3f3c",
  verification_item_id: "i-execute-exactly-one-fresh-isolat-bde0",
  gate_id: "gate-149bc71f",
  baseline_root: "/Users/kregenrek/projects/planr-dogfood/outcome-batching-ac014-alpha4-baseline-final",
  baseline_revision: "9fdc5cbc38f5f015928deabddf1acc2088c4596b",
  baseline_tree: "ea536b31b023e41a8f364a219ff6ae934036ace2",
  oracle_plan_id: "pln-d1031732",
  prompt_digest: "sha256:2c45ff5939cbabfef798598ccb32f7448e1f565615eeb44ede273e420eadcef0",
  spec_digest: "sha256:cde84864a4708343de26d291585812a66de896a54495502d3c89b0b1a403c64f",
  oracle_digest: "sha256:730da278890bebc354649ea83ea757ea0b6cd7b17cf8e9d564e90ab2d6245b08",
  ceilings: Object.freeze({ wall_time_seconds: 998.015, total_tokens: 5977896, tool_call_envelopes: 93 }),
});

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const requestPath = valueAfter("--request", process.argv.slice(2));
  if (!requestPath) throw new Error("usage: node scripts/ac014-prepare-verification-capsule.mjs --request <request.json>");
  const result = prepareVerificationCapsule(JSON.parse(readFileSync(path.resolve(requestPath), "utf8")));
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

export function prepareVerificationCapsule(request, injected = {}) {
  requireRequest(request);
  const repoRoot = canonicalDirectory(request.repo_root, "repo_root");
  const planrBinary = immutableBinary(request.planr_binary, request.planr_binary_sha256);
  const sourceRevision = git(repoRoot, ["rev-parse", "HEAD"]);
  const sourceTree = git(repoRoot, ["rev-parse", "HEAD^{tree}"]);
  if (git(repoRoot, ["status", "--porcelain"]) !== "") fail("candidate source must be clean");
  if (path.basename(path.dirname(planrBinary)) !== sourceRevision) {
    fail("candidate binary path must be SHA-bound to the current source revision");
  }

  const trace = injected.trace ?? runPlanr(planrBinary, repoRoot, ["trace", "item", FIXED.verification_item_id, "--json"]);
  const state = trace.execution_state;
  const gate = state?.review_gate;
  const binding = state?.review_source_binding;
  const attempts = state?.review_attempts ?? [];
  if (!gate || gate.id !== FIXED.gate_id || gate.status !== "accepted") fail("stale or unaccepted ReviewGate");
  if (gate.latest_attempt !== request.accepted_review_attempt_number) fail("stale accepted ReviewGate attempt number");
  const accepted = attempts.find((entry) => entry.attempt_number === gate.latest_attempt);
  if (!accepted || accepted.verdict !== "accepted" || accepted.id !== request.accepted_review_attempt_id) {
    fail("stale accepted ReviewGate attempt identity");
  }
  if (!binding || binding.gate_id !== gate.id || binding.freeze_id !== request.accepted_source_freeze_id
    || binding.source_revision !== accepted.source_revision || binding.source_digest !== request.accepted_source_digest) {
    fail("stale accepted source freeze binding");
  }
  if (request.accepted_source_revision !== accepted.source_revision) fail("stale accepted source revision");
  const acceptedSourceTree = injected.acceptedSourceTree
    ?? git(repoRoot, ["rev-parse", `${accepted.source_revision}^{tree}`]);
  if (request.accepted_source_tree !== acceptedSourceTree) fail("stale accepted source tree");
  if (request.review_required !== true) fail("review_required must be true");
  if (state?.feature_run?.id !== FIXED.feature_run_id || state.feature_run.phase !== "implementation") {
    fail("FeatureRun must be in the maker-owned implementation repair phase");
  }
  if (state.owner?.worker_id !== "cumulative-recovery-maker" || state.owner?.role !== "maker") {
    fail("capsule preparation requires cumulative-recovery-maker ownership");
  }

  const proofAttempts = trace.proof?.attempts ?? [];
  const proofReceipts = trace.proof?.receipts ?? [];
  const durableAttempts = injected.attempts ?? runPlanr(planrBinary, repoRoot, ["evidence", "attempts", "--obligation", request.superseded_obligation_id, "--json"]);
  const durableReceipts = injected.receipts ?? runPlanr(planrBinary, repoRoot, ["evidence", "receipts", "--obligation", request.superseded_obligation_id, "--json"]);
  const durableClaims = injected.claims ?? readOneShotClaims(repoRoot, FIXED.feature_run_id);
  if (proofAttempts.length !== 0 || proofReceipts.length !== 0
    || records(durableAttempts, "attempts").length !== 0 || records(durableReceipts, "receipts").length !== 0
    || durableClaims.length !== 0) {
    fail("prior Evidence claim, attempt, or receipt makes this one-shot capsule ineligible");
  }

  const baselineRoot = canonicalDirectory(request.baseline_root, "baseline_root");
  if (baselineRoot !== FIXED.baseline_root || git(baselineRoot, ["rev-parse", "HEAD"]) !== FIXED.baseline_revision
    || git(baselineRoot, ["rev-parse", "HEAD^{tree}"]) !== FIXED.baseline_tree) {
    fail("matched AC-014 baseline identity mismatch");
  }
  const oraclePath = canonicalFile(path.join(repoRoot, "scripts/ac014-exact-product-oracle.mjs"), "oracle");
  if (shaFile(oraclePath) !== FIXED.oracle_digest) fail("fixed AC-014 oracle digest mismatch");
  const runnerPath = canonicalFile(path.join(repoRoot, "scripts/ac014-fresh-arm-runner.mjs"), "runner");

  const freshRoot = freshAbsolute(request.fresh_root, "fresh_root");
  const resultPath = freshAbsolute(request.result_path, "result_path");
  const codexHome = freshAbsolute(request.codex_home, "codex_home");
  const artifactDir = requiredString(request.artifact_dir, "artifact_dir");
  if (path.isAbsolute(artifactDir) || artifactDir.split(path.sep).includes("..")) fail("artifact_dir must be a contained relative path");
  const freshArtifactPath = path.join(freshRoot, artifactDir);
  for (const [label, value] of [["fresh_root", freshRoot], ["result_path", resultPath], ["codex_home", codexHome], ["artifact path", freshArtifactPath]]) {
    if (existsSync(value)) fail(`${label} must be a fresh never-used path: ${value}`);
  }
  if (new Set([freshRoot, resultPath, codexHome, freshArtifactPath]).size !== 4) fail("fresh capsule paths must be distinct");

  const short = sourceRevision.slice(0, 12);
  const ids = identities(short);
  const reportPath = path.resolve(repoRoot, `.planr/artifacts/planr2-ac014/verification-report-${FIXED.plan_id}-${short}.json`);
  if (existsSync(reportPath)) fail(`terminal report path must be unused: ${reportPath}`);
  const config = buildEvidenceConfiguration({ ids, sourceRevision, sourceTree, planrBinary, reportPath, request });

  // Policy/schema/manifest are repository configuration. Publish them together before probing.
  publishJsonSet(repoRoot, new Map([
    [".planr/evidence.yaml", config.policy],
    [config.schemaPath, config.schema],
    [config.manifestPath, config.manifest],
    [config.migrationPath, config.migration],
    [config.configuredRunIndexPath, config.configuredRunIndex],
  ]));

  const capabilityList = typeof injected.capabilityList === "function"
    ? injected.capabilityList(config)
    : injected.capabilityList ?? runPlanr(planrBinary, repoRoot, ["evidence", "capability", "list", "--json"]);
  const instance = findAvailableCapability(capabilityList, ids.manifest, config.manifestDigest);
  const migrationAbsolute = path.join(repoRoot, config.migrationPath);
  const migrationPreview = injected.migrationPreview
    ?? runPlanr(planrBinary, repoRoot, ["evidence", "migrate", "--input", migrationAbsolute, "--json"]);
  const migrationApplied = injected.migrationApplied
    ?? runPlanr(planrBinary, repoRoot, ["evidence", "migrate", "--input", migrationAbsolute, "--apply", "--json"]);
  if (migrationPreview.ok === false || migrationApplied.ok === false) fail("AC-014 Evidence migration did not settle successfully");
  const runInputPath = `.planr/evidence/obligations/${ids.obligation}.run-input.json`;
  const runnerInputPath = `.planr/artifacts/planr2-ac014/fresh-arm-input-${FIXED.plan_id}-${short}-cli0147.json`;
  const handoffPath = `.planr/artifacts/planr2-ac014/verifier-handoff-${FIXED.plan_id}-${short}.json`;
  for (const candidate of [runInputPath, runnerInputPath, handoffPath]) {
    if (existsSync(path.join(repoRoot, candidate))) fail(`capsule output already exists: ${candidate}`);
  }

  const controlHandoff = {
    schema_version: "planr.ac014.control_handoff.v1",
    plan_id: FIXED.plan_id,
    preflight_item_id: FIXED.preflight_item_id,
    verification_item_id: FIXED.verification_item_id,
    obligation_id: ids.obligation,
    policy_digest: config.policy.policy_digest,
    review_required: true,
    accepted_fix_review_gate_id: FIXED.gate_id,
    accepted_review_attempt_id: accepted.id,
    accepted_review_attempt_number: accepted.attempt_number,
    source_freeze: {
      id: binding.freeze_id,
      source_revision: binding.source_revision,
      source_tree: acceptedSourceTree,
      source_digest: binding.source_digest,
    },
    planr_candidate: {
      root: repoRoot,
      source_revision: sourceRevision,
      source_tree: sourceTree,
      binary_path: planrBinary,
      binary_sha256: request.planr_binary_sha256,
      accepted_fix_review_gate_id: FIXED.gate_id,
    },
  };
  const runnerInput = {
    schema_version: "planr.ac014.fresh_arm_run.v1",
    control_handoff: controlHandoff,
    baseline_root: baselineRoot,
    fresh_root: freshRoot,
    db_path: ".planr/planr.sqlite",
    planr_bin: planrBinary,
    project_id: "p-b3f1c75f",
    evidence_prepare_commands: [[process.execPath, path.join(repoRoot, "scripts/ac014-configure-sparziele-evidence.mjs"), FIXED.oracle_plan_id, planrBinary, request.planr_binary_sha256]],
    evidence_migration_input: ".planr/evidence/obligations/sparziele.migration.json",
    prompt_path: "ALPHA4_PROMPT.txt",
    spec_path: "DOGFOOD_SPEC.md",
    codex_home: codexHome,
    codex_surface: "identical",
    oracle_id: "sparziele-exact-product-flow-v1",
    oracle_plan_id: FIXED.oracle_plan_id,
    fixed_contract: {
      candidate_sha: FIXED.baseline_revision,
      candidate_version: "1.10.0-alpha.4",
      candidate_binary_sha256: request.planr_binary_sha256,
      prompt_digest: FIXED.prompt_digest,
      spec_digest: FIXED.spec_digest,
      model: "gpt-5.6-sol",
      effort: "medium",
      surface: "identical",
      cli_version: "0.147.0",
      oracle_id: "sparziele-exact-product-flow-v1",
      oracle_plan_id: FIXED.oracle_plan_id,
      oracle_sha256: FIXED.oracle_digest,
    },
    ceilings: FIXED.ceilings,
    monitor_poll_ms: 250,
    copy_excludes: ["target", "node_modules"],
    artifact_dir: artifactDir,
    oracle_command: [oraclePath],
  };
  const runInput = {
    obligation_id: ids.obligation,
    capability_instance_id: instance.id,
    target: config.target,
    environment: instance.capability?.environment ?? instance.host_fingerprint?.environment,
    fixture_disclosure: { fixtures_used: false, mocks_used: false },
    execution_contract: config.execution,
  };
  const runnerArgv = [process.execPath, runnerPath, "--input", path.join(repoRoot, runnerInputPath), "--result", resultPath];
  const evidenceArgv = [planrBinary, "evidence", "run", "--input", path.join(repoRoot, runInputPath), "--json"];
  const handoff = {
    schema_version: "planr.ac014.verifier_handoff.v4",
    status: "pending_independent_review",
    plan_id: FIXED.plan_id,
    feature_run_id: FIXED.feature_run_id,
    verification_item_id: FIXED.verification_item_id,
    review_required: true,
    accepted_preparation_review: { gate_id: gate.id, attempt_id: accepted.id, attempt_number: accepted.attempt_number, source_freeze: controlHandoff.source_freeze },
    candidate: controlHandoff.planr_candidate,
    fresh_identity: { fresh_root: freshRoot, result_path: resultPath, artifact_path: freshArtifactPath, codex_home: codexHome, terminal_report_path: reportPath },
    evidence: {
      obligation_id: ids.obligation,
      manifest_id: ids.manifest,
      capability_instance_id: instance.id,
      migration_path: config.migrationPath,
      configured_run_index_path: config.configuredRunIndexPath,
      migration_preview: migrationSummary(migrationPreview),
      migration_applied: migrationSummary(migrationApplied),
    },
    commands: {
      runner: { executable: runnerArgv[0], argv: runnerArgv.slice(1), executable_sha256: shaFile(runnerArgv[0]), path_lookup_allowed: false },
      evidence_run: { executable: evidenceArgv[0], argv: evidenceArgv.slice(1), executable_sha256: request.planr_binary_sha256, path_lookup_allowed: false },
    },
    execution_guard: { launch_allowed: false, requires_same_gate_acceptance_for_candidate_source: true, runner_invocations: 0, evidence_attempts: 0, evidence_receipts: 0 },
  };

  mkdirSync(codexHome, { recursive: false, mode: 0o700 });
  publishJsonSet(repoRoot, new Map([[runnerInputPath, runnerInput], [runInputPath, runInput], [handoffPath, handoff]]), true);
  return { status: "prepared_pending_review", source_revision: sourceRevision, source_tree: sourceTree, policy_digest: config.policy.policy_digest, obligation_id: ids.obligation, runner_input: runnerInputPath, run_input: runInputPath, verifier_handoff: handoffPath };
}

function buildEvidenceConfiguration({ ids, sourceRevision, sourceTree, planrBinary, reportPath, request }) {
  const type = `com.planr.planr2.ac014.terminal_arm.${sourceRevision.slice(0, 12)}.v1`;
  const schemaRef = `schema://${type}`;
  const target = { kind: "process", uri: `local://planr2/ac014/${FIXED.plan_id}/${sourceRevision.slice(0, 12)}` };
  const exact = {
    status: "passed", plan_id: FIXED.plan_id, verification_item_id: FIXED.verification_item_id,
    accepted_gate_id: FIXED.gate_id, accepted_review_attempt_id: request.accepted_review_attempt_id,
    baseline_sha: FIXED.baseline_revision, planr_candidate_source_revision: sourceRevision,
    planr_candidate_source_tree: sourceTree, planr_candidate_binary_sha256: request.planr_binary_sha256,
    oracle_plan_id: FIXED.oracle_plan_id, runner_invocations: 1, codex_exec_invocations: 1,
    evidence_adapter_runs: 1, retry: false, resume: false, cleanup_status: "passed",
  };
  const properties = Object.fromEntries(Object.entries(exact).map(([key, value]) => [key, { const: value }]));
  properties.terminal_result = { enum: ["passed", "failed", "external_invalid"] };
  properties.classification = { enum: ["green", "product", "instrumentation", "external_restriction"] };
  properties.oracle_status = { enum: ["passed", "failed", "not_started"] };
  properties.report_digest = { type: "string", pattern: "^sha256:[0-9a-f]{64}$" };
  const required = [...Object.keys(exact), "terminal_result", "classification", "oracle_status", "report_digest"];
  const schema = { schema_version: "evidence.contract.v1", type, schema_ref: schemaRef, json_schema: { type: "object", required, additionalProperties: false, properties } };
  const schemaDigest = shaJson(schema);
  const validator = terminalValidator(reportPath, exact);
  const execution = { kind: "process", executable: process.execPath, args: ["-e", validator], working_directory: ".", timeout_ms: 5000, stdout_limit_bytes: 8192, stderr_limit_bytes: 4096, payload_schema: { type, schema_ref: schemaRef, schema_digest: schemaDigest } };
  const adapterDigest = shaJson({ schema_version: "planr.process_adapter.binding.v1", execution_contract: execution, file_arguments: [] });
  const manifest = {
    id: ids.manifest, schema_version: "evidence.contract.v1", version: `1.0.0-${sourceRevision.slice(0, 12)}`,
    adapter_kind: "process", adapter_digest: adapterDigest, supported_surfaces: ["local-process"],
    supported_observations: [execution.payload_schema], supported_interactions: ["process"],
    supported_artifacts: ["stdout", "planr.generic_adapter_predicate.v1"], runtime_targets: [{ kind: "process", id: ids.runtime }],
    provenance_path: "planr_observed_execution", permissions: { network: "none", filesystem: "read_workspace" }, costs: {},
    determinism: "deterministic", repeatability: "non_repeatable_one_shot",
    independence: "validates the exact reviewed candidate and terminal one-shot report",
    blind_spots: ["cannot satisfy before the separately leased verifier completes the authorized one-shot arm"],
    availability_probe: { kind: "process", execution },
  };
  const manifestDigest = shaJson(manifest);
  const policy = {
    id: ids.policy, schema_version: "evidence.contract.v1",
    defaults: { preset_id: ids.preset, binding: true, assurance_level: "standard" },
    named_presets: [{ id: ids.preset, schema_version: "evidence.contract.v1", namespace: ids.namespace, observations: [{ id: ids.observation, type, subject: "accepted exact-source one-shot AC-014 terminal report", expected: { status: "passed" }, target }] }],
    observation_schema_registrations: [{ type, schema_ref: schemaRef, schema_digest: schemaDigest, owning_namespace: ids.namespace }],
    adapter_registrations: [{ manifest_id: ids.manifest, manifest_path: ids.manifestPath, manifest_digest: manifestDigest, observation_types: [type], payload_schemas: [execution.payload_schema], provenance_path: "planr_observed_execution", execution_contract: execution }],
    extension_namespaces: [ids.namespace], trust_policy: { accepted_provenance: ["planr_observed_execution"], min_receipt_status: "trusted", allow_user_attestation: false },
    freshness_policy: { max_age_seconds: 3600, invalidate_on: ["source_change", "target_change", "policy_change", "adapter_schema_change", "configuration_change"] },
    fixture_policy: { fixtures_allowed: false, mocks_allowed: false, disclosure_required: true },
    completion_policy: { require_satisfied_or_waived: true, allow_inconclusive_completion: false, require_review_evidence: true },
    layering_policy: { mode: "monotonic_strengthening", weakening_requires_waiver: true, layers: [{ scope: { kind: "plan", id: FIXED.plan_id }, policy_digest: `sha256:${"a".repeat(64)}` }] },
  };
  policy.policy_digest = shaJson(policy);
  const observation = { id: ids.requirement, type, subject: "One terminal result bound to the accepted gate, exact candidate, and canonical verification outcome", expected: exact, target, payload_schema: { schema_ref: schemaRef } };
  const migration = { schema_version: "planr.evidence.migration.v1", plan_id: FIXED.plan_id, obligations: [{ id: ids.obligation, schema_version: "evidence.contract.v1", criterion_id: "AC014-CUMULATIVE-ONE-TERMINAL-ARM", plan_id: FIXED.plan_id, item_id: FIXED.verification_item_id, title: `Accepted exact Planr 2.0 candidate one-shot AC-014 terminal arm (${sourceRevision.slice(0, 12)})`, binding: true, observations: [observation], fixture_policy: policy.fixture_policy, freshness_policy: policy.freshness_policy, assurance_policy: {}, supersedes: request.superseded_obligation_id }] };
  const configuredRunIndex = { schema_version: "planr.ac014.evidence_run_configuration.v1", plan_id: FIXED.plan_id, obligation_id: ids.obligation, manifest_id: ids.manifest, target, review_required: true };
  return { policy, schema, manifest, migration, configuredRunIndex, schemaPath: ids.schemaPath, manifestPath: ids.manifestPath, migrationPath: `.planr/evidence/obligations/${ids.obligation}.migration.json`, configuredRunIndexPath: `.planr/evidence/obligations/${ids.obligation}.run-index.json`, manifestDigest, execution, target };
}

function terminalValidator(reportPath, exact) {
  return `const fs=require("node:fs"),crypto=require("node:crypto");if(!process.env.PLANR_EVIDENCE_TARGET_JSON){process.stdout.write(JSON.stringify({probe:true}));process.exit(0)}const bytes=fs.readFileSync(${JSON.stringify(reportPath)}),r=JSON.parse(bytes);const exact=${JSON.stringify(exact)};for(const [k,v] of Object.entries(exact))if(r[k]!==v)throw new Error("AC-014 terminal report identity mismatch: "+k);if(!new Set(["passed","failed","external_invalid"]).has(r.terminal_result)||!new Set(["green","product","instrumentation","external_restriction"]).has(r.classification)||!new Set(["passed","failed","not_started"]).has(r.oracle_status))throw new Error("AC-014 terminal report classification mismatch");process.stdout.write(JSON.stringify({...exact,terminal_result:r.terminal_result,classification:r.classification,oracle_status:r.oracle_status,report_digest:"sha256:"+crypto.createHash("sha256").update(bytes).digest("hex")}));`;
}

function identities(short) {
  const namespace = `com.planr.planr2.ac014.terminal_arm.${short}`;
  return {
    namespace, policy: `epolicy-planr2-ac014-${short}-v1`, preset: `preset-verifier-planr2-ac014-${short}-v1`,
    observation: `obs-verifier-planr2-ac014-${short}-v1`, requirement: `obs-planr2-ac014-${short}-v1`,
    obligation: `pob-planr2-ac014-${short}-v1`, manifest: `verifier-planr2-ac014-${short}-v1`, runtime: `runtime-planr2-ac014-${short}-v1`,
    manifestPath: `.planr/evidence/adapters/verifier-planr2-ac014-${short}-v1.manifest.json`,
    schemaPath: `.planr/evidence/schemas/${namespace}.v1.schema.json`,
  };
}

function findAvailableCapability(result, manifestId, manifestDigest) {
  const instances = result.object?.instances ?? result.instances ?? [];
  const instance = instances.find((entry) => entry.manifest_id === manifestId && entry.manifest_digest === manifestDigest && (entry.availability_status === "available" || entry.capability?.availability?.status === "available"));
  if (!instance) fail(`configured capability is unavailable: ${manifestId}`);
  if (!instance.capability?.environment && !instance.host_fingerprint?.environment) fail("configured capability is missing a sealed environment identity");
  return instance;
}

function requireRequest(request) {
  if (!request || request.schema_version !== "planr.ac014.capsule_preparation_request.v1") fail("invalid capsule preparation request schema");
  for (const key of ["repo_root", "planr_binary", "planr_binary_sha256", "accepted_review_attempt_id", "accepted_source_freeze_id", "accepted_source_digest", "accepted_source_revision", "accepted_source_tree", "superseded_obligation_id", "baseline_root", "fresh_root", "result_path", "artifact_dir", "codex_home"]) requiredString(request[key], key);
  if (!Number.isInteger(request.accepted_review_attempt_number) || request.accepted_review_attempt_number < 1) fail("accepted_review_attempt_number must be positive");
  if (!/^[0-9a-f]{40}$/.test(request.accepted_source_revision) || !/^[0-9a-f]{40}$/.test(request.accepted_source_tree)) fail("accepted source revision and tree must be exact git identities");
  if (!/^sha256:[0-9a-f]{64}$/.test(request.accepted_source_digest)) fail("accepted source digest must be exact");
}

function runPlanr(binary, cwd, args) {
  const result = spawnSync(binary, args, { cwd, encoding: "utf8", env: { ...process.env, PLANR_WORKER_ID: "cumulative-recovery-maker" } });
  let parsed;
  try {
    parsed = JSON.parse(result.stdout);
  } catch {
    fail(`Planr command did not return a structured envelope: ${args.join(" ")}\n${result.stderr || result.stdout}`);
  }
  const envelopeSucceeded = parsed.ok === true && parsed.exit?.code === 0;
  const legacyDirectSucceeded = !("ok" in parsed) && result.status === 0;
  if (!envelopeSucceeded && !legacyDirectSucceeded) {
    fail(`Planr command failed: ${args.join(" ")}\n${result.stderr || result.stdout}`);
  }
  return parsed;
}

function records(value, key) {
  return value.object?.[key] ?? value[key] ?? [];
}

function migrationSummary(value) {
  const object = value.object ?? value;
  return { dry_run: object.dry_run ?? null, summary: object.summary ?? null, status: object.status ?? "passed" };
}

function readOneShotClaims(repoRoot, runId) {
  const database = new DatabaseSync(path.join(repoRoot, ".planr/planr.sqlite"), { readOnly: true });
  try {
    return database.prepare("SELECT freeze_id, obligation_id, capability_instance_id FROM feature_run_one_shot_claims WHERE run_id = ? ORDER BY freeze_id").all(runId);
  } finally {
    database.close();
  }
}

function immutableBinary(value, expectedDigest) {
  const file = canonicalFile(value, "planr_binary");
  const metadata = lstatSync(file);
  if (metadata.isSymbolicLink() || (metadata.mode & 0o777) !== 0o555) fail("candidate Planr binary must be a regular non-symlink 0555 file");
  if (!/^sha256:[0-9a-f]{64}$/.test(expectedDigest) || shaFile(file) !== expectedDigest) fail("candidate Planr binary digest mismatch");
  return file;
}

function canonicalDirectory(value, label) {
  const resolved = path.resolve(requiredString(value, label));
  try {
    if (!statSync(resolved).isDirectory() || realpathSync(resolved) !== resolved) fail(`${label} must be an existing canonical directory`);
  } catch (error) {
    if (String(error.message).startsWith("AC-014 capsule preparation rejected:")) throw error;
    fail(`${label} must be an existing canonical directory`);
  }
  return resolved;
}

function canonicalFile(value, label) {
  const resolved = path.resolve(requiredString(value, label));
  try {
    const metadata = lstatSync(resolved);
    if (!metadata.isFile() || metadata.isSymbolicLink() || realpathSync(resolved) !== resolved) fail(`${label} must be an existing canonical regular file`);
  } catch (error) {
    if (String(error.message).startsWith("AC-014 capsule preparation rejected:")) throw error;
    fail(`${label} must be an existing canonical regular file`);
  }
  return resolved;
}

function freshAbsolute(value, label) {
  if (!path.isAbsolute(value)) fail(`${label} must be absolute`);
  return path.resolve(value);
}

function git(root, args) {
  const result = spawnSync("git", ["-C", root, ...args], { encoding: "utf8" });
  if (result.status !== 0) fail(`git ${args.join(" ")} failed: ${result.stderr}`);
  return result.stdout.trim();
}

function publishJsonSet(root, files, exclusive = false) {
  const stage = path.join(root, ".planr", `.ac014-capsule-stage-${process.pid}`);
  if (existsSync(stage)) fail(`staging path already exists: ${stage}`);
  mkdirSync(stage, { recursive: true, mode: 0o700 });
  try {
    for (const [relative, value] of files) {
      const destination = path.join(root, relative);
      if (exclusive && existsSync(destination)) fail(`immutable capsule output already exists: ${relative}`);
      const staged = path.join(stage, relative);
      mkdirSync(path.dirname(staged), { recursive: true });
      writeFileSync(staged, `${JSON.stringify(value, null, 2)}\n`, { flag: "wx", mode: 0o600 });
    }
    for (const [relative] of files) {
      const destination = path.join(root, relative);
      mkdirSync(path.dirname(destination), { recursive: true });
      renameSync(path.join(stage, relative), destination);
    }
  } finally {
    rmSync(stage, { recursive: true, force: true });
  }
}

function shaFile(file) { return `sha256:${createHash("sha256").update(readFileSync(file)).digest("hex")}`; }
function shaJson(value) { return `sha256:${createHash("sha256").update(canonical(value)).digest("hex")}`; }
function canonical(value) {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`;
}
function requiredString(value, label) { if (typeof value !== "string" || value.length === 0) fail(`${label} must be a non-empty string`); return value; }
function valueAfter(flag, argv) { const index = argv.indexOf(flag); return index === -1 ? null : argv[index + 1]; }
function fail(message) { throw new Error(`AC-014 capsule preparation rejected: ${message}`); }
