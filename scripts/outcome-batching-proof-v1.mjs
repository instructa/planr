#!/usr/bin/env node
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { constants } from 'node:fs';
import {
  access,
  mkdir,
  readdir,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  adapterSpec,
  buildEvidencePolicy,
  canonicalJson,
  obligation as sharedObligation,
  scenarioRunInput,
  sha256,
  sha256Json,
} from '../apps/docs/scripts/evidence-fixture-builder.mjs';

const args = new Set(process.argv.slice(2));
const mode = args.has('--generate') ? 'generate' : args.has('--check') ? 'check' : null;
if (!mode || (args.has('--generate') && args.has('--check'))) {
  throw new Error('usage: node scripts/outcome-batching-proof-v1.mjs --generate|--check');
}

const repositoryRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const fixturePath = path.join(repositoryRoot, 'tests/fixtures/outcome-batching/v1/acceptance-bindings.json');
const fixture = JSON.parse(await readFile(fixturePath, 'utf8'));
const benchmarkFixturePath = path.join(repositoryRoot, 'tests/fixtures/outcome-batching/v1/ac014-benchmark-input.json');
const benchmarkInput = JSON.parse(await readFile(benchmarkFixturePath, 'utf8'));
const artifactRoot = path.join(repositoryRoot, fixture.local_artifact_root);
const inputsRoot = path.join(artifactRoot, 'inputs');
const reportPath = path.join(artifactRoot, 'outcome-batching-proof.report.json');
const receiptPath = path.join(artifactRoot, 'matched-dogfood-ac014.receipt.json');
const benchmarkArtifactPath = path.join(artifactRoot, 'ac014-benchmark-input.json');
const migrationPath = path.join(artifactRoot, 'evidence-migration.json');
const planrBin = process.env.PLANR_BIN
  ? path.resolve(process.cwd(), process.env.PLANR_BIN)
  : path.join(repositoryRoot, 'target/debug/planr');

const acIds = fixture.acceptance_criteria;
assert.deepEqual(acIds, Array.from({ length: 14 }, (_, index) => `AC-${String(index + 1).padStart(3, '0')}`));
assert.match(fixture.supersedes_binding_suffix, /^[0-9a-f]{12}$/u);
for (const [ac, suffix] of Object.entries(fixture.gap_supersedes_binding_suffixes ?? {})) {
  assert(acIds.includes(ac), `unknown gap supersedes AC ${ac}`);
  assert.match(suffix, /^[0-9a-f]{12}$/u);
}
await access(planrBin, constants.X_OK);
assertDocsContract();

const sourceRevision = git(['rev-parse', 'HEAD']);
const worktreeStatus = git(['status', '--short']);
const generatedAt = '2026-08-03T00:00:00Z';
const benchmarkInputDigest = sha256Json(benchmarkInput);
assertAc014BenchmarkInput(benchmarkInput);

const focusedCoverage = {
  'AC-001': passed(
    ['cargo test --test e2e done_persists_materiality_and_gates_review_from_existing_policy -- --exact'],
    ['non-material docs outcome closes with review:none and no review gate'],
  ),
  'AC-002': passed(
    ['cargo test --test e2e done_persists_materiality_and_gates_review_from_existing_policy -- --exact'],
    ['protected public API trigger requires independent_high_signal'],
  ),
  'AC-003': passed(
    [
      'cargo test --test e2e done_elevates_missing_actual_change_facts_under_materiality_policy -- --exact',
      'cargo test --test e2e done_fails_safely_to_review_when_materiality_policy_is_missing -- --exact',
    ],
    ['missing policy, missing facts, untracked files, and contradictory file claims elevate instead of weakening review'],
  ),
  'AC-004': passed(
    [
      'cargo test --test e2e cosmetic_batch_stable_shapes_ids_and_worker_identity -- --exact',
      'node scripts/test-planr-risk-based-guidance.mjs',
    ],
    ['bounded same-plan maker batch guidance and stable worker identity are executable and drift-tested'],
  ),
  'AC-005': passed(
    [
      'cargo test --test e2e done_persists_materiality_and_gates_review_from_existing_policy -- --exact',
      'cargo test --test e2e plan_final_product_review_is_single_plan_scoped_gate_across_cli_mcp_http_status_trace_and_package -- --exact',
    ],
    ['mixed material/non-material plan creates item reviews only for material outcomes and one final product review'],
  ),
  'AC-006': passed(
    ['cargo test --test e2e failed_review_reuses_stable_gate_and_single_fix_item -- --exact'],
    ['the corrected durable review gate regression ran one exact test and passed with bounded findings persisted in attempts and logs'],
  ),
  'AC-007': passed(
    ['cargo test --test e2e failed_review_reuses_stable_gate_and_single_fix_item -- --exact'],
    ['the same gate records repeated attempts without opening a second active fix item or relying on random id ordering'],
  ),
  'AC-008': passed(
    ['node scripts/test-planr-risk-based-guidance.mjs'],
    ['canonical guidance says reviewers selectively replay only cheap, missing, failing, or explicitly high-risk evidence'],
  ),
  'AC-009': passed(
    ['node scripts/test-planr-risk-based-guidance.mjs'],
    ['selective replay examples direct fix replays to affected evidence unless new risk expands scope'],
  ),
  'AC-010': passed(
    [
      'cargo test --test e2e failed_review_reuses_stable_gate_and_single_fix_item -- --exact',
      'cargo test --test e2e review_generated_material_fix_uses_parent_gate_for_audit_assurance -- --exact',
      'cargo test --test e2e review_generated_material_fix_maker_parent_attempt_does_not_satisfy_audit -- --exact',
    ],
    ['review modes remain derived from recorded worker identity and parent gate assurance'],
  ),
  'AC-011': passed(
    [
      'cargo test --test e2e plan_audit_accepts_legacy_multiple_complete_material_reviews_but_rejects_duplicate_final_review -- --exact',
      'cargo test --test e2e review_generated_material_fix_uses_parent_gate_for_audit_assurance -- --exact',
    ],
    ['audit requires final product review, required material reviews, approvals, and canonical Evidence coverage'],
  ),
  'AC-012': passed(
    [
      'cargo test --test e2e plan_final_product_review_is_single_plan_scoped_gate_across_cli_mcp_http_status_trace_and_package -- --exact',
      'node scripts/test-planr-risk-based-guidance.mjs',
    ],
    ['CLI, MCP, HTTP/package inspection, skills, generated host roles, and docs expose the same contract'],
  ),
  'AC-013': passed(
    ['cargo test --test e2e failed_review_reuses_stable_gate_and_single_fix_item -- --exact'],
    ['corrected selector runs the intended one-test stable-gate regression and proves no recursive follow-up review item is created'],
  ),
  'AC-014': gap(
    'failed_efficiency_thresholds',
    ['node scripts/outcome-batching-proof-v1.mjs --check'],
    [
      'matched Vanilla/current/alpha2 runs used identical Codex CLI, model, effort, product specification, oracle, and local browser surface',
      'all three products passed the neutral exact product oracle, while the current candidate exceeded every efficiency ceiling',
    ],
  ),
};

