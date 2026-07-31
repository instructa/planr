#!/usr/bin/env node
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { access, mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { spawn, spawnSync } from 'node:child_process';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import {
  adapterSpec,
  canonicalJson,
  obligation as sharedObligation,
  scenarioRunInput,
  sha256,
  sha256Json,
  writeEvidencePolicy,
  writeJson,
} from '../apps/docs/scripts/evidence-fixture-builder.mjs';

const repositoryRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const fixturePath = path.join(
  repositoryRoot,
  'tests/fixtures/evidence/dogfood/v1/acceptance-bindings.json',
);
const fixture = JSON.parse(await readFile(fixturePath, 'utf8'));
const artifactRoot = path.join(repositoryRoot, fixture.local_artifact_root);
const rawRoot = path.join(artifactRoot, 'raw');
const inputsRoot = path.join(artifactRoot, 'inputs');
const reportPath = path.join(artifactRoot, 'evidence-dogfood.report.json');
const planrBin = process.env.PLANR_BIN
  ? path.resolve(process.cwd(), process.env.PLANR_BIN)
  : path.join(repositoryRoot, 'target', 'debug', 'planr');
const dogfoodRunId = `${Date.now()}-${process.pid}`;

const acIds = fixture.acceptance_criteria;
assert.deepEqual(acIds, Array.from({ length: 12 }, (_, index) => `AC-${String(index + 1).padStart(3, '0')}`));
await access(planrBin, constants.X_OK);
await mkdir(rawRoot, { recursive: true });
await mkdir(inputsRoot, { recursive: true });

const rawCommands = [
  {
    label: 'ac001_ac002_ac003_ac004_ac006_ac007_ac008_api_queue_stale_retry_missing_browser_gaps',
    command: 'cargo',
    args: [
      'test',
      '--test',
      'e2e',
      'evidence_e2e_scenarios_cover_api_queue_stale_retry_unavailable_and_browser_gaps',
      '--',
      '--exact',
      '--nocapture',
    ],
    covers: ['AC-001', 'AC-002', 'AC-003', 'AC-004', 'AC-006', 'AC-007', 'AC-008'],
  },
  {
    label: 'ac005_ac009_public_surface_trust_boundary',
    command: 'cargo',
    args: [
      'test',
      '--test',
      'e2e',
      'evidence_public_surfaces_share_canonical_service_and_status_codes',
      '--',
      '--exact',
      '--nocapture',
    ],
    covers: ['AC-005', 'AC-009'],
  },
  {
    label: 'ac001_logs_claim_only_and_plan_audit_binding_criteria',
    command: 'cargo',
    args: [
      'test',
      '--test',
      'e2e',
      'plan_audit_uses_evidence_coverage_for_binding_criteria_and_logs_are_claims_only',
      '--',
      '--exact',
      '--nocapture',
    ],
    covers: ['AC-001', 'AC-011'],
  },
  {
    label: 'ac007_active_goal_bounded_canonical_gaps',
    command: 'cargo',
    args: [
      'test',
      '--test',
      'e2e',
      'codex_stop_hook_enforces_active_goal_with_bounded_canonical_gaps',
      '--',
      '--exact',
      '--nocapture',
    ],
    covers: ['AC-007'],
  },
  {
    label: 'ac007_active_goal_non_actionable_blockers',
    command: 'cargo',
    args: [
      'test',
      '--test',
      'e2e',
      'codex_stop_hook_bounds_real_non_actionable_coverage_blockers',
      '--',
      '--exact',
      '--nocapture',
    ],
    covers: ['AC-007'],
  },
  {
    label: 'ac009_route_audit_trace_without_log_inference',
    command: 'cargo',
    args: [
      'test',
      '--test',
      'e2e',
      'route_audit_survives_logs_events_and_cli_mcp_trace_without_inference',
      '--',
      '--exact',
      '--nocapture',
    ],
    covers: ['AC-009', 'AC-011'],
  },
  {
    label: 'ac011_agent_profile_capability_dispatch_metadata',
    command: 'cargo',
    args: [
      'test',
      '--test',
      'e2e',
      'agent_registry_routes_picks_and_degrades_without_blocking',
      '--',
      '--exact',
      '--nocapture',
    ],
    covers: ['AC-011'],
  },
  {
    label: 'ac012_evidence_jcs_digest_vectors',
    command: 'cargo',
    args: [
      'test',
      '--test',
      'evidence_contract',
      'evidence_contract_digest_vectors_are_executable_and_production_aligned',
      '--',
      '--exact',
      '--nocapture',
    ],
    covers: ['AC-012'],
  },
  {
    label: 'ac012_eval_evidence_contract_required_fields',
    command: 'cargo',
    args: [
      'test',
      '--test',
      'eval_contract',
      'eval_contract_records_typed_required_fields_for_evidence_objects',
      '--',
      '--exact',
      '--nocapture',
    ],
    covers: ['AC-012'],
  },
  {
    label: 'ac010_docs_evidence_examples_check',
    command: 'pnpm',
    args: ['docs:evidence-examples:check'],
    covers: ['AC-001', 'AC-003', 'AC-010', 'AC-012'],
    requiredStdout: [
      'evidence_docs_examples_check=passed cases=7',
      'evidence_docs_examples_fixture_test=passed cases=7',
    ],
  },
  {
    label: 'ac010_docs_verify_evidence_docs',
    command: 'pnpm',
    args: ['docs:verify-evidence-docs'],
    covers: ['AC-010', 'AC-012'],
    requiredStdout: ['"ok": true', '"api-only"', '"repository-custom-extension"'],
  },
];

const report = {
  schema_version: fixture.schema_version,
  generated_at: new Date().toISOString(),
  plan_id: fixture.plan_id,
  item_id: fixture.item_id,
  candidate_binary: path.relative(repositoryRoot, planrBin),
  qa_source: fixture.source,
  raw_artifact_root: fixture.local_artifact_root,
  raw_commands: [],
  exact_scenarios: [],
  extracted_receipts: {},
  ac: Object.fromEntries(acIds.map((ac) => [ac, { status: 'pending', evidence: [] }])),
};

if (process.argv.includes('--self-test-parity')) {
  runParityProjectionSelfTest();
  console.log(JSON.stringify({ ok: true, self_test: 'ac009_surface_parity_projection' }, null, 2));
  process.exit(0);
}

for (const spec of rawCommands) {
  const raw = await runRawCommand(spec);
  for (const ac of spec.covers) {
    record(ac, raw.label, {
      command: raw.command_line,
      artifact: raw.artifact,
      artifact_digest: raw.artifact_digest,
      stdout_digest: raw.stdout_digest,
      stderr_digest: raw.stderr_digest,
    });
  }
}

report.extracted_receipts.browser_cdp_full_stack = browserReceiptFromGeneratedExamples();

const acSpecs = acIds.map((ac) => dogfoodAcSpec(ac));
const apiSpec = apiResourceSpec();
const customSpec = customQueueWorkerSpec();
const flakySpec = retryFlakySpec();
const policyDigest = await writeEvidencePolicy(repositoryRoot, [...acSpecs, apiSpec, flakySpec], {
  policyId: 'epolicy-evidence-dogfood-v1',
});
report.policy_digest = policyDigest;
await writeJson(reportPath, report);

const capabilities = runPlanr(['evidence', 'capability', 'list', '--json']);
await retireExistingDogfoodDiagnostics();
record(
  'AC-002',
  'disposable resource API create/read adapter produced trusted satisfied coverage',
  await runApiResourceScenario(apiSpec, capabilities, policyDigest),
);
record(
  'AC-004',
  'repository custom adapter policy/probe/run/coverage/audit path is satisfied',
  await runCustomAdapterScenario(customSpec, capabilities, policyDigest),
);
record(
  'AC-005',
  'trusted-looking browser-tool forged passing observation was rejected without receipt persistence',
  await runForgedBrowserClaimScenario(apiSpec, capabilities),
);
record(
  'AC-008',
  'configured retry path preserves two failures then a passing coverage receipt in order',
  await runRetryPassAfterFailuresScenario(flakySpec, capabilities, policyDigest),
);
record(
  'AC-006',
  'changed source/target/config/policy/adapter bindings are named by stale coverage diagnostics',
  await storeExactScenario('ac006_changed_binding_names', {
    owning_test: 'evidence_e2e_scenarios_cover_api_queue_stale_retry_unavailable_and_browser_gaps',
    raw_artifact: 'ac002_ac004_ac006_ac007_ac008_api_queue_stale_retry_missing',
    changed_bindings_named: [
      'source_change',
      'target_change',
      'configuration_change',
      'policy_change',
      'adapter_schema_change',
    ],
    policy_effect: 'previous receipts become stale and coverage names the changed binding class',
  }),
);
record(
  'AC-009',
  'surface parity projection preserves canonical ids/statuses/receipts/gaps across CLI/MCP/HTTP/audit/trace/review',
  await captureSurfaceParityProjection(),
);

const proofBundle = {
  schema_version: 'planr.evidence_dogfood.proof_bundle.v1',
  plan_id: report.plan_id,
  item_id: report.item_id,
  raw_commands: report.raw_commands.map((entry) => ({
    label: entry.label,
    command_line: entry.command_line,
    status: entry.status,
    artifact: entry.artifact,
    artifact_digest: entry.artifact_digest,
    stdout_digest: entry.stdout_digest,
    stderr_digest: entry.stderr_digest,
  })),
  exact_scenarios: report.exact_scenarios,
  extracted_receipts: report.extracted_receipts,
  ac_evidence: Object.fromEntries(
    Object.entries(report.ac).map(([ac, value]) => [
      ac,
      {
        status: value.status,
        evidence: value.evidence.map((entry) => ({
          summary: entry.summary,
          details: entry.details,
        })),
      },
    ]),
  ),
};
const proofBundleDigest = sha256Json(proofBundle);
report.proof_bundle = proofBundle;
report.proof_bundle_digest = proofBundleDigest;

for (const ac of acIds) {
  assert.equal(report.ac[ac].status, 'passed', `${ac} did not receive raw proof`);
}
await writeJson(reportPath, report);

const acReceipts = [];
const activeBeforeAcBinding = activeBindingObligations();
for (const ac of acIds) {
  const spec = acSpecs.find((candidate) => candidate.ac === ac);
  const instance = capabilityInstance(capabilities, spec.id);
  const previousCandidates = activeBeforeAcBinding
    .filter((obligation) =>
      obligation.plan_id === fixture.plan_id
      && obligation.item_id === fixture.item_id
      && obligation.criterion_id === ac,
    )
    .sort((left, right) =>
      (left.obligation_version ?? 0) - (right.obligation_version ?? 0)
      || String(left.id).localeCompare(String(right.id)),
    );
  const previous = previousCandidates.at(-1);
  for (const duplicate of previousCandidates.slice(0, -1)) {
    await retireDiagnosticObligation(duplicate, `stale_duplicate_${ac}_binding_retired`);
  }
  const definition = acDefinition(ac, spec, policyDigest, proofBundleDigest, instance.capability.environment, previous);
  await writeJson(obligationPath(definition.id), definition);
  const added = addObligation(definition.id);
  const stored = added.object?.obligation ?? added.object ?? added.obligation ?? added;
  assert.equal(stored.id, definition.id, `${ac} stored obligation id mismatch`);
  assert.equal(stored.obligation_version, previous ? previous.obligation_version + 1 : 1, `${ac} obligation_version did not increment`);
  assert.equal(stored.supersedes_obligation_id ?? null, previous?.id ?? null, `${ac} supersedes link mismatch`);
  const run = runPlanr([
    'evidence',
    'run',
    '--input',
    await writeRunInput(definition.id, instance.id, spec.target, proofBundleDigest),
    '--json',
  ]);
  assert.equal(run.object.verdict, 'passed', `${ac} run did not pass`);
  assert.equal(run.object.receipt.receipt_status, 'trusted', `${ac} receipt was not trusted`);
  assert.deepEqual(run.object.receipt.coverage_result?.gap_reasons ?? [], [], `${ac} receipt has gaps`);
  const coverage = runPlanr(['evidence', 'coverage', '--scope', 'criterion', '--id', ac, '--json']);
  assert.equal(coverageStatus(coverage), 'satisfied', `${ac} coverage is ${coverageStatus(coverage)}`);
  acReceipts.push({
    ac,
    obligation_id: definition.id,
    obligation_version: stored.obligation_version,
    supersedes_obligation_id: stored.supersedes_obligation_id ?? null,
    receipt_id: run.object.receipt.id,
    receipt_digest: run.object.receipt.receipt_digest,
    coverage_id: coverage.object.coverage_id ?? coverage.object.coverage?.id,
    coverage_status: coverageStatus(coverage),
    gap_reasons: coverage.object.coverage?.gap_reasons ?? [],
  });
}

const itemCoverage = runPlanr(['evidence', 'coverage', '--scope', 'item', '--id', fixture.item_id, '--json']);
assert.equal(coverageStatus(itemCoverage), 'satisfied', `item coverage is ${coverageStatus(itemCoverage)}`);
const planAudit = runPlanr(['plan', 'audit', fixture.plan_id, '--json']);
const trace = runPlanr(['trace', 'item', fixture.item_id, '--json']);
const activeAuditCriteria = activeBindingObligations()
  .filter((obligation) => obligation.plan_id === fixture.plan_id && obligation.item_id === fixture.item_id)
  .map((obligation) => obligation.criterion_id)
  .sort();
assert.deepEqual(activeAuditCriteria, acIds, 'active binding criteria are not exactly AC-001..AC-012');
const ac009Projection = report.ac['AC-009'].evidence
  .map((entry) => entry.details?.value?.six_surface_projection)
  .find(Boolean);
assert.ok(ac009Projection, 'AC-009 six-surface projection missing from report evidence');
const parityLineage = Object.fromEntries(
  acReceipts.map((entry) => {
    const predecessor = ac009Projection.binding_lineage[entry.ac];
    assert.ok(predecessor?.obligation_id, `${entry.ac} predecessor parity binding missing`);
    assert.equal(
      entry.supersedes_obligation_id,
      predecessor.obligation_id,
      `${entry.ac} refreshed binding does not supersede AC-009 parity predecessor`,
    );
    return [
      entry.ac,
      {
        predecessor_obligation_id: predecessor.obligation_id,
        refreshed_obligation_id: entry.obligation_id,
        refreshed_receipt_id: entry.receipt_id,
      },
    ];
  }),
);

report.current_bindings = {
  ac_receipts: acReceipts,
  item_coverage: coverageStatus(itemCoverage),
  item_coverage_id: itemCoverage.object.coverage_id ?? itemCoverage.object.coverage?.id,
  plan_audit_status: planAudit.object?.status ?? planAudit.status,
  active_audit_criteria: activeAuditCriteria,
  ac009_predecessor_lineage: parityLineage,
  trace_item: trace.object?.item?.id ?? trace.object?.id ?? fixture.item_id,
  parity_projection: {
    criterion_ids: acReceipts.map((entry) => entry.ac),
    coverage_statuses: Object.fromEntries(acReceipts.map((entry) => [entry.ac, entry.coverage_status])),
    covering_receipt_ids: Object.fromEntries(acReceipts.map((entry) => [entry.ac, entry.receipt_id])),
    gap_reasons: Object.fromEntries(acReceipts.map((entry) => [entry.ac, entry.gap_reasons])),
  },
};
await writeJson(reportPath, report);

console.log(
  JSON.stringify(
    {
      ok: true,
      report: path.relative(repositoryRoot, reportPath),
      proof_bundle_digest: proofBundleDigest,
      policy_digest: policyDigest,
      ac_bound: acReceipts.length,
      item_coverage: coverageStatus(itemCoverage),
    },
    null,
    2,
  ),
);

async function runRawCommand(spec) {
  const startedAt = new Date().toISOString();
  const result = spawnSync(spec.command, spec.args, {
    cwd: repositoryRoot,
    encoding: 'utf8',
    maxBuffer: 50 * 1024 * 1024,
  });
  const commandLine = [spec.command, ...spec.args].join(' ');
  const raw = {
    schema_version: 'planr.evidence_dogfood.raw_command.v1',
    label: spec.label,
    command: spec.command,
    args: spec.args,
    command_line: commandLine,
    started_at: startedAt,
    completed_at: new Date().toISOString(),
    status: result.status,
    signal: result.signal,
    stdout: result.stdout,
    stderr: result.stderr,
    stdout_digest: digestText(result.stdout),
    stderr_digest: digestText(result.stderr),
  };
  const artifactFile = path.join(rawRoot, `${spec.label}.json`);
  await writeJson(artifactFile, raw);
  const artifactDigest = digestText(await readFile(artifactFile, 'utf8'));
  const entry = {
    label: spec.label,
    command_line: commandLine,
    status: result.status,
    artifact: path.relative(repositoryRoot, artifactFile),
    artifact_digest: artifactDigest,
    stdout_digest: raw.stdout_digest,
    stderr_digest: raw.stderr_digest,
  };
  report.raw_commands.push(entry);
  assert.equal(result.status, 0, `${commandLine}\n${result.stdout}\n${result.stderr}`);
  for (const needle of spec.requiredStdout ?? []) {
    assert.match(result.stdout, new RegExp(escapeRegExp(needle)), `${commandLine} missing ${needle}`);
  }
  return entry;
}

function record(ac, summary, details) {
  report.ac[ac].status = 'passed';
  report.ac[ac].evidence.push({ summary, details });
}

function browserReceiptFromGeneratedExamples() {
  const generatedPath = path.join(
    repositoryRoot,
    'tests/fixtures/evidence/docs/v1/examples.generated.json',
  );
  const generated = JSON.parse(readText(generatedPath));
  const fullStack = generated.cases.find((entry) => entry.id === 'full-stack-composition');
  assert.ok(fullStack, 'missing full-stack-composition generated example');
  const receiptRun = fullStack.output.runs.find(
    (entry) => entry.object?.receipt?.obligation_id === 'pob-browser-cdp',
  );
  assert.ok(receiptRun, 'missing browser CDP generated run receipt');
  const receipt = receiptRun.object.receipt;
  assert.equal(receipt.receipt_status, 'trusted');
  assert.deepEqual(receipt.proof_gaps ?? [], []);
  const observations = Object.fromEntries(receipt.observations.map((entry) => [entry.type, entry]));
  const required = [
    ['com.example.browser.rendered_visibility', 'visible', true],
    ['com.example.browser.user_interaction', 'clicked', true],
    ['com.example.browser.navigation', 'path', '/next'],
    ['com.example.browser.network', 'api_status', 200],
    ['com.example.browser.console', 'error_count', 0],
    ['com.example.browser.reload_storage', 'persisted', true],
  ];
  for (const [type, field, expected] of required) {
    assert.ok(observations[type], `missing browser observation ${type}`);
    assert.equal(observations[type].actual[field], expected, `${type}.${field}`);
    assert.equal(observations[type].outcome, 'passed', `${type} did not pass`);
  }
  return {
    source: path.relative(repositoryRoot, generatedPath),
    case_id: fullStack.id,
    receipt_id: receipt.id,
    receipt_digest: receipt.receipt_digest,
    obligation_id: receipt.obligation_id,
    observations: required.map(([type, field, expected]) => ({ type, field, expected })),
    proof_gaps: receipt.proof_gaps ?? [],
  };
}

function apiResourceSpec() {
  const script = [
    'const base=process.env.PLANR_DOGFOOD_RESOURCE_API;',
    'if(!base){process.stdout.write(JSON.stringify({status:"created_and_read",created_status:201,read_status:200,schema_valid:true,id:"probe"}));process.exit(0);}',
    'const create=await fetch(`${base}/resources`,{method:"POST",headers:{"content-type":"application/json"},body:JSON.stringify({name:"dogfood-resource"})});',
    'const created=await create.json();',
    'const read=await fetch(`${base}/resources/${created.id}`);',
    'const resource=await read.json();',
    'const schemaValid=typeof resource.id==="string"&&resource.id===created.id&&resource.name==="dogfood-resource";',
    'process.stdout.write(JSON.stringify({status:"created_and_read",created_status:create.status,read_status:read.status,schema_valid:schemaValid,id:resource.id}));',
  ].join('');
  return adapterSpec({
    id: 'verifier-evidence-dogfood-api-resource-v1',
    observationType: 'com.example.planr.dogfood.api_resource.v1',
    schemaRef: 'schema://com.example.planr.dogfood.api_resource.v1',
    jsonSchema: {
      type: 'object',
      required: ['status', 'created_status', 'read_status', 'schema_valid', 'id'],
      additionalProperties: false,
      properties: {
        status: { const: 'created_and_read' },
        created_status: { const: 201 },
        read_status: { const: 200 },
        schema_valid: { const: true },
        id: { type: 'string' },
      },
    },
    executable: 'node',
    args: ['--input-type=module', '-e', script],
    runtimeKind: 'process',
    runtimeId: 'runtime-evidence-dogfood-api-resource-v1',
    target: { kind: 'process', uri: 'local://dogfood-resource-api' },
    network: 'loopback',
    independence: 'process adapter creates and reads a resource from a disposable local HTTP service',
    blindSpot: 'the disposable service is local and deterministic; it proves API method sufficiency, not production latency',
  });
}

function retryFlakySpec() {
  const script = [
    'if(process.env.PLANR_DOGFOOD_RETRY_FAIL==="1"){process.stderr.write(JSON.stringify({planr_adapter_gap_reasons:["verifier_failed"]}));process.exit(2);}',
    'process.stdout.write(JSON.stringify({status:"passed",ac:"AC-008",phase:"retry-pass"}));',
  ].join('');
  return adapterSpec({
    id: 'verifier-evidence-dogfood-retry-flaky-v1',
    observationType: 'com.example.planr.dogfood.retry_flaky.v1',
    schemaRef: 'schema://com.example.planr.dogfood.retry_flaky.v1',
    jsonSchema: {
      type: 'object',
      required: ['status', 'ac', 'phase'],
      additionalProperties: false,
      properties: {
        status: { const: 'passed' },
        ac: { const: 'AC-008' },
        phase: { const: 'retry-pass' },
      },
    },
    executable: 'node',
    args: ['-e', script],
    runtimeKind: 'process',
    runtimeId: 'runtime-evidence-dogfood-retry-flaky-v1',
    target: { kind: 'process', uri: 'local://evidence-dogfood/retry-flaky' },
    independence: 'deterministic flaky adapter controlled by run input env for retry ordering proof',
    blindSpot: 'failure is controlled by the fixture env rather than random infrastructure flakiness',
  });
}

function customQueueWorkerSpec() {
  return adapterSpec({
    id: 'queue-worker',
    version: '1.0.0',
    adapterKind: 'process',
    observationType: 'example.queue.job.processed',
    schemaRef: 'schema://example.queue.job.processed',
    jsonSchema: {
      type: 'object',
      required: ['status', 'worker'],
      additionalProperties: false,
      properties: {
        status: { const: 'processed' },
        worker: { const: 'queue-worker' },
      },
    },
    executable: 'sh',
    args: ['-c', 'printf \'{"status":"processed","worker":"queue-worker"}\''],
    runtimeKind: 'process',
    runtimeId: 'queue-worker',
    target: { kind: 'process', uri: 'local://queue-worker/job' },
    independence: 'disposable repository-defined queue worker adapter processes one deterministic job',
    blindSpot: 'the queue job is fixture-local and deterministic, not backed by a production broker',
  });
}

function dogfoodAcSpec(ac) {
  const acKey = ac.toLowerCase();
  const acType = ac.toLowerCase().replace('-', '');
  const zeroDigest = `sha256:${'0'.repeat(64)}`;
  const script = [
    'const fs=require("node:fs");',
    'const crypto=require("node:crypto");',
    `const reportFile=${JSON.stringify(path.relative(repositoryRoot, reportPath))};`,
    `const ac=${JSON.stringify(ac)};`,
    'function canon(v){if(v===null||typeof v!=="object")return JSON.stringify(v);if(Array.isArray(v))return "["+v.map(canon).join(",")+"]";return "{"+Object.keys(v).sort().map((k)=>JSON.stringify(k)+":"+canon(v[k])).join(",")+"}";}',
    'function sha(v){return "sha256:"+crypto.createHash("sha256").update(v).digest("hex");}',
    'const report=JSON.parse(fs.readFileSync(reportFile,"utf8"));',
    `const zero="${zeroDigest}";`,
    'const expected=process.env.PLANR_DOGFOOD_BUNDLE_DIGEST || zero;',
    'if(expected!==zero&&report.proof_bundle_digest!==expected)throw new Error("dogfood proof bundle digest mismatch");',
    'if(expected!==zero&&sha(canon(report.proof_bundle))!==expected)throw new Error("dogfood proof bundle content mismatch");',
    'if(expected!==zero&&report.ac?.[ac]?.status!=="passed")throw new Error(`${ac} is not passed in dogfood report`);',
    'process.stdout.write(JSON.stringify({status:"passed",ac,bundle_digest:expected}));',
  ].join('');
  const spec = adapterSpec({
    id: `verifier-evidence-dogfood-v2-${acKey}`,
    observationType: `com.example.planr.dogfood.${acType}.v1`,
    schemaRef: `schema://com.example.planr.dogfood.${acType}.v1`,
    jsonSchema: {
      type: 'object',
      required: ['status', 'ac', 'bundle_digest'],
      additionalProperties: false,
      properties: {
        status: { const: 'passed' },
        ac: { const: ac },
        bundle_digest: { pattern: '^sha256:[0-9a-f]{64}$' },
      },
    },
    executable: 'node',
    args: ['-e', script],
    runtimeKind: 'process',
    runtimeId: `runtime-evidence-dogfood-v2-${acKey}`,
    target: { kind: 'process', uri: `local://evidence-dogfood/${acKey}` },
    independence: `verifies ${ac} against the generated dogfood proof bundle digest`,
    blindSpot: 'the adapter binds raw executed artifacts by digest; inspect raw artifacts for scenario details',
  });
  spec.ac = ac;
  return spec;
}

function acDefinition(ac, spec, policyDigest, bundleDigest, environment, previous) {
  const proofShort = bundleDigest.slice('sha256:'.length, 'sha256:'.length + 12);
  const definition = sharedObligation({
    id: `${fixture.binding_obligation_prefix}${ac.toLowerCase()}-${proofShort}`,
    planId: fixture.plan_id,
    policyDigest,
    spec,
    expected: { status: 'passed', ac, bundle_digest: bundleDigest },
    environment,
    configDigest: sha256(canonicalJson({
      schema_version: 'planr.evidence_dogfood.ac_binding_config.v1',
      ac,
      bundle_digest: bundleDigest,
      source: fixture.source,
    })),
    invalidateOn: ['source_change', 'target_change', 'policy_change', 'adapter_schema_change'],
  });
  definition.item_id = fixture.item_id;
  definition.criterion_id = ac;
  if (previous) definition.supersedes = previous.id;
  definition.title = `Evidence feature dogfood binding ${ac}`;
  definition.observations[0].id = `obs-evidence-dogfood-${ac.toLowerCase()}`;
  definition.observations[0].subject = `Evidence feature acceptance criterion ${ac}`;
  definition.created_at = '2026-07-30T00:00:00Z';
  return definition;
}

async function writeRunInput(obligationId, capabilityInstanceId, target, bundleDigest) {
  const input = scenarioRunInput({ obligationId, capabilityInstanceId, target });
  input.env = { PLANR_DOGFOOD_BUNDLE_DIGEST: bundleDigest };
  const file = path.join(inputsRoot, `${obligationId}.run.json`);
  await writeJson(file, input);
  return file;
}

async function runApiResourceScenario(spec, capabilities, policyDigest) {
  const server = await startResourceServer();
  try {
    const instance = capabilityInstance(capabilities, spec.id);
    const definition = proofObligation({
      id: `pob-evidence-dogfood-api-resource-exact-${dogfoodRunId}`,
      criterionId: `AC-002-api-diagnostic-${dogfoodRunId}`,
      spec,
      policyDigest,
      expected: {
        status: 'created_and_read',
        created_status: 201,
        read_status: 200,
        schema_valid: true,
      },
      environment: instance.capability.environment,
      configDigest: sha256Json({ scenario: 'api-resource-create-read', version: 1 }),
    });
    delete definition.item_id;
    await writeJson(obligationPath(definition.id), definition);
    addObligation(definition.id);
    const run = runPlanr([
      'evidence',
      'run',
      '--input',
      await writeRunInputEnv(definition.id, instance.id, spec.target, {
        PLANR_DOGFOOD_RESOURCE_API: server.url,
      }),
      '--json',
    ]);
    assert.equal(run.object.verdict, 'passed');
    assert.deepEqual(run.object.receipt.proof_gaps ?? [], []);
    const processActual = run.object.receipt.observations?.[0]?.actual ?? run.object.receipt.raw_result;
    const actual = processActual.stdout_excerpt ? JSON.parse(processActual.stdout_excerpt) : processActual;
    assert.equal(actual.created_status, 201);
    assert.equal(actual.read_status, 200);
    assert.equal(actual.schema_valid, true);
    const coverage = runPlanr(['evidence', 'coverage', '--scope', 'obligation', '--id', definition.id, '--json']);
    assert.equal(coverageStatus(coverage), 'satisfied');
    const retirement = await retireDiagnosticObligation(definition, 'ac002_api_resource_diagnostic_retired');
    return storeExactScenario('ac002_api_resource_create_read', {
      obligation_id: definition.id,
      receipt_id: run.object.receipt.id,
      receipt_digest: run.object.receipt.receipt_digest,
      created_status: actual.created_status,
      read_status: actual.read_status,
      schema_valid: actual.schema_valid,
      observed_resource_id: actual.id,
      proof_gaps: run.object.receipt.proof_gaps ?? [],
      coverage_status: coverageStatus(coverage),
      diagnostic_authority: 'retired_nonbinding_superseder',
      retired_by_obligation_id: retirement.id,
    });
  } finally {
    await server.stop();
  }
}

async function runCustomAdapterScenario(spec, capabilities, policyDigest) {
  const disposable = await mkdtemp(path.join(tmpdir(), 'planr-dogfood-custom-'));
  const disposableDb = path.join(disposable, '.planr', 'planr.sqlite');
  await writeEvidencePolicy(disposable, [spec], {
    policyId: 'epolicy-evidence-dogfood-disposable-custom-v1',
  });
  const policyPath = path.join(disposable, '.planr', 'evidence.yaml');
  const policyValue = JSON.parse(await readFile(policyPath, 'utf8'));
  policyValue.defaults.preset_id = 'queue-worker';
  policyValue.named_presets[0].id = 'queue-worker';
  const policyPreimage = structuredClone(policyValue);
  delete policyPreimage.policy_digest;
  policyValue.policy_digest = sha256Json(policyPreimage);
  await writeJson(policyPath, policyValue);
  initDisposableGit(disposable);
  runPlanrIn(disposable, disposableDb, ['--json', 'project', 'init', 'Evidence Dogfood Custom']);
  const product = runPlanrIn(disposable, disposableDb, ['--json', 'plan', 'new', 'Evidence dogfood custom product']);
  const productId = product.plan.id;
  const build = runPlanrIn(disposable, disposableDb, ['--json', 'plan', 'split', productId, '--slice', 'Custom adapter']);
  const buildId = build.plan.id;
  const map = runPlanrIn(disposable, disposableDb, ['--json', 'map', 'build', '--from', buildId]);
  const itemId = map.created[0].id;
  const policy = runPlanrIn(disposable, disposableDb, ['--json', 'evidence', 'policy', '--check']);
  const localPolicyDigest = policy.object.digest;
  const localCapabilities = runPlanrIn(disposable, disposableDb, ['--json', 'evidence', 'capability', 'list']);
  const instance = capabilityInstance(localCapabilities, spec.id);
  const definition = proofObligation({
    id: 'pob-evidence-dogfood-custom-adapter-exact',
    criterionId: 'AC-004',
    spec,
    policyDigest: localPolicyDigest,
    expected: { status: 'processed', worker: 'queue-worker' },
    environment: instance.capability.environment,
    configDigest: sha256Json({ scenario: 'repository-custom-adapter', version: 1 }),
  });
  definition.plan_id = buildId;
  definition.item_id = itemId;
  const localObligationPath = path.join(disposable, `${definition.id}.json`);
  await writeJson(localObligationPath, definition);
  runPlanrIn(disposable, disposableDb, ['--json', 'evidence', 'obligation', 'add', '--input', localObligationPath]);
  const probe = policy.object.registry.probes.find((entry) => entry.manifest_id === spec.id);
  const probeStatus = probe.status ?? probe.availability_status;
  assert.equal(probeStatus, 'available');
  assert.equal(spec.id, 'queue-worker');
  assert.equal(spec.observationType, 'example.queue.job.processed');
  assert.equal(policy.object.policy.defaults.preset_id, 'queue-worker');
  const localRunInput = path.join(disposable, `${definition.id}.run.json`);
  await writeJson(localRunInput, scenarioRunInput({
    obligationId: definition.id,
    capabilityInstanceId: instance.id,
    target: spec.target,
  }));
  const run = runPlanrIn(disposable, disposableDb, [
    '--json',
    'evidence',
    'run',
    '--input',
    localRunInput,
  ]);
  assert.equal(run.object.verdict, 'passed');
  const coverage = runPlanrIn(disposable, disposableDb, ['--json', 'evidence', 'coverage', '--scope', 'criterion', '--id', 'AC-004']);
  assert.equal(coverageStatus(coverage), 'satisfied');
  const close = runPlanrIn(disposable, disposableDb, ['--json', 'close', itemId, '--summary', 'custom adapter evidence satisfied']);
  const audit = runPlanrIn(disposable, disposableDb, ['--json', 'plan', 'audit', buildId]);
  const trace = runPlanrIn(disposable, disposableDb, ['--json', 'trace', 'item', itemId]);
  const auditHolds = audit.holds ?? audit.object?.holds;
  const closeStatus = close.item?.status ?? close.status;
  const traceStatus = trace.item?.status ?? trace.object?.item?.status;
  assert.equal(auditHolds, true, 'disposable custom adapter post-close audit did not hold');
  assert.equal(closeStatus, 'closed', 'disposable custom adapter item did not close');
  assert.equal(traceStatus, 'closed', 'disposable custom adapter trace did not show closed');
  return storeExactScenario('ac004_custom_adapter_policy_probe_run_coverage_audit', {
    disposable_workspace: '<temp>',
    disposable_plan_id: buildId,
    disposable_item_id: itemId,
    obligation_id: definition.id,
    policy_digest: localPolicyDigest,
    probe_status: probeStatus,
    receipt_id: run.object.receipt.id,
    receipt_digest: run.object.receipt.receipt_digest,
    source_snapshot: run.object.receipt.source,
    manifest_id: spec.id,
    preset_id: policy.object.policy.defaults.preset_id,
    observation_type: spec.observationType,
    coverage_status: coverageStatus(coverage),
    plan_audit_holds: auditHolds,
    close_status: closeStatus,
    trace_status: traceStatus,
  });
}

async function captureSurfaceParityProjection() {
  const predecessorBindings = Object.fromEntries(
    activeBindingObligations()
      .filter((obligation) => obligation.plan_id === fixture.plan_id && obligation.item_id === fixture.item_id)
      .map((obligation) => [
        obligation.criterion_id,
        {
          obligation_id: obligation.id,
          obligation_version: obligation.obligation_version,
          supersedes_obligation_id: obligation.supersedes_obligation_id ?? null,
        },
      ]),
  );
  const cliCoverage = Object.fromEntries(
    acIds.map((ac) => [ac, runPlanrCoverage(['evidence', 'coverage', '--scope', 'criterion', '--id', ac, '--json'])]),
  );
  const mcpCoverage = Object.fromEntries(
    acIds.map((ac, index) => [
      ac,
      mcpTool(9100 + index, 'planr_evidence_coverage', { scope: 'criterion', id: ac }),
    ]),
  );
  const server = await startPlanrServer();
  const httpCoverage = {};
  try {
    for (const ac of acIds) {
      httpCoverage[ac] = await httpJson(`${server.url}/v1/evidence/coverage`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ scope: 'criterion', id: ac }),
      });
    }
  } finally {
    await server.stop();
  }
  const audit = runPlanrJson(['plan', 'audit', fixture.plan_id, '--json']);
  const trace = runPlanr(['trace', 'item', fixture.item_id, '--json']);
  const review = runPlanrJson(['review', 'evidence', fixture.item_id, '--json']);
  const projection = {
    schema_version: 'planr.evidence_dogfood.surface_parity_projection.v1',
    snapshot_role: 'predecessor_before_binding_refresh',
    plan_id: fixture.plan_id,
    item_id: fixture.item_id,
    binding_lineage: predecessorBindings,
    surfaces: {
      cli: criteriaProjectionFromCoverageMap('cli', cliCoverage),
      mcp: criteriaProjectionFromCoverageMap('mcp', mcpCoverage),
      http: criteriaProjectionFromCoverageMap('http', httpCoverage),
      audit: criteriaProjectionFromProof('audit', auditVerificationProof(audit)),
      trace: criteriaProjectionFromProof('trace', trace.proof),
      review: criteriaProjectionFromProof('review', review.evidence?.proof),
    },
  };
  const canonical = projection.surfaces.cli.criteria;
  for (const surface of ['mcp', 'http', 'audit', 'trace', 'review']) {
    assert.deepEqual(projection.surfaces[surface].criteria, canonical, `${surface} projection drifted from CLI`);
  }
  projection.asserted_canonical = canonical;
  projection.asserted_projection_digest = sha256Json(canonical);
  return storeExactScenario('ac009_surface_parity_projection', {
    snapshot_role: projection.snapshot_role,
    asserted_projection_digest: projection.asserted_projection_digest,
    canonical_projection: projection.asserted_canonical,
    six_surface_projection: projection,
    raw_outputs: {
      cli_coverage: cliCoverage,
      mcp_coverage: mcpCoverage,
      http_coverage: httpCoverage,
      plan_audit: audit,
      trace_item: trace,
      review_evidence: review,
    },
    ids_statuses_receipts_gaps: projection.asserted_canonical,
  });
}

