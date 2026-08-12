#!/usr/bin/env node
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, linkSync, lstatSync, mkdirSync, readFileSync, realpathSync, renameSync, rmSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { DatabaseSync } from "node:sqlite";

const FIXED = Object.freeze({
  plan_id: "pln-8bd38ca8",
  feature_run_id: "frun-0e9e5adc",
  preflight_item_id: "i-build-hash-freeze-and-independen-3f3c",
  verification_item_id: "i-execute-exactly-one-fresh-isolat-bde0",
  gate_id: "gate-149bc71f",
  maker: "cumulative-recovery-maker",
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
  const argv = process.argv.slice(2);
  const draftRequest = valueAfter("--draft", argv);
  const sealDraft = valueAfter("--seal", argv);
  if (Boolean(draftRequest) === Boolean(sealDraft)) {
    throw new Error("usage: node scripts/ac014-prepare-verification-capsule.mjs (--draft <request.json> | --seal <review-draft.json>)");
  }
  const result = draftRequest
    ? prepareReviewDraft(JSON.parse(readFileSync(path.resolve(draftRequest), "utf8")))
    : sealLaunchCapsule(path.resolve(sealDraft));
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

export function prepareReviewDraft(request, injected = {}) {
  requireDraftRequest(request);
  const repoRoot = canonicalDirectory(request.repo_root, "repo_root");
  const planrBinary = immutableBinary(request.planr_binary, request.planr_binary_sha256);
  const sourceRevision = git(repoRoot, ["rev-parse", "HEAD"]);
  const sourceTree = git(repoRoot, ["rev-parse", "HEAD^{tree}"]);
  if (git(repoRoot, ["status", "--porcelain"]) !== "") fail("candidate source must be clean");
  if (path.basename(path.dirname(planrBinary)) !== sourceRevision) fail("candidate binary path must be SHA-bound to the current source revision");

  const trace = injected.trace ?? runPlanr(planrBinary, repoRoot, ["trace", "item", FIXED.verification_item_id, "--json"]);
  const state = trace.execution_state;
  const gate = state?.review_gate;
  if (!gate || gate.id !== FIXED.gate_id || !["pending", "changes_requested", "accepted"].includes(gate.status)) fail("missing current ReviewGate context");
  if (state?.feature_run?.id !== FIXED.feature_run_id || state.feature_run.phase !== "implementation") fail("review draft requires the maker-owned implementation phase");
  if (state.owner?.worker_id !== FIXED.maker || state.owner?.role !== "maker") fail("review draft requires cumulative-recovery-maker ownership");
  assertZeroUse(trace, request.superseded_obligation_id, planrBinary, repoRoot, injected);

  const baselineRoot = canonicalDirectory(request.baseline_root, "baseline_root");
  if (baselineRoot !== FIXED.baseline_root || git(baselineRoot, ["rev-parse", "HEAD"]) !== FIXED.baseline_revision
    || git(baselineRoot, ["rev-parse", "HEAD^{tree}"]) !== FIXED.baseline_tree) fail("matched AC-014 baseline identity mismatch");
  const oraclePath = canonicalFile(path.join(repoRoot, "scripts/ac014-exact-product-oracle.mjs"), "oracle");
  if (shaFile(oraclePath) !== FIXED.oracle_digest) fail("fixed AC-014 oracle digest mismatch");
  const runnerPath = canonicalFile(path.join(repoRoot, "scripts/ac014-fresh-arm-runner.mjs"), "runner");
  const freshIdentity = validateFreshIdentity(request, repoRoot);
  const short = sourceRevision.slice(0, 12);
  const ids = identities(short);
  const paths = runtimePaths(short);
  for (const candidate of Object.values(paths)) if (existsSync(path.join(repoRoot, candidate))) fail(`unused runtime artifact required: ${candidate}`);

  const config = buildEvidenceConfiguration({ ids, paths, sourceRevision, sourceTree, reportPath: freshIdentity.terminal_report_path, request });
  publishJsonSet(repoRoot, new Map([
    [".planr/evidence.yaml", config.policy],
    [config.schemaPath, config.schema],
    [config.manifestPath, config.manifest],
    [config.migrationPath, config.migration],
    [config.configuredRunIndexPath, config.configuredRunIndex],
  ]));
  const migrationAbsolute = path.join(repoRoot, config.migrationPath);
  const preview = injected.migrationPreview ?? runPlanr(planrBinary, repoRoot, ["evidence", "migrate", "--input", migrationAbsolute, "--json"]);
  const applied = injected.migrationApplied ?? runPlanr(planrBinary, repoRoot, ["evidence", "migrate", "--input", migrationAbsolute, "--apply", "--json"]);
  if (preview.ok === false || applied.ok === false) fail("AC-014 Evidence migration did not settle successfully");
  const configurationPaths = [".planr/evidence.yaml", config.schemaPath, config.manifestPath, config.migrationPath, config.configuredRunIndexPath];
  const configurationDigests = Object.fromEntries(configurationPaths.map((relative) => {
    const absolute = path.join(repoRoot, relative);
    chmodSync(absolute, 0o400);
    return [relative, shaFile(absolute)];
  }));

  const draft = {
    schema_version: "planr.ac014.review_draft.v1",
    status: "pending_exact_source_review",
    plan_id: FIXED.plan_id,
    feature_run_id: FIXED.feature_run_id,
    verification_item_id: FIXED.verification_item_id,
    gate_context: { id: gate.id, observed_status: gate.status, observed_latest_attempt: gate.latest_attempt },
    candidate: { root: repoRoot, source_revision: sourceRevision, source_tree: sourceTree, binary_path: planrBinary, binary_sha256: request.planr_binary_sha256 },
    fixed_contract: { baseline_root: baselineRoot, baseline_revision: FIXED.baseline_revision, baseline_tree: FIXED.baseline_tree, oracle_path: oraclePath, oracle_plan_id: FIXED.oracle_plan_id, ceilings: FIXED.ceilings },
    fresh_identity: freshIdentity,
    evidence: { obligation_id: ids.obligation, manifest_id: ids.manifest, policy_digest: config.policy.policy_digest, migration_path: config.migrationPath, configured_run_index_path: config.configuredRunIndexPath, configuration_digests: configurationDigests, migration_preview: migrationSummary(preview), migration_applied: migrationSummary(applied) },
    runtime_paths: paths,
    runner_path: runnerPath,
    launch_guard: { review_required: true, launch_allowed: false, authorization: null, claims: 0, attempts: 0, receipts: 0 },
    post_review_seal: { executable: process.execPath, argv: [path.join(repoRoot, "scripts/ac014-prepare-verification-capsule.mjs"), "--seal", path.join(repoRoot, paths.draft)], path_lookup_allowed: false },
  };
  publishJsonSet(repoRoot, new Map([[paths.draft, draft]]), true, true);
  return { status: draft.status, source_revision: sourceRevision, source_tree: sourceTree, policy_digest: config.policy.policy_digest, obligation_id: ids.obligation, review_draft: paths.draft, launch_allowed: false };
}

export function sealLaunchCapsule(draftPath, injected = {}) {
  const canonicalDraft = canonicalFile(draftPath, "review_draft");
  const draft = JSON.parse(readFileSync(canonicalDraft, "utf8"));
  requireReviewDraft(draft);
  const repoRoot = canonicalDirectory(draft.candidate.root, "candidate.root");
  if (!isPathInside(canonicalDraft, path.join(repoRoot, ".planr"))) fail("review draft must be Planr runtime state");
  const planrBinary = immutableBinary(draft.candidate.binary_path, draft.candidate.binary_sha256);
  if (git(repoRoot, ["status", "--porcelain"]) !== "" || git(repoRoot, ["rev-parse", "HEAD"]) !== draft.candidate.source_revision
    || git(repoRoot, ["rev-parse", "HEAD^{tree}"]) !== draft.candidate.source_tree) fail("reviewed candidate source identity changed");

  const trace = injected.trace ?? runPlanr(planrBinary, repoRoot, ["trace", "item", FIXED.verification_item_id, "--json"]);
  const state = trace.execution_state;
  const gate = state?.review_gate;
  const binding = state?.review_source_binding;
  const accepted = (state?.review_attempts ?? []).find((entry) => entry.attempt_number === gate?.latest_attempt);
  if (!gate || gate.id !== FIXED.gate_id || gate.status !== "accepted") fail("post-review seal requires the same accepted ReviewGate");
  if (!accepted || accepted.verdict !== "accepted" || accepted.reviewer_mode !== "independent"
    || accepted.reviewer_worker_id === FIXED.maker || accepted.source_revision !== draft.candidate.source_revision) fail("latest independently accepted attempt is not for the exact draft source");
  if (!binding || binding.gate_id !== gate.id || binding.source_revision !== draft.candidate.source_revision
    || !/^freeze-[0-9a-f]+$/.test(binding.freeze_id) || !/^sha256:[0-9a-f]{64}$/.test(binding.source_digest)) fail("accepted source freeze is missing, invalidated, or stale");
  const owner = state?.owner;
  if (state?.feature_run?.id !== FIXED.feature_run_id || state.feature_run.phase !== "verification"
    || owner?.role !== "verifier" || !requiredPositiveInteger(owner.lease_generation)
    || trace.item?.worker_id !== owner.worker_id || !["picked", "running"].includes(trace.item?.status)) fail("post-review seal requires the current verifier lease and picked item");
  assertZeroUse(trace, draft.evidence.obligation_id, planrBinary, repoRoot, injected);

  const ids = identities(draft.candidate.source_revision.slice(0, 12));
  const paths = draft.runtime_paths;
  for (const key of ["seal", "runner_input", "run_input", "handoff"]) if (existsSync(path.join(repoRoot, paths[key]))) fail(`launch capsule cannot be resealed or overwritten: ${paths[key]}`);
  for (const [relative, digest] of Object.entries(draft.evidence.configuration_digests ?? {})) {
    if (shaFile(canonicalFile(path.join(repoRoot, relative), `reviewed Evidence configuration ${relative}`)) !== digest) fail(`reviewed Evidence configuration changed after draft: ${relative}`);
  }
  if (Object.keys(draft.evidence.configuration_digests ?? {}).length !== 5) fail("reviewed Evidence configuration digest set is incomplete");
  assertUnusedFreshIdentity(draft.fresh_identity);
  const capabilityList = typeof injected.capabilityList === "function"
    ? injected.capabilityList(draft)
    : injected.capabilityList ?? runPlanr(planrBinary, repoRoot, ["evidence", "capability", "list", "--json"]);
  const instance = findAvailableCapability(capabilityList, ids.manifest);
  const authorization = {
    schema_version: "planr.ac014.launch_authorization.v1",
    launch_allowed: true,
    plan_id: FIXED.plan_id,
    feature_run_id: FIXED.feature_run_id,
    verification_item_id: FIXED.verification_item_id,
    obligation_id: ids.obligation,
    accepted_gate_id: gate.id,
    accepted_review_attempt_id: accepted.id,
    accepted_review_attempt_number: accepted.attempt_number,
    source_freeze: { id: binding.freeze_id, source_revision: binding.source_revision, source_tree: draft.candidate.source_tree, source_digest: binding.source_digest },
    verifier_lease: { worker_id: owner.worker_id, generation: owner.lease_generation },
    candidate: draft.candidate,
    fresh_identity: draft.fresh_identity,
    draft_path: path.relative(repoRoot, canonicalDraft),
    draft_digest: shaFile(canonicalDraft),
  };
  const controlHandoff = {
    schema_version: "planr.ac014.control_handoff.v1", plan_id: FIXED.plan_id, preflight_item_id: FIXED.preflight_item_id,
    verification_item_id: FIXED.verification_item_id, obligation_id: ids.obligation, policy_digest: draft.evidence.policy_digest,
    review_required: true, accepted_fix_review_gate_id: gate.id, accepted_review_attempt_id: accepted.id,
    accepted_review_attempt_number: accepted.attempt_number, source_freeze: authorization.source_freeze,
    planr_candidate: { ...draft.candidate, accepted_fix_review_gate_id: gate.id },
  };
  const runnerInput = buildRunnerInput(draft, controlHandoff, authorization, planrBinary);
  const config = readConfiguredEvidence(repoRoot, ids);
  const runInput = { obligation_id: ids.obligation, capability_instance_id: instance.id, target: config.target, environment: instance.capability?.environment ?? instance.host_fingerprint?.environment, fixture_disclosure: { fixtures_used: false, mocks_used: false }, execution_contract: config.execution };
  const runnerArgv = [process.execPath, draft.runner_path, "--input", path.join(repoRoot, paths.runner_input), "--result", draft.fresh_identity.result_path];
  const evidenceArgv = [planrBinary, "evidence", "run", "--input", path.join(repoRoot, paths.run_input), "--json"];
  const handoff = {
    schema_version: "planr.ac014.verifier_handoff.v5", status: "sealed_authorized", plan_id: FIXED.plan_id,
    feature_run_id: FIXED.feature_run_id, verification_item_id: FIXED.verification_item_id,
    launch_authorization: authorization, evidence: { ...draft.evidence, capability_instance_id: instance.id },
    commands: {
      runner: { executable: runnerArgv[0], argv: runnerArgv.slice(1), executable_sha256: shaFile(runnerArgv[0]), path_lookup_allowed: false },
      evidence_run: { executable: evidenceArgv[0], argv: evidenceArgv.slice(1), executable_sha256: draft.candidate.binary_sha256, path_lookup_allowed: false },
    },
    execution_guard: { launch_allowed: true, requires_live_revalidation: true, runner_invocations: 0, evidence_attempts: 0, evidence_receipts: 0 },
  };
  const seal = { ...authorization, handoff_path: paths.handoff, runner_input_path: paths.runner_input, run_input_path: paths.run_input };
  mkdirSync(draft.fresh_identity.codex_home, { recursive: false, mode: 0o700 });
  try {
    publishJsonSet(repoRoot, new Map([[paths.seal, seal], [paths.runner_input, runnerInput], [paths.run_input, runInput], [paths.handoff, handoff]]), true, true);
  } catch (error) {
    rmSync(draft.fresh_identity.codex_home, { recursive: false, force: true });
    throw error;
  }
  return { status: "sealed_authorized", accepted_review_attempt_id: accepted.id, source_freeze_id: binding.freeze_id, verifier_worker_id: owner.worker_id, verifier_lease_generation: owner.lease_generation, launch_seal: paths.seal, runner_input: paths.runner_input, run_input: paths.run_input, verifier_handoff: paths.handoff };
}

function buildEvidenceConfiguration({ ids, paths, sourceRevision, sourceTree, reportPath, request }) {
  const type = `${ids.namespace}.v1`;
  const schemaRef = `schema://${type}`;
  const target = { kind: "process", uri: `local://planr2/ac014/${FIXED.plan_id}/${sourceRevision.slice(0, 12)}` };
  const exact = { status: "passed", plan_id: FIXED.plan_id, verification_item_id: FIXED.verification_item_id, accepted_gate_id: FIXED.gate_id, baseline_sha: FIXED.baseline_revision, planr_candidate_source_revision: sourceRevision, planr_candidate_source_tree: sourceTree, planr_candidate_binary_sha256: request.planr_binary_sha256, oracle_plan_id: FIXED.oracle_plan_id, runner_invocations: 1, codex_exec_invocations: 1, evidence_adapter_runs: 1, retry: false, resume: false, cleanup_status: "passed" };
  const properties = Object.fromEntries(Object.entries(exact).map(([key, value]) => [key, { const: value }]));
  Object.assign(properties, {
    accepted_review_attempt_id: { type: "string", pattern: "^attempt-[0-9a-f]+$" }, accepted_review_attempt_number: { type: "integer", minimum: 1 },
    source_freeze_id: { type: "string", pattern: "^freeze-[0-9a-f]+$" }, source_digest: { type: "string", pattern: "^sha256:[0-9a-f]{64}$" },
    verifier_worker_id: { type: "string", minLength: 1 }, verifier_lease_generation: { type: "integer", minimum: 1 },
    terminal_result: { enum: ["passed", "failed", "external_invalid"] }, classification: { enum: ["green", "product", "instrumentation", "external_restriction"] },
    oracle_status: { enum: ["passed", "failed", "not_started"] }, report_digest: { type: "string", pattern: "^sha256:[0-9a-f]{64}$" },
  });
  const required = Object.keys(properties);
  const schema = { schema_version: "evidence.contract.v1", type, schema_ref: schemaRef, json_schema: { type: "object", required, additionalProperties: false, properties } };
  const schemaDigest = shaJson(schema);
  const validator = terminalValidator(reportPath, path.join(request.repo_root, paths.seal), exact);
  const execution = { kind: "process", executable: "node", args: ["-e", validator], working_directory: ".", timeout_ms: 5000, stdout_limit_bytes: 8192, stderr_limit_bytes: 4096, payload_schema: { type, schema_ref: schemaRef, schema_digest: schemaDigest } };
  const adapterDigest = shaJson({ schema_version: "planr.process_adapter.binding.v1", execution_contract: execution, file_arguments: [] });
  const manifest = { id: ids.manifest, schema_version: "evidence.contract.v1", version: `2.0.0-${sourceRevision.slice(0, 12)}`, adapter_kind: "process", adapter_digest: adapterDigest, supported_surfaces: ["local-process"], supported_observations: [execution.payload_schema], supported_interactions: ["process"], supported_artifacts: ["stdout", "planr.generic_adapter_predicate.v1"], runtime_targets: [{ kind: "process", id: ids.runtime }], provenance_path: "planr_observed_execution", permissions: { network: "none", filesystem: "read_workspace" }, costs: {}, determinism: "deterministic", repeatability: "non_repeatable_one_shot", independence: "validates the exact post-review launch seal and terminal one-shot report", blind_spots: ["cannot satisfy before the separately leased verifier seals and completes the authorized arm"], availability_probe: { kind: "process", execution } };
  const manifestDigest = shaJson(manifest);
  const policy = { id: ids.policy, schema_version: "evidence.contract.v1", defaults: { preset_id: ids.preset, binding: true, assurance_level: "standard" }, named_presets: [{ id: ids.preset, schema_version: "evidence.contract.v1", namespace: ids.namespace, observations: [{ id: ids.observation, type, subject: "post-review sealed exact-source one-shot AC-014 terminal report", expected: { status: "passed" }, target }] }], observation_schema_registrations: [{ type, schema_ref: schemaRef, schema_digest: schemaDigest, owning_namespace: ids.namespace }], adapter_registrations: [{ manifest_id: ids.manifest, manifest_path: ids.manifestPath, manifest_digest: manifestDigest, observation_types: [type], payload_schemas: [execution.payload_schema], provenance_path: "planr_observed_execution", execution_contract: execution }], extension_namespaces: [ids.namespace], trust_policy: { accepted_provenance: ["planr_observed_execution"], min_receipt_status: "trusted", allow_user_attestation: false }, freshness_policy: { max_age_seconds: 3600, invalidate_on: ["source_change", "target_change", "policy_change", "adapter_schema_change", "configuration_change"] }, fixture_policy: { fixtures_allowed: false, mocks_allowed: false, disclosure_required: true }, completion_policy: { require_satisfied_or_waived: true, allow_inconclusive_completion: false, require_review_evidence: true }, layering_policy: { mode: "monotonic_strengthening", weakening_requires_waiver: true, layers: [{ scope: { kind: "plan", id: FIXED.plan_id }, policy_digest: `sha256:${"a".repeat(64)}` }] } };
  policy.policy_digest = shaJson(policy);
  const observation = { id: ids.requirement, type, subject: "One terminal result bound to the post-review seal, exact candidate, and current verifier lease", expected: exact, target, payload_schema: { schema_ref: schemaRef } };
  const migration = { schema_version: "planr.evidence.migration.v1", plan_id: FIXED.plan_id, obligations: [{ id: ids.obligation, schema_version: "evidence.contract.v1", criterion_id: "AC014-CUMULATIVE-ONE-TERMINAL-ARM", plan_id: FIXED.plan_id, item_id: FIXED.verification_item_id, title: `Post-review sealed Planr 2.0 one-shot AC-014 terminal arm (${sourceRevision.slice(0, 12)})`, binding: true, observations: [observation], fixture_policy: policy.fixture_policy, freshness_policy: policy.freshness_policy, assurance_policy: {}, supersedes: request.superseded_obligation_id }] };
  const configuredRunIndex = { schema_version: "planr.ac014.evidence_run_configuration.v2", plan_id: FIXED.plan_id, obligation_id: ids.obligation, manifest_id: ids.manifest, target, review_required: true, post_review_seal_path: paths.seal };
  return { policy, schema, manifest, migration, configuredRunIndex, schemaPath: ids.schemaPath, manifestPath: ids.manifestPath, migrationPath: `.planr/evidence/obligations/${ids.obligation}.migration.json`, configuredRunIndexPath: `.planr/evidence/obligations/${ids.obligation}.run-index.json` };
}

function terminalValidator(reportPath, sealPath, exact) {
  return `const fs=require("node:fs"),crypto=require("node:crypto");if(!process.env.PLANR_EVIDENCE_TARGET_JSON){process.stdout.write(JSON.stringify({probe:true}));process.exit(0)}const bytes=fs.readFileSync(${JSON.stringify(reportPath)}),r=JSON.parse(bytes),s=JSON.parse(fs.readFileSync(${JSON.stringify(sealPath)},"utf8")),exact=${JSON.stringify(exact)};if(s.schema_version!=="planr.ac014.launch_authorization.v1"||s.launch_allowed!==true)throw new Error("AC-014 launch seal is not authorized");for(const [k,v] of Object.entries(exact))if(r[k]!==v)throw new Error("AC-014 terminal report identity mismatch: "+k);const live={accepted_review_attempt_id:s.accepted_review_attempt_id,accepted_review_attempt_number:s.accepted_review_attempt_number,source_freeze_id:s.source_freeze.id,source_digest:s.source_freeze.source_digest,verifier_worker_id:s.verifier_lease.worker_id,verifier_lease_generation:s.verifier_lease.generation};for(const [k,v] of Object.entries(live))if(r[k]!==v)throw new Error("AC-014 terminal report launch binding mismatch: "+k);if(!new Set(["passed","failed","external_invalid"]).has(r.terminal_result)||!new Set(["green","product","instrumentation","external_restriction"]).has(r.classification)||!new Set(["passed","failed","not_started"]).has(r.oracle_status))throw new Error("AC-014 terminal report classification mismatch");process.stdout.write(JSON.stringify({...exact,...live,terminal_result:r.terminal_result,classification:r.classification,oracle_status:r.oracle_status,report_digest:"sha256:"+crypto.createHash("sha256").update(bytes).digest("hex")}));`;
}

function buildRunnerInput(draft, controlHandoff, authorization, planrBinary) {
  return { schema_version: "planr.ac014.fresh_arm_run.v1", control_handoff: controlHandoff, launch_authorization: authorization, baseline_root: draft.fixed_contract.baseline_root, fresh_root: draft.fresh_identity.fresh_root, result_output_path: draft.fresh_identity.result_path, terminal_report_path: draft.fresh_identity.terminal_report_path, db_path: ".planr/planr.sqlite", planr_bin: planrBinary, project_id: "p-b3f1c75f", evidence_prepare_commands: [[process.execPath, path.join(draft.candidate.root, "scripts/ac014-configure-sparziele-evidence.mjs"), FIXED.oracle_plan_id, planrBinary, draft.candidate.binary_sha256]], evidence_migration_input: ".planr/evidence/obligations/sparziele.migration.json", prompt_path: "ALPHA4_PROMPT.txt", spec_path: "DOGFOOD_SPEC.md", codex_home: draft.fresh_identity.codex_home, codex_surface: "identical", oracle_id: "sparziele-exact-product-flow-v1", oracle_plan_id: FIXED.oracle_plan_id, fixed_contract: { candidate_sha: FIXED.baseline_revision, candidate_version: "1.10.0-alpha.4", candidate_binary_sha256: draft.candidate.binary_sha256, prompt_digest: FIXED.prompt_digest, spec_digest: FIXED.spec_digest, model: "gpt-5.6-sol", effort: "medium", surface: "identical", cli_version: "0.147.0", oracle_id: "sparziele-exact-product-flow-v1", oracle_plan_id: FIXED.oracle_plan_id, oracle_sha256: FIXED.oracle_digest }, ceilings: FIXED.ceilings, monitor_poll_ms: 250, copy_excludes: ["target", "node_modules"], artifact_dir: path.relative(draft.fresh_identity.fresh_root, draft.fresh_identity.artifact_path), oracle_command: [draft.fixed_contract.oracle_path] };
}

function readConfiguredEvidence(root, ids) {
  const policy = JSON.parse(readFileSync(path.join(root, ".planr/evidence.yaml"), "utf8"));
  const registration = policy.adapter_registrations?.find((entry) => entry.manifest_id === ids.manifest);
  const preset = policy.named_presets?.find((entry) => entry.id === ids.preset);
  if (!registration || !preset?.observations?.[0]) fail("reviewed Evidence configuration is missing at seal time");
  return { execution: registration.execution_contract, target: preset.observations[0].target };
}

function assertZeroUse(trace, obligationId, binary, root, injected) {
  const attempts = injected.attempts ?? runPlanr(binary, root, ["evidence", "attempts", "--obligation", obligationId, "--json"]);
  const receipts = injected.receipts ?? runPlanr(binary, root, ["evidence", "receipts", "--obligation", obligationId, "--json"]);
  const claims = injected.claims ?? readOneShotClaims(root, FIXED.feature_run_id);
  if ((trace.proof?.attempts ?? []).length || (trace.proof?.receipts ?? []).length || records(attempts, "attempts").length || records(receipts, "receipts").length || claims.length) fail("prior one-shot claim, attempt, or receipt exists");
}

function validateFreshIdentity(request, repoRoot) {
  const freshRoot = freshAbsolute(request.fresh_root, "fresh_root");
  const resultPath = freshAbsolute(request.result_path, "result_path");
  const codexHome = freshAbsolute(request.codex_home, "codex_home");
  const artifactDir = requiredString(request.artifact_dir, "artifact_dir");
  if (path.isAbsolute(artifactDir) || artifactDir.split(path.sep).includes("..")) fail("artifact_dir must be a contained relative path");
  const identity = { fresh_root: freshRoot, result_path: resultPath, artifact_path: path.join(freshRoot, artifactDir), codex_home: codexHome, terminal_report_path: path.join(repoRoot, `.planr/artifacts/planr2-ac014/verification-report-${FIXED.plan_id}-${git(repoRoot, ["rev-parse", "HEAD"]).slice(0, 12)}.json`) };
  assertUnusedFreshIdentity(identity);
  if (new Set(Object.values(identity)).size !== Object.keys(identity).length) fail("fresh capsule paths must be distinct");
  return identity;
}

function assertUnusedFreshIdentity(identity) {
  for (const [label, value] of Object.entries(identity)) if (existsSync(value)) fail(`${label} must be a fresh never-used path: ${value}`);
}

function requireDraftRequest(value) {
  if (!value || value.schema_version !== "planr.ac014.review_draft_request.v1") fail("invalid review draft request schema");
  for (const key of ["repo_root", "planr_binary", "planr_binary_sha256", "superseded_obligation_id", "baseline_root", "fresh_root", "result_path", "artifact_dir", "codex_home"]) requiredString(value[key], key);
  if (value.review_required !== true) fail("review_required must be true");
}

function requireReviewDraft(value) {
  if (!value || value.schema_version !== "planr.ac014.review_draft.v1" || value.status !== "pending_exact_source_review" || value.launch_guard?.launch_allowed !== false || value.launch_guard?.authorization !== null) fail("invalid or already authorized review draft");
  if (value.gate_context?.id !== FIXED.gate_id || value.plan_id !== FIXED.plan_id || value.feature_run_id !== FIXED.feature_run_id) fail("review draft scope mismatch");
}

function runtimePaths(short) {
  return { draft: `.planr/artifacts/planr2-ac014/review-draft-${FIXED.plan_id}-${short}.json`, seal: `.planr/artifacts/planr2-ac014/launch-seal-${FIXED.plan_id}-${short}.json`, runner_input: `.planr/artifacts/planr2-ac014/fresh-arm-input-${FIXED.plan_id}-${short}-cli0147.json`, run_input: `.planr/evidence/obligations/pob-planr2-ac014-${short}-v2.run-input.json`, handoff: `.planr/artifacts/planr2-ac014/verifier-handoff-${FIXED.plan_id}-${short}.json` };
}

function identities(short) {
  const namespace = `com.planr.planr2.ac014.terminal_arm.${short}`;
  return { namespace, policy: `epolicy-planr2-ac014-${short}-v2`, preset: `preset-verifier-planr2-ac014-${short}-v2`, observation: `obs-verifier-planr2-ac014-${short}-v2`, requirement: `obs-planr2-ac014-${short}-v2`, obligation: `pob-planr2-ac014-${short}-v2`, manifest: `verifier-planr2-ac014-${short}-v2`, runtime: `runtime-planr2-ac014-${short}-v2`, manifestPath: `.planr/evidence/adapters/verifier-planr2-ac014-${short}-v2.manifest.json`, schemaPath: `.planr/evidence/schemas/${namespace}.v2.schema.json` };
}

function findAvailableCapability(value, manifestId) {
  const instance = (value.object?.instances ?? value.instances ?? []).find((entry) => entry.manifest_id === manifestId && (entry.availability_status === "available" || entry.capability?.availability?.status === "available"));
  if (!instance || (!instance.capability?.environment && !instance.host_fingerprint?.environment)) fail(`configured capability is unavailable: ${manifestId}`);
  return instance;
}

function runPlanr(binary, cwd, args) {
  const result = spawnSync(binary, args, { cwd, encoding: "utf8", maxBuffer: 16 * 1024 * 1024, env: { ...process.env } });
  let parsed;
  try { parsed = JSON.parse(result.stdout); } catch { fail(`Planr command did not return a structured envelope: ${args.join(" ")}\n${result.stderr || result.stdout}`); }
  if (!(parsed.ok === true && parsed.exit?.code === 0) && !( !("ok" in parsed) && result.status === 0)) fail(`Planr command failed: ${args.join(" ")}\n${result.stderr || result.stdout}`);
  return parsed;
}

function readOneShotClaims(repoRoot, runId) {
  const database = new DatabaseSync(path.join(repoRoot, ".planr/planr.sqlite"), { readOnly: true });
  try { return database.prepare("SELECT freeze_id, obligation_id, capability_instance_id FROM feature_run_one_shot_claims WHERE run_id = ? ORDER BY freeze_id").all(runId); }
  finally { database.close(); }
}

function immutableBinary(value, digest) { const file = canonicalFile(value, "planr_binary"); const metadata = lstatSync(file); if ((metadata.mode & 0o777) !== 0o555 || !/^sha256:[0-9a-f]{64}$/.test(digest) || shaFile(file) !== digest) fail("candidate Planr binary must be an exact immutable 0555 digest-bound file"); return file; }
function canonicalDirectory(value, label) { const resolved = path.resolve(requiredString(value, label)); try { if (!statSync(resolved).isDirectory() || realpathSync(resolved) !== resolved) fail(`${label} must be an existing canonical directory`); } catch (error) { if (String(error.message).startsWith("AC-014 capsule preparation rejected:")) throw error; fail(`${label} must be an existing canonical directory`); } return resolved; }
function canonicalFile(value, label) { const resolved = path.resolve(requiredString(value, label)); try { const metadata = lstatSync(resolved); if (!metadata.isFile() || metadata.isSymbolicLink() || realpathSync(resolved) !== resolved) fail(`${label} must be an existing canonical regular file`); } catch (error) { if (String(error.message).startsWith("AC-014 capsule preparation rejected:")) throw error; fail(`${label} must be an existing canonical regular file`); } return resolved; }
function git(root, args) { const result = spawnSync("git", ["-C", root, ...args], { encoding: "utf8" }); if (result.status !== 0) fail(`git ${args.join(" ")} failed: ${result.stderr}`); return result.stdout.trim(); }
function publishJsonSet(root, files, exclusive = false, readOnly = false) {
  const stage = path.join(root, ".planr", `.ac014-capsule-stage-${process.pid}`);
  if (existsSync(stage)) fail(`staging path already exists: ${stage}`);
  for (const [relative] of files) if (exclusive && existsSync(path.join(root, relative))) fail(`immutable capsule output already exists: ${relative}`);
  const published = [];
  mkdirSync(stage, { recursive: true, mode: 0o700 });
  try {
    for (const [relative, value] of files) {
      const staged = path.join(stage, relative);
      mkdirSync(path.dirname(staged), { recursive: true });
      writeFileSync(staged, `${JSON.stringify(value, null, 2)}\n`, { flag: "wx", mode: 0o600 });
    }
    for (const [relative] of files) {
      const source = path.join(stage, relative);
      const destination = path.join(root, relative);
      mkdirSync(path.dirname(destination), { recursive: true });
      if (exclusive) {
        try { linkSync(source, destination); } catch { fail(`immutable capsule output already exists: ${relative}`); }
        published.push(destination);
        unlinkSync(source);
      } else {
        renameSync(source, destination);
      }
      if (readOnly) chmodSync(destination, 0o400);
    }
  } catch (error) {
    for (const destination of published.reverse()) rmSync(destination, { force: true });
    throw error;
  } finally {
    rmSync(stage, { recursive: true, force: true });
  }
}
function records(value, key) { return value.object?.[key] ?? value[key] ?? []; }
function migrationSummary(value) { const object = value.object ?? value; return { dry_run: object.dry_run ?? null, summary: object.summary ?? null, status: object.status ?? "passed" }; }
function shaFile(file) { return `sha256:${createHash("sha256").update(readFileSync(file)).digest("hex")}`; }
function shaJson(value) { return `sha256:${createHash("sha256").update(canonical(value)).digest("hex")}`; }
function canonical(value) { if (value === null || typeof value !== "object") return JSON.stringify(value); if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`; return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`; }
function isPathInside(candidate, root) { const relative = path.relative(root, candidate); return relative !== "" && !relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative); }
function freshAbsolute(value, label) { if (!path.isAbsolute(value)) fail(`${label} must be absolute`); return path.resolve(value); }
function requiredString(value, label) { if (typeof value !== "string" || value.length === 0) fail(`${label} must be a non-empty string`); return value; }
function requiredPositiveInteger(value) { return Number.isInteger(value) && value > 0; }
function valueAfter(flag, argv) { const index = argv.indexOf(flag); return index === -1 ? null : argv[index + 1]; }
function fail(message) { throw new Error(`AC-014 capsule preparation rejected: ${message}`); }
