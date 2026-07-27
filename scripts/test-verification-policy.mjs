import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  classifyChanges,
  parseGitNameStatus,
  POLICY_DIGEST,
  POLICY_RULES,
  policyDigestForRules,
} from "./verification-policy.mjs";

const fixtureUrl = new URL("./fixtures/verification-policy/cases.json", import.meta.url);
const fixtures = JSON.parse(readFileSync(fixtureUrl, "utf8"));

for (const fixture of fixtures) {
  const selection = classifyChanges(fixture.changes);
  assert.equal(selection.profile, fixture.profile, `${fixture.name}: profile`);
  assert.deepEqual(selection.matchedPathClasses, fixture.classes, `${fixture.name}: path classes`);
  for (const gate of fixture.included ?? []) {
    assert.ok(selection.selectedGates.includes(gate), `${fixture.name}: expected gate ${gate}`);
  }
  for (const gate of fixture.excluded ?? []) {
    assert.ok(!selection.selectedGates.includes(gate), `${fixture.name}: excluded gate ${gate}`);
  }
  if (fixture.escalation) {
    assert.ok(selection.escalationReasons.some(({ code }) => code === fixture.escalation), `${fixture.name}: escalation reason`);
  }
  assert.equal(selection.reasons.length, selection.selectedGates.length, `${fixture.name}: every gate has a reason`);
  assert.ok(selection.reasons.every(({ gate, detail }) => gate && detail), `${fixture.name}: reasons are explanatory`);
}

const deterministicA = classifyChanges([
  { status: "M", path: "README.md" },
  { status: "M", path: "src/model.rs" },
]);
const deterministicB = classifyChanges([
  { status: "modified", path: "src/model.rs" },
  { status: "modified", path: "README.md" },
]);
assert.equal(deterministicA.policyDigest, POLICY_DIGEST);
assert.equal(deterministicA.changedFilesDigest, deterministicB.changedFilesDigest, "change digest is order-independent");
assert.deepEqual(deterministicA.selectedGates, deterministicB.selectedGates, "selection is order-independent");

const remappedRules = structuredClone(POLICY_RULES);
const docsContentRule = remappedRules.find(({ id }) => id === "docs-content");
docsContentRule.matchers[0].source = "^only-this-path-would-be-docs-content$";
assert.notEqual(
  policyDigestForRules(remappedRules),
  POLICY_DIGEST,
  "changing a path-to-profile matcher changes policy identity",
);

const revisionBound = classifyChanges([{ status: "M", path: "README.md" }], {
  baseRevision: "main",
  headRevision: "HEAD",
});
assert.equal(revisionBound.baseRevision, "main");
assert.equal(revisionBound.headRevision, "HEAD");

for (const runnerPath of ["scripts/verification-runner.mjs", "scripts/test-verification-runner.mjs"]) {
  const runnerSelection = classifyChanges([{ status: "M", path: runnerPath }]);
  assert.equal(runnerSelection.profile, "full", `${runnerPath}: runner changes require full verification`);
  assert.deepEqual(runnerSelection.matchedPathClasses, ["policy"], `${runnerPath}: runner is owned by policy infrastructure`);
}

assert.deepEqual(parseGitNameStatus("M\0README.md\0R100\0src/old.rs\0src/new.rs\0"), [
  { status: "M", path: "README.md" },
  { status: "R", oldPath: "src/old.rs", newPath: "src/new.rs" },
]);

for (const invalid of [
  undefined,
  [],
  [{ status: "M", path: "../escape" }],
  [{ status: "R", oldPath: "README.md" }],
  [{ status: "AA", path: "src/model.rs" }],
  [{ status: "Modified-ish", path: "src/model.rs" }],
]) {
  const selection = classifyChanges(invalid);
  assert.equal(selection.profile, "full", "invalid input fails closed");
  assert.equal(selection.escalatedToFull, true, "invalid input records escalation");
}

// This suite imports the pure classifier directly. Gate identifiers are data;
// no command runner or child process is reachable from classifier tests.
console.log(JSON.stringify({
  verdict: "pass",
  fixtures: fixtures.length,
  expensive_gate_commands_run: 0,
  policy_digest: POLICY_DIGEST,
}, null, 2));