const ac014Receipt = buildAc014Receipt();
const report = buildReport(ac014Receipt);
const bindingSuffix = sha256Json({
  report_digest: report.report_digest,
  binding_revision: 'ac014-current-diagnostic-v3',
}).slice('sha256:'.length, 'sha256:'.length + 12);
const specs = acIds.map((ac) => {
  const result = focusedCoverage[ac];
  return acSpec(ac, result.status, bindingSuffix, { coverage: result.coverage });
});
const policy = buildEvidencePolicy(specs, { policyId: 'epolicy-outcome-batching-v1' });
const policyDigest = policy.policy_digest;
const migration = {
  schema_version: 'planr.evidence.migration.v1',
  plan_id: fixture.plan_id,
  obligations: specs.map((spec) => acObligation(spec, policyDigest, report.report_digest)),
};
const expectedFiles = await buildExpectedFiles(policy, migration, report, ac014Receipt, specs);

if (mode === 'generate') {
  await writeExpectedFiles(expectedFiles);
  await removeStaleGeneratedFiles(new Set([...expectedFiles.keys()].map((file) => path.join(repositoryRoot, file))));
} else {
  await assertNoDrift(expectedFiles);
}

console.log(JSON.stringify({
  ok: true,
  mode,
  report: path.relative(repositoryRoot, reportPath),
  receipt: path.relative(repositoryRoot, receiptPath),
  migration: path.relative(repositoryRoot, migrationPath),
  refreshed_bindings: specs.length,
  explicit_gaps: acIds.filter((ac) => focusedCoverage[ac].coverage === 'gap'),
  report_digest: report.report_digest,
  ac014_status: ac014Receipt.current_planr_run.status,
}, null, 2));

function passed(commands, evidence) {
  return { status: 'passed', coverage: 'covered', commands, evidence };
}

function gap(status, commands, evidence) {
  return { status, coverage: 'gap', commands, evidence };
}

