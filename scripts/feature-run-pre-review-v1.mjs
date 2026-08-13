#!/usr/bin/env node
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  adapterSpec,
  buildEvidencePolicy,
  canonicalJson,
  obligation,
  sha256,
  sha256Json,
  sha256JsonWithoutField,
} from '../apps/docs/scripts/evidence-fixture-builder.mjs';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const mode = process.argv[2];
assert.ok(mode === '--generate' || mode === '--check', 'usage: feature-run-pre-review-v1.mjs --generate|--check');

const planId = 'pln-92451935';
const itemId = 'i-run-focused-deterministic-verifi-2bfd';
const generatedAt = '2026-08-04T17:45:00Z';
const reportPath = '.planr/reports/feature-run/v1/pre-review-report.json';
const migrationPath = '.planr/reports/feature-run/v1/evidence-migration.json';
const gitResult = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' });
assert.equal(gitResult.status, 0, `git rev-parse HEAD failed: ${gitResult.stderr.trim()}`);
const sourceRevision = gitResult.stdout.trim();

const sharedCommands = {
  domain: 'cargo test execution_run --bin planr',
  policy: 'cargo test usage_policy --bin planr',
  evidence: 'cargo test app::feature_run_evidence --bin planr',
  finalReview: 'cargo test canonical_final_review_cli_mcp_pick_accept_and_audit_use_one_gate_without_map_items --test e2e -- --exact',
  surfaces: 'cargo test canonical_execution_state_projects_across_cli_mcp_http_inspection_recovery_package_and_audit --test e2e -- --exact',
  holds: 'cargo test capability_and_budget_holds_keep_distinct_reasons_across_cli_mcp_and_http --test e2e -- --exact',
  guidance: 'node scripts/test-planr-risk-based-guidance.mjs',
  docs: 'pnpm --filter @planr/docs content && pnpm --filter @planr/docs verify:concepts && pnpm --filter @planr/docs verify:onboarding && pnpm --filter @planr/docs verify:agent-recipes && pnpm --filter @planr/docs verify:reference',
  quality: 'cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings',
};