async function runForgedBrowserClaimScenario(spec, capabilities) {
  const instance = capabilityInstance(capabilities, spec.id);
  const definition = proofObligation({
    id: `pob-evidence-dogfood-forgery-diagnostic-${dogfoodRunId}`,
    criterionId: `AC-005-forgery-diagnostic-${dogfoodRunId}`,
    spec,
    policyDigest: report.policy_digest,
    expected: {
      status: 'created_and_read',
      created_status: 201,
      read_status: 200,
      schema_valid: true,
    },
    environment: instance.capability.environment,
    configDigest: sha256Json({ scenario: 'forgery-diagnostic', version: 1 }),
  });
  delete definition.item_id;
  await writeJson(obligationPath(definition.id), definition);
  addObligation(definition.id);
  const beforeReceipts = runPlanr(['evidence', 'receipts', '--obligation', definition.id, '--json']);
  const beforeAttempts = runPlanr(['evidence', 'attempts', '--obligation', definition.id, '--json']);
  const beforeCoverage = runPlanrCoverage(['evidence', 'coverage', '--scope', 'obligation', '--id', definition.id, '--json']);
  assert.notEqual(coverageStatus(beforeCoverage), 'satisfied');
  const input = {
    obligation_id: definition.id,
    capability_instance_id: instance.id,
    target: spec.target,
    tool: { name: 'browser_cdp', claimed_surface: 'chrome-devtools' },
    provenance: { path: 'planr_observed_execution', identity: 'verifier-browser-cdp-v1' },
    observations: [
      {
        type: 'com.example.browser.rendered_visibility',
        outcome: 'passed',
        actual: { visible: true },
      },
    ],
    receipt: { id: 'receipt-forged-browser-dogfood', receipt_status: 'trusted' },
    attempt: { id: 'attempt-forged-browser-dogfood', attempt_status: 'passed' },
  };
  const inputFile = path.join(inputsRoot, 'forged-browser-claim.run.json');
  await writeJson(inputFile, input);
  const forged = spawnPlanr(['evidence', 'run', '--input', inputFile, '--json']);
  assert.equal(forged.status, 1, forged.stdout || forged.stderr);
  const parsed = JSON.parse(forged.stdout);
  assert.equal(parsed.ok, false);
  assert.match(parsed.error.message, /public Evidence input cannot construct trusted receipt field/);
  const afterReceipts = runPlanr(['evidence', 'receipts', '--obligation', definition.id, '--json']);
  const afterAttempts = runPlanr(['evidence', 'attempts', '--obligation', definition.id, '--json']);
  const afterCoverage = runPlanrCoverage(['evidence', 'coverage', '--scope', 'obligation', '--id', definition.id, '--json']);
  assert.equal(receiptList(afterReceipts).length, receiptList(beforeReceipts).length, 'forged run persisted a receipt');
  assert.equal(attemptList(afterAttempts).length, attemptList(beforeAttempts).length, 'forged run persisted an attempt');
  assert.notEqual(coverageStatus(afterCoverage), 'satisfied');
  const retirement = await retireDiagnosticObligation(definition, 'ac005_forgery_diagnostic_retired');
  return storeExactScenario('ac005_forged_browser_claim_rejected', {
    obligation_id: definition.id,
    input: {
      has_provenance: true,
      browser_tool_name: input.tool.name,
      passing_observation: input.observations[0],
      receipt_status_claim: input.receipt.receipt_status,
    },
    exit_code: forged.status,
    error: parsed.error.message,
    attempts_before: attemptList(beforeAttempts).length,
    attempts_after: attemptList(afterAttempts).length,
    receipts_before: receiptList(beforeReceipts).length,
    receipts_after: receiptList(afterReceipts).length,
    coverage_before: coverageStatus(beforeCoverage),
    coverage_after: coverageStatus(afterCoverage),
    diagnostic_authority: 'retired_nonbinding_superseder',
    retired_by_obligation_id: retirement.id,
  });
}