function assertAc014BenchmarkInput(input) {
  assert.equal(input.schema_version, 'planr.outcome_batching.ac014_benchmark_input.v1');
  assert.equal(input.spec.digest, 'sha256:cde84864a4708343de26d291585812a66de896a54495502d3c89b0b1a403c64f');
  assert.equal(input.required_match.cli_version, '0.146.0');
  assert.equal(input.required_match.model, 'gpt-5.6-sol');
  assert.equal(input.required_match.effort, 'medium');
  for (const field of ['spec', 'oracle', 'surface']) {
    assert.equal(input.required_match[field], 'identical', `AC-014 ${field} must be identical`);
  }
  const expectedDigests = {
    vanilla: { raw_session_sha256: 'f1c75950cbff59dcc4e5058874c6109a0a78bbba991080e88368505afd39e290' },
    current_candidate: {
      root_raw_session_sha256: '87927629aacd9fda4fbafcff7fc904a14960d9ac930ffbfc0a9fc45c8dafc884',
      maker_raw_session_sha256: '3bb8eadb6650cd4ef1aa3dce4a25783a516b1760762e65ebb7871a708d0c7c09',
      reviewer_raw_session_sha256: 'c6f33f8e94714cc21226501dd94a2010c8259d53c6742d13b8641ea7ef36ee82',
    },
    alpha2: {
      root_raw_session_sha256: 'eb6681d27673726bd529b2f843fd26c0bdba435ce807bbbcd84e74c37fec536b',
      maker_raw_session_sha256: 'afa32e010d9d3f7dabc8939f68055917c00855addc007e64eef8e8ec179bf265',
      reviewer_raw_session_sha256: '7da34dfaf9a8a6342a5585554387ffac74ed08f272066272aaea43fc4ce33f17',
    },
  };
  for (const [arm, digests] of Object.entries(expectedDigests)) {
    for (const [field, value] of Object.entries(digests)) {
      assert.equal(input.runs[arm][field], value, `${arm}.${field} digest changed`);
    }
  }
  for (const arm of Object.values(input.runs)) {
    assert.equal(arm.exact_product_oracle, 'passed', 'all AC-014 arms must pass the neutral oracle');
  }
  assert.equal(
    input.runs.current_candidate.historical_label,
    'pre_fix_current_candidate',
    'AC-014 current_candidate must remain the pre-fix historical run',
  );
  const [stoppedFix1] = input.stopped_fix_runs ?? [];
  assert.equal(stoppedFix1?.label, 'fix1_after_materiality_guidance');
  assert.equal(stoppedFix1.root_session_id, '019fc8e9-9d72-7423-89e3-c8f4b3a91aa7');
  assert.deepEqual(stoppedFix1.child_session_ids, [
    '019fc8ea-78cb-7de0-826f-e07007fd6d9c',
    '019fc8ec-f8d3-7d01-9e13-913be15f3f35',
    '019fc8f2-430a-75d1-a463-4ac4b61d2f3f',
  ]);
  assert.equal(stoppedFix1.wall_time_seconds, 1030.916);
  assert.equal(stoppedFix1.total_tokens, 13704278);
  assert.equal(stoppedFix1.tool_call_envelopes, 168);
  assert.equal(stoppedFix1.exact_product_oracle, 'not_reached');
  assert.equal(stoppedFix1.stop_reason, 'objective_ac014_wall_ceiling_exceeded');
  const stoppedFix2 = input.stopped_fix_runs?.[1];
  assert.equal(stoppedFix2?.label, 'fix2_after_materiality_correction');
  assert.equal(stoppedFix2.root_session_id, '019fc917-dc39-7933-a3fe-2094c1794ae5');
  assert.equal(stoppedFix2.maker_session_id, '019fc918-b5fe-7733-874b-b4315b081863');
  assert.equal(stoppedFix2.reviewer_session_id, '019fc91c-edeb-7021-95cc-804ebc273560');
  assert.equal(stoppedFix2.wall_time_seconds, 677.238);
  assert.equal(stoppedFix2.total_tokens, 5448154);
  assert.equal(stoppedFix2.tool_call_envelopes, 94);
  assert.deepEqual(stoppedFix2.tool_call_breakdown, {
    root: 49,
    maker: 33,
    checker: 12,
    wait_agent: 22,
    list_agents: 4,
    spawn_agent: 3,
    send_message: 3,
    followup_task: 1,
  });
  assert.equal(stoppedFix2.exact_product_oracle, 'not_reached');
  assert.equal(stoppedFix2.stop_reason, 'objective_ac014_tool_call_ceiling_exceeded');
  const stoppedFix3 = input.stopped_fix_runs?.[2];
  assert.equal(stoppedFix3?.label, 'fix3_after_dispatch_correction');
  assert.equal(stoppedFix3.root_session_id, '019fc957-46b2-7d03-8e84-7672267dc708');
  assert.deepEqual(stoppedFix3.maker_session_ids, [
    '019fc957-d572-7762-9e75-ad25513fd105',
    '019fc95d-d03c-7dc3-b198-ceb0db955f7a',
    '019fc961-5c03-70d3-87bb-17c94f09ed0b',
  ]);
  assert.equal(stoppedFix3.wall_time_seconds, 754.663);
  assert.equal(stoppedFix3.total_tokens, 4973319);
  assert.equal(stoppedFix3.tool_call_envelopes, 95);
  assert.deepEqual(stoppedFix3.tool_call_breakdown, {
    root: 33,
    maker_1: 21,
    maker_2: 27,
    evidence_maker: 14,
    wait_agent: 12,
    list_agents: 7,
    spawn_agent: 3,
    send_message: 2,
  });
  assert.equal(stoppedFix3.dogfood_state.code_outcomes_closed, 6);
  assert.equal(stoppedFix3.dogfood_state.covered_evidence_observations, 0);
  assert.equal(stoppedFix3.exact_product_oracle, 'not_reached');
  assert.equal(stoppedFix3.stop_reason, 'objective_ac014_tool_call_ceiling_exceeded');
  const stoppedFix4 = input.stopped_fix_runs?.[3];
  assert.equal(stoppedFix4?.label, 'fix4_after_freeze_and_wait_correction');
  assert.equal(stoppedFix4.root_session_id, '019fcb52-507c-7372-befd-1c728b34e6cb');
  assert.equal(stoppedFix4.maker_session_id, '019fcb53-3936-70a0-9f69-0068b2db8d48');
  assert.equal(stoppedFix4.wall_time_seconds, 915.163);
  assert.equal(stoppedFix4.total_tokens, 6276045);
  assert.equal(stoppedFix4.tool_call_envelopes, 76);
  assert.deepEqual(stoppedFix4.tool_call_breakdown, {
    root: 8,
    maker: 68,
    wait_agent: 1,
    list_agents: 0,
    spawn_agent: 1,
    send_message: 0,
  });
  assert.equal(stoppedFix4.dogfood_state.code_outcomes_closed, 6);
  assert.equal(stoppedFix4.dogfood_state.covered_evidence_observations, 0);
  assert.equal(stoppedFix4.exact_product_oracle, 'not_reached');
  assert.equal(stoppedFix4.stop_reason, 'objective_ac014_token_ceiling_exceeded');
  const stoppedFix5 = input.stopped_fix_runs?.[4];
  assert.equal(stoppedFix5?.label, 'fix5_after_prebound_evidence_readiness');
  assert.equal(stoppedFix5.root_session_id, '019fcb6f-154f-7570-88a5-641382f2551a');
  assert.deepEqual(stoppedFix5.maker_session_ids, [
    '019fcb6f-ef52-7162-ac38-dfd17a2ad55f',
    '019fcb76-b20f-7f13-8fbf-f3fcfcd7b19d',
    '019fcb7a-798f-7aa3-b5ed-1b411e49dd53',
  ]);
  assert.deepEqual(stoppedFix5.reviewer_session_ids, [
    '019fcb74-ea47-73c0-86e2-da74451c1fb6',
    '019fcb78-812a-7b90-855b-efc12ef4bad4',
  ]);
  assert.deepEqual(stoppedFix5.raw_session_sha256, {
    root: 'f83c328f3a86dcc1417b34a2bb0b2c4f52d3bedbaafd206f758fbc17ddb68339',
    maker_1: '7e7f6542b59a7cf8ee36ba9d9b16723e0cfdb96d7ad33539d012ac5682c76f31',
    checker_1: '842d0b9a014ecdf87ee7955fa07e42072bf651db1d0af58d78251d3c3b53341a',
    fix_maker: '2461fdc4d8ec1c6346c5f36a2e33dd39f4ac21b0b67393afeb991cdb3752b83e',
    checker_2: '79df4560b659c3905d145ce6a70ced227bd75d241db564dd7feb8989f4245b8d',
    maker_2: '09f2214931f358862cf8f7e49b0359976b222d37f8e54daf4f393c7e5b54889d',
  });
  assert.equal(stoppedFix5.wall_time_seconds, 889.815);
  assert.equal(stoppedFix5.total_tokens, 3864372);
  assert.equal(stoppedFix5.tool_call_envelopes, 94);
  assert.deepEqual(stoppedFix5.tool_call_breakdown, {
    root: 23,
    maker_1: 25,
    checker_1: 11,
    fix_maker: 11,
    checker_2: 14,
    maker_2: 10,
    wait_agent: 5,
    list_agents: 0,
    spawn_agent: 5,
    send_message: 1,
  });
  assert.equal(stoppedFix5.dogfood_state.code_outcomes_closed, 1);
  assert.equal(stoppedFix5.dogfood_state.map_items_settled, 3);
  assert.equal(stoppedFix5.dogfood_state.material_reviews_complete, 1);
  assert.equal(stoppedFix5.dogfood_state.covered_evidence_observations, 0);
  assert.equal(stoppedFix5.exact_product_oracle, 'not_reached');
  assert.equal(stoppedFix5.stop_reason, 'objective_ac014_tool_call_ceiling_exceeded');
  assert.equal(
    stoppedFix5.review_finding,
    'accepted safe-integer cents lost one cent when formatted through floating-point EUR conversion at Number.MAX_SAFE_INTEGER; the fix preserved integer major/minor units and passed a fresh independent re-review',
  );
  assert.equal(
    stoppedFix5.first_avoidable_cause,
    'not established by Fix5; the warranted material review, fix, and independent re-review cycle consumed most of the remaining tool-call budget before the second product outcome settled',
  );
  assert.equal(
    stoppedFix5.next_experiment,
    'test whether a narrower foundation outcome preserves the same defect detection and final assurance while reducing process-boundary cost; Fix5 does not prove that split would be safe or faster',
  );
  const stoppedFix6 = input.stopped_fix_runs?.[5];
  assert.equal(stoppedFix6?.label, 'fix6_after_stable_maker_reuse');
  assert.equal(stoppedFix6.root_session_id, '019fcbd0-c8e2-7590-bec4-07f7d2712364');
  assert.equal(stoppedFix6.maker_session_id, '019fcbd1-c859-7bc0-bdb9-efba9a83413d');
  assert.deepEqual(stoppedFix6.raw_session_sha256, {
    root: '38703a941a964d5846e467e45ab072dacb484e0edabddc361853b6f53db06100',
    maker: '3ad17e89e065665db34c40fbf7932a7d78c799bcc52681ed4bf95ba2edfc6acb',
  });
  assert.equal(stoppedFix6.wall_time_seconds, 978.178);
  assert.equal(stoppedFix6.total_tokens, 6355059);
  assert.equal(stoppedFix6.tool_call_envelopes, 73);
  assert.deepEqual(stoppedFix6.tool_call_breakdown, {
    root: 7,
    maker: 66,
    wait_agent: 1,
    spawn_agent: 1,
    maker_wait: 2,
  });
  assert.equal(stoppedFix6.durable_domain_tests, 12);
  assert.equal(stoppedFix6.final_plan_acceptance, 'not_reached');
  assert.equal(stoppedFix6.dogfood_state.code_outcomes_closed, 6);
  assert.equal(stoppedFix6.dogfood_state.code_outcomes_picked, 1);
  assert.equal(stoppedFix6.dogfood_state.code_outcomes_pending, 0);
  assert.equal(stoppedFix6.dogfood_state.map_items_settled, 6);
  assert.equal(stoppedFix6.dogfood_state.material_reviews_complete, 0);
  assert.equal(stoppedFix6.dogfood_state.final_product_reviews, 0);
  assert.equal(stoppedFix6.dogfood_state.required_evidence_observations, 13);
  assert.equal(stoppedFix6.dogfood_state.covered_evidence_observations, 0);
  assert.equal(stoppedFix6.exact_product_oracle, 'not_reached');
  assert.equal(stoppedFix6.stop_reason, 'objective_ac014_token_ceiling_exceeded');
  assert.equal(
    stoppedFix6.maker_reuse_observation,
    'one stable maker session implemented and settled all six compatible non-material outcomes, then continued into the final acceptance item without a replacement maker or context reconstruction',
  );
  assert.equal(
    stoppedFix6.first_avoidable_cause,
    'stable maker reuse removed session-boundary churn, but the single maker accumulated a large repeated context and then performed the final source-digest Evidence rebind; the token ceiling was exceeded before the first build or browser receipt was collected',
  );
  assert.equal(
    stoppedFix6.next_experiment,
    'keep stable maker reuse through implementation, then after source freeze and successful Evidence readiness hand deterministic build and browser Evidence collection to a fresh dedicated verifier so implementation context is not replayed; retain exactly one independent final product review',
  );
  const stoppedFix7 = input.stopped_fix_runs?.[6];
  assert.equal(stoppedFix7?.label, 'fix7_after_bounded_batches_and_fresh_verifier');
  assert.equal(stoppedFix7.root_session_id, '019fcc48-b79e-7063-ae3e-ec82c9a56dc8');
  assert.deepEqual(stoppedFix7.maker_session_ids, [
    '019fcc49-e226-7e13-8f16-f951a4da1639',
    '019fcc4e-851e-7b03-a51a-cba4d270f2fd',
  ]);
  assert.deepEqual(stoppedFix7.reviewer_session_ids, [
    '019fcc4d-0219-7150-87b5-61d99e5eee25',
    '019fcc53-4e60-7e61-86aa-b1e36b4c5bb9',
  ]);
  assert.deepEqual(stoppedFix7.raw_session_sha256, {
    root: '051550b8fa5b0b05a4955559a5e1d292297dccf7485e0f4a3e2067ca24fd770c',
    maker_1: 'cc4c020bdd7389d9d86856c18f24c720426428981b2056e8d4fb510f0c640b24',
    checker_1: '055c7363745df25c92e7334483c71522ca9b53bd532162f6d4bcfc27390d574b',
    maker_2: '3157325b3c6df6dd1a7bb52d7e421c6432c727460b61b14f22b6a2634ba5801f',
    checker_2: 'b36a41499ac21697b52b67b8fb80396248bd3accd6877d3577461fc3486701e6',
  });
  assert.equal(stoppedFix7.wall_time_seconds, 724.301);
  assert.equal(stoppedFix7.total_tokens, 4645088);
  assert.equal(stoppedFix7.tool_call_envelopes, 94);
  assert.deepEqual(stoppedFix7.tool_call_breakdown, {
    root_exec: 17,
    maker_1: 15,
    checker_1: 11,
    maker_2: 37,
    checker_2: 6,
    spawn_agent: 4,
    wait_agent: 4,
  });
  assert.equal(stoppedFix7.durable_domain_tests, 6);
  assert.equal(stoppedFix7.final_plan_acceptance, 'not_reached');
  assert.equal(stoppedFix7.dogfood_state.code_outcomes_closed, 1);
  assert.equal(stoppedFix7.dogfood_state.code_outcomes_in_review, 1);
  assert.equal(stoppedFix7.dogfood_state.code_outcomes_pending, 5);
  assert.equal(stoppedFix7.dogfood_state.map_items_settled, 2);
  assert.equal(stoppedFix7.dogfood_state.material_reviews_complete, 1);
  assert.equal(stoppedFix7.dogfood_state.final_product_reviews, 0);
  assert.equal(stoppedFix7.dogfood_state.required_evidence_observations, 13);
  assert.equal(stoppedFix7.dogfood_state.covered_evidence_observations, 0);
  assert.equal(stoppedFix7.exact_product_oracle, 'not_reached');
  assert.equal(stoppedFix7.stop_reason, 'objective_ac014_tool_call_ceiling_exceeded');
  assert.equal(
    stoppedFix7.batch_observation,
    'the first maker stopped correctly after one genuinely material foundation outcome, but the driver replaced that still-available maker after accepted review instead of resuming the paused batch; the second maker then explicitly requested review for a dashboard outcome that Planr computed as material false and review none',
  );
  assert.equal(
    stoppedFix7.first_avoidable_cause,
    'two unenforced orchestration rules recreated process churn: a material-review pause was treated as the end of maker identity, and an adapter-drift diagnostic caused the maker to call done --review on a non-material outcome; the resulting second checker crossed the tool-call ceiling before that outcome could settle',
  );
  assert.equal(
    stoppedFix7.next_experiment,
    'do not launch Fix8 without approval; first make review pauses preserve the live maker and require a structured mechanically auditable escalation reason before --review may override computed review none, with missing or failed Evidence explicitly forbidden as an escalation reason',
  );
  const protocol = input.post_fix7_protocol;
  assert.equal(protocol?.status, 'tested_failed_tool_call_ceiling');
  assert.equal(protocol.maker_batch_compatible_outcome_min, 2);
  assert.equal(protocol.maker_batch_compatible_outcome_max, 3);
  assert.equal(protocol.handoff, 'compact_durable');
  assert.equal(protocol.source_freeze_before_binding_evidence, true);
  assert.equal(protocol.readiness_required_before_verifier, true);
  assert.equal(protocol.frozen_source_boundary?.mechanism, 'canonical_evidence_source_digest');
  assert.equal(protocol.frozen_source_boundary.scope, 'src/evidence/policy.rs::SOURCE_PATHS');
  assert.equal(protocol.frozen_source_boundary.enforcement, 'planr_evidence_run_transaction');
  assert.equal(protocol.frozen_source_boundary.failure_mode, 'failed_non_covering_attempt_zero_trusted_receipts');
  assert.equal(protocol.frozen_source_boundary.product_source_write_regression, 'cli_mutation_rejected_before_receipt_commit');
  assert.equal(protocol.frozen_source_boundary.runtime_write_regression, 'planr_runtime_writes_allowed');
  assert.equal(protocol.frozen_source_boundary.fix_replay_boundary, 'rerun_readiness_refreeze_then_selective_replay');
  assert.equal(protocol.verifier.fresh_worker, true);
  assert.equal(protocol.verifier.product_source, 'canonical_source_digest_read_only');
  assert.deepEqual(protocol.verifier.allowed_writes, ['planr_runtime_state', 'receipts', 'logs', 'artifacts']);
  assert.equal(protocol.verifier.product_findings_route, 'responsible_maker');
  assert.equal(protocol.verifier.rerun_scope, 'invalidated_evidence_only');
  assert.equal(protocol.final_product_review.required, true);
  assert.equal(protocol.final_product_review.count, 1);
  assert.equal(protocol.final_product_review.mode, 'independent');
}

