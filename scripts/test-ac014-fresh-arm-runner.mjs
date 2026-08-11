import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { runFreshArmForTest } from "./ac014-fresh-arm-runner.mjs";

const repoRoot = path.dirname(path.dirname(new URL(import.meta.url).pathname));
const root = mkdtempSync(path.join(tmpdir(), "planr-ac014-runner-"));
process.on("exit", () => rmSync(root, { recursive: true, force: true }));

const fixedCeilings = {
  wall_time_seconds: 998.015,
  total_tokens: 5977896,
  tool_call_envelopes: 93,
};

const baseline = path.join(root, "baseline");
const codexHome = path.join(root, "codex-home");
mkdirSync(path.join(baseline, ".planr", "plans", "build"), { recursive: true });
mkdirSync(path.join(codexHome, "sessions"), { recursive: true });
writeFileSync(path.join(baseline, "README.md"), "# baseline\n");
writeFileSync(path.join(baseline, "prompt.txt"), "exact AC-014 prompt\n");
writeFileSync(path.join(baseline, "spec.txt"), "exact AC-014 spec\n");
writeFileSync(path.join(baseline, ".planr", "planr.sqlite"), "fixture db\n");
writeFileSync(path.join(baseline, ".planr", "plans", "build", "arm.plan.md"), "# Arm\n");
writeFileSync(path.join(baseline, ".planr", "evidence-migration.json"), JSON.stringify({ schema_version: "planr.evidence.migration.v1" }));
execFileSync("git", ["init"], { cwd: baseline, stdio: "ignore" });
execFileSync("git", ["config", "user.email", "planr@example.invalid"], { cwd: baseline });
execFileSync("git", ["config", "user.name", "Planr Test"], { cwd: baseline });
execFileSync("git", ["add", "."], { cwd: baseline });
execFileSync("git", ["commit", "-m", "fixture"], { cwd: baseline, stdio: "ignore" });
const candidateSha = execFileSync("git", ["rev-parse", "HEAD"], { cwd: baseline, encoding: "utf8" }).trim();
const planrCandidate = path.join(root, "planr-candidate");
mkdirSync(planrCandidate);
writeFileSync(path.join(planrCandidate, "README.md"), "# exact Planr candidate\n");
execFileSync("git", ["init"], { cwd: planrCandidate, stdio: "ignore" });
execFileSync("git", ["config", "user.email", "planr@example.invalid"], { cwd: planrCandidate });
execFileSync("git", ["config", "user.name", "Planr Test"], { cwd: planrCandidate });
execFileSync("git", ["add", "."], { cwd: planrCandidate });
execFileSync("git", ["commit", "-m", "fixture"], { cwd: planrCandidate, stdio: "ignore" });
const planrCandidateSha = execFileSync("git", ["rev-parse", "HEAD"], { cwd: planrCandidate, encoding: "utf8" }).trim();
const planrCandidateTree = execFileSync("git", ["rev-parse", "HEAD^{tree}"], { cwd: planrCandidate, encoding: "utf8" }).trim();

const callsPath = path.join(root, "calls.jsonl");
const stubPath = path.join(root, "planr-stub.mjs");
writeExecutable(stubPath, stubPlanr({ outsidePath: false }));
const codexStubPath = path.join(root, "codex-stub.mjs");
writeExecutable(codexStubPath, stubCodexSession({ rootTokens: 5977800, childTokens: 96, rootTools: 90, childTools: 3 }));
const oracleStubPath = path.join(root, "oracle-stub.mjs");
writeExecutable(oracleStubPath, "#!/usr/bin/env node\nprocess.exit(0);\n");

const fixedContract = {
  candidate_sha: candidateSha,
  candidate_version: "1.10.0-alpha.4",
  candidate_binary_sha256: sha256File(stubPath),
  prompt_digest: sha256File(path.join(baseline, "prompt.txt")),
  spec_digest: sha256File(path.join(baseline, "spec.txt")),
  model: "gpt-5.6-sol",
  effort: "medium",
  surface: "identical",
  cli_version: "0.147.0",
  oracle_id: "sparziele-exact-product-flow-v1",
  oracle_sha256: sha256File(oracleStubPath),
};

const controlHandoff = {
  schema_version: "planr.ac014.control_handoff.v1",
  plan_id: "pln-canonical-arm",
  preflight_item_id: "item-preflight",
  verification_item_id: "item-verification",
  obligation_id: "pob-canonical-arm",
  policy_digest: `sha256:${"a".repeat(64)}`,
  review_required: true,
  planr_candidate: {
    root: planrCandidate,
    source_revision: planrCandidateSha,
    source_tree: planrCandidateTree,
    binary_sha256: sha256File(stubPath),
    accepted_fix_review_gate_id: "i-review-verifier-phase-fix",
  },
};

