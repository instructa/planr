#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const verifier = path.join(repo, "scripts/verify-release-eval-receipt.mjs");
const revision = spawnSync(process.execPath, [verifier, "--print-candidate-revision"], { encoding: "utf8" }).stdout.trim();
const subjectRevision = spawnSync(process.execPath, [verifier, "--print-evaluated-subject-revision"], { encoding: "utf8" }).stdout.trim();
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "planr-release-eval-gate-"));
const fakePlanr = path.join(tmp, "planr-fake.mjs");
const suitePath = path.join(tmp, "maintainer-suite.json");
const suiteDigest = `sha256:${"d".repeat(64)}`;
fs.writeFileSync(suitePath, JSON.stringify({ schema_version: "planr.eval-suite.v1", digest: suiteDigest }));
fs.writeFileSync(fakePlanr, `#!/usr/bin/env node
import fs from 'node:fs';
const scenario = process.env.FAKE_SCENARIO;
const args = process.argv.slice(2);
fs.appendFileSync(process.env.FAKE_CALL_LOG, JSON.stringify(args) + '\\n');
const kind = args.at(-2);
const id = args.at(-1);
const subjectRevision = process.env.FAKE_SUBJECT_REVISION;
if (args.includes('suite-check')) {
  if (process.cwd() !== process.env.FAKE_SUITE_DIR) process.exit(5);
  if (scenario === 'tampered-suite-content') process.exit(3);
  console.log(JSON.stringify({ok:true,object:{suite:{digest:process.env.FAKE_SUITE}}}));
} else if (args.includes('compare')) {
  if (scenario === 'regressed') process.exit(1);
  if (scenario === 'insufficient' || scenario === 'stale-run') process.exit(2);
  const verdict = scenario === 'no-material' ? 'no_material_difference' : 'improved';
  console.log(JSON.stringify({ok:true,object:{comparison:{id:'comparison-recomputed',baseline_run_id:'baseline-run',candidate_run_id:'candidate-run',policy_digest:'policy-v1',recompute_of:'comparison-pass',verdict}}}));
} else if (args.includes('gate')) {
  console.log(JSON.stringify({ok:true,object:{verdict:scenario === 'no-material' ? 'no_material_difference' : 'improved'}}));
} else if (kind === 'comparison') {
  console.log(JSON.stringify({ok:true,object:{comparison:{id,baseline_run_id:'baseline-run',candidate_run_id:'candidate-run',policy_digest:'policy-v1',verdict:'improved'}}}));
} else if (kind === 'run') {
  const effective = {client:'codex',provider:'openai',runtime:'local-host',model:'observed-model',effort:'observed-effort',profile_id:'observed-profile',profile_digest:'sha256:'+'a'.repeat(64),route_policy_digest:'sha256:'+'c'.repeat(64),runner_version:'release-eval-runner-v1',harness_version:'release-eval-v1',confidence:'verified'};
  const validation = {source:'planr.route_audit.v1',status:'verified',effective:{client:effective.client,provider:effective.provider,runtime:effective.runtime,model:effective.model,effort:effective.effort,profile_id:effective.profile_id,profile_config_digest:effective.profile_digest,runner_harness_version:effective.harness_version,agent_type:'codex-worker'}};
  const attempt = {countable:true,effective_client:effective.client,effective_provider:effective.provider,effective_runtime:effective.runtime,effective_model:effective.model,effective_effort:effective.effort,effective_profile_id:effective.profile_id,profile_config_digest:effective.profile_digest,runner_harness_version:effective.harness_version,route_observation_validation:validation};
  if (scenario === 'unauthenticated') attempt.route_observation_validation = {source:'planr.route_audit.v1',status:'invalid'};
  if (scenario === 'unproven-treatment') attempt.route_observation_validation = {source:'planr.route_audit.v1',status:'invalid'};
  console.log(JSON.stringify({ok:true,object:{run:{id,suite_digest:process.env.FAKE_SUITE,subject_revision:scenario === 'mismatched' ? 'sha256:'+'b'.repeat(64) : subjectRevision,status:'success',invalidated_by:null,cases:[{attempts:[attempt]}]}}}));
} else process.exit(4);
`);
fs.chmodSync(fakePlanr, 0o755);

