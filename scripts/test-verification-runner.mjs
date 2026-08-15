import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { classifyChanges, POLICY_DIGEST } from "./verification-policy.mjs";
import {
  commandPlanFor,
  linuxTargetCommandPlan,
  runLinuxTargetVerification,
  runVerification,
  selectVerificationGates,
  selectionFromInput,
  verifyLinuxTargetReceipt,
  verifyReceipt,
  worktreeChanges,
} from "./verification-runner.mjs";

const root = mkdtempSync(path.join(tmpdir(), "planr-verification-runner-"));
process.on("exit", () => rmSync(root, { recursive: true, force: true }));
mkdirSync(path.join(root, "apps/docs/components"), { recursive: true });
mkdirSync(path.join(root, "apps/docs/out"), { recursive: true });
writeFileSync(path.join(root, "apps/docs/components/card.tsx"), "export const Card = () => null;\n");
writeFileSync(path.join(root, "apps/docs/out/index.html"), "<h1>Planr</h1>\n");
writeFileSync(path.join(root, "README.md"), "# Fixture\n");
git("init", "-q");
git("config", "user.name", "Planr Test");
git("config", "user.email", "planr-test@example.invalid");
git("add", ".");
git("commit", "-qm", "fixture");

const docsSelection = classifyChanges([{ status: "M", path: "apps/docs/components/card.tsx" }], {
  baseRevision: "HEAD^",
  headRevision: "HEAD",
});
const calls = [];
const receipt = runVerification({
  selection: docsSelection,
  repoRoot: root,
  execute(executable, args) {
    calls.push([executable, ...args]);
    return { status: 0 };
  },
});

assert.equal(receipt.verdict, "pass");
assert.equal(receipt.policy.digest, POLICY_DIGEST);
assert.equal(calls.filter((call) => call.join(" ").includes("@planr/docs build")).length, 1, "docs build runs exactly once");
assert.equal(calls.filter((call) => call.join(" ").includes("cargo install")).length, 0, "docs work never runs release-profile cargo install");
assert.equal(calls.filter((call) => call.join(" ").includes("security:check")).length, 0, "docs work never runs local security tooling");
assert.equal(receipt.selection.liveVerification.browser, false, "interactive docs receipts remain Chrome-free in CI");
assert.equal(new Set(calls.map(JSON.stringify)).size, calls.length, "runner de-duplicates exact commands");
assert.deepEqual(verifyReceipt(receipt, { selection: docsSelection, repoRoot: root }).verdict, "pass");

const d076SelectionInput = classifyChanges([
  { status: "M", path: "scripts/ac014-fresh-arm-runner.mjs" },
  { status: "M", path: "scripts/test-ac014-fresh-arm-runner.mjs" },
  { status: "M", path: "tests/e2e.rs" },
  { status: "A", path: "tests/fixtures/ac014/failed-transcript-min/history.jsonl" },
  { status: "A", path: "tests/fixtures/ac014/failed-transcript-min/manifest.json" },
  { status: "A", path: "tests/fixtures/ac014/failed-transcript-min/sessions/2026/08/08/rollout-2026-08-08T12-22-48-019fe0e5-8cef-7210-aea7-40722b23874e.jsonl" },
  { status: "M", path: "tests/fixtures/outcome-batching/v1/ac014-benchmark-input.json" },
], {
  baseRevision: "06a0fdaa3ec2981374dec9b5062d13d6bb70f435",
  headRevision: "d076723ab2ceab5669981d371edd9d15bb8d7eac",
});
const persistedD076Selection = selectionFromInput(structuredClone(d076SelectionInput));
assert.deepEqual(persistedD076Selection, d076SelectionInput, "persisted normalized selections are preserved exactly");
assert.deepEqual(
  persistedD076Selection.changes.flatMap(({ paths }) => paths),
  [
    "scripts/ac014-fresh-arm-runner.mjs",
    "scripts/test-ac014-fresh-arm-runner.mjs",
    "tests/e2e.rs",
    "tests/fixtures/ac014/failed-transcript-min/history.jsonl",
    "tests/fixtures/ac014/failed-transcript-min/manifest.json",
    "tests/fixtures/ac014/failed-transcript-min/sessions/2026/08/08/rollout-2026-08-08T12-22-48-019fe0e5-8cef-7210-aea7-40722b23874e.jsonl",
    "tests/fixtures/outcome-batching/v1/ac014-benchmark-input.json",
  ],
  "the d076 persisted selection keeps all seven remaining bound paths",
);
assert.equal(persistedD076Selection.changedFilesDigest, d076SelectionInput.changedFilesDigest);
assert.throws(
  () => selectionFromInput({ changes: structuredClone(d076SelectionInput.changes) }),
  /complete selection envelope/,
  "normalized persisted changes without their selection envelope fail closed",
);
const digestMismatchSelection = structuredClone(d076SelectionInput);
digestMismatchSelection.changedFilesDigest = `sha256:${"0".repeat(64)}`;
assert.throws(
  () => selectionFromInput(digestMismatchSelection),
  /changed-file digest mismatch/,
  "persisted selection digest drift fails closed",
);
const gateMismatchSelection = structuredClone(d076SelectionInput);
gateMismatchSelection.selectedGates = gateMismatchSelection.selectedGates.filter((gate) => gate !== "rust-test");
assert.throws(
  () => selectionFromInput(gateMismatchSelection),
  /selectedGates mismatch/,
  "persisted selection gate drift fails closed",
);
const receiptPath = ".planr/receipts/verification-receipt.json";
mkdirSync(path.dirname(path.join(root, receiptPath)), { recursive: true });
writeFileSync(path.join(root, receiptPath), `${JSON.stringify(receipt)}\n`);
assert.deepEqual(
  verifyReceipt(receipt, { selection: docsSelection, repoRoot: root, receiptPath }).verdict,
  "pass",
  "the explicitly supplied receipt output is not treated as a source input",
);
rmSync(path.join(root, receiptPath));

