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
  const runs = selected.map((criterion) => ({ input: { target: { kind: "browser", uri: "http://127.0.0.1:3000/#" + criterion } } }));
  runs.push({ input: { target: { kind: "process", uri: "local://pnpm-build" } } });
  console.log(JSON.stringify({ object: { status: "passed", run_index: { schema_version: "planr.evidence.run-index.v1", repository_path: ".planr/evidence/runs/sealed.json", run_index_digest: "sha256:" + "a".repeat(64), source: { dirty: false, revision: mode === "wrong-source" ? "2".repeat(40) : "1".repeat(40) }, runs } } }));
} else if (args[0] === "evidence" && args[1] === "run") {
  const results = requirements.map((requirement_id) => ({ verdict: "passed", receipt: { receipt_status: mode === "untrusted" ? "diagnostic" : "trusted", proof_gaps: [], source: { dirty: false, revision: "1".repeat(40) }, observations: [{ requirement_id, outcome: "passed" }] } }));
  console.log(JSON.stringify({ object: { status: "passed", verdict: "passed", results } }));
} else if (args[0] === "evidence" && args[1] === "coverage") {
  const validation_details = Object.fromEntries(["completion", "fixture", "freshness", "provenance", "schema", "target", "trust"].map((name) => [name, { status: mode === "stale" && name === "freshness" ? "failed" : "passed" }]));
  console.log(JSON.stringify({ object: { status: "satisfied", verdict: "satisfied", waiver_digests: mode === "waived" ? ["sha256:" + "b".repeat(64)] : [], coverage: { validation_details }, canonical_projection: { pass: true, waiver_refs: [], waiver_digests: [], observations: requirements.map((requirement_id) => ({ requirement_id, status: "covered" })) } } }));
} else if (args[0] === "plan" && args[1] === "audit") {
  console.log(JSON.stringify({ holds: true }));
} else process.exit(2);
`;
}
