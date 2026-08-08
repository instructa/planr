#!/usr/bin/env node
import { createHash } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import {
  constants,
  accessSync,
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  renameSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";

const EXPECTED = Object.freeze({
  candidate_version: "1.10.0-alpha.4",
  model: "gpt-5.6-sol",
  effort: "medium",
  surface: "identical",
  cli_version: "0.146.0",
  oracle_id: "sparziele-exact-product-flow-v1",
  ceilings: Object.freeze({
    wall_time_seconds: 998.015,
    total_tokens: 5977896,
    tool_call_envelopes: 93,
  }),
});

const argv = process.argv.slice(2);
const inputPath = valueAfter("--input");
const resultPath = valueAfter("--result");
if (!inputPath) {
  throw new Error("usage: node scripts/ac014-fresh-arm-runner.mjs --input <run.json> [--result <result.json>]");
}

const startedAt = new Date().toISOString();
const input = readJson(inputPath);
const transcript = [];

let result;
try {
  result = await runFreshArm(input, transcript);
} catch (error) {
  writeFailureArtifacts(input, error, transcript);
  result = classifiedResult(error, transcript);
}
result.started_at = startedAt;
result.finished_at = new Date().toISOString();

const serialized = `${JSON.stringify(result, null, 2)}\n`;
if (resultPath) {
  mkdirSync(path.dirname(path.resolve(resultPath)), { recursive: true });
  writeFileSync(resultPath, serialized, { flag: "wx", mode: 0o600 });
} else {
  process.stdout.write(serialized);
}
if (result.status !== "passed") {
  process.exitCode = 1;
}

async function runFreshArm(config, commands) {
  requireSchema(config);
  if ("replace_fresh_root" in config) {
    throw admission("AC-014 fresh_root must never be destructively replaced");
  }
  const baselineRoot = canonicalExistingDir(config.baseline_root, "baseline_root");
  const freshRoot = path.resolve(config.fresh_root);
  if (isPathInside(freshRoot, baselineRoot) || isPathInside(baselineRoot, freshRoot)) {
    throw admission(`fresh_root and baseline_root must be separate: ${freshRoot}`);
  }
  if (existsSync(freshRoot)) {
    throw admission(`fresh_root must be a new non-existing path: ${freshRoot}`, { rejected_fresh_root: freshRoot });
  }
  validateBaselineNoSymlinkEscapes(baselineRoot);

  mkdirSync(path.dirname(freshRoot), { recursive: true });
  cpSync(baselineRoot, freshRoot, {
    recursive: true,
    verbatimSymlinks: false,
    filter(source) {
      const relative = path.relative(baselineRoot, source);
      if (!relative) {
        return true;
      }
      return !new Set(config.copy_excludes ?? ["target", "node_modules"]).has(relative.split(path.sep)[0]);
    },
  });
  const freshCanonical = realpathSync(freshRoot);
  if ("metrics" in config || "metrics_path" in config) {
    throw admission("AC-014 metrics must be derived from CODEX_HOME sessions, not supplied by config");
  }

  const planrBin = path.resolve(config.planr_bin ?? process.env.PLANR_BIN ?? "target/debug/planr");
  assertExecutable(planrBin);
  const preCodexContract = observeStaticContract(config, freshCanonical, planrBin);
  const dbPath = resolveFreshPath(config.db_path ?? ".planr/planr.sqlite", freshCanonical, "db_path", false);
  const projectId = requiredString(config.project_id, "project_id");

  const preview = runPlanrJson(planrBin, dbPath, [
    "project",
    "relocate",
    projectId,
    "--destination",
    freshCanonical,
  ], commands, "instrumentation");
  const applied = runPlanrJson(planrBin, dbPath, [
    "project",
    "relocate",
    projectId,
    "--destination",
    freshCanonical,
    "--apply",
  ], commands, "instrumentation");

  const validation = validateLocalPlanrState(planrBin, dbPath, freshCanonical, commands);
  const migrationInput = resolveFreshPath(
    requiredString(config.evidence_migration_input, "evidence_migration_input"),
    freshCanonical,
    "evidence_migration_input",
    true,
  );
  const migrationPreview = runPlanrJson(planrBin, dbPath, [
    "evidence",
    "migrate",
    "--input",
    migrationInput,
  ], commands, "instrumentation");
  const migrationApplied = runPlanrJson(planrBin, dbPath, [
    "evidence",
    "migrate",
    "--input",
    migrationInput,
    "--apply",
  ], commands, "instrumentation");
  const evidenceMigration = {
    input: path.relative(freshCanonical, migrationInput),
    preview: summarizeJson(migrationPreview),
    applied: summarizeJson(migrationApplied),
  };

  const codexHome = canonicalExistingDir(config.codex_home ?? process.env.CODEX_HOME, "CODEX_HOME");
  const armMonitor = createArmMonitor(config);
  let codex;
  try {
    codex = await runMonitoredCommand(config, freshCanonical, codexHome, commands, "codex", config.codex_command, armMonitor);
  } catch (error) {
    const payload = artifactPayload(preview, applied, validation, evidenceMigration, preCodexContract, null, [], null, null, armMonitor);
    writeImmutableArtifacts(config, freshCanonical, payload, "external_invalid", "codex_failed", armMonitor);
    error.details = { ...(error.details ?? {}), ...payload };
    throw error;
  }
  const sessions = discoverCodexSessionTree(codexHome, freshCanonical, config.root_session_id);
  if (sessions.length === 0) {
    throw instrumentation("at least one CODEX_HOME session is required for AC-014 usage accounting");
  }
  const observedContract = observeEffectiveContract(config, preCodexContract, sessions);
  const metrics = usageMetricsFromSessions(sessions, elapsedArmSeconds(armMonitor));
  const ceilingVerdict = enforceCeilings(metrics, config.ceilings);
  const failedCeiling = Object.values(ceilingVerdict.checks).find((check) => check.status !== "passed");
  if (failedCeiling) {
    const payload = {
      preview: summarizeJson(preview),
      applied: summarizeJson(applied),
      validation,
      evidence_migration: evidenceMigration,
      codex,
      sessions,
      ceilings: ceilingVerdict,
      oracle: null,
    };
    writeImmutableArtifacts(config, freshCanonical, { ...payload, observed_contract: observedContract }, "failed", "ceiling_exceeded", armMonitor);
    throw product(`AC-014 ceiling exceeded: ${failedCeiling.name}`, payload);
  }
  let oracleRun;
  try {
    oracleRun = await runMonitoredCommand(config, freshCanonical, codexHome, commands, "oracle", config.oracle_command, armMonitor);
  } catch (error) {
    const payload = artifactPayload(preview, applied, validation, evidenceMigration, observedContract, codex, sessions, ceilingVerdict, null, armMonitor);
    writeImmutableArtifacts(config, freshCanonical, payload, "external_invalid", "oracle_failed", armMonitor);
    error.details = { ...(error.details ?? {}), ...payload };
    throw error;
  }
  const sessionsAfterOracle = discoverCodexSessionTree(codexHome, freshCanonical, config.root_session_id);
  const oracleMetrics = usageMetricsFromSessions(sessionsAfterOracle, elapsedArmSeconds(armMonitor));
  const oracleCeilingVerdict = enforceCeilings(oracleMetrics, config.ceilings);
  const failedOracleCeiling = Object.values(oracleCeilingVerdict.checks).find((check) => check.status !== "passed");
  if (failedOracleCeiling) {
    const payload = {
      preview: summarizeJson(preview),
      applied: summarizeJson(applied),
      validation,
      evidence_migration: evidenceMigration,
      codex,
      sessions: sessionsAfterOracle,
      ceilings: oracleCeilingVerdict,
      oracle: oracleRun,
    };
    writeImmutableArtifacts(config, freshCanonical, { ...payload, observed_contract: observedContract }, "failed", "ceiling_exceeded", armMonitor);
    throw product(`AC-014 ceiling exceeded during oracle: ${failedOracleCeiling.name}`, payload);
  }
  const oracle = {
    id: EXPECTED.oracle_id,
    command: oracleRun.command,
    status: oracleRun.exit_code === 0 ? "passed" : "failed",
    exit_code: oracleRun.exit_code,
  };
  if (oracle.status !== "passed") {
    const payload = {
      preview: summarizeJson(preview),
      applied: summarizeJson(applied),
      validation,
      evidence_migration: evidenceMigration,
      codex,
      sessions: sessionsAfterOracle,
      ceilings: oracleCeilingVerdict,
      oracle,
    };
    writeImmutableArtifacts(config, freshCanonical, { ...payload, observed_contract: observedContract }, "failed", "oracle_failed", armMonitor);
    throw product("AC-014 exact product oracle failed", payload);
  }

  const passed = {
    schema_version: "planr.ac014.fresh_arm_result.v1",
    status: "passed",
    failure_class: null,
    retry: false,
    fresh_root: freshCanonical,
    baseline_root: baselineRoot,
    copied_from_declared_baseline_only: true,
    relocation: {
      preview: summarizeJson(preview),
      applied: summarizeJson(applied),
    },
    validation,
    evidence_migration: evidenceMigration,
    observed_contract: observedContract,
    codex,
    sessions: sessionsAfterOracle,
    ceilings: oracleCeilingVerdict,
    oracle,
    commands,
  };
  writeImmutableArtifacts(config, freshCanonical, passed, "passed", "completed", armMonitor);
  return passed;
}

function artifactPayload(preview, applied, validation, evidenceMigration, observedContract, codex, sessions, ceilings, oracle, armMonitor) {
  return {
    preview: summarizeJson(preview),
    applied: summarizeJson(applied),
    validation,
    evidence_migration: evidenceMigration,
    observed_contract: observedContract,
    codex,
    sessions,
    ceilings,
    oracle,
    arm: armMonitor ? { elapsed_wall_seconds: elapsedArmSeconds(armMonitor), deadline_wall_seconds: EXPECTED.ceilings.wall_time_seconds } : null,
  };
}

function validateLocalPlanrState(planrBin, dbPath, freshRoot, commands) {
  const project = runPlanrJson(planrBin, dbPath, ["project", "show"], commands, "instrumentation").project;
  const plans = runPlanrJson(planrBin, dbPath, ["plan", "list"], commands, "instrumentation").plans ?? [];
  const items = runPlanrJson(planrBin, dbPath, ["map", "show"], commands, "instrumentation").items ?? [];
  const paths = [];
  if (project?.root_path) {
    paths.push(["project", project.id ?? "default", project.root_path]);
  }
  for (const plan of plans) {
    if (plan.path) {
      paths.push(["plan", plan.id, plan.path]);
    }
  }
  for (const item of items) {
    if (item.plan_path) {
      paths.push(["item", item.id, item.plan_path]);
    }
  }
  for (const [kind, id, candidate] of paths) {
    assertLocalPath(candidate, freshRoot, `${kind}:${id}`);
  }
  return {
    projects_checked: project?.root_path ? 1 : 0,
    plans_checked: plans.length,
    items_checked: items.length,
    descendant_counts: descendantCounts(items),
    all_paths_local: true,
  };
}

function descendantCounts(items) {
  const children = new Map();
  for (const item of items) {
    if (item.parent_item_id) {
      const siblings = children.get(item.parent_item_id) ?? [];
      siblings.push(item.id);
      children.set(item.parent_item_id, siblings);
    }
  }
  const memo = new Map();
  const countFor = (id) => {
    if (memo.has(id)) {
      return memo.get(id);
    }
    const total = (children.get(id) ?? []).reduce((sum, child) => sum + 1 + countFor(child), 0);
    memo.set(id, total);
    return total;
  };
  return Object.fromEntries(items.map((item) => [item.id, countFor(item.id)]).sort());
}

function enforceCeilings(metrics, ceilings) {
  if (ceilings && JSON.stringify(ceilings) !== JSON.stringify(EXPECTED.ceilings)) {
    throw admission("AC-014 ceilings are fixed and must not be changed");
  }
  const effective = EXPECTED.ceilings;
  const observed = metrics ?? {};
  const checks = {};
  for (const [name, max] of Object.entries(effective)) {
    const value = observed[name];
    checks[name] = {
      name,
      max,
      observed: value ?? null,
      status: typeof value === "number" && value <= max ? "passed" : "failed",
    };
  }
  return { checks, all_unchanged_ceilings_enforced: true, immutable_ceilings: effective };
}

function assertArmDeadline(monitor, phase) {
  const failure = deadlineFailure(monitor, phase);
  if (failure) {
    throw failure;
  }
}

function deadlineFailure(monitor, phase) {
  const elapsed = elapsedArmSeconds(monitor);
  if (elapsed > EXPECTED.ceilings.wall_time_seconds) {
    return product(`AC-014 continuous wall deadline exceeded during ${phase}`, {
      ceilings: enforceCeilings({ wall_time_seconds: elapsed, total_tokens: 0, tool_call_envelopes: 0 }),
      arm: { elapsed_wall_seconds: elapsed, deadline_wall_seconds: EXPECTED.ceilings.wall_time_seconds },
    });
  }
  return null;
}

function createArmMonitor(config) {
  if (config.ceilings && JSON.stringify(config.ceilings) !== JSON.stringify(EXPECTED.ceilings)) {
    throw admission("AC-014 ceilings are fixed and must not be changed");
  }
  const startedWall = Date.now();
  return {
    startedWall,
    deadlineWall: startedWall + EXPECTED.ceilings.wall_time_seconds * 1000,
  };
}

function elapsedArmSeconds(monitor) {
  return Math.round(((Date.now() - monitor.startedWall) / 1000) * 1000) / 1000;
}

async function runMonitoredCommand(config, freshRoot, codexHome, commands, phase, commandValue, armMonitor) {
  const command = requiredCommand(commandValue, `${phase}_command`).map((part, index) =>
    index === 0 ? path.resolve(part) : String(part)
  );
  assertExecutable(command[0]);
  const startedTick = performance.now();
  const phaseEnv = phase === "oracle" ? config.oracle_env : config.codex_env;
  const child = spawn(command[0], command.slice(1), {
    cwd: freshRoot,
    env: { ...process.env, ...(phaseEnv ?? {}) },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => { stdout += chunk; });
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  let closed = false;
  const exitPromise = waitForChild(child).then((exit) => {
    closed = true;
    return exit;
  });
  let interrupted = null;
  const pollMs = Number.isInteger(config.monitor_poll_ms) ? config.monitor_poll_ms : 1000;
  while (!closed) {
    await sleep(pollMs);
    if (closed) {
      break;
    }
    const elapsed = elapsedArmSeconds(armMonitor);
    const sessions = discoverCodexSessionTree(codexHome, freshRoot, config.root_session_id, { allowEmpty: true });
    const metrics = usageMetricsFromSessions(sessions, elapsed);
    const verdict = enforceCeilings(metrics, config.ceilings);
    const failed = Object.values(verdict.checks).find((check) => check.status !== "passed");
    if (failed) {
      interrupted = { ceiling: failed.name, metrics, elapsed_wall_seconds: elapsed };
      terminateChild(child);
      break;
    }
  }
  const exit = await exitPromise;
  const durationSeconds = Math.round(((performance.now() - startedTick) / 1000) * 1000) / 1000;
  commands.push({
    command: command.join(" "),
    phase,
    cwd: ".",
    status: exit.code,
    signal: exit.signal,
    stdout_digest: `sha256:${createHash("sha256").update(stdout).digest("hex")}`,
    stderr: trim(stderr),
  });
  const result = {
    command: command.map((part, index) => (index === 0 ? path.basename(part) : part)),
    exit_code: exit.code,
    signal: exit.signal,
    elapsed_wall_seconds: durationSeconds,
    interrupted,
  };
  if (interrupted) {
    return result;
  }
  if (exit.code !== 0) {
    throw instrumentation(`AC-014 ${phase} command failed`, {
      command: command.join(" "),
      status: exit.code,
      stderr: trim(stderr),
    });
  }
  return result;
}

function usageMetricsFromSessions(sessions, wallSeconds) {
  return {
    wall_time_seconds: wallSeconds,
    total_tokens: sessions.reduce((sum, session) => sum + session.usage.total_tokens, 0),
    tool_call_envelopes: sessions.reduce((sum, session) => sum + session.usage.tool_call_envelopes, 0),
  };
}

function discoverCodexSessionTree(codexHome, freshRoot, rootSessionId, { allowEmpty = false } = {}) {
  const files = allFiles(codexHome).filter((file) => file.endsWith(".jsonl"));
  const sessions = files.map((file) => parseSessionFile(codexHome, file)).filter(Boolean);
  const seen = new Map();
  for (const session of sessions) {
    if (seen.has(session.id)) {
      throw instrumentation(`duplicate Codex session id discovered: ${session.id}`);
    }
    if (session.parent === session.id) {
      throw instrumentation(`cyclic Codex session lineage discovered: ${session.id}`);
    }
    seen.set(session.id, session);
  }
  const root = rootSessionId
    ? sessions.find((session) => session.id === rootSessionId)
    : sessions.find((session) => session.cwd === freshRoot && !session.parent);
  if (!root) {
    if (allowEmpty) return [];
    throw instrumentation("launched root Codex session was not discovered under CODEX_HOME");
  }
  const byParent = new Map();
  for (const session of sessions) {
    if (!session.parent) continue;
    const children = byParent.get(session.parent) ?? [];
    children.push(session);
    byParent.set(session.parent, children);
  }
  const discovered = [];
  const stack = [root];
  const visiting = new Set();
  while (stack.length > 0) {
    const session = stack.pop();
    if (visiting.has(session.id)) {
      throw instrumentation(`cyclic Codex session lineage discovered: ${session.id}`);
    }
    visiting.add(session.id);
    discovered.push(session);
    for (const child of byParent.get(session.id) ?? []) {
      stack.push(child);
    }
  }
  return discovered.sort((a, b) => a.id.localeCompare(b.id));
}

function parseSessionFile(root, file) {
  const bytes = readFileSync(file);
  const lines = bytes.toString("utf8").split(/\n/u).filter((line) => line.trim());
  let id = idFromFilename(file);
  let parent = null;
  let cwd = null;
  let role = null;
  let cliVersion = null;
  let surface = null;
  let turnContext = null;
  let latestTokens = 0;
  let latestToolEnvelopeTotal = 0;
  const uniqueToolCalls = new Set();
  for (const line of lines) {
    if (!line.trim()) continue;
    let value;
    try {
      value = JSON.parse(line);
    } catch {
      continue;
    }
    const meta = sessionMeta(value);
    id = meta.id ?? id;
    parent = meta.parent ?? parent;
    cwd = meta.cwd ?? cwd;
    role = meta.role ?? role;
    cliVersion = meta.cli_version ?? cliVersion;
    surface = meta.surface ?? surface;
    turnContext = meta.turn_context ?? turnContext;
    latestTokens = Math.max(latestTokens, tokenTotal(value));
    latestToolEnvelopeTotal = Math.max(
      latestToolEnvelopeTotal,
      numericDeep(value, new Set(["tool_call_envelopes", "toolCallEnvelopes"])),
    );
    collectToolCalls(value, uniqueToolCalls);
  }
  if (!id) return null;
  return {
    id,
    parent,
    role,
    cli_version: cliVersion,
    surface,
    turn_context: turnContext,
    cwd,
    path: path.relative(root, file),
    bytes: bytes.length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
    usage: {
      total_tokens: latestTokens,
      tool_call_envelopes: Math.max(latestToolEnvelopeTotal, uniqueToolCalls.size),
    },
  };
}

function sessionMeta(value) {
  const payload = value?.payload && typeof value.payload === "object" ? value.payload : value;
  if (value?.type === "session_meta" || payload?.type === "session_meta" || payload?.source?.subagent?.thread_spawn) {
    return {
      id: stringField(payload, "id") ?? stringField(payload, "thread_id") ?? stringField(payload, "threadId") ?? stringField(payload, "session_id"),
      parent: payload?.source?.subagent?.thread_spawn?.parent_thread_id
        ?? payload?.source?.subagent?.thread_spawn?.parentThreadId
        ?? stringField(payload, "parent_thread_id")
        ?? stringField(payload, "parentThreadId"),
      cwd: stringField(payload, "cwd") ?? stringField(payload, "working_directory") ?? stringField(payload, "workingDirectory"),
      role: stringField(payload, "role") ?? stringField(payload, "agent_role") ?? stringField(payload, "agentRole"),
      cli_version: stringField(payload, "cli_version") ?? stringField(payload, "cliVersion"),
      surface: surfaceFromSessionMeta(payload),
      turn_context: turnContext(payload),
    };
  }
  return {
    id: stringField(value, "session_id") ?? stringField(value, "sessionId") ?? stringField(value, "thread_id") ?? stringField(value, "threadId"),
    parent: stringField(value, "parent_thread_id") ?? stringField(value, "parentThreadId") ?? stringField(value, "parent_id") ?? stringField(value, "parentId"),
    cwd: stringField(value, "cwd") ?? stringField(value, "working_directory") ?? stringField(value, "workingDirectory"),
    role: stringField(value, "role") ?? stringField(value, "agent_role") ?? stringField(value, "agentRole"),
    cli_version: stringField(value, "cli_version") ?? stringField(value, "cliVersion"),
    surface: surfaceFromSessionMeta(value),
    turn_context: turnContext(value),
  };
}

function turnContext(value) {
  const payload = value?.payload && typeof value.payload === "object" ? value.payload : value;
  const context = value?.type === "turn_context" || payload?.type === "turn_context"
    ? payload
    : payload?.turn_context ?? payload?.turnContext;
  if (!context || typeof context !== "object") return null;
  return {
    model: stringField(context, "model"),
    effort: stringField(context, "effort") ?? stringField(context, "reasoning_effort") ?? stringField(context, "reasoningEffort"),
  };
}

function surfaceFromSessionMeta(value) {
  const originator = stringField(value, "originator");
  const source = stringField(value, "source");
  if (originator === "codex_exec" && source === "exec") {
    return "identical";
  }
  if (originator || source) {
    return "non_cli";
  }
  return null;
}

function tokenTotal(value) {
  const payload = value?.payload && typeof value.payload === "object" ? value.payload : value;
  const canonical = payload?.info?.total_token_usage?.total_tokens ?? value?.payload?.info?.total_token_usage?.total_tokens;
  if (typeof canonical === "number") return canonical;
  const totalUsage = payload?.total_token_usage ?? value?.total_token_usage;
  if (typeof totalUsage === "number") return totalUsage;
  const usage = payload?.usage ?? value?.usage;
  if (usage && typeof usage.total_tokens === "number") return usage.total_tokens;
  if (usage && typeof usage.totalTokens === "number") return usage.totalTokens;
  return 0;
}

function stringField(value, key) {
  return typeof value?.[key] === "string" && value[key].length > 0 ? value[key] : null;
}

function numericDeep(value, keys) {
  if (Array.isArray(value)) {
    return value.reduce((sum, child) => sum + numericDeep(child, keys), 0);
  }
  if (!value || typeof value !== "object") {
    return 0;
  }
  let sum = 0;
  for (const [key, child] of Object.entries(value)) {
    if (keys.has(key) && typeof child === "number") {
      sum += child;
    } else if (child && typeof child === "object") {
      sum += numericDeep(child, keys);
    }
  }
  return sum;
}

function stringDeep(value, keys) {
  if (Array.isArray(value)) {
    for (const child of value) {
      const found = stringDeep(child, keys);
      if (found) return found;
    }
    return null;
  }
  if (!value || typeof value !== "object") return null;
  for (const [key, child] of Object.entries(value)) {
    if (keys.has(key) && typeof child === "string" && child.length > 0) return child;
    if (child && typeof child === "object") {
      const found = stringDeep(child, keys);
      if (found) return found;
    }
  }
  return null;
}

function collectToolCalls(value, calls) {
  if (Array.isArray(value)) {
    value.forEach((child) => collectToolCalls(child, calls));
    return;
  }
  if (!value || typeof value !== "object") return;
  const type = value.type ?? value.kind ?? value.call_type;
  if (type === "function_call" || type === "custom_tool_call" || type === "tool_call") {
    calls.add(String(value.id ?? value.call_id ?? value.name ?? calls.size));
  }
  for (const child of Object.values(value)) collectToolCalls(child, calls);
}

function writeImmutableArtifacts(config, freshRoot, result, status, stopReason, armMonitor = null) {
  if (armMonitor) {
    assertArmDeadline(armMonitor, "artifact_finalization");
  }
  if (Number.isInteger(config.finalization_delay_ms) && config.finalization_delay_ms > 0) {
    sleepSync(config.finalization_delay_ms);
  }
  const artifactDirValue = config.artifact_dir ?? ".planr/artifacts/ac014";
  const artifactDir = resolveFreshPath(artifactDirValue, freshRoot, "artifact_dir", false);
  const resultPath = path.join(artifactDir, "BENCHMARK_RESULT.json");
  const preflightPath = path.join(artifactDir, "PREFLIGHT.json");
  const monitorPath = path.join(artifactDir, "monitor-status.json");
  const buildArtifacts = (artifactStatus, artifactStopReason) => {
    const base = {
      status: artifactStatus,
      verdict: artifactStatus === "passed" ? "passed" : "failed",
      stop_reason: artifactStopReason,
      retry: false,
    };
    const artifacts = new Map();
    artifacts.set(preflightPath, {
      schema_version: "planr.ac014.preflight.v1",
      ...base,
      expected_contract: EXPECTED,
      observed_contract: result.observed_contract ?? null,
      relocation: {
        preview: result.preview ?? result.relocation?.preview ?? null,
        applied: result.applied ?? result.relocation?.applied ?? null,
      },
      validation: result.validation ?? null,
      evidence_migration: result.evidence_migration ?? null,
    });
    artifacts.set(resultPath, {
      schema_version: "planr.ac014.benchmark_result.v1",
      ...result,
      ...base,
    });
    artifacts.set(monitorPath, {
      schema_version: "planr.ac014.monitor_status.v1",
      ...base,
      sessions: result.sessions ?? [],
      ceilings: result.ceilings ?? null,
      oracle: result.oracle ?? null,
    });
    return artifacts;
  };
  let publicationDeadline = null;
  const publication = writeJsonSetImmutable(buildArtifacts(status, stopReason), {
    afterStage() {
      if (Number.isInteger(config.test_stage_write_delay_ms) && config.test_stage_write_delay_ms > 0) {
        sleepSync(config.test_stage_write_delay_ms);
      }
      if (Number.isFinite(config.test_publication_elapsed_wall_seconds) && armMonitor) {
        armMonitor.startedWall = Date.now() - Math.ceil(config.test_publication_elapsed_wall_seconds * 1000);
      }
      if (Number.isFinite(config.test_stage_deadline_elapsed_wall_seconds) && armMonitor) {
        armMonitor.startedWall = Date.now() - Math.ceil(config.test_stage_deadline_elapsed_wall_seconds * 1000);
      }
      publicationDeadline = armMonitor ? deadlineFailure(armMonitor, "artifact_publication") : null;
      return publicationDeadline ? buildArtifacts("failed", "deadline_exceeded") : null;
    },
  });
  if (publicationDeadline) {
    throw publicationDeadline;
  }
  return publication;
}

function writeJsonSetImmutable(artifacts, options = {}) {
  const files = [...artifacts.keys()];
  const artifactDir = commonDirectory(files);
  if (existsSync(artifactDir)) {
    try {
      validateCommittedArtifactSet(files, artifactDir);
    } catch {
      // Any preexisting public artifact directory is immutable. Markerless or
      // incomplete contents are treated as a collision and preserved.
    }
    throw instrumentation(`immutable artifact already exists: ${artifactDir}`);
  }
  mkdirSync(path.dirname(artifactDir), { recursive: true });
  for (const file of files) {
    if (existsSync(file)) {
      throw instrumentation(`immutable artifact already exists: ${file}`);
    }
  }
  const stageDir = path.join(path.dirname(artifactDir), `.publish-${path.basename(artifactDir)}-${process.pid}-${Date.now()}`);
  try {
    mkdirSync(stageDir, { recursive: true });
    writeStagedArtifactSet(stageDir, artifacts);
    const replacement = options.afterStage?.() ?? null;
    if (replacement) {
      rmSync(stageDir, { recursive: true, force: true });
      mkdirSync(stageDir, { recursive: true });
      writeStagedArtifactSet(stageDir, replacement);
    }
    renameSync(stageDir, artifactDir);
    validateCommittedArtifactSet(files, artifactDir);
    return { commit_marker: path.join(artifactDir, "COMMIT.json") };
  } catch (error) {
    rmSync(stageDir, { recursive: true, force: true });
    throw error;
  }
}

function writeStagedArtifactSet(stageDir, artifacts) {
  const manifestFiles = [];
  for (const [file, value] of artifacts.entries()) {
    const contents = `${JSON.stringify(value, null, 2)}\n`;
    const name = path.basename(file);
    writeFileSync(path.join(stageDir, name), contents, { flag: "wx", mode: 0o600 });
    manifestFiles.push({
      name,
      sha256: `sha256:${createHash("sha256").update(contents).digest("hex")}`,
    });
  }
  writeFileSync(path.join(stageDir, "COMMIT.json"), `${JSON.stringify({
    schema_version: "planr.ac014.artifact_commit.v1",
    files: manifestFiles.sort((left, right) => left.name.localeCompare(right.name)),
  }, null, 2)}\n`, { flag: "wx", mode: 0o600 });
}

function commonDirectory(files) {
  const directories = [...new Set(files.map((file) => path.dirname(file)))];
  if (directories.length !== 1) {
    throw instrumentation("AC-014 artifact publication requires one artifact directory");
  }
  return directories[0];
}

function validateCommittedArtifactSet(files, artifactDir) {
  const marker = path.join(artifactDir, "COMMIT.json");
  if (!existsSync(marker)) {
    throw instrumentation("AC-014 artifact publication missing commit marker");
  }
  const commit = readJson(marker);
  const manifest = new Map((commit.files ?? []).map((entry) => [entry.name, entry.sha256]));
  for (const file of files) {
    if (!existsSync(file)) {
      throw instrumentation(`AC-014 artifact publication missing file: ${file}`);
    }
    const name = path.basename(file);
    const expected = manifest.get(name);
    if (!expected) {
      throw instrumentation(`AC-014 artifact publication missing manifest digest: ${name}`);
    }
    const actual = sha256File(file);
    if (actual !== expected) {
      throw instrumentation(`AC-014 artifact publication digest mismatch: ${name}`);
    }
  }
}

function allFiles(root) {
  const stack = [root];
  const files = [];
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

function idFromFilename(file) {
  return /([0-9a-f]{8}-[0-9a-f-]{20,})/iu.exec(path.basename(file))?.[1] ?? path.basename(file, ".jsonl");
}

function validateBaselineNoSymlinkEscapes(root) {
  const stack = [root];
  while (stack.length > 0) {
    const current = stack.pop();
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const candidate = path.join(current, entry.name);
      if (entry.isSymbolicLink()) {
        const resolved = realpathSync(candidate);
        if (!isPathInside(resolved, root) && resolved !== root) {
          throw admission(`baseline symlink escapes declared baseline: ${candidate}`);
        }
      } else if (entry.isDirectory()) {
        stack.push(candidate);
      } else {
        lstatSync(candidate);
      }
    }
  }
}

function runPlanrJson(planrBin, dbPath, args, commands, failureClass) {
  const fullArgs = ["--db", dbPath, "--json", ...args];
  const output = spawnSync(planrBin, fullArgs, { encoding: "utf8", cwd: path.dirname(path.dirname(dbPath)) });
  commands.push({
    command: [planrBin, ...fullArgs].join(" "),
    status: output.status,
    stdout_digest: `sha256:${createHash("sha256").update(output.stdout ?? "").digest("hex")}`,
    stderr: trim(output.stderr),
  });
  if (output.status !== 0) {
    throw classified(failureClass, `planr command failed: ${args.join(" ")}`, {
      command: [planrBin, ...fullArgs].join(" "),
      status: output.status,
      stderr: trim(output.stderr),
    });
  }
  try {
    return JSON.parse(output.stdout);
  } catch (error) {
    throw instrumentation(`planr command did not emit JSON: ${args.join(" ")}`, { parse_error: error.message });
  }
}

function observeStaticContract(config, freshRoot, planrBin) {
  const expected = config.fixed_contract;
  if (!expected || typeof expected !== "object") {
    throw admission("AC-014 fixed_contract must declare the expected observed identity");
  }
  const oracleCommand = requiredCommand(config.oracle_command, "oracle_command");
  const observed = {
    candidate_sha: gitHead(freshRoot),
    candidate_version: observedPlanrVersion(planrBin),
    candidate_binary_sha256: sha256File(planrBin),
    prompt_digest: sha256File(resolveFreshPath(requiredString(config.prompt_path, "prompt_path"), freshRoot, "prompt_path", true)),
    spec_digest: sha256File(resolveFreshPath(requiredString(config.spec_path, "spec_path"), freshRoot, "spec_path", true)),
    oracle_id: config.oracle_id ?? null,
    oracle_sha256: sha256File(path.resolve(oracleCommand[0])),
  };
  for (const [key, value] of Object.entries(observed)) {
    if (!value) {
      throw admission(`AC-014 observed identity is missing: ${key}`);
    }
    if (expected[key] !== value) {
      throw admission(`AC-014 observed identity mismatch: ${key}`);
    }
  }
  return observed;
}

function observeEffectiveContract(config, staticContract, sessions) {
  const expected = config.fixed_contract;
  const root = sessions.find((session) => !session.parent);
  const observed = {
    ...staticContract,
    model: root?.turn_context?.model ?? null,
    effort: root?.turn_context?.effort ?? null,
    surface: root?.surface ?? null,
    cli_version: root?.cli_version ?? null,
  };
  for (const [key, value] of Object.entries(observed)) {
    if (!value) {
      throw admission(`AC-014 observed identity is missing: ${key}`);
    }
    if (expected[key] !== value) {
      throw admission(`AC-014 observed identity mismatch: ${key}`);
    }
  }
  for (const [key, value] of Object.entries(EXPECTED)) {
    if (key === "ceilings") continue;
    if (observed[key] !== value) {
      throw admission(`AC-014 fixed runner identity mismatch: ${key}`);
    }
  }
  return observed;
}

function gitHead(root) {
  const output = spawnSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" });
  if (output.status !== 0) {
    throw admission(`fresh_root git HEAD is not observable: ${trim(output.stderr)}`);
  }
  return output.stdout.trim();
}

function observedPlanrVersion(planrBin) {
  const output = spawnSync(planrBin, ["--version"], { encoding: "utf8" });
  if (output.status !== 0) {
    throw admission(`planr --version failed: ${trim(output.stderr)}`);
  }
  return output.stdout.trim().replace(/^planr\s+/, "");
}

function sha256File(file) {
  return `sha256:${createHash("sha256").update(readFileSync(file)).digest("hex")}`;
}

function classifiedResult(error, commands) {
  const details = error.details ?? {};
  return {
    schema_version: "planr.ac014.fresh_arm_result.v1",
    status: "failed",
    failure_class: error.failure_class ?? "instrumentation",
    retry: false,
    error: error.message,
    ...details,
    commands,
  };
}

function writeFailureArtifacts(config, error, commands) {
  if (!config || typeof config !== "object" || typeof config.fresh_root !== "string") {
    return;
  }
  const freshRoot = path.resolve(config.fresh_root);
  const rejected = error.details?.rejected_fresh_root === freshRoot;
  const collision = /immutable artifact already exists/.test(error.message ?? "");
  const artifactRoot = config.failure_artifact_root
    ? path.resolve(config.failure_artifact_root)
    : rejected || collision
      ? path.join(path.dirname(freshRoot), ".planr-ac014-failures", path.basename(freshRoot))
      : freshRoot;
  const artifactDir = path.join(artifactRoot, config.artifact_dir ?? ".planr/artifacts/ac014");
  const files = ["PREFLIGHT.json", "BENCHMARK_RESULT.json", "monitor-status.json"].map((name) => path.join(artifactDir, name));
  if (files.every((file) => existsSync(file))) {
    return;
  }
  try {
    const result = classifiedResult(error, commands);
    writeJsonSetImmutable(new Map([
      [files[0], {
        schema_version: "planr.ac014.preflight.v1",
        status: "external_invalid",
        verdict: "failed",
        stop_reason: result.failure_class,
        retry: false,
        expected_contract: EXPECTED,
        observed_contract: result.observed_contract ?? null,
        relocation: null,
        validation: null,
        evidence_migration: null,
      }],
      [files[1], {
        ...result,
        schema_version: "planr.ac014.benchmark_result.v1",
        status: "external_invalid",
        verdict: "failed",
        stop_reason: result.failure_class,
      }],
      [files[2], {
        schema_version: "planr.ac014.monitor_status.v1",
        status: "external_invalid",
        verdict: "failed",
        stop_reason: result.failure_class,
        retry: false,
        sessions: result.sessions ?? [],
        ceilings: result.ceilings ?? null,
        oracle: result.oracle ?? null,
      }],
    ]));
  } catch {
    // The primary result file remains the authoritative failure if artifact admission itself fails.
  }
}

function admission(message, details) {
  return classified("admission", message, details);
}

function instrumentation(message, details) {
  return classified("instrumentation", message, details);
}

function product(message, details) {
  return classified("product", message, details);
}

function classified(failureClass, message, details = {}) {
  const error = new Error(message);
  error.failure_class = failureClass;
  error.details = details;
  return error;
}

function requireSchema(config) {
  if (config.schema_version !== "planr.ac014.fresh_arm_run.v1") {
    throw admission("schema_version must be planr.ac014.fresh_arm_run.v1");
  }
}

function canonicalExistingDir(value, label) {
  const resolved = path.resolve(requiredString(value, label));
  try {
    if (!statSync(resolved).isDirectory()) {
      throw new Error("not a directory");
    }
    return realpathSync(resolved);
  } catch {
    throw admission(`${label} must be an existing directory: ${resolved}`);
  }
}

function assertExecutable(file) {
  try {
    statSync(file);
    accessSync(file, constants.X_OK);
  } catch {
    throw admission(`planr_bin is not executable: ${file}`);
  }
}

function assertLocalPath(candidate, root, label) {
  if (!path.isAbsolute(candidate)) {
    throw instrumentation(`${label} path is not absolute: ${candidate}`);
  }
  if (!existsSync(candidate)) {
    throw instrumentation(`${label} path does not exist: ${candidate}`);
  }
  const real = realpathSync(candidate);
  if (!isPathInside(real, root) && real !== root) {
    throw instrumentation(`${label} path escapes fresh root: ${candidate}`);
  }
}

function resolveFreshPath(value, root, label, mustExist) {
  const resolved = path.isAbsolute(value) ? value : path.join(root, value);
  if (!isPathInside(resolved, root) && resolved !== root) {
    throw admission(`${label} escapes fresh root: ${value}`);
  }
  if (mustExist && !existsSync(resolved)) {
    throw admission(`${label} does not exist: ${resolved}`);
  }
  return resolved;
}

function readOptionalJson(value, root) {
  if (!value) {
    return {};
  }
  return readJson(resolveFreshPath(value, root, "metrics_path", true));
}

function summarizeJson(value) {
  return {
    mode: value?.mode ?? null,
    project_id: value?.project?.id ?? value?.relocation?.project?.id ?? null,
    plan_count: value?.plans?.length ?? value?.relocation?.plans?.length ?? null,
    item_count: value?.items?.length ?? value?.relocation?.items?.length ?? null,
    status: value?.status ?? null,
  };
}

function readJson(file) {
  return JSON.parse(readFileSync(path.resolve(file), "utf8"));
}

function requiredString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    throw admission(`${label} must be a non-empty string`);
  }
  return value;
}

function requiredCommand(value, label) {
  if (!Array.isArray(value) || value.length === 0 || value.some((part) => typeof part !== "string" || part.length === 0)) {
    throw admission(`${label} must be a non-empty string array`);
  }
  return value;
}

function waitForChild(child) {
  return new Promise((resolve) => {
    child.on("close", (code, signal) => resolve({ code, signal }));
  });
}

function sleepSync(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

function terminateChild(child) {
  child.kill("SIGTERM");
  setTimeout(() => {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL");
    }
  }, 1000).unref();
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function isPathInside(candidate, root) {
  const relative = path.relative(root, candidate);
  return Boolean(relative) && !relative.startsWith("..") && !path.isAbsolute(relative);
}

function trim(value) {
  return (value ?? "").trim().slice(0, 1000);
}

function valueAfter(flag) {
  const index = argv.indexOf(flag);
  return index === -1 ? null : argv[index + 1];
}