writeFileSync(path.join(root, "README.md"), `${JSON.stringify(receipt)}\n`);
assert.throws(
  () => verifyReceipt(receipt, { selection: docsSelection, repoRoot: root, receiptPath: "README.md" }),
  /receipt path must be a JSON file directly under \.planr\/receipts/,
  "a tracked source path cannot be disguised as the receipt output exclusion",
);
writeFileSync(path.join(root, "README.md"), "# Fixture\n");

const receiptAliasPath = ".planr/receipts/alias.json";
symlinkSync(path.join(root, "README.md"), path.join(root, receiptAliasPath));
assert.throws(
  () => verifyReceipt(receipt, { selection: docsSelection, repoRoot: root, receiptPath: receiptAliasPath }),
  /receipt path must not contain symbolic-link aliases/,
  "a symbolic-link alias cannot disguise a source path as receipt output",
);
rmSync(path.join(root, receiptAliasPath));

writeFileSync(path.join(root, receiptPath), "{}\n");
assert.throws(
  () => verifyReceipt(receipt, { selection: docsSelection, repoRoot: root, receiptPath }),
  /receipt path content does not match/,
  "the excluded receipt output must contain the exact receipt being verified",
);
rmSync(path.join(root, receiptPath));

writeFileSync(path.join(root, "README.md"), "# Dirty fixture\n");
assert.throws(
  () => runVerification({ selection: docsSelection, repoRoot: root, execute: () => ({ status: 0 }) }),
  /source worktree must be clean/,
  "an unselected tracked modification invalidates source binding",
);
writeFileSync(path.join(root, "README.md"), "# Fixture\n");
writeFileSync(path.join(root, "unselected-input.txt"), "dirty\n");
assert.throws(
  () => runVerification({ selection: docsSelection, repoRoot: root, execute: () => ({ status: 0 }) }),
  /source worktree must be clean/,
  "an unselected untracked input invalidates source binding",
);
rmSync(path.join(root, "unselected-input.txt"));

assert.throws(
  () => runVerification({
    selection: docsSelection,
    repoRoot: root,
    execute() {
      writeFileSync(path.join(root, "created-during-gates.txt"), "dirty\n");
      return { status: 0 };
    },
  }),
  /source worktree must be clean/,
  "a gate cannot create an unbound source input and still emit a green receipt",
);
rmSync(path.join(root, "created-during-gates.txt"));

const stalePolicy = structuredClone(receipt);
stalePolicy.policy.digest = `sha256:${"0".repeat(64)}`;
assert.throws(() => verifyReceipt(stalePolicy, { selection: docsSelection, repoRoot: root }), /policy digest is stale/);
const missingArtifact = structuredClone(receipt);
missingArtifact.artifacts = [];
assert.throws(() => verifyReceipt(missingArtifact, { selection: docsSelection, repoRoot: root }), /artifact set mismatch/);