const fresh = path.join(root, "fresh");
const passed = await run(config({
  fresh_root: fresh,
  oracle_command: [oracleStubPath],
  evidence_prepare_commands: [[
    process.execPath,
    "-e",
    "require('node:fs').writeFileSync('.planr/evidence-prepared', 'ok')",
  ]],
}), path.join(root, "result.json"), callsPath, codexCommand(codexStubPath));
assert.equal(passed.status, 0, passed.output);
const result = JSON.parse(readFileSync(path.join(root, "result.json"), "utf8"));
assert.equal(result.status, "passed");
assert.equal(result.retry, false);
assert.equal(result.copied_from_declared_baseline_only, true);
assert.deepEqual(result.control_handoff, controlHandoff);
assert.equal(statSync(path.join(fresh, "README.md")).isFile(), true);
assert.equal(result.observed_contract.candidate_sha, candidateSha);
assert.equal(result.observed_contract.candidate_binary_sha256, fixedContract.candidate_binary_sha256);
assert.deepEqual(result.observed_contract.planr_candidate, controlHandoff.planr_candidate);
assert.equal(result.observed_contract.prompt_digest, fixedContract.prompt_digest);
assert.equal(result.observed_contract.spec_digest, fixedContract.spec_digest);
assert.equal(result.observed_contract.model, "gpt-5.6-sol");
assert.equal(result.observed_contract.effort, "medium");
assert.equal(result.observed_contract.surface, "identical");
assert.equal(result.observed_contract.cli_version, "0.147.0");
assert.equal(result.codex.launch_identity.subcommand, "exec");
assert.equal(result.codex.launch_identity.json, true);
assert.equal(result.codex.launch_identity.model, "gpt-5.6-sol");
assert.equal(result.codex.launch_identity.effort_config, 'model_reasoning_effort="medium"');
assert.equal(result.codex.launch_identity.bypass_approvals_and_sandbox, true);
assert.equal(result.codex.launch_identity.bypass_hook_trust, true);
assert.equal(result.codex.launch_identity.color, "never");
assert.equal(result.codex.launch_identity.prompt_delivery.sha256, fixedContract.prompt_digest);
assert.equal(result.codex.launch_identity.prompt_delivery.byte_length, readFileSync(path.join(baseline, "prompt.txt")).byteLength);
assert.equal(result.codex.launch_identity.argv.at(-1), `<prompt ${fixedContract.prompt_digest} bytes:${readFileSync(path.join(baseline, "prompt.txt")).byteLength}>`);
assert.equal(result.codex.command.at(-1), `<prompt ${fixedContract.prompt_digest} bytes:${readFileSync(path.join(baseline, "prompt.txt")).byteLength}>`);
assert.equal(result.commands.some((command) => command.command.includes("exact AC-014 prompt")), false);
assert.equal(result.codex.launch_identity.environment.codex_home, codexHome);
assert.equal(result.codex.launch_identity.environment.node_realpath, realpathSync(process.execPath));
assert.equal(result.codex.launch_identity.environment.injection_variables_absent, true);
const preflight = JSON.parse(readFileSync(path.join(fresh, ".planr", "artifacts", "ac014", "PREFLIGHT.json"), "utf8"));
assert.deepEqual(preflight.control_handoff, controlHandoff);
assert.equal(preflight.codex_launch_identity.model, "gpt-5.6-sol");
assert.equal(preflight.codex_launch_identity.effort_config, 'model_reasoning_effort="medium"');
assert.equal(preflight.codex_launch_identity.prompt_delivery.sha256, fixedContract.prompt_digest);
assert.equal(preflight.codex_launch_identity.environment.codex_home, codexHome);
assert.equal(result.validation.all_paths_local, true);
assert.equal(result.validation.descendant_counts["item-parent"], 1);
assert.deepEqual(result.sessions.map((session) => session.id), ["session-child", "session-root"]);
assert.equal(result.sessions.reduce((sum, session) => sum + session.usage.total_tokens, 0), 5977896);
assert.equal(result.sessions.reduce((sum, session) => sum + session.usage.tool_call_envelopes, 0), 93);
assert.equal(result.ceilings.checks.wall_time_seconds.status, "passed");
assert.equal(result.ceilings.checks.total_tokens.status, "passed");
assert.equal(result.ceilings.checks.tool_call_envelopes.status, "passed");
assert.equal(result.oracle.status, "passed");
assert.equal(result.evidence_migration.input, ".planr/evidence-migration.json");
assert.equal(result.evidence_migration.preparation.length, 1);
assert.equal(result.evidence_migration.preparation[0].status, 0);
assert.equal(readFileSync(path.join(fresh, ".planr", "evidence-prepared"), "utf8"), "ok");
assert.equal(result.sessions.find((session) => session.id === "session-child").parent, "session-root");
assert.equal(statSync(path.join(fresh, ".planr", "artifacts", "ac014", "PREFLIGHT.json")).isFile(), true);
assert.equal(statSync(path.join(fresh, ".planr", "artifacts", "ac014", "BENCHMARK_RESULT.json")).isFile(), true);
assert.equal(statSync(path.join(fresh, ".planr", "artifacts", "ac014", "monitor-status.json")).isFile(), true);
assertCompleteArtifactSet(fresh);

const falsePositiveRoot = "019fe0e5-8cef-7210-aea7-40722b23874e";
const falsePositiveChild = "019fe0e5-8cef-7210-aea7-40722b23874f";
const falsePositiveStubPath = path.join(root, "codex-false-positive-stub.mjs");
writeExecutable(falsePositiveStubPath, stubCodexSession({
  rootTokens: 5977800,
  childTokens: 96,
  rootTools: 90,
  childTools: 3,
  rootSessionId: falsePositiveRoot,
  childSessionId: falsePositiveChild,
  embeddedParentSessionId: falsePositiveChild,
  inheritedSecondSessionMeta: true,
}));
const falsePositiveRun = await run(config({
  fresh_root: path.join(root, "fresh-false-positive"),
  oracle_command: [oracleStubPath],
}), path.join(root, "false-positive-result.json"), path.join(root, "calls-false-positive.jsonl"), codexCommand(falsePositiveStubPath));
assert.equal(falsePositiveRun.status, 0, falsePositiveRun.output);
const falsePositiveResult = JSON.parse(readFileSync(path.join(root, "false-positive-result.json"), "utf8"));
assert.deepEqual(falsePositiveResult.sessions.map((session) => session.id), [falsePositiveRoot, falsePositiveChild]);
assert.equal(falsePositiveResult.sessions.find((session) => session.id === falsePositiveChild).parent, falsePositiveRoot);
assert.deepEqual(falsePositiveResult.sessions.find((session) => session.id === falsePositiveChild).turn_context, { model: "gpt-5.6-sol", effort: "medium" });

const sharedHistoryStubPath = path.join(root, "codex-shared-history-stub.mjs");
writeExecutable(sharedHistoryStubPath, stubCodexSession({
  rootTokens: 5977800,
  childTokens: 96,
  rootTools: 90,
  childTools: 3,
  extraUnrelatedDuplicate: true,
}));
const sharedHistoryRun = await run(config({
  fresh_root: path.join(root, "fresh-shared-history"),
  oracle_command: [oracleStubPath],
}), path.join(root, "shared-history-result.json"), path.join(root, "calls-shared-history.jsonl"), codexCommand(sharedHistoryStubPath));
assert.equal(sharedHistoryRun.status, 0, sharedHistoryRun.output);
const sharedHistory = JSON.parse(readFileSync(path.join(root, "shared-history-result.json"), "utf8"));
assert.deepEqual(sharedHistory.sessions.map((session) => session.path).sort(), ["sessions/session-child.jsonl", "sessions/session-root.jsonl"]);

const missingCodexHomeInput = config({
  fresh_root: path.join(root, "fresh-missing-codex-home"),
  oracle_command: [oracleStubPath],
});
delete missingCodexHomeInput.codex_home;
const missingCodexHome = await run(missingCodexHomeInput, path.join(root, "missing-codex-home-result.json"), path.join(root, "calls-missing-codex-home.jsonl"), codexCommand(codexStubPath));
assert.equal(missingCodexHome.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "missing-codex-home-result.json"), "utf8")).error, /isolated persistent benchmark CODEX_HOME/);

const activeSharedCodexHome = path.join(root, "active-shared-codex-home");
mkdirSync(activeSharedCodexHome);
const activeSharedCodexHomeRun = await withEnv("CODEX_HOME", activeSharedCodexHome, () => run(config({
  fresh_root: path.join(root, "fresh-active-shared-codex-home"),
  oracle_command: [oracleStubPath],
  codex_home: activeSharedCodexHome,
}), path.join(root, "active-shared-codex-home-result.json"), path.join(root, "calls-active-shared-codex-home.jsonl"), codexCommand(codexStubPath)));
assert.equal(activeSharedCodexHomeRun.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "active-shared-codex-home-result.json"), "utf8")).error, /active shared Codex profile/);