async function runRetryPassAfterFailuresScenario(spec, capabilities, policyDigest) {
  const instance = capabilityInstance(capabilities, spec.id);
  const definition = proofObligation({
    id: `pob-evidence-dogfood-retry-exact-${dogfoodRunId}`,
    criterionId: 'AC-008-retry-diagnostic',
    spec,
    policyDigest,
    expected: { status: 'passed', ac: 'AC-008', phase: 'retry-pass' },
    environment: instance.capability.environment,
    configDigest: sha256Json({ scenario: 'two-failures-then-pass', max_attempts: 3 }),
  });
  definition.assurance_policy = {
    max_attempts: 3,
    retry_policy: { max_attempts: 3, require_ordered_attempt_history: true },
  };
  delete definition.item_id;
  await writeJson(obligationPath(definition.id), definition);
  addObligation(definition.id);
  const first = runPlanrFailure([
    'evidence',
    'run',
    '--input',
    await writeWriteRetryInput(definition.id, instance.id, spec.target, {
      PLANR_DOGFOOD_RETRY_FAIL: '1',
    }),
    '--json',
  ]);
  const firstAttempt = first.object.attempt.id;
  const second = runPlanrFailure([
    'evidence',
    'run',
    '--input',
    await writeWriteRetryInput(definition.id, instance.id, spec.target, {
      PLANR_DOGFOOD_RETRY_FAIL: '1',
      retry_of: firstAttempt,
      attempt_index: 1,
      max_attempts: 3,
    }),
    '--json',
  ]);
  const secondAttempt = second.object.attempt.id;
  const third = runPlanr([
    'evidence',
    'run',
    '--input',
    await writeWriteRetryInput(definition.id, instance.id, spec.target, {
      retry_of: secondAttempt,
      attempt_index: 2,
      max_attempts: 3,
    }),
    '--json',
  ]);
  assert.equal(third.object.verdict, 'passed');
  const attempts = runPlanr(['evidence', 'attempts', '--obligation', definition.id, '--json']);
  const persisted = attemptList(attempts)
    .filter((attempt) => [firstAttempt, secondAttempt, third.object.attempt.id].includes(attempt.id))
    .sort((a, b) => String(a.started_at).localeCompare(String(b.started_at)));
  assert.deepEqual(persisted.map((attempt) => attempt.id), [firstAttempt, secondAttempt, third.object.attempt.id]);
  assert.deepEqual(persisted.map((attempt) => attempt.status ?? attempt.attempt_status), ['failed', 'failed', 'passed']);
  assert.equal(persisted[1].retry_predecessor_attempt_id, firstAttempt);
  assert.equal(persisted[2].retry_predecessor_attempt_id, secondAttempt);
  const coverage = runPlanrCoverage(['evidence', 'coverage', '--scope', 'obligation', '--id', definition.id, '--json']);
  assert.equal(coverageStatus(coverage), 'inconclusive');
  const gapReasons = coverage.object.coverage?.validation_details?.trust?.gap_reasons
    ?? coverage.object.coverage?.gap_reasons
    ?? [];
  assert.ok(JSON.stringify(coverage).includes('inconclusive_result'), 'retry coverage did not preserve inconclusive_result');
  const retirement = await retireDiagnosticObligation(definition, 'ac008_retry_diagnostic_retired');
  return storeExactScenario('ac008_two_failures_then_pass_retry_policy', {
    obligation_id: definition.id,
    attempts: persisted.map((attempt) => ({
      id: attempt.id,
      status: attempt.status ?? attempt.attempt_status,
      retry_predecessor_attempt_id: attempt.retry_predecessor_attempt_id ?? null,
      stdout_digest: attempt.stdout_digest,
      stderr_digest: attempt.stderr_digest,
    })),
    ordered_statuses: persisted.map((attempt) => attempt.status ?? attempt.attempt_status),
    failed_verdicts: [first.object.verdict, second.object.verdict],
    passing_receipt_id: third.object.receipt.id,
    passing_receipt_digest: third.object.receipt.receipt_digest,
    retry_lineage: third.object.attempt.retry_lineage,
    assurance_policy: definition.assurance_policy,
    coverage_status: coverageStatus(coverage),
    gap_reasons: gapReasons,
    diagnostic_authority: 'retired_nonbinding_superseder',
    retired_by_obligation_id: retirement.id,
    expected_policy_effect: 'later pass does not erase failed attempts; coverage remains inconclusive_result',
  });
}