writeFileSync(path.join(root, "apps/docs/components/card.tsx"), "export const Card = () => 'altered';\n");
assert.throws(() => verifyReceipt(receipt, { selection: docsSelection, repoRoot: root }), /source worktree must be clean/);
const worktreeSelection = classifyChanges([{ status: "M", path: "apps/docs/components/card.tsx" }], {
  baseRevision: "HEAD",
  headRevision: "worktree",
});
const worktreeReceipt = runVerification({
  selection: worktreeSelection,
  repoRoot: root,
  execute: () => ({ status: 0 }),
});
assert.equal(worktreeReceipt.verdict, "pass", "selected worktree changes can produce a source-bound receipt");
assert.deepEqual(
  verifyReceipt(worktreeReceipt, { selection: worktreeSelection, repoRoot: root }).verdict,
  "pass",
  "worktree receipts verify against the same selected dirty source",
);
writeFileSync(path.join(root, "apps/docs/components/new-card.tsx"), "export const NewCard = () => null;\n");
const detectedWorktreeChanges = worktreeChanges(root, "HEAD");
assert.ok(
  detectedWorktreeChanges.some((change) => change.status === "A" && change.path === "apps/docs/components/new-card.tsx"),
  "worktree selection includes untracked source files",
);
const worktreeWithUntrackedSelection = classifyChanges(detectedWorktreeChanges, {
  baseRevision: "HEAD",
  headRevision: "worktree",
});
assert.equal(
  runVerification({ selection: worktreeWithUntrackedSelection, repoRoot: root, execute: () => ({ status: 0 }) }).verdict,
  "pass",
  "selected untracked worktree changes can produce a source-bound receipt",
);
rmSync(path.join(root, "apps/docs/components/new-card.tsx"));
writeFileSync(path.join(root, "README.md"), "# Also dirty\n");
assert.throws(
  () => runVerification({ selection: worktreeSelection, repoRoot: root, execute: () => ({ status: 0 }) }),
  /source worktree must be clean/,
  "worktree receipts still reject unrelated dirty inputs",
);
writeFileSync(path.join(root, "README.md"), "# Fixture\n");
writeFileSync(path.join(root, "apps/docs/components/card.tsx"), "export const Card = () => null;\n");
writeFileSync(path.join(root, "apps/docs/out/index.html"), "<h1>Altered</h1>\n");
assert.throws(() => verifyReceipt(receipt, { selection: docsSelection, repoRoot: root }), /artifact changed/);
writeFileSync(path.join(root, "apps/docs/out/index.html"), "<h1>Planr</h1>\n");

const bundledReceipt = runVerification({
  selection: docsSelection,
  repoRoot: root,
  artifactRoot: ".planr/artifacts/verification/docs-replay",
  execute: () => ({ status: 0 }),
});
assert.equal(
  verifyReceipt(bundledReceipt, {
    selection: docsSelection,
    repoRoot: root,
    artifactRoot: ".planr/artifacts/verification/docs-replay",
  }).verdict,
  "pass",
  "artifact bundles replay against the preserved docs output",
);
writeFileSync(path.join(root, "apps/docs/out/index.html"), "<h1>Mutated after bundle</h1>\n");
assert.throws(
  () => verifyReceipt(bundledReceipt, { selection: docsSelection, repoRoot: root }),
  /source worktree must be clean|artifact changed/,
  "normal verification cannot replay without the preserved artifact root",
);
assert.equal(
  verifyReceipt(bundledReceipt, {
    selection: docsSelection,
    repoRoot: root,
    artifactRoot: ".planr/artifacts/verification/docs-replay",
  }).verdict,
  "pass",
  "preserved artifact bundles keep exact receipt replayable after live output changes",
);
assert.throws(
  () => runVerification({
    selection: docsSelection,
    repoRoot: root,
    artifactRoot: "apps/docs/out",
    execute: () => ({ status: 0 }),
  }),
  /artifact root must be under \.planr\/artifacts\/verification/,
  "artifact bundles cannot be written over tracked or live source paths",
);
writeFileSync(path.join(root, "apps/docs/out/index.html"), "<h1>Planr</h1>\n");
rmSync(path.join(root, ".planr/artifacts/verification/docs-replay"), { recursive: true, force: true });

for (const mutation of [
  (value) => ({ ...value, environment: { PATH: process.env.PATH } }),
  (value) => ({ ...value, raw_prompt: "private prompt" }),
  (value) => ({ ...value, raw_completion: "private completion" }),
  (value) => ({ ...value, credential: "xai-example-secret-value" }),
  (value) => ({ ...value, diagnostics: { path: "examples/eval/private-suite.json" } }),
]) {
  assert.throws(() => verifyReceipt(mutation(structuredClone(receipt)), { selection: docsSelection, repoRoot: root }), /allowlisted|forbidden|private path/);
}