const defaultHome = path.join(root, "default-home");
const defaultSharedCodexHome = path.join(defaultHome, ".codex");
mkdirSync(defaultSharedCodexHome, { recursive: true });
const defaultSharedCodexHomeRun = await withEnv("HOME", defaultHome, () => run(config({
  fresh_root: path.join(root, "fresh-default-shared-codex-home"),
  oracle_command: [oracleStubPath],
  codex_home: defaultSharedCodexHome,
}), path.join(root, "default-shared-codex-home-result.json"), path.join(root, "calls-default-shared-codex-home.jsonl"), codexCommand(codexStubPath)));
assert.equal(defaultSharedCodexHomeRun.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "default-shared-codex-home-result.json"), "utf8")).error, /active shared Codex profile/);

const userSharedCodexHome = path.join(process.env.HOME ?? "", ".codex-kevin");
if (existsSync(userSharedCodexHome)) {
  const userSharedCodexHomeRun = await run(config({
    fresh_root: path.join(root, "fresh-user-shared-codex-home"),
    oracle_command: [oracleStubPath],
    codex_home: userSharedCodexHome,
  }), path.join(root, "user-shared-codex-home-result.json"), path.join(root, "calls-user-shared-codex-home.jsonl"), codexCommand(codexStubPath));
  assert.equal(userSharedCodexHomeRun.status, 1);
  assert.match(JSON.parse(readFileSync(path.join(root, "user-shared-codex-home-result.json"), "utf8")).error, /active shared Codex profile/);
}

const nestedCodexHome = path.join(baseline, "nested-codex-home");
mkdirSync(nestedCodexHome);
const nestedCodexHomeRun = await run(config({
  fresh_root: path.join(root, "fresh-nested-codex-home"),
  oracle_command: [oracleStubPath],
  codex_home: nestedCodexHome,
}), path.join(root, "nested-codex-home-result.json"), path.join(root, "calls-nested-codex-home.jsonl"), codexCommand(codexStubPath));
assert.equal(nestedCodexHomeRun.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "nested-codex-home-result.json"), "utf8")).error, /codex_home must be isolated/);

const cleanupPidFile = path.join(root, "cleanup-child.pid");
const cleanupStubPath = path.join(root, "codex-cleanup-stub.mjs");
writeExecutable(cleanupStubPath, stubCodexMonitorFailureWithDescendant(cleanupPidFile));
const cleanupRun = await run(config({
  fresh_root: path.join(root, "fresh-cleanup"),
  oracle_command: [oracleStubPath],
  monitor_poll_ms: 5,
  root_session_id: "session-root",
}), path.join(root, "cleanup-result.json"), path.join(root, "calls-cleanup.jsonl"), codexCommand(cleanupStubPath));
assert.equal(cleanupRun.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "cleanup-result.json"), "utf8")).error, /cyclic Codex session lineage/);
const cleanupPid = Number.parseInt(readFileSync(cleanupPidFile, "utf8"), 10);
assert.equal(Number.isInteger(cleanupPid), true);
await sleep(1200);
assert.equal(isPidAlive(cleanupPid), false);

const transcriptFixtureDir = path.join(repoRoot, "tests", "fixtures", "ac014", "failed-transcript-min");
const transcriptManifest = JSON.parse(readFileSync(path.join(transcriptFixtureDir, "manifest.json"), "utf8"));
for (const file of transcriptManifest.files) {
  assert.equal(sha256File(path.join(transcriptFixtureDir, file.path)).slice("sha256:".length), file.sha256);
}
assert.equal(transcriptManifest.source_files[0].extracted_lines[0].record_sha256, "d64ebb3bb4881a0f7666cb7b2f969d986790a5f371d937517683aa4c85f16bf1");
assert.equal(transcriptManifest.source_files[1].extracted_lines[0].snapshot_sha256, "6763f85949efc84e4565f06ad958074c7be062bbf1092ba14712aa9aa0303613");
assert.equal(transcriptManifest.source_files[1].extracted_lines[1].record_sha256, "2452e0bce2b8dba0df12c0cf9c0d4e01dcc0e9aba2748010f124e487b79c54f9");
const oldMatches = oldRecursiveJsonlSessions(transcriptFixtureDir).filter((session) => session.id === transcriptManifest.root_session_id);
assert.deepEqual(oldMatches.map((session) => session.path).sort(), [
  "history.jsonl",
  "sessions/2026/08/08/rollout-2026-08-08T12-22-48-019fe0e5-8cef-7210-aea7-40722b23874e.jsonl",
]);
const fixtureReplayStubPath = path.join(root, "codex-fixture-replay-stub.mjs");
const fixtureCodexHome = path.join(root, "codex-home-fixture");
mkdirSync(fixtureCodexHome, { recursive: true });
writeExecutable(fixtureReplayStubPath, stubCodexFixtureReplay(transcriptFixtureDir, transcriptManifest.files.map((file) => file.path), fixtureCodexHome));
const fixtureReplayRun = await run(config({
  fresh_root: path.join(root, "fresh-fixture-replay"),
  oracle_command: [oracleStubPath],
  codex_home: fixtureCodexHome,
  root_session_id: transcriptManifest.root_session_id,
}), path.join(root, "fixture-replay-result.json"), path.join(root, "calls-fixture-replay.jsonl"), codexCommand(fixtureReplayStubPath));
assert.equal(fixtureReplayRun.status, 1);
const fixtureReplay = JSON.parse(readFileSync(path.join(root, "fixture-replay-result.json"), "utf8"));
assert.match(fixtureReplay.error, /observed identity is missing: model/);
assert.doesNotMatch(fixtureReplay.error, /duplicate Codex session id/);
assert.equal(fixtureReplay.sessions[0].id, transcriptManifest.root_session_id);
assert.equal(fixtureReplay.sessions.length, 1);

const calls = readFileSync(callsPath, "utf8").trim().split("\n").map(JSON.parse);
assert.deepEqual(calls.slice(0, 2).map((call) => call.command), ["project relocate", "project relocate"]);
assert.equal(calls[0].cwd, fresh);
assert.equal(calls[0].args.includes("--apply"), false);
assert.equal(calls[1].args.includes("--apply"), true);
assert.equal(calls.filter((call) => call.command === "evidence migrate").length, 2);

const staleCliStubPath = path.join(root, "codex-stale-cli-stub.mjs");
writeExecutable(staleCliStubPath, stubCodexSession({
  rootTokens: 5977800,
  childTokens: 96,
  rootTools: 90,
  childTools: 3,
  cliVersion: "0.146.0",
}));
const staleCli = await run(config({
  fresh_root: path.join(root, "fresh-stale-cli"),
  oracle_command: [oracleStubPath],
}), path.join(root, "stale-cli-result.json"), path.join(root, "calls-stale-cli.jsonl"), codexCommand(staleCliStubPath));
assert.equal(staleCli.status, 1);
assert.match(staleCli.output, /observed identity mismatch: cli_version/);