const callLog = path.join(tmp, "planr-calls.jsonl");
const baseReceipt = {
  schema_version: "planr.release-eval-receipt.v2",
  comparison_id: "comparison-pass",
  candidate_run_id: "candidate-run",
  suite_digest: suiteDigest,
  candidate_revision: revision,
  evaluated_subject_revision: subjectRevision,
  created_at: new Date(Date.now() - 60_000).toISOString(),
  expires_at: new Date(Date.now() + 60 * 60_000).toISOString(),
};

function runCase(name, mutate = (receipt) => receipt) {
  const receipt = mutate(structuredClone(baseReceipt));
  const receiptPath = path.join(tmp, `${name}.json`);
  fs.writeFileSync(receiptPath, JSON.stringify(receipt));
  return spawnSync(process.execPath, [verifier, "--receipt", receiptPath, "--db", path.join(tmp, "eval.sqlite"), "--suite", suitePath, "--planr-bin", fakePlanr], {
    cwd: repo,
    encoding: "utf8",
    env: { ...process.env, FAKE_SCENARIO: name, FAKE_SUBJECT_REVISION: subjectRevision, FAKE_SUITE: suiteDigest, FAKE_SUITE_DIR: path.dirname(suitePath), FAKE_CALL_LOG: callLog },
  });
}

fs.writeFileSync(callLog, "");
const passing = runCase("passing");
assert.equal(passing.status, 0, `passing evidence must proceed: ${passing.stderr}`);
const passingCalls = fs.readFileSync(callLog, "utf8").trim().split("\n").map((line) => JSON.parse(line));
assert.deepEqual(
  passingCalls.map((args) => args.includes("suite-check") ? "suite-check" : args.includes("compare") ? "compare" : args.includes("gate") ? "gate" : `show-${args.at(-2)}`),
  ["suite-check", "show-comparison", "show-run", "compare", "gate"],
  "release verifier must canonicalize suite and recompute comparison before gate"
);
for (const scenario of ["regressed", "insufficient", "no-material", "mismatched", "unauthenticated"]) {
  assert.notEqual(runCase(scenario).status, 0, `${scenario} evidence must fail closed`);
}
assert.notEqual(runCase("stale", (receipt) => ({ ...receipt, expires_at: new Date(Date.now() - 1000).toISOString() })).status, 0, "stale receipt must fail closed");
assert.notEqual(runCase("unsafe", (receipt) => ({ ...receipt, raw_prompt: "forbidden" })).status, 0, "unsafe receipt must fail closed");
assert.notEqual(runCase("stale-subject", (receipt) => ({ ...receipt, evaluated_subject_revision: `sha256:${"e".repeat(64)}` })).status, 0, "stale evaluated subject must fail closed");
for (const scenario of ["stale-run", "tampered-suite-content", "unproven-treatment"]) {
  assert.notEqual(runCase(scenario).status, 0, `${scenario} evidence must fail closed`);
}