function thresholdResult({
  vanillaValue,
  currentValue,
  alpha2Value,
  vanillaMaxMultiplier,
  alpha2RequiredReduction,
}) {
  const actualVanillaMultiplier = currentValue / vanillaValue;
  const actualAlpha2Reduction = (alpha2Value - currentValue) / alpha2Value;
  return {
    vanilla_max_multiplier: vanillaMaxMultiplier,
    alpha2_required_reduction: alpha2RequiredReduction,
    actual_vanilla_multiplier: actualVanillaMultiplier,
    actual_alpha2_reduction: actualAlpha2Reduction,
    status: actualVanillaMultiplier <= vanillaMaxMultiplier
      && actualAlpha2Reduction >= alpha2RequiredReduction
      ? 'passed'
      : 'failed',
  };
}

function buildAc014Receipt() {
  const { runs, thresholds } = benchmarkInput;
  const vanilla = runs.vanilla;
  const current = runs.current_candidate;
  const alpha2 = runs.alpha2;
  const thresholdResults = {
    median_wall_time: thresholdResult({
      vanillaValue: vanilla.wall_time_seconds,
      currentValue: current.wall_time_seconds,
      alpha2Value: alpha2.wall_time_seconds,
      vanillaMaxMultiplier: thresholds.median_wall_time.vanilla_max_multiplier,
      alpha2RequiredReduction: thresholds.median_wall_time.alpha2_required_reduction,
    }),
    tokens: thresholdResult({
      vanillaValue: vanilla.total_tokens,
      currentValue: current.total_tokens,
      alpha2Value: alpha2.total_tokens,
      vanillaMaxMultiplier: thresholds.tokens.vanilla_max_multiplier,
      alpha2RequiredReduction: thresholds.tokens.alpha2_required_reduction,
    }),
    tool_call_envelopes: thresholdResult({
      vanillaValue: vanilla.tool_call_envelopes,
      currentValue: current.tool_call_envelopes,
      alpha2Value: alpha2.tool_call_envelopes,
      vanillaMaxMultiplier: thresholds.tool_call_envelopes.vanilla_max_multiplier,
      alpha2RequiredReduction: thresholds.tool_call_envelopes.alpha2_required_reduction,
    }),
    quality: {
      no_material_regression_required: thresholds.quality.no_material_regression_required,
      status: benchmarkInput.oracle_output.neutral_product_oracle.all_arms_status === 'passed'
        && current.exact_product_oracle === 'passed'
        ? 'passed'
        : 'failed',
      notes: 'All arms passed the same exact product oracle; AC-014 still fails because efficiency thresholds do not pass.',
    },
  };
  const efficiencyStatus = Object.values(thresholdResults).every((result) => result.status === 'passed')
    ? 'passed'
    : 'failed_efficiency_thresholds';
  return {
    schema_version: 'planr.outcome_batching.matched_dogfood_receipt.v1',
    criterion_id: 'AC-014',
    generated_at: generatedAt,
    plan_id: fixture.plan_id,
    item_id: fixture.item_id,
    source_revision: sourceRevision,
    worktree_status_digest: sha256(worktreeStatus),
    benchmark_input: {
      path: path.relative(repositoryRoot, benchmarkArtifactPath),
      fixture_path: path.relative(repositoryRoot, benchmarkFixturePath),
      digest: benchmarkInputDigest,
      raw_session_digests: {
        vanilla_root: `sha256:${vanilla.raw_session_sha256}`,
        current_root: `sha256:${current.root_raw_session_sha256}`,
        current_maker: `sha256:${current.maker_raw_session_sha256}`,
        current_reviewer: `sha256:${current.reviewer_raw_session_sha256}`,
        alpha2_root: `sha256:${alpha2.root_raw_session_sha256}`,
        alpha2_maker: `sha256:${alpha2.maker_raw_session_sha256}`,
        alpha2_reviewer: `sha256:${alpha2.reviewer_raw_session_sha256}`,
      },
    },
    suite: benchmarkInput.spec,
    required_match: benchmarkInput.required_match,
    current_planr_run: {
      status: efficiencyStatus,
      matched_vanilla_status: 'completed',
      matched_alpha2_status: 'loop_budget_exhausted_before_final_acceptance',
      reason: 'The current candidate passed the neutral product oracle and had no material product-quality regression, but it was slower and more expensive than alpha.2 and exceeded every allowed Vanilla multiplier.',
    },
    matched_runs: {
      vanilla: {
        root_session_id: vanilla.root_session_id,
        wall_time_seconds: vanilla.wall_time_seconds,
        total_tokens: vanilla.total_tokens,
        tool_call_envelopes: vanilla.tool_call_envelopes,
        exact_product_oracle: vanilla.exact_product_oracle,
        durable_domain_tests: vanilla.durable_domain_tests,
        source_lines: vanilla.source_lines,
      },
      current_candidate: {
        root_session_id: current.root_session_id,
        child_session_ids: current.child_session_ids,
        wall_time_seconds: current.wall_time_seconds,
        total_tokens: current.total_tokens,
        tool_call_envelopes: current.tool_call_envelopes,
        exact_product_oracle: current.exact_product_oracle,
        durable_domain_tests: current.durable_domain_tests,
        source_lines: current.source_lines,
        final_plan_acceptance: current.final_plan_acceptance,
      },
      alpha2: {
        root_session_id: alpha2.root_session_id,
        child_session_ids: alpha2.child_session_ids,
        wall_time_seconds: alpha2.wall_time_seconds,
        total_tokens: alpha2.total_tokens,
        tool_call_envelopes: alpha2.tool_call_envelopes,
        exact_product_oracle: alpha2.exact_product_oracle,
        durable_domain_tests: alpha2.durable_domain_tests,
        source_lines: alpha2.source_lines,
        final_plan_acceptance: alpha2.final_plan_acceptance,
      },
    },
    stopped_fix_runs: benchmarkInput.stopped_fix_runs,
    post_fix7_protocol: benchmarkInput.post_fix7_protocol,
    neutral_product_oracle: benchmarkInput.oracle_output.neutral_product_oracle,
    thresholds: thresholdResults,
    process_shape: {
      current_candidate_items: { code_closed: 5, code_open: 2, fixes_closed: 2, reviews_closed: 5 },
      alpha2_items: { code_closed: 5, code_open: 2, fixes_closed: 4, fixes_open: 1, reviews_closed: 14, reviews_open: 1 },
      current_candidate_evidence_rows: { obligations: 221, receipts: 21, manifests: 22, attempts: 21, coverage_verdict_history: 128 },
      alpha2_evidence_rows: { obligations: 26, receipts: 0, manifests: 10, attempts: 0, coverage_verdict_history: 471 },
      finding: 'The hard cut reduced recursive review items, but the shipped worker skill explicitly escalated every settlement with --review and the loop bound Evidence while source was still mutable. Five avoidable item reviews, 221 accumulated obligations, repeated rebinding, and long-lived context reconstruction dominated cost.',
    },
  };
}

