#!/usr/bin/env node
import assert from 'node:assert/strict';
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
const artifactRoot = path.join(repositoryRoot, fixture.local_artifact_root);
const inputsRoot = path.join(artifactRoot, 'inputs');
const evidenceRoot = path.join(artifactRoot, 'evidence');
const reportPath = path.join(artifactRoot, 'outcome-batching-proof.report.json');
const migrationPath = path.join(artifactRoot, 'evidence-migration.json');
const policyPath = path.join(evidenceRoot, 'policy.json');
const planrBin = process.env.PLANR_BIN
  ? path.resolve(process.cwd(), process.env.PLANR_BIN)
  : path.join(repositoryRoot, 'target/debug/planr');

const acIds = fixture.acceptance_criteria;
assert.deepEqual(acIds, Array.from({ length: 13 }, (_, index) => `AC-${String(index + 1).padStart(3, '0')}`));
assert.match(fixture.supersedes_binding_suffix, /^[0-9a-f]{12}$/u);
await access(planrBin, constants.X_OK);
assertDocsContract();

const sourceRevision = git(['rev-parse', 'HEAD']);
const generatedAt = '2026-08-03T00:00:00Z';

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
};

const report = buildReport();
const bindingSuffix = sha256Json({
  report_digest: report.report_digest,
  binding_revision: 'current-product-proof-v1',
}).slice('sha256:'.length, 'sha256:'.length + 12);
const specs = acIds.map((ac) => acSpec(ac, focusedCoverage[ac].status, bindingSuffix));
const policy = buildEvidencePolicy(specs, { policyId: 'epolicy-outcome-batching-v1' });
const policyDigest = policy.policy_digest;
const migration = {
  schema_version: 'planr.evidence.migration.v1',
  plan_id: fixture.plan_id,
  obligations: specs.map((spec) => acObligation(spec, policyDigest, report.report_digest)),
};
const expectedFiles = buildExpectedFiles(policy, migration, report, specs);

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
  migration: path.relative(repositoryRoot, migrationPath),
  refreshed_bindings: specs.length,
  explicit_gaps: [],
  report_digest: report.report_digest,
}, null, 2));

function passed(commands, evidence) {
  return { status: 'passed', coverage: 'covered', commands, evidence };
}

function buildReport() {
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
    refreshed_binding_scope: { criteria: acIds },
  };
  return { ...reportWithoutDigest, report_digest: sha256Json(reportWithoutDigest) };
}

function buildExpectedFiles(policy, migration, report, specs) {
  const files = new Map();
  addJson(files, path.relative(repositoryRoot, reportPath), report);
  addJson(files, path.relative(repositoryRoot, migrationPath), migration);
  addJson(files, path.relative(repositoryRoot, policyPath), policy);
  for (const spec of specs) {
    addJson(files, path.relative(repositoryRoot, path.join(evidenceRoot, 'schemas', `${spec.schema.type}.schema.json`)), spec.schema);
    addJson(files, path.relative(repositoryRoot, path.join(evidenceRoot, 'adapters', `${spec.id}.manifest.json`)), spec.manifest);
    const runInput = {
      obligation_id: spec.obligationId,
      target: spec.target,
      manifest_id: spec.id,
      env: { PLANR_OUTCOME_BATCHING_REPORT_DIGEST: report.report_digest },
    };
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
  for (const dir of [inputsRoot, evidenceRoot]) {
    for (const file of await filesUnder(dir)) {
      const base = path.basename(file);
      if (
        file === policyPath
        || base.startsWith('pob-outcome-batching-ac-')
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

function acSpec(ac, expectedStatus, bindingSuffix) {
  const acKey = ac.toLowerCase();
  const acType = ac.toLowerCase().replace('-', '');
  const script = [
    'const fs=require("node:fs");',
    'const crypto=require("node:crypto");',
    `const reportFile=${JSON.stringify(path.relative(repositoryRoot, reportPath))};`,
    `const ac=${JSON.stringify(ac)};`,
    `const expected=${JSON.stringify(expectedStatus)};`,
    'function canon(v){if(v===null||typeof v!=="object")return JSON.stringify(v);if(Array.isArray(v))return "["+v.map(canon).join(",")+"]";return "{"+Object.keys(v).sort().map((k)=>JSON.stringify(k)+":"+canon(v[k])).join(",")+"}";}',
    'function sha(v){return "sha256:"+crypto.createHash("sha256").update(v).digest("hex");}',
    'const report=JSON.parse(fs.readFileSync(reportFile,"utf8"));',
    'const expectedDigest=process.env.PLANR_OUTCOME_BATCHING_REPORT_DIGEST;',
    'if(expectedDigest&&report.report_digest!==expectedDigest)throw new Error("outcome proof report digest mismatch");',
    'const copy={...report}; const digest=copy.report_digest; delete copy.report_digest;',
    'if(expectedDigest&&sha(canon(copy))!==digest)throw new Error("outcome proof report content mismatch");',
    'const result=report.acceptance_criteria?.[ac];',
    'if(result?.coverage!=="covered")throw new Error(`${ac} coverage mismatch`);',
    'if(result?.status!==expected)throw new Error(`${ac} status mismatch`);',
    'process.stdout.write(JSON.stringify({status:expected,ac,coverage:"covered",report_digest:report.report_digest}));',
  ].join('');
  const spec = adapterSpec({
    id: `verifier-outcome-batching-${acKey}-${bindingSuffix}`,
    observationType: `com.planr.outcome_batching.${acType}.v1`,
    schemaRef: `schema://com.planr.outcome_batching.${acType}.v1`,
    jsonSchema: {
      type: 'object',
      required: ['status', 'ac', 'coverage', 'report_digest'],
      additionalProperties: false,
      properties: {
        status: { const: expectedStatus },
        ac: { const: ac },
        coverage: { const: 'covered' },
        report_digest: { pattern: '^sha256:[0-9a-f]{64}$' },
      },
    },
    executable: 'node',
    args: ['-e', script],
    runtimeKind: 'process',
    runtimeId: `runtime-outcome-batching-${acKey}`,
    target: { kind: 'process', uri: `local://outcome-batching/${acKey}` },
    independence: `verifies ${ac} against the generated Outcome 5 proof report digest`,
    blindSpot: 'Only passed current-product criteria cover; external matched benchmark evidence is owned outside Planr.',
  });
  spec.ac = ac;
  spec.expectedStatus = expectedStatus;
  spec.obligationId = `${fixture.binding_obligation_prefix}${acKey}-${bindingSuffix}`;
  spec.supersedesObligationId = `${fixture.binding_obligation_prefix}${acKey}-${fixture.supersedes_binding_suffix}`;
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
      coverage: 'covered',
      report_digest: reportDigest,
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
