import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const workflowsRoot = path.join(repoRoot, ".github", "workflows");

const expectedActions = new Map([
  ["actions/checkout", { sha: "3d3c42e5aac5ba805825da76410c181273ba90b1", version: "v7.0.1", runtime: "node24" }],
  ["actions/setup-node", { sha: "820762786026740c76f36085b0efc47a31fe5020", version: "v7.0.0", runtime: "node24" }],
  ["actions/upload-artifact", { sha: "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a", version: "v7.0.1", runtime: "node24" }],
  ["actions/download-artifact", { sha: "3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c", version: "v8.0.1", runtime: "node24" }],
  ["aquasecurity/trivy-action", { sha: "ed142fd0673e97e23eac54620cfb913e5ce36c25", version: "v0.36.0", runtime: "composite" }],
  ["trufflesecurity/trufflehog", { sha: "6f3c981e7b77f235fd2702dd74af25fc4b72bf11", version: "v3.96.0", runtime: "composite" }],
]);

const workflowFiles = (await readdir(workflowsRoot))
  .filter((file) => file.endsWith(".yml") || file.endsWith(".yaml"))
  .sort();
const seen = new Map();

for (const file of workflowFiles) {
  const source = await readFile(path.join(workflowsRoot, file), "utf8");
  const lines = source.split("\n");

  for (const [index, line] of lines.entries()) {
    if (!/^\s*uses:/u.test(line)) continue;

    const match = /^\s*uses:\s*([^@\s]+)@([0-9a-f]{40})\s+#\s+(\S+)\s*$/u.exec(line);
    assert.ok(match, `${file}:${index + 1} must use a full immutable commit SHA and exact version comment`);

    const [, action, sha, version] = match;
    const expected = expectedActions.get(action);
    assert.ok(expected, `${file}:${index + 1} uses unreviewed action ${action}`);
    assert.equal(sha, expected.sha, `${file}:${index + 1} has stale SHA for ${action}`);
    assert.equal(version, expected.version, `${file}:${index + 1} has stale version comment for ${action}`);
    seen.set(action, (seen.get(action) ?? 0) + 1);
  }
}

for (const [action, expected] of expectedActions) {
  assert.ok(seen.has(action), `expected workflow action is missing: ${action}`);
  if (expected.runtime === "node24") {
    assert.match(expected.version, /^v(?:7|8)\./u, `${action} must remain on its Node 24 major`);
  }
}

const securityWorkflow = await readFile(path.join(workflowsRoot, "security.yml"), "utf8");
assert.ok(securityWorkflow.includes("version: 3.96.0"), "TruffleHog image version must match the pinned action release");

const releaseWorkflow = await readFile(path.join(workflowsRoot, "release.yml"), "utf8");
for (const target of ["darwin-arm64", "darwin-x86_64", "linux-x86_64", "linux-arm64"]) {
  assert.ok(releaseWorkflow.includes(`target: ${target}`), `release matrix must include ${target}`);
}

const smokeStepMarker = "      - name: Smoke-test binary\n";
const smokeStepStart = releaseWorkflow.indexOf(smokeStepMarker);
assert.notEqual(smokeStepStart, -1, "release workflow must contain the binary smoke step");
assert.equal(
  releaseWorkflow.indexOf(smokeStepMarker, smokeStepStart + smokeStepMarker.length),
  -1,
  "release workflow must have exactly one shared matrix smoke step",
);
const nextStepStart = releaseWorkflow.indexOf("\n      - name:", smokeStepStart + smokeStepMarker.length);
assert.notEqual(nextStepStart, -1, "binary smoke step must be followed by another release step");
const smokeStep = releaseWorkflow.slice(smokeStepStart, nextStepStart);
assert.doesNotMatch(smokeStep, /^\s+if:/mu, "binary smoke step must be unconditional for every matrix target");
assert.doesNotMatch(smokeStep, /^\s+continue-on-error:/mu, "binary smoke step must fail the build job on error");
assert.match(
  smokeStep,
  /reported="\$\("\.\/target\/\$PLANR_CARGO_TARGET\/release\/planr" --version\)"/u,
  "release workflow must execute every matrix binary",
);
assert.match(
  smokeStep,
  /expected="planr \$\{TAG#v\}"/u,
  "release workflow must compare runtime output with the release tag",
);

console.log(
  `github_actions_check=passed workflows=${workflowFiles.length} uses=${[...seen.values()].reduce((sum, count) => sum + count, 0)} node24_actions=4 immutable_sha_pins=true release_arch_runtime_gate=4`,
);