function buildReport(receipt) {
  const reportWithoutDigest = {
    schema_version: 'planr.outcome_batching.proof_report.v1',
    generated_at: generatedAt,
    plan_id: fixture.plan_id,
    item_id: fixture.item_id,
    source: fixture.source,
    candidate_binary: path.relative(repositoryRoot, planrBin),
    coverage_policy: {
      only_passed_results_cover_criteria: true,
      blocked_or_unavailable_are_explicit_gaps: true,
      canonical_input_and_adapter_per_criterion: true,
      adapter_id_strategy: 'digest_suffixed_v2',
    },
    acceptance_criteria: Object.fromEntries(acIds.map((ac) => [ac, focusedCoverage[ac]])),
    docs_contract: {
      stale_follow_up_review_language_absent: true,
      examples_cover: [
        'outcome batching',
        'materiality',
        'stable review gates',
        'selective replay',
        'final product review',
        'configurable evidence',
      ],
    },
    ac014_receipt: {
      path: path.relative(repositoryRoot, receiptPath),
      benchmark_input_path: path.relative(repositoryRoot, benchmarkArtifactPath),
      benchmark_input_digest: benchmarkInputDigest,
      status: receipt.current_planr_run.status,
      threshold_status: 'failed_efficiency_thresholds',
      stopped_fix_runs: (benchmarkInput.stopped_fix_runs ?? []).map((run) => ({
        label: run.label,
        status: run.stop_reason,
        wall_time_seconds: run.wall_time_seconds,
        total_tokens: run.total_tokens,
        tool_call_envelopes: run.tool_call_envelopes,
      })),
      post_fix7_protocol: benchmarkInput.post_fix7_protocol,
    },
    refreshed_binding_scope: {
      criteria: acIds.filter((ac) => ac !== 'AC-014'),
      excluded_gap_criteria: ['AC-014'],
    },
  };
  return { ...reportWithoutDigest, report_digest: sha256Json(reportWithoutDigest) };
}