const fullSelection = classifyChanges([{ status: "M", path: "scripts/verification-policy.mjs" }]);
const plan = commandPlanFor(fullSelection);
assert.equal(plan.filter(({ executable, args }) => [executable, ...args].join(" ").includes("@planr/docs build")).length, 1);
assert.equal(plan.filter(({ executable, args }) => [executable, ...args].join(" ").includes("cargo install")).length, 0);
assert.equal(new Set(plan.map((entry) => JSON.stringify([entry.executable, entry.args]))).size, plan.length);

const fullDocsSelection = selectVerificationGates(fullSelection, [
  "docs-content", "docs-typecheck", "docs-lint", "docs-build", "docs-artifact",
]);
const fullDocsReceipt = runVerification({
  selection: fullDocsSelection,
  repoRoot: root,
  execute: () => ({ status: 0 }),
});
assert.equal(fullDocsReceipt.verdict, "pass", "a full-profile docs job runs without parallel Linux receipts");
assert.equal(fullDocsReceipt.commands.every(({ gate }) => gate.startsWith("docs-")), true);
assert.equal(fullDocsReceipt.commands.filter(({ gate }) => gate === "docs-build").length, 1);
assert.throws(
  () => selectVerificationGates(docsSelection, ["linux-portability"]),
  /verification gate was not selected/,
  "a job cannot claim a gate excluded by the exact change selection",
);

const releaseSelection = classifyChanges([{ status: "M", path: "scripts/release.sh" }]);
const releasePlan = commandPlanFor(releaseSelection);
for (const candidatePlan of [releasePlan, plan]) {
  const linuxCommands = candidatePlan.filter(({ gate }) => gate === "linux-portability");
  assert.equal(linuxCommands.length, 1, "the candidate host only executes the aggregate Linux checksum command");
  assert.equal(
    linuxCommands.filter(({ executable, args }) => executable === "sh" && args.join(" ").includes("sha256sum -c SHA256SUMS")).length,
    1,
    "Linux portability records exactly one aggregate checksum command",
  );
}
assert.equal(releaseSelection.profile, "release-critical");
assert.equal(releasePlan.some(({ args }) => args.includes("security:check")), false, "release CI never runs local security tooling");
assert.equal(new Set(releasePlan.map((entry) => JSON.stringify([entry.executable, entry.args]))).size, releasePlan.length);

const targetDefinitions = [
  ["linux-x86_64", "x86_64-unknown-linux-musl", "x64"],
  ["linux-arm64", "aarch64-unknown-linux-musl", "arm64"],
];
for (const [target, cargoTarget] of targetDefinitions) {
  const targetPlan = linuxTargetCommandPlan(target);
  assert.equal(targetPlan.length, 2, `${target} has one build and one verifier`);
  assert.ok(targetPlan.every(({ executable, args }) =>
    executable === "env"
    && args.includes(`PLANR_TARGET=${target}`)
    && args.includes(`PLANR_CARGO_TARGET=${cargoTarget}`)), `${target} commands carry explicit target bindings`);
  assert.deepEqual(
    targetPlan.map(({ args }) => args.at(-1)),
    ["scripts/build-linux-release.sh", "scripts/verify-linux-release-artifact.sh"],
    `${target} build and verification order is deterministic`,
  );
  assert.equal(new Set(targetPlan.map(({ executable, args }) => JSON.stringify([executable, args]))).size, 2);
}

let incompatibleHostCalls = 0;
assert.throws(
  () => runLinuxTargetVerification({
    selection: releaseSelection,
    target: "linux-arm64",
    repoRoot: root,
    host: { platform: "linux", architecture: "x64" },
    execute: () => { incompatibleHostCalls += 1; return { status: 0 }; },
  }),
  /requires native linux\/arm64/,
  "a target pair cannot start on an incompatible native host",
);
assert.equal(incompatibleHostCalls, 0);
assert.throws(
  () => runVerification({ selection: releaseSelection, repoRoot: root, execute: () => ({ status: 0 }) }),
  /both independent Linux target receipts are required/,
  "zero target receipts and no dist artifacts cannot produce a green release receipt",
);

mkdirSync(path.join(root, "dist"), { recursive: true });
const archiveContents = new Map([
  ["linux-x86_64", "independent-x86_64-archive\n"],
  ["linux-arm64", "independent-arm64-archive\n"],
]);
for (const [target, contents] of archiveContents) writeFileSync(path.join(root, `dist/planr-${target}.tar.gz`), contents);
const checksumContents = "arm64-digest  planr-linux-arm64.tar.gz\nx86_64-digest  planr-linux-x86_64.tar.gz\n";
writeFileSync(path.join(root, "dist/SHA256SUMS"), checksumContents);