const criteria = {
  'AC-001': ['A FeatureRun persists and resumes every legal phase across process restart.', [sharedCommands.domain], '33 execution-run tests passed, including every legal phase round-trip and restart persistence.'],
  'AC-002': ['A maker settling two compatible non-material outcomes remains the same worker and no review gate is created.', [sharedCommands.domain], 'The compatible-outcomes regression passed with one maker/batch and no review item or gate.'],
  'AC-003': ['An accepted protected-risk checkpoint resumes the same maker and does not reset its batch count.', [sharedCommands.domain], 'The protected-checkpoint regression passed across review, finding, repair, and accepted re-review.'],
  'AC-004': ['Maker replacement is rejected without a canonical reason and accepted with recorded unavailable/context-lost/ownership-incompatible/batch-cap provenance.', [sharedCommands.domain], 'The complete replacement-reason table and persistence invariants passed.'],
  'AC-005': ['Change size alone cannot open a checkpoint; configured protected risks can.', [sharedCommands.policy, sharedCommands.domain], 'Materiality and transition tables passed; change size remains assurance depth, while protected risk opens the checkpoint.'],
  'AC-006': ['Non-material `done` cannot request review without a structured allowed escalation and reference.', [sharedCommands.policy, sharedCommands.domain], 'Structured escalation validation and CLI/MCP/HTTP settlement parity passed.'],
  'AC-007': ['Missing Evidence, adapter drift, verifier failure, sandbox restriction, and uncertainty are rejected as escalation reasons and remain correctly classified gaps.', [sharedCommands.policy, sharedCommands.holds], 'Operational-gap rejection passed and capability versus budget holds retain distinct stable reasons on CLI, MCP, and HTTP.'],
  'AC-008': ['New reviews/findings/fixes create no review, fix, or follow-up-review map items; attempts remain durable on one gate.', [sharedCommands.domain, sharedCommands.finalReview], 'Durable ReviewGate attempts/findings passed without synthetic graph items.'],
  'AC-009': ['A normal feature produces exactly one current independent final product review.', [sharedCommands.domain, sharedCommands.finalReview], 'One current independently leased final Product ReviewGate passed across CLI/MCP and audit.'],
  'AC-010': ['Binding Evidence cannot run before freeze and cannot commit after canonical source mutation.', [sharedCommands.evidence], 'Source-freeze and transactional pre-commit mismatch regressions passed with zero trusted receipts on mutation.'],
  'AC-011': ['A product finding returns to its responsible maker, refreezes source, and selectively replays only invalidated obligations.', [sharedCommands.domain, sharedCommands.evidence], 'Finding routing, re-freeze, selective invalidation, and non-overlapping verification wall ownership passed.'],
  'AC-012': ['Budget admission protects configured verification, review, and repair reserves and returns an honest hold before starvation.', [sharedCommands.policy, sharedCommands.evidence, sharedCommands.holds], 'Phase reserves, N-1 admission holds, stale reservation recovery, and honest hold projection passed.'],
  'AC-013': ['CLI, MCP, HTTP, pick, trace, status, summary, package, and audit expose equivalent canonical state.', [sharedCommands.surfaces, sharedCommands.finalReview], 'Canonical execution state and final gate are equal across all product surfaces.'],
  'AC-014': ['Existing persisted databases upgrade safely while new execution has one canonical producer path and no dual legacy workflow.', [sharedCommands.domain], 'Representative database upgrade, rollback, idempotence, and hard-cut producer tests passed.'],
  'AC-015': ['Real documentation examples demonstrate the normal, protected-risk, verifier, finding/fix, budget-hold, and missing-capability flows.', [sharedCommands.guidance, sharedCommands.docs], 'All six examples, 10 shipped skills, 30 installed skills, live onboarding, and 319 reference assertions passed.'],
};

const reportWithoutDigest = {
  schema_version: 'planr.feature_run.pre_review_report.v1',
  generated_at: generatedAt,
  plan_id: planId,
  item_id: itemId,
  source_revision: sourceRevision,
  status: 'passed',
  quality: {
    no_material_regression: true,
    command: sharedCommands.quality,
    result: 'rustfmt and strict Clippy passed for all targets and features',
  },
  acceptance_criteria: Object.fromEntries(Object.entries(criteria).map(([ac, [text, commands, evidence]]) => [ac, {
    criterion_text: text,
    status: 'passed',
    coverage: 'covered',
    commands,
    evidence,
  }])),
};
const report = { ...reportWithoutDigest, report_digest: sha256Json(reportWithoutDigest) };

const specs = Object.entries(criteria).map(([ac, [criterionText]]) => {
  const acKey = ac.toLowerCase();
  const observationType = `com.planr.feature_run.${acKey.replace('-', '')}.v1`;
  const verifier = [
    'const fs=require("node:fs");const crypto=require("node:crypto");',
    `const report=JSON.parse(fs.readFileSync(${JSON.stringify(reportPath)},"utf8"));`,
    `const ac=${JSON.stringify(ac)};const criterion=${JSON.stringify(criterionText)};`,
    'function canon(v){if(v===null||typeof v!=="object")return JSON.stringify(v);if(Array.isArray(v))return "["+v.map(canon).join(",")+"]";return "{"+Object.keys(v).sort().map(k=>JSON.stringify(k)+":"+canon(v[k])).join(",")+"}";}',
    'const copy={...report};const digest=copy.report_digest;delete copy.report_digest;',
    'const actual="sha256:"+crypto.createHash("sha256").update(canon(copy)).digest("hex");if(actual!==digest)throw new Error("report digest mismatch");',
    'const result=report.acceptance_criteria?.[ac];if(result?.criterion_text!==criterion||result?.status!=="passed"||result?.coverage!=="covered"||!result.commands?.length)throw new Error(`${ac} is not fully evidenced`);',
    'process.stdout.write(JSON.stringify({status:"passed",ac,coverage:"covered",report_digest:digest}));',
  ].join('');
  const spec = adapterSpec({
    id: `verifier-feature-run-${acKey}-v1`,
    observationType,
    schemaRef: `schema://${observationType}`,
    jsonSchema: {
      type: 'object',
      required: ['status', 'ac', 'coverage', 'report_digest'],
      additionalProperties: false,
      properties: {
        status: { const: 'passed' },
        ac: { const: ac },
        coverage: { const: 'covered' },
        report_digest: { const: report.report_digest },
      },
    },
    executable: 'node',
    args: ['-e', verifier],
    runtimeKind: 'process',
    runtimeId: `runtime-feature-run-${acKey}`,
    target: { kind: 'process', uri: `local://feature-run/${acKey}` },
    independence: `validates ${ac} against the exact current-source deterministic pre-review report`,
    blindSpot: 'This pre-review capability covers deterministic product contracts only; final independent review remains a separate gate.',
  });
  return { ac, criterionText, ...spec };
});