async function buildExpectedFiles(policy, migration, report, receipt, specs) {
  receipt.report_digest = report.report_digest;
  const files = new Map();
  addJson(files, path.relative(repositoryRoot, reportPath), report);
  addJson(files, path.relative(repositoryRoot, receiptPath), receipt);
  addJson(files, path.relative(repositoryRoot, benchmarkArtifactPath), benchmarkInput);
  addJson(files, path.relative(repositoryRoot, migrationPath), migration);
  addText(files, '.planr/evidence.yaml', `${JSON.stringify(policy, null, 2)}\n`);
  for (const spec of specs) {
    addJson(files, `.planr/evidence/schemas/${spec.schema.type}.schema.json`, spec.schema);
    addJson(files, `.planr/evidence/adapters/${spec.id}.manifest.json`, spec.manifest);
    const runInput = scenarioRunInput({
      obligationId: spec.obligationId,
      capabilityInstanceId: 'placeholder',
      target: spec.target,
    });
    delete runInput.capability_instance_id;
    runInput.manifest_id = spec.id;
    runInput.env = { PLANR_OUTCOME_BATCHING_REPORT_DIGEST: report.report_digest };
    addJson(files, path.relative(repositoryRoot, path.join(inputsRoot, `${spec.obligationId}.run.json`)), runInput);
  }
  return files;
}

