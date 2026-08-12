import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { runExactProductOracle } from "./ac014-exact-product-oracle.mjs";

const root = mkdtempSync(path.join(tmpdir(), "planr-ac014-oracle-"));
process.on("exit", () => rmSync(root, { recursive: true, force: true }));
mkdirSync(path.join(root, ".planr", "evidence", "runs"), { recursive: true });
writeFileSync(path.join(root, ".planr", "evidence", "runs", "sealed.json"), "{}\n");
const calls = path.join(root, "calls.jsonl");
const binary = path.join(root, "planr");
writeFileSync(binary, stubPlanr(calls));
execFileSync("chmod", ["555", binary]);
const digest = sha256(binary);
const sourceRevision = "1".repeat(40);

const passed = runExactProductOracle({ planrBin: binary, candidateSha256: digest, planId: "pln-dogfood", sourceRevision, cwd: root });
assert.equal(passed.status, "passed");
assert.equal(passed.candidate_binary_sha256, digest);
assert.equal(passed.browser_requirements_covered, 12);
assert.equal(passed.build_requirements_covered, 1);
assert.equal(passed.waivers, 0);
assert.deepEqual(readFileSync(calls, "utf8").trim().split("\n").map(JSON.parse), [
  ["evidence", "readiness"],
  ["evidence", "run"],
  ["evidence", "coverage"],
  ["plan", "audit"],
]);

assert.throws(() => runExactProductOracle({
  planrBin: binary,
  candidateSha256: `sha256:${"0".repeat(64)}`,
  planId: "pln-dogfood",
  sourceRevision,
  cwd: root,
}), /candidate binary digest mismatch/);

process.env.PLANR_ORACLE_STUB_MODE = "missing-browser";
assert.throws(() => runExactProductOracle({ planrBin: binary, candidateSha256: digest, planId: "pln-dogfood", sourceRevision, cwd: root }), /exactly 12 browser runs and one build run/);
delete process.env.PLANR_ORACLE_STUB_MODE;

process.env.PLANR_ORACLE_STUB_MODE = "waived";
assert.throws(() => runExactProductOracle({ planrBin: binary, candidateSha256: digest, planId: "pln-dogfood", sourceRevision, cwd: root }), /may not be waived/);
delete process.env.PLANR_ORACLE_STUB_MODE;

process.env.PLANR_ORACLE_STUB_MODE = "untrusted";
assert.throws(() => runExactProductOracle({ planrBin: binary, candidateSha256: digest, planId: "pln-dogfood", sourceRevision, cwd: root }), /not trusted and gap-free/);
delete process.env.PLANR_ORACLE_STUB_MODE;

process.env.PLANR_ORACLE_STUB_MODE = "wrong-source";
assert.throws(() => runExactProductOracle({ planrBin: binary, candidateSha256: digest, planId: "pln-dogfood", sourceRevision, cwd: root }), /run index source mismatch/);
delete process.env.PLANR_ORACLE_STUB_MODE;

process.env.PLANR_ORACLE_STUB_MODE = "stale";
assert.throws(() => runExactProductOracle({ planrBin: binary, candidateSha256: digest, planId: "pln-dogfood", sourceRevision, cwd: root }), /freshness validation is stale or incomplete/);
delete process.env.PLANR_ORACLE_STUB_MODE;

process.env.PLANR_ORACLE_STUB_MODE = "unavailable";
assert.throws(() => runExactProductOracle({ planrBin: binary, candidateSha256: digest, planId: "pln-dogfood", sourceRevision, cwd: root }), /readiness is not passed/);
delete process.env.PLANR_ORACLE_STUB_MODE;

for (const [mode, expected] of [
  ["missing-result", /exactly 13 results/],
  ["bundled-observations", /exactly one run-bound observation/],
  ["duplicate-result", /duplicate or ambiguous sealed-run result|cross-run reuse/],
  ["extra-result", /exactly 13 results/],
  ["cross-run-receipt-reuse", /cross-run reuse of receipt id/],
  ["wrong-result-target", /target does not match its sealed run/],
  ["cross-run-observation", /observation does not match its sealed run requirement/],
  ["missing-sealed-capability", /capability.instance_id is required/],
  ["wrong-sealed-capability", /capability metadata does not match its input/],
  ["missing-wrapper-digest", /must carry exact sha256 digests/],
  ["invalid-wrapper-digest", /must carry exact sha256 digests/],
  ["invalid-embedded-digest", /must carry exact sha256 digests/],
  ["mismatched-wrapper-digest", /wrapper digest does not match its trusted receipt/],
  ["reused-wrapper-digest", /cross-run reuse of bound receipt digest/],
]) {
  process.env.PLANR_ORACLE_STUB_MODE = mode;
  assert.throws(() => runExactProductOracle({ planrBin: binary, candidateSha256: digest, planId: "pln-dogfood", sourceRevision, cwd: root }), expected);
  delete process.env.PLANR_ORACLE_STUB_MODE;
}

console.log("AC-014 exact candidate/browser oracle contract passed");

function sha256(file) {
  return `sha256:${createHash("sha256").update(readFileSync(file)).digest("hex")}`;
}