const slowOverStubPath = path.join(root, "codex-over-stub.mjs");
writeExecutable(slowOverStubPath, stubCodexSession({
  rootTokens: 5977897,
  childTokens: 0,
  rootTools: 94,
  childTools: 0,
  sleepMs: 5000,
}));
const interrupted = await run(config({
  fresh_root: path.join(root, "fresh-over-ceiling"),
  oracle_command: [oracleStubPath],
  monitor_poll_ms: 5,
}), path.join(root, "over-ceiling-result.json"), path.join(root, "calls-over.jsonl"), codexCommand(slowOverStubPath));
assert.equal(interrupted.status, 1);
const overCeiling = JSON.parse(readFileSync(path.join(root, "over-ceiling-result.json"), "utf8"));
assert.equal(overCeiling.failure_class, "product");
assert.equal(overCeiling.retry, false);
assert.equal(overCeiling.ceilings.checks.total_tokens.status, "failed");
assert.equal(overCeiling.codex.interrupted.ceiling, "total_tokens");
assert.equal(statSync(path.join(root, "fresh-over-ceiling", ".planr", "artifacts", "ac014", "monitor-status.json")).isFile(), true);

const suppliedMetrics = await run(config({
  fresh_root: path.join(root, "fresh-supplied-metrics"),
  oracle_command: [oracleStubPath],
  metrics: { wall_time_seconds: 1, total_tokens: 1, tool_call_envelopes: 1 },
}), path.join(root, "supplied-metrics-result.json"), path.join(root, "calls-supplied-metrics.jsonl"), codexCommand(codexStubPath));
assert.equal(suppliedMetrics.status, 1);
assert.equal(JSON.parse(readFileSync(path.join(root, "supplied-metrics-result.json"), "utf8")).failure_class, "admission");
assertCompleteArtifactSet(path.join(root, "fresh-supplied-metrics"));

const mismatch = await run(config({
  fresh_root: path.join(root, "fresh-contract-mismatch"),
  oracle_command: [oracleStubPath],
  fixed_contract: { ...fixedContract, model: "gpt-4" },
}), path.join(root, "contract-mismatch-result.json"), path.join(root, "calls-contract.jsonl"), codexCommand(codexStubPath));
assert.equal(mismatch.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "contract-mismatch-result.json"), "utf8")).error, /observed identity mismatch: model/);

const tuiStubPath = path.join(root, "codex-tui-stub.mjs");
writeExecutable(tuiStubPath, stubCodexSession({
  rootTokens: 5977800,
  childTokens: 96,
  rootTools: 90,
  childTools: 3,
  rootOriginator: "codex-tui",
  rootSource: "cli",
}));
const tuiSurface = await run(config({
  fresh_root: path.join(root, "fresh-tui-surface"),
  oracle_command: [oracleStubPath],
}), path.join(root, "tui-surface-result.json"), path.join(root, "calls-tui-surface.jsonl"), codexCommand(tuiStubPath));
assert.equal(tuiSurface.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "tui-surface-result.json"), "utf8")).error, /observed identity mismatch: surface/);

const partialOriginatorStubPath = path.join(root, "codex-partial-originator-stub.mjs");
writeExecutable(partialOriginatorStubPath, stubCodexSession({
  rootTokens: 5977800,
  childTokens: 96,
  rootTools: 90,
  childTools: 3,
  rootOriginator: "codex_exec",
  rootSource: "cli",
}));
const partialOriginator = await run(config({
  fresh_root: path.join(root, "fresh-partial-originator-surface"),
  oracle_command: [oracleStubPath],
}), path.join(root, "partial-originator-surface-result.json"), path.join(root, "calls-partial-originator-surface.jsonl"), codexCommand(partialOriginatorStubPath));
assert.equal(partialOriginator.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "partial-originator-surface-result.json"), "utf8")).error, /observed identity mismatch: surface/);

const partialSourceStubPath = path.join(root, "codex-partial-source-stub.mjs");
writeExecutable(partialSourceStubPath, stubCodexSession({
  rootTokens: 5977800,
  childTokens: 96,
  rootTools: 90,
  childTools: 3,
  rootOriginator: "codex-tui",
  rootSource: "exec",
}));
const partialSource = await run(config({
  fresh_root: path.join(root, "fresh-partial-source-surface"),
  oracle_command: [oracleStubPath],
}), path.join(root, "partial-source-surface-result.json"), path.join(root, "calls-partial-source-surface.jsonl"), codexCommand(partialSourceStubPath));
assert.equal(partialSource.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "partial-source-surface-result.json"), "utf8")).error, /observed identity mismatch: surface/);

const suppliedCodexCommand = await run(config({
  fresh_root: path.join(root, "fresh-supplied-codex-command"),
  codex_command: codexCommand(codexStubPath),
  oracle_command: [oracleStubPath],
}), path.join(root, "supplied-codex-command-result.json"), path.join(root, "calls-supplied-codex-command.jsonl"), codexCommand(codexStubPath));
assert.equal(suppliedCodexCommand.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "supplied-codex-command-result.json"), "utf8")).error, /codex_command is not accepted/);

const wrongSubcommand = await run(config({
  fresh_root: path.join(root, "fresh-wrong-subcommand"),
  oracle_command: [oracleStubPath],
}), path.join(root, "wrong-subcommand-result.json"), path.join(root, "calls-wrong-subcommand.jsonl"), codexCommand(codexStubPath, { subcommand: "run" }));
assert.equal(wrongSubcommand.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "wrong-subcommand-result.json"), "utf8")).error, /Codex subcommand mismatch/);

const wrongModel = await run(config({
  fresh_root: path.join(root, "fresh-wrong-model-launch"),
  oracle_command: [oracleStubPath],
}), path.join(root, "wrong-model-launch-result.json"), path.join(root, "calls-wrong-model-launch.jsonl"), codexCommand(codexStubPath, { model: "gpt-4" }));
assert.equal(wrongModel.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "wrong-model-launch-result.json"), "utf8")).error, /Codex model launch mismatch/);

const wrongEffort = await run(config({
  fresh_root: path.join(root, "fresh-wrong-effort-launch"),
  oracle_command: [oracleStubPath],
}), path.join(root, "wrong-effort-launch-result.json"), path.join(root, "calls-wrong-effort-launch.jsonl"), codexCommand(codexStubPath, { effortConfig: 'model_reasoning_effort="high"' }));
assert.equal(wrongEffort.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "wrong-effort-launch-result.json"), "utf8")).error, /Codex effort launch mismatch/);

const missingBypass = await run(config({
  fresh_root: path.join(root, "fresh-missing-bypass-launch"),
  oracle_command: [oracleStubPath],
}), path.join(root, "missing-bypass-launch-result.json"), path.join(root, "calls-missing-bypass-launch.jsonl"), codexCommand(codexStubPath, { approvalsBypass: null }));
assert.equal(missingBypass.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "missing-bypass-launch-result.json"), "utf8")).error, /approvals\/sandbox bypass flag mismatch/);

const wrongHookTrust = await run(config({
  fresh_root: path.join(root, "fresh-wrong-hook-trust-launch"),
  oracle_command: [oracleStubPath],
}), path.join(root, "wrong-hook-trust-launch-result.json"), path.join(root, "calls-wrong-hook-trust-launch.jsonl"), codexCommand(codexStubPath, { hookTrustBypass: "--hook-trust" }));
assert.equal(wrongHookTrust.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "wrong-hook-trust-launch-result.json"), "utf8")).error, /hook trust bypass flag mismatch/);

