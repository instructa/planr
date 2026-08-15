import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

if (!process.env.PLANR_EVIDENCE_TARGET_JSON) {
  process.stdout.write(`${JSON.stringify({ probe: true })}\n`);
  process.exit(0);
}

const target = JSON.parse(process.env.PLANR_EVIDENCE_TARGET_JSON);
const signal = new URL(target.uri).pathname.split("/").filter(Boolean).at(-1);
const tests = {
  "plan-criteria": "complete_binding_plan_criteria_contract_rejects_invalid_identity_sets",
  authority: "complete_binding_authority_requires_the_exact_declared_criterion_set",
  lifecycle: "complete_binding_lifecycle_fails_closed_for_partial_active_rows",
};

function fail(message) {
  process.stderr.write(String(message).slice(-4096));
  process.exit(1);
}

function productionRustFiles(root) {
  const files = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const candidate = path.join(root, entry.name);
    if (entry.isDirectory()) files.push(...productionRustFiles(candidate));
    else if (entry.isFile() && entry.name.endsWith(".rs")) files.push(candidate);
  }
  return files;
}

function verifySingleOwnerInventory() {
  const root = process.cwd();
  const writes = [];
  for (const file of productionRustFiles(path.join(root, "src"))) {
    const production = readFileSync(file, "utf8").split(/#\[cfg\(test\)\]/u, 1)[0];
    if (production.includes("INSERT INTO proof_obligations")) {
      writes.push(path.relative(root, file));
    }
  }
  if (writes.length !== 1 || writes[0] !== "src/app/evidence.rs") {
    fail(`production ProofObligation writers drifted: ${JSON.stringify(writes)}`);
  }

  const migration = readFileSync(path.join(root, "src/app/evidence.rs"), "utf8");
  if (!migration.includes("fn insert_migrated_evidence_obligation")) {
    fail("the sole production obligation writer is not the migration boundary");
  }
  const proof = readFileSync(path.join(root, "src/app/proof.rs"), "utf8");
  if (!proof.includes("authoritative_plan_obligation_bindings")
      || proof.includes("proof_obligations")
      || proof.includes("rusqlite::params")) {
    fail("app/proof must consume the typed Evidence loader without SQL");
  }
  const coverage = readFileSync(path.join(root, "src/evidence/coverage.rs"), "utf8");
  if (!coverage.includes("pub struct AuthoritativeObligationBindingRow")
      || !coverage.includes("pub fn authoritative_plan_obligation_bindings")) {
    fail("Evidence coverage must expose the one typed authoritative-row loader");
  }

  for (const relative of [
    "src/app/audit_evidence.rs",
    "src/app/final_review_admission.rs",
    "src/app/execution_state.rs",
    "src/app/stop.rs",
  ]) {
    const source = readFileSync(path.join(root, relative), "utf8");
    if (/build_plan_criteria|parse_plan_metadata|proof_obligations/u.test(source)
        || !/plan_evidence_authority|proof_status_for_plan/u.test(source)) {
      fail(`${relative} recomputes completeness instead of consuming app/proof`);
    }
  }
  const planSkill = readFileSync(path.join(root, "plugins/planr/skills/planr-plan/SKILL.md"), "utf8");
  const goalSkill = readFileSync(path.join(root, "plugins/planr/skills/planr-goal/SKILL.md"), "utf8");
  if (!planSkill.includes("readable narrative, never an identity source")
      || !planSkill.includes("Do not infer criterion IDs from prose")
      || !goalSkill.includes("Never write obligations directly")
      || !goalSkill.includes("duplicate `app/proof` completeness rules")) {
    fail("Planr skills must delegate identity and completeness to their canonical owners");
  }
  return [
    "migration-is-sole-production-obligation-writer",
    "evidence-coverage-owns-authoritative-row-selection",
    "app-proof-is-sole-completeness-owner",
    "lifecycle-surfaces-and-skills-only-consume-authority",
  ];
}

if (signal === "single-owner") {
  process.stdout.write(`${JSON.stringify({
    status: "passed",
    verification_mode: "no_model",
    signal,
    checks: verifySingleOwnerInventory(),
  })}\n`);
  process.exit(0);
}

const testName = tests[signal];
if (!testName) fail(`unsupported focused verification signal: ${signal}`);
const result = spawnSync(
  "cargo",
  ["test", "--test", "e2e", testName, "--", "--exact", "--test-threads=1"],
  {
    cwd: process.cwd(),
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 1_200_000,
  },
);

if (result.error) fail(result.error.message);
if (result.status !== 0) {
  fail(result.stderr || result.stdout || "focused verification failed");
}

process.stdout.write(`${JSON.stringify({
  status: "passed",
  verification_mode: "no_model",
  signal,
  checks: [`e2e:${testName}`],
})}\n`);