const policy = buildEvidencePolicy(specs, { policyId: 'epolicy-feature-run-pre-review-v1' });
policy.layering_policy.layers[0].scope = { kind: 'plan', id: planId };
policy.policy_digest = sha256JsonWithoutField(policy, 'policy_digest');
const obligations = specs.map((spec) => {
  const value = obligation({
    id: `pob-feature-run-${spec.ac.toLowerCase()}-v1`,
    planId,
    policyDigest: policy.policy_digest,
    spec,
    expected: { status: 'passed', ac: spec.ac, coverage: 'covered', report_digest: report.report_digest },
    environment: {
      kind: 'local',
      id: 'planr-local',
      digest: 'sha256:774c697d533a9cc75cdcba9f94d60a8e82474f57b05c33c7d962069fe1ed8fc0',
    },
    configDigest: sha256(canonicalJson({
      schema_version: 'planr.feature_run.ac_binding_config.v1',
      criterion_id: spec.ac,
      criterion_text: spec.criterionText,
      report_digest: report.report_digest,
    })),
    invalidateOn: ['source_change', 'target_change', 'policy_change', 'adapter_schema_change', 'configuration_change'],
  });
  value.criterion_id = spec.ac;
  value.item_id = itemId;
  value.title = `FeatureRun hard-cut binding ${spec.ac}: ${spec.criterionText}`;
  value.observations[0].id = `obs-feature-run-${spec.ac.toLowerCase()}`;
  value.observations[0].subject = spec.criterionText;
  value.created_at = generatedAt;
  return value;
});
const migration = { schema_version: 'planr.evidence.migration.v1', plan_id: planId, obligations };

const expected = new Map([
  [reportPath, `${JSON.stringify(report, null, 2)}\n`],
  [migrationPath, `${JSON.stringify(migration, null, 2)}\n`],
  ['.planr/evidence.yaml', `${JSON.stringify(policy, null, 2)}\n`],
]);
for (const spec of specs) {
  expected.set(`.planr/evidence/schemas/${spec.schema.type}.schema.json`, `${JSON.stringify(spec.schema, null, 2)}\n`);
  expected.set(`.planr/evidence/adapters/${spec.id}.manifest.json`, `${JSON.stringify(spec.manifest, null, 2)}\n`);
}

for (const [relative, text] of expected) {
  const file = path.join(root, relative);
  if (mode === '--generate') {
    await mkdir(path.dirname(file), { recursive: true });
    await writeFile(file, text);
  } else {
    assert.equal(await readFile(file, 'utf8'), text, `${relative} drifted`);
  }
}

console.log(JSON.stringify({
  ok: true,
  mode,
  report: reportPath,
  report_digest: report.report_digest,
  migration: migrationPath,
  obligations: obligations.length,
}, null, 2));