const wrongColor = await run(config({
  fresh_root: path.join(root, "fresh-wrong-color-launch"),
  oracle_command: [oracleStubPath],
}), path.join(root, "wrong-color-launch-result.json"), path.join(root, "calls-wrong-color-launch.jsonl"), codexCommand(codexStubPath, { color: "always" }));
assert.equal(wrongColor.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "wrong-color-launch-result.json"), "utf8")).error, /Codex color launch mismatch/);

const missingPrompt = await run(config({
  fresh_root: path.join(root, "fresh-missing-prompt-launch"),
  oracle_command: [oracleStubPath],
}), path.join(root, "missing-prompt-launch-result.json"), path.join(root, "calls-missing-prompt-launch.jsonl"), codexCommand(codexStubPath, { prompt: null }));
assert.equal(missingPrompt.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "missing-prompt-launch-result.json"), "utf8")).error, /prompt positional argument is missing|argv shape mismatch/);

const codexEnvPath = await run(config({
  fresh_root: path.join(root, "fresh-codex-env-path"),
  oracle_command: [oracleStubPath],
  codex_env: { PATH: path.dirname(codexStubPath) },
}), path.join(root, "codex-env-path-result.json"), path.join(root, "calls-codex-env-path.jsonl"), codexCommand(codexStubPath));
assert.equal(codexEnvPath.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "codex-env-path-result.json"), "utf8")).error, /codex_env may not set executable injection variable: PATH/);

const codexEnvHome = await run(config({
  fresh_root: path.join(root, "fresh-codex-env-home"),
  oracle_command: [oracleStubPath],
  codex_env: { CODEX_HOME: path.join(root, "other-codex-home") },
}), path.join(root, "codex-env-home-result.json"), path.join(root, "calls-codex-env-home.jsonl"), codexCommand(codexStubPath));
assert.equal(codexEnvHome.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "codex-env-home-result.json"), "utf8")).error, /codex_env may not set executable injection variable: CODEX_HOME/);

const codexEnvNodeOptions = await run(config({
  fresh_root: path.join(root, "fresh-codex-env-node-options"),
  oracle_command: [oracleStubPath],
  codex_env: { NODE_OPTIONS: "--require ./inject.js" },
}), path.join(root, "codex-env-node-options-result.json"), path.join(root, "calls-codex-env-node-options.jsonl"), codexCommand(codexStubPath));
assert.equal(codexEnvNodeOptions.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "codex-env-node-options-result.json"), "utf8")).error, /codex_env may not set executable injection variable: NODE_OPTIONS/);

const codexEnvNodePath = await run(config({
  fresh_root: path.join(root, "fresh-codex-env-node-path"),
  oracle_command: [oracleStubPath],
  codex_env: { NODE_PATH: path.join(root, "node-path") },
}), path.join(root, "codex-env-node-path-result.json"), path.join(root, "calls-codex-env-node-path.jsonl"), codexCommand(codexStubPath));
assert.equal(codexEnvNodePath.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "codex-env-node-path-result.json"), "utf8")).error, /codex_env may not set executable injection variable: NODE_PATH/);

const reusedFresh = path.join(root, "fresh-reused");
mkdirSync(reusedFresh);
writeFileSync(path.join(reusedFresh, "sentinel.txt"), "do not mutate\n");
const reused = await run(config({
  fresh_root: reusedFresh,
  oracle_command: [oracleStubPath],
}), path.join(root, "reused-result.json"), path.join(root, "calls-reused.jsonl"), codexCommand(codexStubPath));
assert.equal(reused.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "reused-result.json"), "utf8")).error, /must be a new non-existing path/);
assert.equal(readFileSync(path.join(reusedFresh, "sentinel.txt"), "utf8"), "do not mutate\n");
assert.equal(existsSync(path.join(reusedFresh, ".planr")), false);
assertCompleteArtifactSet(path.join(root, ".planr-ac014-failures", "fresh-reused"));

const artifactBaseline = path.join(root, "artifact-baseline");
execFileSync("cp", ["-R", `${baseline}/.`, artifactBaseline]);
mkdirSync(path.join(artifactBaseline, ".planr", "artifacts", "ac014"), { recursive: true });
writeFileSync(path.join(artifactBaseline, ".planr", "artifacts", "ac014", "BENCHMARK_RESULT.json"), "{}\n");
const artifactOverwriteFresh = path.join(root, "fresh-artifact-overwrite");
const artifactOverwrite = await run(config({
  baseline_root: artifactBaseline,
  fresh_root: artifactOverwriteFresh,
  oracle_command: [oracleStubPath],
}), path.join(root, "artifact-overwrite-result.json"), path.join(root, "calls-artifact-overwrite.jsonl"), codexCommand(codexStubPath));
assert.equal(artifactOverwrite.status, 1);
assert.equal(JSON.parse(readFileSync(path.join(root, "artifact-overwrite-result.json"), "utf8")).failure_class, "instrumentation");
assert.equal(readFileSync(path.join(artifactOverwriteFresh, ".planr", "artifacts", "ac014", "BENCHMARK_RESULT.json"), "utf8"), "{}\n");
assertCompleteArtifactSet(path.join(root, ".planr-ac014-failures", "fresh-artifact-overwrite"));

const publicationDeadlineFresh = path.join(root, "fresh-publication-deadline");
const publicationDeadline = await run(config({
  fresh_root: publicationDeadlineFresh,
  oracle_command: [oracleStubPath],
  test_stage_write_delay_ms: 1,
  test_stage_deadline_elapsed_wall_seconds: fixedCeilings.wall_time_seconds + 0.001,
}), path.join(root, "publication-deadline-result.json"), path.join(root, "calls-publication-deadline.jsonl"), codexCommand(codexStubPath));
assert.equal(publicationDeadline.status, 1);
const publicationDeadlineResult = JSON.parse(readFileSync(path.join(root, "publication-deadline-result.json"), "utf8"));
assert.equal(publicationDeadlineResult.status, "failed");
assert.equal(publicationDeadlineResult.failure_class, "product");
assert.equal(publicationDeadlineResult.sessions.length, 2);
assert.equal(publicationDeadlineResult.sessions.reduce((sum, session) => sum + session.usage.total_tokens, 0), 5977896);
assert.equal(publicationDeadlineResult.sessions.reduce((sum, session) => sum + session.usage.tool_call_envelopes, 0), 93);
assert.equal(publicationDeadlineResult.oracle, null);
const committedDeadlineResult = JSON.parse(readFileSync(path.join(publicationDeadlineFresh, ".planr", "artifacts", "ac014", "BENCHMARK_RESULT.json"), "utf8"));
assert.equal(committedDeadlineResult.status, "failed");
assert.equal(committedDeadlineResult.verdict, "failed");
assert.equal(committedDeadlineResult.stop_reason, "deadline_exceeded");
assert.equal(committedDeadlineResult.oracle, null);
const publicationDeadlineMonitor = JSON.parse(readFileSync(path.join(publicationDeadlineFresh, ".planr", "artifacts", "ac014", "monitor-status.json"), "utf8"));
assert.equal(publicationDeadlineMonitor.status, "failed");
assert.equal(publicationDeadlineMonitor.verdict, "failed");
assert.equal(publicationDeadlineMonitor.stop_reason, "deadline_exceeded");
assert.equal(publicationDeadlineMonitor.oracle, null);
assertCompleteArtifactSet(publicationDeadlineFresh);

