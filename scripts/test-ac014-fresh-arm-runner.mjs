import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

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
  cli_version: "0.146.0",
  oracle_id: "sparziele-exact-product-flow-v1",
  oracle_sha256: sha256File(oracleStubPath),
};

const fresh = path.join(root, "fresh");
const passed = run(config({
  fresh_root: fresh,
  codex_command: codexCommand(codexStubPath),
  oracle_command: [oracleStubPath],
}), path.join(root, "result.json"), callsPath);
assert.equal(passed.status, 0, passed.output);
const result = JSON.parse(readFileSync(path.join(root, "result.json"), "utf8"));
assert.equal(result.status, "passed");
assert.equal(result.retry, false);
assert.equal(result.copied_from_declared_baseline_only, true);
assert.equal(statSync(path.join(fresh, "README.md")).isFile(), true);
assert.equal(result.observed_contract.candidate_sha, candidateSha);
assert.equal(result.observed_contract.candidate_binary_sha256, fixedContract.candidate_binary_sha256);
assert.equal(result.observed_contract.prompt_digest, fixedContract.prompt_digest);
assert.equal(result.observed_contract.spec_digest, fixedContract.spec_digest);
assert.equal(result.observed_contract.model, "gpt-5.6-sol");
assert.equal(result.observed_contract.effort, "medium");
assert.equal(result.observed_contract.surface, "identical");
assert.equal(result.observed_contract.cli_version, "0.146.0");
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
assert.equal(statSync(path.join(fresh, ".planr", "artifacts", "ac014", "PREFLIGHT.json")).isFile(), true);
assert.equal(statSync(path.join(fresh, ".planr", "artifacts", "ac014", "BENCHMARK_RESULT.json")).isFile(), true);
assert.equal(statSync(path.join(fresh, ".planr", "artifacts", "ac014", "monitor-status.json")).isFile(), true);
assertCompleteArtifactSet(fresh);

const calls = readFileSync(callsPath, "utf8").trim().split("\n").map(JSON.parse);
assert.deepEqual(calls.slice(0, 2).map((call) => call.command), ["project relocate", "project relocate"]);
assert.equal(calls[0].cwd, fresh);
assert.equal(calls[0].args.includes("--apply"), false);
assert.equal(calls[1].args.includes("--apply"), true);
assert.equal(calls.filter((call) => call.command === "evidence migrate").length, 2);

const slowOverStubPath = path.join(root, "codex-over-stub.mjs");
writeExecutable(slowOverStubPath, stubCodexSession({
  rootTokens: 5977897,
  childTokens: 0,
  rootTools: 94,
  childTools: 0,
  sleepMs: 5000,
}));
const interrupted = run(config({
  fresh_root: path.join(root, "fresh-over-ceiling"),
  codex_command: codexCommand(slowOverStubPath),
  oracle_command: [oracleStubPath],
  monitor_poll_ms: 5,
}), path.join(root, "over-ceiling-result.json"), path.join(root, "calls-over.jsonl"));
assert.equal(interrupted.status, 1);
const overCeiling = JSON.parse(readFileSync(path.join(root, "over-ceiling-result.json"), "utf8"));
assert.equal(overCeiling.failure_class, "product");
assert.equal(overCeiling.retry, false);
assert.equal(overCeiling.ceilings.checks.total_tokens.status, "failed");
assert.equal(overCeiling.codex.interrupted.ceiling, "total_tokens");
assert.equal(statSync(path.join(root, "fresh-over-ceiling", ".planr", "artifacts", "ac014", "monitor-status.json")).isFile(), true);

const suppliedMetrics = run(config({
  fresh_root: path.join(root, "fresh-supplied-metrics"),
  codex_command: codexCommand(codexStubPath),
  oracle_command: [oracleStubPath],
  metrics: { wall_time_seconds: 1, total_tokens: 1, tool_call_envelopes: 1 },
}), path.join(root, "supplied-metrics-result.json"), path.join(root, "calls-supplied-metrics.jsonl"));
assert.equal(suppliedMetrics.status, 1);
assert.equal(JSON.parse(readFileSync(path.join(root, "supplied-metrics-result.json"), "utf8")).failure_class, "admission");
assertCompleteArtifactSet(path.join(root, "fresh-supplied-metrics"));

