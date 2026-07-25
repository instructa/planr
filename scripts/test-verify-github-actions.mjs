import assert from "node:assert/strict";
import { cp, mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fixtureRoot = await mkdtemp(path.join(os.tmpdir(), "planr-github-actions-test-"));
const fixtureScripts = path.join(fixtureRoot, "scripts");
const fixtureWorkflows = path.join(fixtureRoot, ".github", "workflows");
const verifier = path.join(fixtureScripts, "verify-github-actions.mjs");
const releaseWorkflow = path.join(fixtureWorkflows, "release.yml");

function runVerifier() {
  return spawnSync(process.execPath, [verifier], {
    cwd: fixtureRoot,
    encoding: "utf8",
  });
}

try {
  await mkdir(fixtureScripts, { recursive: true });
  await cp(path.join(repoRoot, "scripts", "verify-github-actions.mjs"), verifier);
  await cp(path.join(repoRoot, ".github", "workflows"), fixtureWorkflows, { recursive: true });

  const baseline = runVerifier();
  assert.equal(baseline.status, 0, `baseline workflow fixture must pass:\n${baseline.stderr}`);

  const source = await readFile(releaseWorkflow, "utf8");
  const smokeMarker = "      - name: Smoke-test binary\n";
  assert.ok(source.includes(smokeMarker), "release fixture must contain the smoke step");

  await writeFile(releaseWorkflow, source.replace(smokeMarker, `${smokeMarker}        if: always()\n`));
  const conditional = runVerifier();
  assert.notEqual(conditional.status, 0, "a conditional smoke step must fail verification");
  assert.match(
    `${conditional.stdout}\n${conditional.stderr}`,
    /must be unconditional for every matrix target/u,
    "conditional failure must explain the release invariant",
  );

  await writeFile(releaseWorkflow, source.replace(smokeMarker, `${smokeMarker}        continue-on-error: true\n`));
  const toleratedFailure = runVerifier();
  assert.notEqual(toleratedFailure.status, 0, "a non-blocking smoke step must fail verification");
  assert.match(
    `${toleratedFailure.stdout}\n${toleratedFailure.stderr}`,
    /must fail the build job on error/u,
    "continue-on-error failure must explain the release invariant",
  );

  console.log("github_actions_regression=passed conditional_smoke_rejected=true continue_on_error_rejected=true");
} finally {
  await rm(fixtureRoot, { recursive: true, force: true });
}