const finalizationDeadlineStubPath = path.join(root, "codex-finalization-deadline-stub.mjs");
writeExecutable(finalizationDeadlineStubPath, stubCodexSession({
  rootTokens: 3000000,
  childTokens: 600000,
  grandchildTokens: 9041,
  rootTools: 40,
  childTools: 10,
  grandchildTools: 6,
}));
const finalizationDeadlineFresh = path.join(root, "fresh-finalization-deadline");
const finalizationDeadline = await run(config({
  fresh_root: finalizationDeadlineFresh,
  oracle_command: [oracleStubPath],
  test_post_session_deadline_elapsed_wall_seconds: fixedCeilings.wall_time_seconds + 0.1,
}), path.join(root, "finalization-deadline-result.json"), path.join(root, "calls-finalization-deadline.jsonl"), codexCommand(finalizationDeadlineStubPath));
assert.equal(finalizationDeadline.status, 1);
const finalizationDeadlineResult = JSON.parse(readFileSync(path.join(root, "finalization-deadline-result.json"), "utf8"));
assert.equal(finalizationDeadlineResult.status, "failed");
assert.equal(finalizationDeadlineResult.failure_class, "product");
assert.equal(finalizationDeadlineResult.retry, false);
assert.match(finalizationDeadlineResult.error, /deadline exceeded during artifact_finalization/);
assert.equal(finalizationDeadlineResult.sessions.length, 3);
assert.equal(finalizationDeadlineResult.sessions.reduce((sum, session) => sum + session.usage.total_tokens, 0), 3609041);
assert.equal(finalizationDeadlineResult.sessions.reduce((sum, session) => sum + session.usage.tool_call_envelopes, 0), 56);
assert.equal(finalizationDeadlineResult.ceilings.checks.wall_time_seconds.status, "failed");
assert.equal(finalizationDeadlineResult.ceilings.checks.total_tokens.observed, 3609041);
assert.equal(finalizationDeadlineResult.ceilings.checks.total_tokens.status, "passed");
assert.equal(finalizationDeadlineResult.ceilings.checks.tool_call_envelopes.observed, 56);
assert.equal(finalizationDeadlineResult.ceilings.checks.tool_call_envelopes.status, "passed");
assert.equal(finalizationDeadlineResult.oracle, null);
const committedFinalizationDeadline = JSON.parse(readFileSync(path.join(finalizationDeadlineFresh, ".planr", "artifacts", "ac014", "BENCHMARK_RESULT.json"), "utf8"));
assert.equal(committedFinalizationDeadline.status, "failed");
assert.equal(committedFinalizationDeadline.stop_reason, "deadline_exceeded");
assert.equal(committedFinalizationDeadline.sessions.length, 3);
assert.equal(committedFinalizationDeadline.sessions.reduce((sum, session) => sum + session.usage.total_tokens, 0), 3609041);
assert.equal(committedFinalizationDeadline.sessions.reduce((sum, session) => sum + session.usage.tool_call_envelopes, 0), 56);
assert.equal(committedFinalizationDeadline.ceilings.checks.wall_time_seconds.status, "failed");
assert.equal(committedFinalizationDeadline.ceilings.checks.total_tokens.observed, 3609041);
assert.equal(committedFinalizationDeadline.ceilings.checks.tool_call_envelopes.observed, 56);
assert.equal(committedFinalizationDeadline.oracle, null);
const finalizationDeadlineMonitor = JSON.parse(readFileSync(path.join(finalizationDeadlineFresh, ".planr", "artifacts", "ac014", "monitor-status.json"), "utf8"));
assert.equal(finalizationDeadlineMonitor.status, "failed");
assert.equal(finalizationDeadlineMonitor.stop_reason, "deadline_exceeded");
assert.equal(finalizationDeadlineMonitor.sessions.length, 3);
assert.equal(finalizationDeadlineMonitor.ceilings.checks.wall_time_seconds.status, "failed");
assert.equal(finalizationDeadlineMonitor.oracle, null);
assertCompleteArtifactSet(finalizationDeadlineFresh);

const cycleStubPath = path.join(root, "codex-cycle-stub.mjs");
writeExecutable(cycleStubPath, stubCodexCycle());
const cycle = await run(config({
  fresh_root: path.join(root, "fresh-cycle"),
  oracle_command: [oracleStubPath],
  root_session_id: "session-root",
}), path.join(root, "cycle-result.json"), path.join(root, "calls-cycle.jsonl"), codexCommand(cycleStubPath));
assert.equal(cycle.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "cycle-result.json"), "utf8")).error, /cyclic Codex session lineage/);

const outsideStubPath = path.join(root, "planr-outside-stub.mjs");
writeExecutable(outsideStubPath, stubPlanr({ outsidePath: true }));
const outsideContract = { ...fixedContract, candidate_binary_sha256: sha256File(outsideStubPath) };
const outside = await run(config({
  fresh_root: path.join(root, "fresh-outside"),
  planr_bin: outsideStubPath,
  fixed_contract: outsideContract,
  control_handoff: {
    ...controlHandoff,
    planr_candidate: {
      ...controlHandoff.planr_candidate,
      binary_sha256: sha256File(outsideStubPath),
    },
  },
  oracle_command: [oracleStubPath],
}), path.join(root, "outside-result.json"), path.join(root, "calls-outside.jsonl"), codexCommand(codexStubPath));
assert.equal(outside.status, 1);
const outsideResult = JSON.parse(readFileSync(path.join(root, "outside-result.json"), "utf8"));
assert.equal(outsideResult.failure_class, "instrumentation");
assert.match(outsideResult.error, /path does not exist|escapes fresh root/);
assertCompleteArtifactSet(path.join(root, "fresh-outside"));

function config(overrides) {
  return {
    schema_version: "planr.ac014.fresh_arm_run.v1",
    baseline_root: baseline,
    db_path: ".planr/planr.sqlite",
    planr_bin: stubPath,
    project_id: "p-ac014",
    evidence_migration_input: ".planr/evidence-migration.json",
    prompt_path: "prompt.txt",
    spec_path: "spec.txt",
    codex_home: codexHome,
    codex_surface: "identical",
    oracle_id: "sparziele-exact-product-flow-v1",
    fixed_contract: fixedContract,
    control_handoff: controlHandoff,
    ceilings: fixedCeilings,
    monitor_poll_ms: 20,
    ...overrides,
  };
}