function addJson(files, relative, value) {
  addText(files, relative, `${JSON.stringify(value, null, 2)}\n`);
}

function addText(files, relative, text) {
  files.set(relative, text);
}

async function writeExpectedFiles(files) {
  for (const [relative, text] of files) {
    const file = path.join(repositoryRoot, relative);
    await mkdir(path.dirname(file), { recursive: true });
    await writeFile(file, text);
  }
}

async function assertNoDrift(files) {
  const expectedAbsolute = new Set([...files.keys()].map((relative) => path.join(repositoryRoot, relative)));
  for (const [relative, expected] of files) {
    const actual = await readFile(path.join(repositoryRoot, relative), 'utf8');
    assert.equal(actual, expected, `${relative} drifted; run --generate intentionally`);
  }
  const stale = await listGeneratedFiles();
  const extras = stale.filter((file) => !expectedAbsolute.has(file));
  assert.deepEqual(extras.map((file) => path.relative(repositoryRoot, file)).sort(), [], 'stale generated outcome-batching artifacts remain');
}

async function removeStaleGeneratedFiles(expectedAbsolute) {
  const generated = await listGeneratedFiles();
  for (const file of generated) {
    if (!expectedAbsolute.has(file)) {
      await rm(file);
    }
  }
}

async function listGeneratedFiles() {
  const files = [];
  for (const dir of [
    inputsRoot,
    path.join(repositoryRoot, '.planr/evidence/adapters'),
    path.join(repositoryRoot, '.planr/evidence/schemas'),
  ]) {
    for (const file of await filesUnder(dir)) {
      const base = path.basename(file);
      if (
        base.startsWith('pob-outcome-batching-ac-')
        || base.startsWith('verifier-outcome-batching-')
        || base.startsWith('com.planr.outcome_batching.')
      ) {
        files.push(file);
      }
    }
  }
  return files;
}

async function filesUnder(dir) {
  try {
    const entries = await readdir(dir, { withFileTypes: true });
    const found = [];
    for (const entry of entries) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        found.push(...await filesUnder(file));
      } else if (entry.isFile()) {
        found.push(file);
      }
    }
    return found;
  } catch (error) {
    if (error?.code === 'ENOENT') return [];
    throw error;
  }
}

function assertDocsContract() {
  const docs = [
    'apps/docs/content/docs/guides/daily-worker-loop.mdx',
    'apps/docs/content/docs/guides/review-and-fix-loops.mdx',
    'apps/docs/content/docs/concepts/reviews-and-approvals.mdx',
    'apps/docs/content/docs/reference/evidence.mdx',
    'apps/docs/content/docs/faq.mdx',
    'apps/docs/content/docs/troubleshooting.mdx',
    'apps/docs/content/docs/getting-started/why-planr.mdx',
    'plugins/planr/skills/planr-work/SKILL.md',
    'plugins/planr/skills/planr-task-graph/SKILL.md',
  ];
  const corpus = docs.map((relative) => readText(relative)).join('\n');
  for (const required of [
    /bounded compatible same-plan batch/u,
    /materiality\.decision\.review/u,
    /same durable review gate/u,
    /selectively replay only cheap, missing, failing, or explicitly high-risk evidence/u,
    /planr plan final-review <plan-id>/u,
    /planr evidence readiness --scope plan --id <plan-id>/u,
  ]) {
    assert.match(corpus, required, `missing docs contract pattern ${required}`);
  }
  for (const stale of [
    /follow-up review work/iu,
    /follow-up checker item/iu,
    /follow-up review chain/iu,
  ]) {
    assert.doesNotMatch(corpus, stale, `stale review-chain language remains: ${stale}`);
  }
}