const mismatch = run(config({
  fresh_root: path.join(root, "fresh-contract-mismatch"),
  codex_command: codexCommand(codexStubPath),
  oracle_command: [oracleStubPath],
  fixed_contract: { ...fixedContract, model: "gpt-4" },
}), path.join(root, "contract-mismatch-result.json"), path.join(root, "calls-contract.jsonl"));
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
const tuiSurface = run(config({
  fresh_root: path.join(root, "fresh-tui-surface"),
  codex_command: codexCommand(tuiStubPath),
  oracle_command: [oracleStubPath],
}), path.join(root, "tui-surface-result.json"), path.join(root, "calls-tui-surface.jsonl"));
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
const partialOriginator = run(config({
  fresh_root: path.join(root, "fresh-partial-originator-surface"),
  codex_command: codexCommand(partialOriginatorStubPath),
  oracle_command: [oracleStubPath],
}), path.join(root, "partial-originator-surface-result.json"), path.join(root, "calls-partial-originator-surface.jsonl"));
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
const partialSource = run(config({
  fresh_root: path.join(root, "fresh-partial-source-surface"),
  codex_command: codexCommand(partialSourceStubPath),
  oracle_command: [oracleStubPath],
}), path.join(root, "partial-source-surface-result.json"), path.join(root, "calls-partial-source-surface.jsonl"));
assert.equal(partialSource.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "partial-source-surface-result.json"), "utf8")).error, /observed identity mismatch: surface/);

const reusedFresh = path.join(root, "fresh-reused");
mkdirSync(reusedFresh);
writeFileSync(path.join(reusedFresh, "sentinel.txt"), "do not mutate\n");
const reused = run(config({
  fresh_root: reusedFresh,
  codex_command: codexCommand(codexStubPath),
  oracle_command: [oracleStubPath],
}), path.join(root, "reused-result.json"), path.join(root, "calls-reused.jsonl"));
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
const artifactOverwrite = run(config({
  baseline_root: artifactBaseline,
  fresh_root: artifactOverwriteFresh,
  codex_command: codexCommand(codexStubPath),
  oracle_command: [oracleStubPath],
}), path.join(root, "artifact-overwrite-result.json"), path.join(root, "calls-artifact-overwrite.jsonl"));
assert.equal(artifactOverwrite.status, 1);
assert.equal(JSON.parse(readFileSync(path.join(root, "artifact-overwrite-result.json"), "utf8")).failure_class, "instrumentation");
assert.equal(readFileSync(path.join(artifactOverwriteFresh, ".planr", "artifacts", "ac014", "BENCHMARK_RESULT.json"), "utf8"), "{}\n");
assertCompleteArtifactSet(path.join(root, ".planr-ac014-failures", "fresh-artifact-overwrite"));

const publicationDeadlineFresh = path.join(root, "fresh-publication-deadline");
const publicationDeadline = run(config({
  fresh_root: publicationDeadlineFresh,
  codex_command: codexCommand(codexStubPath),
  oracle_command: [oracleStubPath],
  test_stage_write_delay_ms: 1,
  test_stage_deadline_elapsed_wall_seconds: fixedCeilings.wall_time_seconds + 0.001,
}), path.join(root, "publication-deadline-result.json"), path.join(root, "calls-publication-deadline.jsonl"));
assert.equal(publicationDeadline.status, 1);
const publicationDeadlineResult = JSON.parse(readFileSync(path.join(root, "publication-deadline-result.json"), "utf8"));
assert.equal(publicationDeadlineResult.status, "failed");
assert.equal(publicationDeadlineResult.failure_class, "product");
const committedDeadlineResult = JSON.parse(readFileSync(path.join(publicationDeadlineFresh, ".planr", "artifacts", "ac014", "BENCHMARK_RESULT.json"), "utf8"));
assert.equal(committedDeadlineResult.status, "failed");
assert.equal(committedDeadlineResult.stop_reason, "deadline_exceeded");
assertCompleteArtifactSet(publicationDeadlineFresh);

const cycleStubPath = path.join(root, "codex-cycle-stub.mjs");
writeExecutable(cycleStubPath, stubCodexCycle());
const cycle = run(config({
  fresh_root: path.join(root, "fresh-cycle"),
  codex_command: codexCommand(cycleStubPath),
  oracle_command: [oracleStubPath],
}), path.join(root, "cycle-result.json"), path.join(root, "calls-cycle.jsonl"));
assert.equal(cycle.status, 1);
assert.match(JSON.parse(readFileSync(path.join(root, "cycle-result.json"), "utf8")).error, /cyclic Codex session lineage/);