function codexCommand(file, {
  subcommand = "exec",
  model = "gpt-5.6-sol",
  effortConfig = 'model_reasoning_effort="medium"',
  approvalsBypass = "--dangerously-bypass-approvals-and-sandbox",
  hookTrustBypass = "--dangerously-bypass-hook-trust",
  color = "never",
  prompt = readFileSync(path.join(baseline, "prompt.txt"), "utf8"),
} = {}) {
  return [
    file,
    subcommand,
    "--json",
    "--model",
    model,
    "-c",
    effortConfig,
    ...(approvalsBypass === null ? [] : [approvalsBypass]),
    ...(hookTrustBypass === null ? [] : [hookTrustBypass]),
    "--color",
    color,
    ...(prompt === null ? [] : [prompt]),
  ];
}

async function run(input, resultPath, callsFile, testCodexCommand = codexCommand(codexStubPath)) {
  const inputPath = `${resultPath}.input.json`;
  writeFileSync(inputPath, JSON.stringify(input));
  if (input.codex_home === codexHome) {
    rmSync(path.join(codexHome, "sessions"), { recursive: true, force: true });
    mkdirSync(path.join(codexHome, "sessions"), { recursive: true });
  }
  const previousCalls = process.env.PLANR_STUB_CALLS;
  try {
    process.env.PLANR_STUB_CALLS = callsFile;
    const result = await runFreshArmForTest(input, { testCodexCommand });
    const resultText = `${JSON.stringify(result, null, 2)}\n`;
    writeFileSync(resultPath, resultText);
    return {
      status: result.status === "passed" ? 0 : 1,
      output: resultText,
    };
  } finally {
    if (previousCalls === undefined) {
      delete process.env.PLANR_STUB_CALLS;
    } else {
      process.env.PLANR_STUB_CALLS = previousCalls;
    }
  }
}

function writeExecutable(file, contents) {
  writeFileSync(file, contents);
  execFileSync("chmod", ["+x", file]);
}

function sha256File(file) {
  return `sha256:${createHash("sha256").update(readFileSync(file)).digest("hex")}`;
}

function assertCompleteArtifactSet(freshRoot) {
  const artifactDir = path.join(freshRoot, ".planr", "artifacts", "ac014");
  const commit = JSON.parse(readFileSync(path.join(artifactDir, "COMMIT.json"), "utf8"));
  const manifest = new Map(commit.files.map((entry) => [entry.name, entry.sha256]));
  for (const name of ["PREFLIGHT.json", "BENCHMARK_RESULT.json", "monitor-status.json"]) {
    const file = path.join(artifactDir, name);
    assert.equal(statSync(file).isFile(), true);
    assert.equal(manifest.get(name), sha256File(file));
  }
}

function oldRecursiveJsonlSessions(rootDir) {
  return allFixtureFiles(rootDir)
    .filter((file) => file.endsWith(".jsonl"))
    .map((file) => {
      let id = idFromFixtureFilename(file);
      for (const line of readFileSync(file, "utf8").split("\n").filter(Boolean)) {
        const record = JSON.parse(line);
        const payload = record.payload && typeof record.payload === "object" ? record.payload : record;
        if (record.type === "session_meta" || payload.type === "session_meta") {
          id = payload.id ?? payload.thread_id ?? payload.threadId ?? payload.session_id ?? id;
        } else {
          id = payload.session_id ?? record.session_id ?? id;
        }
      }
      return { path: path.relative(rootDir, file), id };
    })
    .filter((session) => session.id);
}

function idFromFixtureFilename(file) {
  const match = path.basename(file).match(/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/u);
  return match?.[1] ?? null;
}

function allFixtureFiles(rootDir) {
  const files = [];
  const stack = [rootDir];
  while (stack.length > 0) {
    const current = stack.pop();
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const candidate = path.join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(candidate);
      } else if (entry.isFile()) {
        files.push(candidate);
      }
    }
  }
  return files;
}

function stubPlanr({ outsidePath }) {
  return `#!/usr/bin/env node
import { appendFileSync } from "node:fs";
import path from "node:path";
const args = process.argv.slice(2);
if (args.includes("--version")) {
  console.log("planr 1.10.0-alpha.4");
  process.exit(0);
}
const commandStart = args.indexOf("--json") + 1;
const command = args.slice(commandStart, commandStart + 2).join(" ");
appendFileSync(process.env.PLANR_STUB_CALLS, JSON.stringify({ command, args, cwd: process.cwd() }) + "\\n");
const destination = args.includes("--destination") ? args[args.indexOf("--destination") + 1] : destinationFromDb(args);
const badPath = "/tmp/outside-plan.plan.md";
const planPath = ${outsidePath ? "badPath" : "destination + '/.planr/plans/build/arm.plan.md'"};
if (command === "project relocate") {
  console.log(JSON.stringify({ mode: args.includes("--apply") ? "apply" : "preview", relocation: { project: { id: "p-ac014", from: "/old", to: destination }, plans: [{ id: "pln-ac014", from: "/old/plan.md", to: planPath }], items: [{ id: "item-parent", from: "/old/plan.md", to: planPath }] } }));
} else if (command === "project show") {
  console.log(JSON.stringify({ project: { id: "p-ac014", root_path: destinationFromDb(args) } }));
} else if (command === "plan list") {
  console.log(JSON.stringify({ plans: [{ id: "pln-ac014", path: planPath }] }));
} else if (command === "map show") {
  console.log(JSON.stringify({ items: [{ id: "item-parent", plan_path: planPath }, { id: "item-child", parent_item_id: "item-parent", plan_path: planPath }] }));
} else if (command === "evidence migrate") {
  console.log(JSON.stringify({ status: "ok" }));
} else {
  console.error("unexpected command", command);
  process.exit(2);
}
function destinationFromDb(args) {
  const db = args[args.indexOf("--db") + 1];
  return path.dirname(path.dirname(db));
}
`;
}