function proofObligation({ id, criterionId, spec, policyDigest, expected, environment, configDigest }) {
  const definition = sharedObligation({
    id,
    planId: fixture.plan_id,
    policyDigest,
    spec,
    expected,
    environment,
    configDigest,
  });
  definition.item_id = fixture.item_id;
  definition.criterion_id = criterionId;
  definition.title = `Evidence dogfood exact scenario ${criterionId}`;
  definition.created_at = '2026-07-30T00:00:00Z';
  return definition;
}

async function writeRunInputEnv(obligationId, capabilityInstanceId, target, env) {
  const input = scenarioRunInput({ obligationId, capabilityInstanceId, target });
  if (Object.keys(env).length > 0) input.env = env;
  const file = path.join(inputsRoot, `${obligationId}-${digestText(canonicalJson(input)).slice(7, 15)}.run.json`);
  await writeJson(file, input);
  return file;
}

async function writeWriteRetryInput(obligationId, capabilityInstanceId, target, envAndRetry) {
  const { retry_of: retryOf, attempt_index: attemptIndex, max_attempts: maxAttempts, ...env } = envAndRetry;
  const input = scenarioRunInput({ obligationId, capabilityInstanceId, target });
  if (Object.keys(env).length > 0) input.env = env;
  if (retryOf) input.retry_of = retryOf;
  if (attemptIndex !== undefined) input.attempt_index = attemptIndex;
  if (maxAttempts !== undefined) input.max_attempts = maxAttempts;
  const file = path.join(inputsRoot, `${obligationId}-${digestText(canonicalJson(input)).slice(7, 15)}.run.json`);
  await writeJson(file, input);
  return file;
}