const candidateFixture = path.join(tmp, "candidate-source-binding");
const candidateVerifier = path.join(candidateFixture, "scripts", "verify-release-eval-receipt.mjs");
fs.mkdirSync(path.dirname(candidateVerifier), { recursive: true });
fs.copyFileSync(verifier, candidateVerifier);
fs.writeFileSync(path.join(candidateFixture, "README.md"), "candidate source v1\n");
for (const args of [["init", "-q"], ["add", "."]]) {
  const git = spawnSync("git", args, { cwd: candidateFixture, encoding: "utf8" });
  assert.equal(git.status, 0, `candidate binding fixture git ${args[0]} failed: ${git.stderr}`);
}
const originalCandidateRevision = spawnSync(
  process.execPath,
  [candidateVerifier, "--print-candidate-revision"],
  { cwd: candidateFixture, encoding: "utf8" },
);
assert.equal(originalCandidateRevision.status, 0, originalCandidateRevision.stderr);
const staleCandidateReceipt = path.join(tmp, "stale-candidate-source-receipt.json");
fs.writeFileSync(staleCandidateReceipt, JSON.stringify({
  ...baseReceipt,
  candidate_revision: originalCandidateRevision.stdout.trim(),
}));
fs.writeFileSync(path.join(candidateFixture, "README.md"), "candidate source v2\n");
const changedCandidateRevision = spawnSync(
  process.execPath,
  [candidateVerifier, "--print-candidate-revision"],
  { cwd: candidateFixture, encoding: "utf8" },
);
assert.equal(changedCandidateRevision.status, 0, changedCandidateRevision.stderr);
assert.notEqual(
  changedCandidateRevision.stdout.trim(),
  originalCandidateRevision.stdout.trim(),
  "changing any release source file must change the candidate revision",
);
const staleCandidateSource = spawnSync(process.execPath, [
  candidateVerifier,
  "--receipt", staleCandidateReceipt,
  "--db", path.join(tmp, "eval.sqlite"),
  "--suite", suitePath,
  "--planr-bin", fakePlanr,
], {
  cwd: candidateFixture,
  encoding: "utf8",
  env: {
    ...process.env,
    FAKE_SCENARIO: "passing",
    FAKE_SUBJECT_REVISION: subjectRevision,
    FAKE_SUITE: suiteDigest,
    FAKE_SUITE_DIR: path.dirname(suitePath),
    FAKE_CALL_LOG: callLog,
  },
});
assert.notEqual(staleCandidateSource.status, 0, "receipt must fail after any release source file changes");

const release = fs.readFileSync(path.join(repo, "scripts/release.sh"), "utf8");
const gateIndex = release.indexOf("node scripts/verify-release-eval-receipt.mjs");
assert.ok(gateIndex > release.indexOf("cargo build --quiet"), "eval gate must run after candidate build");
for (const mutation of ["git tag ", "git push "]) {
  assert.ok(gateIndex < release.indexOf(mutation), `eval gate must precede ${mutation.trim()}`);
}
for (const forbiddenMutation of ["git add ", "git commit "]) {
  assert.ok(!release.includes(forbiddenMutation), `publication must not run ${forbiddenMutation.trim()}`);
}
for (const forbidden of ["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "NPM_TOKEN", "raw_prompt", "raw_completion"]) {
  assert.ok(!release.includes(forbidden), `release path must not request ${forbidden}`);
}
assert.ok(!release.includes("PLANR_RELEASE_PLANR_BIN"), "release gate must execute the freshly built candidate binary");
assert.ok(release.includes("PLANR_RELEASE_EVAL_SUITE"), "release path must require an explicit external suite");
assert.ok(release.includes("PLANR_RELEASE_EVAL_DB"), "release path must require an explicit external eval database");
const privateSuitePath = ["examples", "eval", "lean-skills"].join("/");
for (const source of [release, fs.readFileSync(verifier, "utf8"), fs.readFileSync(fileURLToPath(import.meta.url), "utf8")]) {
  assert.ok(!source.includes(privateSuitePath), "public gate code must not depend on the private lean-skills path");
}
const workflow = fs.readFileSync(path.join(repo, ".github/workflows/release.yml"), "utf8");
for (const forbidden of ["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "NPM_TOKEN", "verify-release-eval-receipt", "planr eval"]) {
  assert.ok(!workflow.includes(forbidden), `CI release workflow must not contain ${forbidden}`);
}
assert.ok(workflow.includes("id-token: write"), "npm Trusted Publishing OIDC permission must remain");
assert.ok(workflow.includes("npm publish --access public"), "npm publication step must remain OIDC-compatible");

console.log(JSON.stringify({verdict:"pass", positive:1, negative:12, exact_candidate_source_bound:true, evaluated_subject_bound:true, gate_before_git_mutations:true, comparison_recomputed:true, current_suite_canonicalized:true, suite_fixtures_resolve_from_external_workspace:true, material_improvement_required:true, planr_route_validation_required:true, second_verdict_engine:false}, null, 2));