function stubPlanr(callsPath) {
  return `#!/usr/bin/env node
import { appendFileSync } from "node:fs";
const args = process.argv.slice(2);
appendFileSync(${JSON.stringify(callsPath)}, JSON.stringify(args.slice(0, 2)) + "\\n");
const criteria = Array.from({ length: 12 }, (_, i) => "AC-" + String(i + 1).padStart(3, "0"));
const requirements = [...criteria.map((criterion) => "obs-" + criterion + "-browser"), "obs-AC-012-build"];
const mode = process.env.PLANR_ORACLE_STUB_MODE;
if (args[0] === "evidence" && args[1] === "readiness") {
  if (mode === "unavailable") { console.log(JSON.stringify({ object: { status: "failed", gaps: [{ reason: "missing_capability" }] } })); process.exit(0); }
  const selected = mode === "missing-browser" ? criteria.slice(0, -1) : criteria;
  const runs = selected.map((criterion, index) => ({ index, capability: { instance_id: "cap-browser" }, input: { obligation_id: "pob-" + criterion, capability_instance_id: "cap-browser", target: { kind: "browser", uri: "http://127.0.0.1:3000/#" + criterion } } }));
  runs.push({ index: runs.length, capability: { instance_id: "cap-build" }, input: { obligation_id: "pob-build", capability_instance_id: "cap-build", target: { kind: "process", uri: "local://pnpm-build" } } });
  if (mode === "missing-sealed-capability") delete runs[0].capability;
  if (mode === "wrong-sealed-capability") runs[0].capability.instance_id = "cap-wrong";
  console.log(JSON.stringify({ object: { status: "passed", run_index: { schema_version: "planr.evidence.run-index.v1", repository_path: ".planr/evidence/runs/sealed.json", run_index_digest: "sha256:" + "a".repeat(64), source: { dirty: false, revision: mode === "wrong-source" ? "2".repeat(40) : "1".repeat(40) }, runs } } }));
} else if (args[0] === "evidence" && args[1] === "run") {
  let results = requirements.map((requirement_id, index) => {
    const browser = index < 12;
    const obligation_id = browser ? "pob-" + criteria[index] : "pob-build";
    const capability_instance_id = browser ? "cap-browser" : "cap-build";
    const target = browser ? { kind: "browser", uri: "http://127.0.0.1:3000/#" + criteria[index] } : { kind: "process", uri: "local://pnpm-build" };
    const attemptId = "attempt-" + index;
    const receipt_digest = "sha256:" + String(index).padStart(64, "0");
    return { verdict: "passed", reused: false, receipt_digest, attempt: { id: attemptId, obligation_id, capability_instance_id }, receipt: { id: "receipt-" + index, receipt_digest, obligation_id, attempt_ids: [attemptId], capability: { instance_id: capability_instance_id }, target, receipt_status: mode === "untrusted" ? "diagnostic" : "trusted", proof_gaps: [], source: { dirty: false, revision: "1".repeat(40) }, observations: [{ requirement_id, outcome: "passed" }] } };
  });
  if (mode === "missing-result") results = results.slice(0, -1);
  if (mode === "bundled-observations") results[0].receipt.observations.push({ requirement_id: requirements[1], outcome: "passed" });
  if (mode === "duplicate-result") results[12] = structuredClone(results[0]);
  if (mode === "extra-result") results.push(structuredClone(results[0]));
  if (mode === "cross-run-receipt-reuse") { results[1].receipt.id = results[0].receipt.id; results[1].receipt_digest = results[0].receipt_digest; results[1].receipt.receipt_digest = results[0].receipt.receipt_digest; }
  if (mode === "wrong-result-target") results[0].receipt.target = structuredClone(results[1].receipt.target);
  if (mode === "cross-run-observation") results[0].receipt.observations[0].requirement_id = requirements[1];
  if (mode === "missing-wrapper-digest") delete results[0].receipt_digest;
  if (mode === "invalid-wrapper-digest") results[0].receipt_digest = "sha256:not-a-digest";
  if (mode === "invalid-embedded-digest") results[0].receipt.receipt_digest = "invalid";
  if (mode === "mismatched-wrapper-digest") results[0].receipt_digest = "sha256:" + "f".repeat(64);
  if (mode === "reused-wrapper-digest") { results[1].receipt_digest = results[0].receipt_digest; results[1].receipt.receipt_digest = results[0].receipt.receipt_digest; }
  console.log(JSON.stringify({ object: { status: "passed", verdict: "passed", results } }));
} else if (args[0] === "evidence" && args[1] === "coverage") {
  const validation_details = Object.fromEntries(["completion", "fixture", "freshness", "provenance", "schema", "target", "trust"].map((name) => [name, { status: mode === "stale" && name === "freshness" ? "failed" : "passed" }]));
  console.log(JSON.stringify({ object: { status: "satisfied", verdict: "satisfied", waiver_digests: mode === "waived" ? ["sha256:" + "b".repeat(64)] : [], coverage: { validation_details }, canonical_projection: { pass: true, waiver_refs: [], waiver_digests: [], observations: requirements.map((requirement_id) => ({ requirement_id, status: "covered" })) } } }));
} else if (args[0] === "plan" && args[1] === "audit") {
  console.log(JSON.stringify({ holds: true }));
} else process.exit(2);
`;
}