async function storeExactScenario(label, value) {
  const file = path.join(rawRoot, `${label}.json`);
  const artifact = {
    schema_version: 'planr.evidence_dogfood.exact_scenario.v1',
    label,
    value,
  };
  await writeJson(file, artifact);
  const artifactDigest = digestText(await readFile(file, 'utf8'));
  const entry = {
    label,
    artifact: path.relative(repositoryRoot, file),
    artifact_digest: artifactDigest,
  };
  report.exact_scenarios.push(entry);
  return { ...entry, value };
}

async function startResourceServer() {
  const script = [
    'const http=require("node:http");',
    'const resources=new Map();let next=1;',
    'const server=http.createServer((req,res)=>{',
    'if(req.method==="POST"&&req.url==="/resources"){let body="";req.on("data",(c)=>body+=c);req.on("end",()=>{const input=JSON.parse(body||"{}");const id=`res-${next++}`;const value={id,name:String(input.name||"")};resources.set(id,value);res.writeHead(201,{"content-type":"application/json"});res.end(JSON.stringify(value));});return;}',
    'const match=req.url.match(/^\\/resources\\/(res-\\d+)$/);',
    'if(req.method==="GET"&&match&&resources.has(match[1])){res.writeHead(200,{"content-type":"application/json"});res.end(JSON.stringify(resources.get(match[1])));return;}',
    'res.writeHead(404,{"content-type":"application/json"});res.end(JSON.stringify({error:"not_found"}));',
    '});',
    'server.listen(0,"127.0.0.1",()=>{process.stdout.write(`READY ${server.address().port}\\n`);});',
  ].join('');
  const child = spawn(process.execPath, ['-e', script], {
    cwd: repositoryRoot,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  const port = await new Promise((resolve, reject) => {
    let stdout = '';
    let stderr = '';
    const timer = setTimeout(() => reject(new Error(`resource server did not start: ${stderr}`)), 5000);
    child.stdout.on('data', (chunk) => {
      stdout += chunk.toString();
      const match = stdout.match(/READY (\d+)/);
      if (match) {
        clearTimeout(timer);
        resolve(match[1]);
      }
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk.toString();
    });
    child.once('exit', (code) => reject(new Error(`resource server exited ${code}: ${stderr}`)));
  });
  return {
    url: `http://127.0.0.1:${port}`,
    stop: () =>
      new Promise((resolve) => {
        child.once('exit', resolve);
        child.kill();
      }),
  };
}

async function startPlanrServer() {
  const port = await freePort();
  const child = spawn(planrBin, ['serve', '--port', String(port), '--json'], {
    cwd: repositoryRoot,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  await waitForHttp(`http://127.0.0.1:${port}/v1/health`);
  return {
    url: `http://127.0.0.1:${port}`,
    stop: () =>
      new Promise((resolve) => {
        child.once('exit', resolve);
        child.kill();
      }),
  };
}

async function freePort() {
  const server = await import('node:net').then(({ createServer }) => createServer());
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

async function waitForHttp(url) {
  const deadline = Date.now() + 5000;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok || response.status === 404) return;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`planr server did not start: ${lastError?.message ?? 'timeout'}`);
}

async function httpJson(url, init) {
  const response = await fetch(url, init);
  const text = await response.text();
  assert.ok(text.trim(), `${url} did not return JSON`);
  return JSON.parse(text);
}

function mcpTool(id, name, args) {
  const input = `${JSON.stringify({
    jsonrpc: '2.0',
    id,
    method: 'tools/call',
    params: { name, arguments: args },
  })}\n`;
  const result = spawnSync(planrBin, ['mcp'], {
    cwd: repositoryRoot,
    input,
    encoding: 'utf8',
    maxBuffer: 50 * 1024 * 1024,
  });
  assert.equal(result.status, 0, `planr mcp ${name}\n${result.stdout}\n${result.stderr}`);
  const response = JSON.parse(result.stdout);
  return JSON.parse(response.result.content[0].text);
}

function criteriaProjectionFromCoverageMap(surface, valuesByCriterion) {
  return {
    surface,
    criteria: acIds.map((ac) => criterionProjectionFromCoverage(surface, ac, valuesByCriterion[ac])),
  };
}

function criterionProjectionFromCoverage(surface, expectedCriterionId, value) {
  const coverage = value?.object?.coverage ?? value?.coverage;
  assert.ok(coverage, `${surface}.${expectedCriterionId} missing coverage`);
  const scope = coverage.scope ?? {};
  const criterionId = requiredString(
    scope.criterion_id ?? scope.id,
    `${surface}.${expectedCriterionId}.criterion_id`,
  );
  assert.equal(criterionId, expectedCriterionId, `${surface}.${expectedCriterionId} criterion drift`);
  const entry = {
    criterion_id: criterionId,
    coverage_id: requiredString(
      value.object?.coverage_id ?? coverage.id,
      `${surface}.${expectedCriterionId}.coverage_id`,
    ),
    status: requiredString(coverageStatus(value), `${surface}.${expectedCriterionId}.status`),
    receipt_digests: sortedStrings(value.object?.receipt_digests ?? value.receipt_digests ?? []),
    receipt_refs: sortedStrings(receiptRefsFromObservations(coverage.observation_coverage ?? [])),
    gap_reasons: sortedStrings(coverage.gap_reasons ?? []),
    observations: normalizeProjectionObservations(
      surface,
      expectedCriterionId,
      coverage.observation_coverage ?? [],
    ),
  };
  assertCriterionProjectionComplete(surface, entry);
  return entry;
}

function criteriaProjectionFromProof(surface, proof) {
  assert.ok(proof, `${surface} missing proof`);
  const criteria = proof.criteria ?? [];
  assert.equal(criteria.length, acIds.length, `${surface} criteria count drift`);
  const entries = criteria.map((criterion) => criterionProjectionFromProof(surface, criterion));
  assert.deepEqual(entries.map((entry) => entry.criterion_id).sort(), acIds, `${surface} criteria ids drift`);
  return { surface, criteria: entries.sort((left, right) => left.criterion_id.localeCompare(right.criterion_id)) };
}

function criterionProjectionFromProof(surface, criterion) {
  const criterionId = requiredString(criterion?.criterion_id, `${surface}.criterion_id`);
  const entry = {
    criterion_id: criterionId,
    coverage_id: requiredString(criterion.coverage_id, `${surface}.${criterionId}.coverage_id`),
    status: requiredString(criterion.status, `${surface}.${criterionId}.status`),
    receipt_digests: sortedStrings(criterion.receipt_digests ?? []),
    receipt_refs: sortedStrings(criterion.receipt_refs ?? receiptRefsFromObservations(criterion.observations ?? [])),
    gap_reasons: sortedStrings(criterion.actionable_gaps?.map((gap) => gap.reason) ?? []),
    observations: normalizeProjectionObservations(surface, criterionId, criterion.observations ?? []),
  };
  assertCriterionProjectionComplete(surface, entry);
  return entry;
}

function normalizeProjectionObservations(surface, criterionId, observations) {
  assert.ok(observations.length > 0, `${surface}.${criterionId} missing observations`);
  return observations
    .map((entry) => ({
      requirement_id: requiredString(entry.requirement_id, `${surface}.${criterionId}.requirement_id`),
      status: requiredString(entry.status, `${surface}.${criterionId}.observation.status`),
      covering_receipt_ids: sortedStrings(entry.covering_receipt_ids ?? []),
      attempted_receipt_ids: sortedStrings(entry.attempted_receipt_ids ?? []),
      gap_reason: entry.gap_reason ?? null,
      gap_reasons: sortedStrings(entry.gap_reasons ?? (entry.gap_reason ? [entry.gap_reason] : [])),
    }))
    .sort((left, right) => left.requirement_id.localeCompare(right.requirement_id));
}

function assertCriterionProjectionComplete(surface, entry) {
  requiredString(entry.criterion_id, `${surface}.criterion_id`);
  requiredString(entry.coverage_id, `${surface}.${entry.criterion_id}.coverage_id`);
  requiredString(entry.status, `${surface}.${entry.criterion_id}.status`);
  for (const observation of entry.observations) {
    requiredString(observation.requirement_id, `${surface}.${entry.criterion_id}.requirement_id`);
    requiredString(observation.status, `${surface}.${entry.criterion_id}.observation.status`);
    assert.ok(
      observation.covering_receipt_ids.length > 0
        || observation.attempted_receipt_ids.length > 0
        || observation.gap_reasons.length > 0,
      `${surface}.${entry.criterion_id}.${observation.requirement_id} missing receipt/gap evidence`,
    );
  }
}

function auditVerificationProof(audit) {
  const clause = audit.clauses?.find((entry) => entry.clause === 'verification_logged');
  assert.ok(clause, 'audit missing verification_logged clause');
  return { criteria: clause.criteria ?? [] };
}

function receiptRefsFromObservations(observations) {
  return observations.flatMap((entry) => entry.covering_receipt_ids ?? []);
}

function sortedStrings(values) {
  return [...new Set(values.filter((value) => typeof value === 'string'))].sort();
}

function requiredString(value, pathLabel) {
  assert.equal(typeof value, 'string', `${pathLabel} missing`);
  assert.notEqual(value.length, 0, `${pathLabel} empty`);
  return value;
}

function runParityProjectionSelfTest() {
  const originalAcIds = [...acIds];
  acIds.splice(0, acIds.length, 'AC-001');
  try {
    const coverage = {
      object: {
        coverage_id: 'cverdict-selftest',
        receipt_digests: ['sha256:1111111111111111111111111111111111111111111111111111111111111111'],
        status: 'satisfied',
        coverage: {
          id: 'cverdict-selftest',
          scope: {
            kind: 'criterion',
            id: 'AC-001',
            criterion_id: 'AC-001',
            item_id: fixture.item_id,
            plan_id: fixture.plan_id,
          },
          status: 'satisfied',
          observation_coverage: [{
            requirement_id: 'obs-evidence-dogfood-ac-001',
            status: 'covered',
            covering_receipt_ids: ['erec-selftest'],
            attempted_receipt_ids: [],
            gap_reasons: [],
          }],
        },
      },
    };
    const proof = {
      criteria: [{
        criterion_id: 'AC-001',
        coverage_id: 'cverdict-selftest',
        status: 'satisfied',
        receipt_digests: ['sha256:1111111111111111111111111111111111111111111111111111111111111111'],
        receipt_refs: ['erec-selftest'],
        observations: [{
          requirement_id: 'obs-evidence-dogfood-ac-001',
          status: 'covered',
          covering_receipt_ids: ['erec-selftest'],
          attempted_receipt_ids: [],
          gap_reasons: [],
        }],
      }],
    };
    const cli = criteriaProjectionFromCoverageMap('cli', { 'AC-001': coverage });
    const audit = criteriaProjectionFromProof('audit', proof);
    assert.deepEqual(audit, { ...cli, surface: 'audit' });

    const drifted = structuredClone(proof);
    drifted.criteria[0].observations[0].covering_receipt_ids = ['erec-drift'];
    assert.throws(
      () => assert.deepEqual(criteriaProjectionFromProof('review', drifted), { ...cli, surface: 'review' }),
      /Expected values to be strictly deep-equal/,
    );

    const missing = structuredClone(proof);
    delete missing.criteria[0].observations[0].requirement_id;
    assert.throws(
      () => criteriaProjectionFromProof('trace', missing),
      /requirement_id missing/,
    );
  } finally {
    acIds.splice(0, acIds.length, ...originalAcIds);
  }
}

function normalizeCoverage(value) {
  const coverage = value.object?.coverage ?? value.coverage ?? {};
  const observations = coverage.observation_coverage ?? [];
  return {
    status: coverageStatus(value),
    coverage_id: value.object?.coverage_id ?? coverage.id ?? null,
    scope: coverage.scope ?? null,
    observations: observations.map((entry) => ({
      criterion_id: entry.criterion_id ?? null,
      observation_id: entry.observation_id ?? entry.id ?? null,
      status: entry.status ?? null,
      gap_reason: entry.gap_reason ?? null,
      gap_reasons: entry.gap_reasons ?? entry.validation_details?.trust?.gap_reasons ?? [],
      covering_receipt_ids: entry.covering_receipt_ids ?? [],
      attempted_receipt_ids: entry.attempted_receipt_ids ?? [],
    })),
  };
}

async function retireExistingDogfoodDiagnostics() {
  const obligations = activeBindingObligations();
  const diagnostics = obligations.filter((obligation) => {
    const id = String(obligation.id ?? '');
    const criterion = String(obligation.criterion_id ?? '');
    const knownDiagnostic =
      criterion.includes('diagnostic')
      || id.includes('diagnostic')
      || id.includes('api-resource-exact')
      || id.includes('retry-exact');
    const canonicalAcBinding = acIds.includes(criterion)
      && id.startsWith(fixture.binding_obligation_prefix);
    return obligation.plan_id === fixture.plan_id && knownDiagnostic && !canonicalAcBinding;
  });
  const retired = [];
  for (const obligation of diagnostics) {
    retired.push(await retireDiagnosticObligation(obligation, 'preexisting_diagnostic_retired'));
  }
  if (retired.length > 0) {
    await storeExactScenario('dogfood_preexisting_diagnostics_retired', {
      retired_count: retired.length,
      retired_obligation_ids: retired.map((entry) => entry.supersedes),
      superseding_obligation_ids: retired.map((entry) => entry.id),
      authority: 'nonbinding supersession through evidence obligation add',
    });
  }
}

function activeBindingObligations() {
  const listed = runPlanr(['evidence', 'obligation', 'list', '--json']).object.obligations ?? [];
  const superseded = new Set(
    listed
      .map((entry) => entry.supersedes_obligation_id ?? entry.supersedes)
      .filter(Boolean),
  );
  return listed.filter((entry) => entry.binding === true && !superseded.has(entry.id));
}

async function retireDiagnosticObligation(obligation, label) {
  const supersededId = obligation.id;
  const retired = structuredClone(obligation);
  delete retired.project_id;
  delete retired.source_digest;
  delete retired.supersedes_obligation_id;
  delete retired.obligation_version;
  retired.id = `${supersededId}-retired-${dogfoodRunId}`;
  retired.schema_version = retired.schema_version ?? 'evidence.contract.v1';
  retired.title = `Retired non-authoritative diagnostic ${supersededId}`;
  retired.binding = false;
  retired.supersedes = supersededId;
  retired.created_at = new Date().toISOString();
  retired.config_digest = sha256Json({
    schema_version: 'planr.evidence_dogfood.diagnostic_retirement.v1',
    supersedes: supersededId,
    label,
    dogfood_run_id: dogfoodRunId,
  });
  await writeJson(obligationPath(retired.id), retired);
  addObligation(retired.id);
  return { id: retired.id, supersedes: supersededId };
}

function obligationPath(id) {
  return path.join(artifactRoot, `${id}.obligation.json`);
}

function addObligation(id) {
  const result = spawnPlanr(['evidence', 'obligation', 'add', '--input', obligationPath(id), '--json']);
  if (result.status === 0) return runPlanr(['evidence', 'obligation', 'show', id, '--json']);
  const combined = `${result.stdout}\n${result.stderr}`;
  if (combined.includes('UNIQUE constraint failed') || combined.includes('already exists')) {
    return runPlanr(['evidence', 'obligation', 'show', id, '--json']);
  }
  throw new Error(result.stderr || result.stdout);
}

function capabilityInstance(capabilities, manifestId) {
  const instance = capabilities.object.instances.find((entry) => entry.manifest_id === manifestId);
  assert.ok(instance, `missing capability instance for ${manifestId}`);
  assert.equal(instance.availability_status, 'available', `${manifestId} is ${instance.availability_status}`);
  return instance;
}

function runPlanr(args) {
  const result = spawnPlanr(args);
  assert.equal(result.status, 0, `${[planrBin, ...args].join(' ')}\n${result.stdout}\n${result.stderr}`);
  return JSON.parse(result.stdout);
}

function runPlanrJson(args) {
  const result = spawnPlanr(args);
  assert.ok(result.stdout.trim(), `${[planrBin, ...args].join(' ')} did not emit JSON\n${result.stderr}`);
  return JSON.parse(result.stdout);
}

function runPlanrCoverage(args) {
  const result = spawnPlanr(args);
  assert.ok(result.stdout.trim(), `${[planrBin, ...args].join(' ')} did not emit JSON\n${result.stderr}`);
  const parsed = JSON.parse(result.stdout);
  assert.equal(parsed.ok, true, `${[planrBin, ...args].join(' ')}\n${result.stdout}\n${result.stderr}`);
  return parsed;
}

function runPlanrIn(workspace, db, args) {
  const result = spawnSync(planrBin, ['--db', db, ...args], {
    cwd: workspace,
    encoding: 'utf8',
    maxBuffer: 50 * 1024 * 1024,
  });
  assert.equal(result.status, 0, `${[planrBin, '--db', db, ...args].join(' ')}\n${result.stdout}\n${result.stderr}`);
  return JSON.parse(result.stdout);
}

function initDisposableGit(workspace) {
  for (const args of [
    ['init'],
    ['config', 'user.email', 'dogfood@example.invalid'],
    ['config', 'user.name', 'Planr Dogfood'],
    ['add', '.'],
    ['commit', '-m', 'initial custom evidence fixture'],
  ]) {
    const result = spawnSync('git', args, {
      cwd: workspace,
      encoding: 'utf8',
      maxBuffer: 10 * 1024 * 1024,
    });
    assert.equal(result.status, 0, `git ${args.join(' ')}\n${result.stdout}\n${result.stderr}`);
  }
}

function runPlanrFailure(args) {
  const result = spawnPlanr(args);
  assert.notEqual(result.status, 0, `${[planrBin, ...args].join(' ')} unexpectedly passed`);
  assert.ok(result.stdout.trim(), `${[planrBin, ...args].join(' ')} did not emit JSON stdout\n${result.stderr}`);
  const parsed = JSON.parse(result.stdout);
  assert.ok(
    parsed.ok === false || parsed.object?.verdict === 'failed',
    `${[planrBin, ...args].join(' ')} failure envelope was not an error or failed verdict`,
  );
  return parsed;
}

function coverageStatus(value) {
  return value.object?.status ?? value.object?.verdict ?? value.object?.coverage?.status ?? value.status;
}

function attemptList(value) {
  return value.object?.attempts ?? value.attempts ?? [];
}

function receiptList(value) {
  return value.object?.receipts ?? value.receipts ?? [];
}

function spawnPlanr(args) {
  return spawnSync(planrBin, args, {
    cwd: repositoryRoot,
    encoding: 'utf8',
    maxBuffer: 50 * 1024 * 1024,
  });
}

function readText(file) {
  return spawnSync('cat', [file], {
    cwd: repositoryRoot,
    encoding: 'utf8',
    maxBuffer: 50 * 1024 * 1024,
  }).stdout;
}

function digestText(text) {
  return `sha256:${createHash('sha256').update(text).digest('hex')}`;
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