const outsideStubPath = path.join(root, "planr-outside-stub.mjs");
writeExecutable(outsideStubPath, stubPlanr({ outsidePath: true }));
const outsideContract = { ...fixedContract, candidate_binary_sha256: sha256File(outsideStubPath) };
const outside = run(config({
  fresh_root: path.join(root, "fresh-outside"),
  planr_bin: outsideStubPath,
  fixed_contract: outsideContract,
  codex_command: codexCommand(codexStubPath),
  oracle_command: [oracleStubPath],
}), path.join(root, "outside-result.json"), path.join(root, "calls-outside.jsonl"));
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
    ceilings: fixedCeilings,
    monitor_poll_ms: 20,
    ...overrides,
  };
}

function codexCommand(file) {
  return [file, "--model", "gpt-5.6-sol", "--effort", "medium", "--surface", "identical"];
}

function run(input, resultPath, callsFile) {
  const inputPath = `${resultPath}.input.json`;
  writeFileSync(inputPath, JSON.stringify(input));
  const outcome = spawnSync("node", [
    path.join(repoRoot, "scripts/ac014-fresh-arm-runner.mjs"),
    "--input",
    inputPath,
    "--result",
    resultPath,
  ], {
    encoding: "utf8",
    env: { ...process.env, PLANR_STUB_CALLS: callsFile },
  });
  let resultText = "";
  try {
    resultText = readFileSync(resultPath, "utf8");
  } catch {
    resultText = "<no result file>";
  }
  outcome.output = `${outcome.stderr}\n${outcome.stdout}\n${resultText}`;
  return outcome;
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

function stubCodexSession({ rootTokens, childTokens, rootTools, childTools, sleepMs = 0, rootOriginator = "codex_exec", rootSource = "exec" }) {
  return `#!/usr/bin/env node
import { appendFileSync } from "node:fs";
import path from "node:path";
const home = ${JSON.stringify(codexHome)};
const cwd = process.cwd();
const rootSession = path.join(home, "sessions", "session-root.jsonl");
appendFileSync(rootSession, JSON.stringify({ type: "event_msg", payload: { type: "session_meta", id: "session-root", cwd, role: "root", cli_version: "0.146.0", originator: ${JSON.stringify(rootOriginator)}, source: ${JSON.stringify(rootSource)} } }) + "\\n");
appendFileSync(rootSession, JSON.stringify({ type: "turn_context", payload: { model: "gpt-5.6-sol", effort: "medium" } }) + "\\n");
appendFileSync(rootSession, JSON.stringify({ type: "event_msg", payload: { type: "token_count", info: { total_token_usage: { total_tokens: ${rootTokens} }, last_token_usage: { total_tokens: 999999999 } }, tool_call_envelopes: ${rootTools} } }) + "\\n");
appendFileSync(rootSession, JSON.stringify({ session_id: "session-root", type: "function_call", id: "root-call" }) + "\\n");
${childTokens > 0 ? `const childSession = path.join(home, "sessions", "session-child.jsonl");
appendFileSync(childSession, JSON.stringify({ type: "event_msg", payload: { type: "session_meta", session_id: "session-root", id: "session-child", cwd, role: "subagent", cli_version: "0.146.0", originator: "codex_exec", source: { subagent: { thread_spawn: { parent_thread_id: "session-root" } } } } }) + "\\n");
appendFileSync(childSession, JSON.stringify({ type: "turn_context", payload: { model: "gpt-5.6-sol", effort: "medium" } }) + "\\n");
appendFileSync(childSession, JSON.stringify({ type: "event_msg", payload: { type: "token_count", info: { total_token_usage: { total_tokens: ${childTokens} }, last_token_usage: { total_tokens: 777777777 } }, tool_call_envelopes: ${childTools} } }) + "\\n");
appendFileSync(childSession, JSON.stringify({ session_id: "session-child", type: "custom_tool_call", id: "child-call" }) + "\\n");` : ""}
${sleepMs > 0 ? `await new Promise((resolve) => setTimeout(resolve, ${sleepMs}));` : ""}
`;
}

function stubCodexCycle() {
  return `#!/usr/bin/env node
import { appendFileSync } from "node:fs";
import path from "node:path";
const home = ${JSON.stringify(codexHome)};
const cwd = process.cwd();
appendFileSync(path.join(home, "sessions", "session-root.jsonl"), JSON.stringify({ type: "event_msg", payload: { type: "session_meta", id: "session-root", cwd, cli_version: "0.146.0", originator: "codex_exec", source: { subagent: { thread_spawn: { parent_thread_id: "session-root" } } } } }) + "\\n");
`;
}