function stubCodexSession({
  rootTokens,
  childTokens,
  grandchildTokens = 0,
  rootTools,
  childTools,
  grandchildTools = 0,
  sleepMs = 0,
  rootOriginator = "codex_exec",
  rootSource = "exec",
  cliVersion = "0.147.0",
  rootSessionId = "session-root",
  childSessionId = "session-child",
  embeddedParentSessionId = "session-root",
  extraUnrelatedDuplicate = false,
  inheritedSecondSessionMeta = false,
}) {
  return `#!/usr/bin/env node
import { appendFileSync } from "node:fs";
import path from "node:path";
const home = ${JSON.stringify(codexHome)};
const cwd = process.cwd();
const rootSession = path.join(home, "sessions", ${JSON.stringify(rootSessionId)} + ".jsonl");
appendFileSync(rootSession, JSON.stringify({ type: "event_msg", payload: { type: "session_meta", id: ${JSON.stringify(rootSessionId)}, cwd, role: "root", cli_version: ${JSON.stringify(cliVersion)}, originator: ${JSON.stringify(rootOriginator)}, source: ${JSON.stringify(rootSource)} } }) + "\\n");
appendFileSync(rootSession, JSON.stringify({ type: "turn_context", payload: { model: "gpt-5.6-sol", effort: "medium" } }) + "\\n");
appendFileSync(rootSession, JSON.stringify({ type: "event_msg", payload: { type: "token_count", info: { total_token_usage: { total_tokens: ${rootTokens} }, last_token_usage: { total_tokens: 999999999 } }, tool_call_envelopes: ${rootTools} } }) + "\\n");
appendFileSync(rootSession, JSON.stringify({ session_id: ${JSON.stringify(rootSessionId)}, type: "function_call", id: "root-call" }) + "\\n");
${childTokens > 0 ? `const childSession = path.join(home, "sessions", ${JSON.stringify(childSessionId)} + ".jsonl");
appendFileSync(childSession, JSON.stringify({ type: "event_msg", payload: { type: "session_meta", session_id: ${JSON.stringify(rootSessionId)}, id: ${JSON.stringify(childSessionId)}, cwd, role: "subagent", cli_version: ${JSON.stringify(cliVersion)}, originator: "codex_exec", source: { subagent: { thread_spawn: { parent_thread_id: ${JSON.stringify(rootSessionId)} } } }, turn_context: { model: "gpt-5.6-sol", effort: "medium" } } }) + "\\n");
${inheritedSecondSessionMeta ? `appendFileSync(childSession, JSON.stringify({ type: "event_msg", payload: { type: "session_meta", session_id: ${JSON.stringify(embeddedParentSessionId)}, id: ${JSON.stringify(childSessionId)}, cwd, role: "subagent", cli_version: ${JSON.stringify(cliVersion)}, originator: "codex_exec", source: { subagent: { thread_spawn: { parent_thread_id: ${JSON.stringify(embeddedParentSessionId)} } } }, turn_context: { model: "gpt-4.1", effort: "high" } } }) + "\\n");` : ""}
appendFileSync(childSession, JSON.stringify({ type: "event_msg", payload: { session_id: ${JSON.stringify(embeddedParentSessionId)}, source: { subagent: { thread_spawn: { parent_thread_id: ${JSON.stringify(embeddedParentSessionId)} } } } } }) + "\\n");
appendFileSync(childSession, JSON.stringify({ type: "turn_context", payload: { model: "gpt-5.6-sol", effort: "medium" } }) + "\\n");
appendFileSync(childSession, JSON.stringify({ type: "event_msg", payload: { type: "token_count", info: { total_token_usage: { total_tokens: ${childTokens} }, last_token_usage: { total_tokens: 777777777 } }, tool_call_envelopes: ${childTools} } }) + "\\n");
appendFileSync(childSession, JSON.stringify({ session_id: ${JSON.stringify(childSessionId)}, type: "custom_tool_call", id: "child-call" }) + "\\n");` : ""}
${grandchildTokens > 0 ? `const grandchildSession = path.join(home, "sessions", "session-grandchild.jsonl");
appendFileSync(grandchildSession, JSON.stringify({ type: "event_msg", payload: { type: "session_meta", session_id: ${JSON.stringify(childSessionId)}, id: "session-grandchild", cwd, role: "subagent", cli_version: ${JSON.stringify(cliVersion)}, originator: "codex_exec", source: { subagent: { thread_spawn: { parent_thread_id: ${JSON.stringify(childSessionId)} } } }, turn_context: { model: "gpt-5.6-sol", effort: "medium" } } }) + "\\n");
appendFileSync(grandchildSession, JSON.stringify({ type: "turn_context", payload: { model: "gpt-5.6-sol", effort: "medium" } }) + "\\n");
appendFileSync(grandchildSession, JSON.stringify({ type: "event_msg", payload: { type: "token_count", info: { total_token_usage: { total_tokens: ${grandchildTokens} }, last_token_usage: { total_tokens: 555555555 } }, tool_call_envelopes: ${grandchildTools} } }) + "\\n");
appendFileSync(grandchildSession, JSON.stringify({ session_id: "session-grandchild", type: "custom_tool_call", id: "grandchild-call" }) + "\\n");` : ""}
${extraUnrelatedDuplicate ? `const unrelated = path.join(home, "sessions", "old-unrelated-duplicate.jsonl");
appendFileSync(unrelated, JSON.stringify({ type: "event_msg", payload: { type: "session_meta", id: ${JSON.stringify(childSessionId)}, cwd: "/old/shared/profile", role: "old", cli_version: ${JSON.stringify(cliVersion)}, originator: "codex_exec", source: "exec" } }) + "\\n");` : ""}
${sleepMs > 0 ? `await new Promise((resolve) => setTimeout(resolve, ${sleepMs}));` : ""}
`;
}

function stubCodexFixtureReplay(fixtureDir, files, home) {
  return `#!/usr/bin/env node
import { copyFileSync, mkdirSync } from "node:fs";
import path from "node:path";
const home = ${JSON.stringify(home)};
const fixtureDir = ${JSON.stringify(fixtureDir)};
for (const file of ${JSON.stringify(files)}) {
  const destination = path.join(home, file);
  mkdirSync(path.dirname(destination), { recursive: true });
  copyFileSync(path.join(fixtureDir, file), destination);
}
`;
}

function stubCodexCycle() {
  return `#!/usr/bin/env node
import { appendFileSync } from "node:fs";
import path from "node:path";
const home = ${JSON.stringify(codexHome)};
const cwd = process.cwd();
appendFileSync(path.join(home, "sessions", "session-root.jsonl"), JSON.stringify({ type: "event_msg", payload: { type: "session_meta", id: "session-root", cwd, cli_version: "0.147.0", originator: "codex_exec", source: { subagent: { thread_spawn: { parent_thread_id: "session-root" } } } } }) + "\\n");
`;
}

function stubCodexMonitorFailureWithDescendant(pidFile) {
  return `#!/usr/bin/env node
import { appendFileSync, writeFileSync } from "node:fs";
import { spawn } from "node:child_process";
import path from "node:path";
const home = ${JSON.stringify(codexHome)};
const cwd = process.cwd();
const child = spawn(process.execPath, ["-e", "setTimeout(() => {}, 5000);"], { stdio: "ignore" });
writeFileSync(${JSON.stringify(pidFile)}, String(child.pid));
appendFileSync(path.join(home, "sessions", "session-root.jsonl"), JSON.stringify({ type: "event_msg", payload: { type: "session_meta", id: "session-root", cwd, cli_version: "0.147.0", originator: "codex_exec", source: { subagent: { thread_spawn: { parent_thread_id: "session-root" } } } } }) + "\\n");
await new Promise((resolve) => setTimeout(resolve, 5000));
`;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function withEnv(name, value, callback) {
  const previous = process.env[name];
  try {
    process.env[name] = value;
    return await callback();
  } finally {
    if (previous === undefined) {
      delete process.env[name];
    } else {
      process.env[name] = previous;
    }
  }
}

function isPidAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}