function acSpec(ac, expectedStatus, bindingSuffix, { coverage = 'covered' } = {}) {
  const acKey = ac.toLowerCase();
  const acType = ac.toLowerCase().replace('-', '');
  const isGap = coverage === 'gap';
  const gapReason = isGap ? 'failed_efficiency_thresholds' : null;
  const script = [
    'const fs=require("node:fs");',
    'const crypto=require("node:crypto");',
    `const reportFile=${JSON.stringify(path.relative(repositoryRoot, reportPath))};`,
    `const ac=${JSON.stringify(ac)};`,
    `const expected=${JSON.stringify(expectedStatus)};`,
    `const expectedCoverage=${JSON.stringify(coverage)};`,
    `const gapReason=${JSON.stringify(gapReason)};`,
    'function canon(v){if(v===null||typeof v!=="object")return JSON.stringify(v);if(Array.isArray(v))return "["+v.map(canon).join(",")+"]";return "{"+Object.keys(v).sort().map((k)=>JSON.stringify(k)+":"+canon(v[k])).join(",")+"}";}',
    'function sha(v){return "sha256:"+crypto.createHash("sha256").update(v).digest("hex");}',
    'const report=JSON.parse(fs.readFileSync(reportFile,"utf8"));',
    'const expectedDigest=process.env.PLANR_OUTCOME_BATCHING_REPORT_DIGEST;',
    'if(expectedDigest&&report.report_digest!==expectedDigest)throw new Error("outcome proof report digest mismatch");',
    'const copy={...report}; const digest=copy.report_digest; delete copy.report_digest;',
    'if(expectedDigest&&sha(canon(copy))!==digest)throw new Error("outcome proof report content mismatch");',
    'const result=report.acceptance_criteria?.[ac];',
    'if(result?.coverage!==expectedCoverage)throw new Error(`${ac} coverage mismatch`);',
    'if(result?.status!==expected)throw new Error(`${ac} status mismatch`);',
    'const actual={status:expected,ac,coverage:expectedCoverage,report_digest:report.report_digest};',
    'if(gapReason){actual.gap_reason=gapReason;process.stdout.write(JSON.stringify(actual));if(expectedDigest){process.stderr.write(JSON.stringify({planr_adapter_gap_reasons:["product_failed"],status:expected,gap_reason:gapReason}));}process.exit(0);}',
    'process.stdout.write(JSON.stringify(actual));',
  ].join('');
  const statusSchema = isGap ? { const: expectedStatus } : { const: expectedStatus };
  const coverageSchema = isGap ? { const: 'gap' } : { const: 'covered' };
  const properties = {
    status: statusSchema,
    ac: { const: ac },
    coverage: coverageSchema,
    report_digest: { pattern: '^sha256:[0-9a-f]{64}$' },
  };
  const required = ['status', 'ac', 'coverage', 'report_digest'];
  if (isGap) {
    properties.gap_reason = { const: gapReason };
    required.push('gap_reason');
  }
  const spec = adapterSpec({
    id: `verifier-outcome-batching-${acKey}-${bindingSuffix}`,
    observationType: `com.planr.outcome_batching.${acType}.v1`,
    schemaRef: `schema://com.planr.outcome_batching.${acType}.v1`,
    jsonSchema: {
      type: 'object',
      required,
      additionalProperties: false,
      properties,
    },
    executable: 'node',
    args: ['-e', script],
    runtimeKind: 'process',
    runtimeId: `runtime-outcome-batching-${acKey}`,
    target: { kind: 'process', uri: `local://outcome-batching/${acKey}` },
    independence: `verifies ${ac} against the generated Outcome 5 proof report digest`,
    blindSpot: isGap
      ? 'AC-014 is a current diagnostic adapter: it records the matched dogfood efficiency failure without satisfying product coverage'
      : 'Only passed criteria cover; blocked or unavailable criteria remain report gaps until real verification passes',
  });
  spec.ac = ac;
  spec.expectedStatus = expectedStatus;
  spec.expectedCoverage = coverage;
  spec.gapReason = gapReason;
  spec.obligationId = `${fixture.binding_obligation_prefix}${acKey}-${bindingSuffix}`;
  const supersedesSuffix = fixture.gap_supersedes_binding_suffixes?.[ac] ?? fixture.supersedes_binding_suffix;
  spec.supersedesObligationId = `${fixture.binding_obligation_prefix}${acKey}-${supersedesSuffix}`;
  return spec;
}

function acObligation(spec, policyDigest, reportDigest) {
  const definition = sharedObligation({
    id: spec.obligationId,
    planId: fixture.plan_id,
    policyDigest,
    spec,
    expected: {
      status: spec.expectedStatus,
      ac: spec.ac,
      coverage: spec.expectedCoverage,
      report_digest: reportDigest,
      ...(spec.gapReason ? { gap_reason: spec.gapReason } : {}),
    },
    environment: {
      kind: 'local',
      id: 'planr-local',
      digest: 'sha256:774c697d533a9cc75cdcba9f94d60a8e82474f57b05c33c7d962069fe1ed8fc0',
    },
    configDigest: sha256(canonicalJson({
      schema_version: 'planr.outcome_batching.ac_binding_config.v1',
      ac: spec.ac,
      expected_status: spec.expectedStatus,
      report_digest: reportDigest,
    })),
    invalidateOn: ['source_change', 'target_change', 'policy_change', 'adapter_schema_change', 'configuration_change'],
  });
  definition.item_id = fixture.item_id;
  definition.criterion_id = spec.ac;
  definition.supersedes = spec.supersedesObligationId;
  definition.title = `Outcome batching hard-cut binding ${spec.ac}`;
  definition.observations[0].id = `obs-outcome-batching-${spec.ac.toLowerCase()}`;
  definition.observations[0].subject = `Outcome batching hard-cut acceptance criterion ${spec.ac}`;
  definition.created_at = generatedAt;
  return definition;
}

function git(gitArgs) {
  const result = spawnSync('git', gitArgs, { cwd: repositoryRoot, encoding: 'utf8' });
  assert.equal(result.status, 0, `git ${gitArgs.join(' ')} failed: ${result.stderr}`);
  return result.stdout.trim();
}

function readText(relative) {
  const result = spawnSync('cat', [relative], { cwd: repositoryRoot, encoding: 'utf8' });
  assert.equal(result.status, 0, `cat ${relative} failed: ${result.stderr}`);
  return result.stdout;
}