const linuxTargetReceipts = targetDefinitions.map(([target, , architecture]) => runLinuxTargetVerification({
  selection: releaseSelection,
  target,
  repoRoot: root,
  host: { platform: "linux", architecture },
  execute: () => ({ status: 0 }),
}));
for (const targetReceipt of linuxTargetReceipts) {
  assert.equal(verifyLinuxTargetReceipt(targetReceipt, { selection: releaseSelection, repoRoot: root }).verdict, "pass");
}
const releaseReceipt = runVerification({
  selection: releaseSelection,
  repoRoot: root,
  linuxTargetReceipts,
  execute: () => ({ status: 0 }),
});
assert.equal(releaseReceipt.linuxTargets.length, 2, "both independent target receipts join one candidate receipt");
assert.deepEqual(releaseReceipt.linuxTargets.map(({ target }) => target), ["linux-x86_64", "linux-arm64"]);
assert.deepEqual(
  releaseReceipt.artifacts.map(({ path: artifactPath }) => artifactPath),
  ["dist/planr-linux-x86_64.tar.gz", "dist/planr-linux-arm64.tar.gz", "dist/SHA256SUMS"],
  "promotion evidence binds both archives and the aggregate checksum file",
);
assert.equal(verifyReceipt(releaseReceipt, { selection: releaseSelection, repoRoot: root }).verdict, "pass");

const swappedArtifacts = structuredClone(linuxTargetReceipts);
swappedArtifacts[0].artifact = structuredClone(swappedArtifacts[1].artifact);
assert.throws(
  () => runVerification({ selection: releaseSelection, repoRoot: root, linuxTargetReceipts: swappedArtifacts, execute: () => ({ status: 0 }) }),
  /linux-x86_64 artifact path mismatch/,
  "target receipts cannot swap archive identities",
);

rmSync(path.join(root, "dist/planr-linux-x86_64.tar.gz"));
assert.throws(() => verifyReceipt(releaseReceipt, { selection: releaseSelection, repoRoot: root }), /artifact is missing/);
writeFileSync(path.join(root, "dist/planr-linux-x86_64.tar.gz"), archiveContents.get("linux-x86_64"));
writeFileSync(path.join(root, "dist/planr-linux-arm64.tar.gz"), "tampered-arm64\n");
assert.throws(() => verifyReceipt(releaseReceipt, { selection: releaseSelection, repoRoot: root }), /linux-arm64 artifact changed/);
writeFileSync(path.join(root, "dist/planr-linux-arm64.tar.gz"), archiveContents.get("linux-arm64"));
rmSync(path.join(root, "dist/SHA256SUMS"));
assert.throws(() => verifyReceipt(releaseReceipt, { selection: releaseSelection, repoRoot: root }), /artifact is missing: dist\/SHA256SUMS/);
writeFileSync(path.join(root, "dist/SHA256SUMS"), "tampered-checksums\n");
assert.throws(() => verifyReceipt(releaseReceipt, { selection: releaseSelection, repoRoot: root }), /artifact changed: dist\/SHA256SUMS/);
writeFileSync(path.join(root, "dist/SHA256SUMS"), checksumContents);
rmSync(path.join(root, "dist"), { recursive: true, force: true });

const historicalRevision = git("rev-parse", "HEAD").trim();
writeFileSync(path.join(root, "README.md"), "# New revision\n");
git("add", "README.md");
git("commit", "-qm", "advance fixture head");
const historicalSelection = classifyChanges([{ status: "M", path: "apps/docs/components/card.tsx" }], {
  baseRevision: `${historicalRevision}^`,
  headRevision: historicalRevision,
});
assert.throws(
  () => runVerification({ selection: historicalSelection, repoRoot: root, execute: () => ({ status: 0 }) }),
  /selected source revision .* is not checked out at HEAD/,
  "a historical --head cannot claim gates executed from the current checkout",
);

console.log(JSON.stringify({
  verdict: "pass",
  docs_commands: calls.length,
  docs_build_commands: 1,
  docs_cargo_install_commands: 0,
  full_commands: plan.length,
  release_commands: releasePlan.length,
  linux_commands_per_candidate_profile: 1,
  independent_linux_target_commands: 4,
  release_receipt_commands: releaseReceipt.commands.length,
  bound_linux_artifacts: releaseReceipt.artifacts.length,
  rejected_incompatible_linux_hosts: 1,
  rejected_missing_linux_compositions: 1,
  rejected_linux_evidence_mutations: 5,
  rejected_sensitive_receipts: 5,
  rejected_dirty_worktrees: 3,
  rejected_unbound_receipt_paths: 3,
  rejected_historical_heads: 1,
}, null, 2));

function git(...args) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8" });
}
