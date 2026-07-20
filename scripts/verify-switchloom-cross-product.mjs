#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir, platform, arch } from "node:os";
import { basename, dirname, join, resolve } from "node:path";

const EXPECTED = {
  packageName: "switchloom",
  version: "0.2.1",
  integrity: "sha512-vUKHxYXHt7Sx7MkYQz5MRZ0Ll544iHoadHGCgvJPUYkpUzQWtzjt1o3xhyeQwExCA6tuLQ5vZnLPz+fO5uMiXg==",
  shasum: "e813283f54d0d64b5fd4835e17687aaaf3b0a6cb",
  bundleId: "balanced-codex-openai@1.0.0+2.0.0",
};

const root = resolve(new URL("..", import.meta.url).pathname);
const planrBin = resolve(process.env.PLANR_BIN || join(root, "target/debug/planr"));
const tempParent = resolve(process.env.PLANR_ORACLE_TEMP_PARENT || "/private/tmp");
const replayRoot = process.env.PLANR_ORACLE_REPLAY_ROOT ? resolve(process.env.PLANR_ORACLE_REPLAY_ROOT) : null;
if (replayRoot) {
  assertReplayRoot(replayRoot);
} else {
  mkdirSync(tempParent, { recursive: true });
}
const tempRoot = replayRoot || mkdtempSync(join(tempParent, "planr-switchloom-cross-product-"));
const receipts = [];

function assertReplayRoot(path) {
  assertOk(
    basename(path).startsWith("planr-switchloom-cross-product-") && existsSync(path),
    "PLANR_ORACLE_REPLAY_ROOT must point at a retained switchloom cross-product root",
    { path },
  );
}

function receipt(step, detail = {}) {
  receipts.push({ step, ...detail });
  process.stderr.write(`[ok] ${step}\n`);
}

function fail(message, detail = {}) {
  const error = new Error(message);
  error.detail = detail;
  throw error;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd || root,
    env: { ...process.env, ...(options.env || {}) },
    encoding: "utf8",
    timeout: options.timeoutMs || 120_000,
    maxBuffer: options.maxBuffer || 20 * 1024 * 1024,
  });
  const record = {
    command: [command, ...args].join(" "),
    cwd: options.cwd || root,
    status: result.status,
    signal: result.signal,
    timedOut: result.error?.code === "ETIMEDOUT",
    stdout: result.stdout || "",
    stderr: result.stderr || "",
  };
  if (options.allowFailure) {
    return record;
  }
  if (result.error) {
    if (result.error.code === "ETIMEDOUT") {
      fail(`command timed out: ${record.command}`, record);
    }
    fail(`command failed to start: ${record.command}`, { ...record, error: result.error.message });
  }
  if (result.status !== 0) {
    fail(`command failed: ${record.command}`, record);
  }
  return record;
}

function parseJson(output, label) {
  try {
    return JSON.parse(output);
  } catch (error) {
    fail(`${label} was not JSON`, { error: error.message, output });
  }
}

function assertEqual(actual, expected, label) {
  if (actual !== expected) {
    fail(`${label} mismatch`, { actual, expected });
  }
}

function assertArrayEqual(actual, expected, label) {
  assertEqual(JSON.stringify(actual), JSON.stringify(expected), label);
}

function assertOk(condition, message, detail = {}) {
  if (!condition) {
    fail(message, detail);
  }
}

function hashFile(path, algorithm) {
  return createHash(algorithm).update(readFileSync(path)).digest("hex");
}

function hashText(text, algorithm = "sha256") {
  return createHash(algorithm).update(text).digest("hex");
}

function integrity(path) {
  return `sha512-${createHash("sha512").update(readFileSync(path)).digest("base64")}`;
}

const USER_CONFIG_SENTINELS = [
  ".codex/config.toml",
  ".claude/settings.json",
  ".claude.json",
  ".cursor/mcp.json",
];

const EXTRA_PROTECTED_WRITE_FILES = [
  ".zshrc",
  ".zprofile",
  ".bashrc",
  ".bash_profile",
  ".profile",
  ".gitconfig",
  ".git-credentials",
  ".codex/auth.json",
  ".netrc",
  ".npmrc",
];

const EXTRA_PROTECTED_WRITE_DIRS = [
  ".ssh",
  ".config",
  ".codex/plugins",
  ".codex/.tmp/plugins",
  ".codex/skills",
  ".codex/agents",
  ".codex/rules",
  ".codex/hooks",
  ".codex/prompts",
  ".codex/personalities",
  "Library/Keychains",
];

function snapshotUserConfigSentinels() {
  return USER_CONFIG_SENTINELS.map((relativePath) => snapshotFileMetadata(join(homedir(), relativePath), relativePath));
}

function snapshotFileMetadata(path, relativePath = path) {
  if (!existsSync(path)) {
    return { path, relativePath, exists: false };
  }
  const stat = statSync(path);
  return {
    path,
    relativePath,
    exists: true,
    mode: stat.mode & 0o777,
    size: stat.size,
    mtimeMs: stat.mtimeMs,
    sha256: hashFile(path, "sha256"),
  };
}

function assertUserConfigSentinelsUnchanged(before) {
  const after = snapshotUserConfigSentinels();
  assertEqual(JSON.stringify(after), JSON.stringify(before), "user/global config sentinel metadata");
  receipt("user/global config sentinels unchanged", {
    sentinels: after.map((entry) => ({
      path: entry.path,
      exists: entry.exists,
      mode: entry.mode,
      size: entry.size,
      mtimeMs: entry.mtimeMs,
      sha256: entry.sha256,
    })),
  });
}

function switchloomSourceRoot() {
  return resolve(process.env.SWITCHLOOM_SOURCE_ROOT || "/Users/kregenrek/projects/model-routing");
}

function snapshotSwitchloomSourceWorktree() {
  const sourceRoot = switchloomSourceRoot();
  if (!existsSync(sourceRoot)) {
    return { sourceRoot, exists: false };
  }
  const head = run("git", ["rev-parse", "HEAD"], { cwd: sourceRoot }).stdout.trim();
  const status = run("git", ["status", "--short"], { cwd: sourceRoot }).stdout;
  const files = run("git", ["ls-files"], { cwd: sourceRoot }).stdout;
  return {
    sourceRoot,
    exists: true,
    head,
    statusSha256: hashText(status),
    statusLines: status.split(/\n/).filter(Boolean),
    trackedFilesSha256: hashText(files),
    trackedFileCount: files.split(/\n/).filter(Boolean).length,
  };
}

function assertSwitchloomSourceWorktreeUnchanged(before) {
  const after = snapshotSwitchloomSourceWorktree();
  assertEqual(JSON.stringify(after), JSON.stringify(before), "Switchloom source worktree inventory");
  receipt("Switchloom source worktree inventory unchanged", after);
}

function nativeTarget() {
  const osName = { darwin: "darwin", linux: "linux" }[platform()];
  const cpu = { arm64: "arm64", x64: "x86_64" }[arch()];
  assertOk(osName && cpu, "unsupported platform for Switchloom native binary", { platform: platform(), arch: arch() });
  return `${osName}-${cpu}`;
}

function packageTarball() {
  if (process.env.SWITCHLOOM_TARBALL) {
    const path = resolve(process.env.SWITCHLOOM_TARBALL);
    assertOk(existsSync(path), "SWITCHLOOM_TARBALL does not exist", { path });
    receipt("using supplied published tarball", { path });
    return path;
  }

  const view = parseJson(
    run("npm", ["view", `${EXPECTED.packageName}@${EXPECTED.version}`, "name", "version", "dist.integrity", "dist.shasum", "dist.tarball", "--json"]).stdout,
    "npm view",
  );
  assertEqual(view.name, EXPECTED.packageName, "npm package name");
  assertEqual(view.version, EXPECTED.version, "npm package version");
  assertEqual(view["dist.integrity"], EXPECTED.integrity, "npm dist.integrity");
  assertEqual(view["dist.shasum"], EXPECTED.shasum, "npm dist.shasum");

  const pack = parseJson(
    run("npm", ["pack", `${EXPECTED.packageName}@${EXPECTED.version}`, "--pack-destination", tempRoot, "--json"]).stdout,
    "npm pack",
  );
  const item = Array.isArray(pack) ? pack[0] : pack;
  assertEqual(item.name, EXPECTED.packageName, "packed package name");
  assertEqual(item.version, EXPECTED.version, "packed package version");
  assertEqual(item.integrity, EXPECTED.integrity, "packed integrity");
  assertEqual(item.shasum, EXPECTED.shasum, "packed shasum");
  const path = join(tempRoot, item.filename || `${EXPECTED.packageName}-${EXPECTED.version}.tgz`);
  assertOk(existsSync(path), "npm pack tarball missing", { path, item });
  receipt("published npm artifact resolved", { tarball: path, integrity: item.integrity, shasum: item.shasum });
  return path;
}

function retainedPublicTarball() {
  const candidates = [];
  if (process.env.SWITCHLOOM_TARBALL) {
    candidates.push(resolve(process.env.SWITCHLOOM_TARBALL));
  }
  candidates.push(join(tempRoot, `${EXPECTED.packageName}-${EXPECTED.version}.tgz`));
  const tarball = candidates.find((path) => existsSync(path));
  assertOk(tarball, "retained replay public tarball missing", { candidates });
  receipt("retained public tarball resolved", { tarball });
  return tarball;
}

function extractTarball(tarball) {
  assertEqual(hashFile(tarball, "sha1"), EXPECTED.shasum, "tarball sha1");
  assertEqual(integrity(tarball), EXPECTED.integrity, "tarball sha512 integrity");

  const listing = run("tar", ["-tzf", tarball]).stdout.trim().split(/\n+/);
  for (const path of [
    "package/package.json",
    "package/npm/bin/model-routing.js",
    `package/npm/native/${nativeTarget()}/model-routing`,
    "package/docs/migration-manifest.tsv",
  ]) {
    assertOk(listing.includes(path), "published tarball missing required file", { path });
  }

  const extractDir = join(tempRoot, "switchloom-package");
  mkdirSync(extractDir, { recursive: true });
  run("tar", ["-xzf", tarball, "-C", extractDir]);
  const packageJson = parseJson(readFileSync(join(extractDir, "package/package.json"), "utf8"), "package.json");
  assertEqual(packageJson.name, EXPECTED.packageName, "tar package name");
  assertEqual(packageJson.version, EXPECTED.version, "tar package version");
  assertOk(packageJson.bin?.switchloom === "npm/bin/model-routing.js", "missing switchloom bin alias", { bin: packageJson.bin });
  assertOk(packageJson.bin?.["model-routing"] === "npm/bin/model-routing.js", "missing compatibility bin alias", { bin: packageJson.bin });
  const bin = join(extractDir, "package/npm/native", nativeTarget(), "model-routing");
  assertOk(existsSync(bin), "native Switchloom binary missing", { bin });
  receipt("published tarball verified and extracted", { bin, tarball: basename(tarball) });
  return bin;
}

function createFreshRepo() {
  const repo = resolve(process.env.PLANR_ORACLE_REPO || join(tempRoot, "fresh-repo"));
  if (process.env.PLANR_ORACLE_REPO) {
    assertOk(
      repo.includes("planr-switchloom-cross-product-") && basename(repo) === "fresh-repo",
      "PLANR_ORACLE_REPO must point at a controlled switchloom cross-product fresh-repo path",
      { repo },
    );
    rmSync(repo, { recursive: true, force: true });
  }
  mkdirSync(repo, { recursive: true });
  run("git", ["init"], { cwd: repo });
  writeFileSync(join(repo, "README.md"), "# Cross Product Oracle\n");
  return repo;
}

function provisionRepoLocalSkill(repo, skillName) {
  const source = join(root, "plugins/planr/skills", skillName, "SKILL.md");
  const target = join(repo, ".codex/skills", skillName, "SKILL.md");
  assertOk(existsSync(source), `canonical ${skillName} skill missing`, { source });
  mkdirSync(dirname(target), { recursive: true });
  copyFileSync(source, target);
  receipt(`repo-local ${skillName} skill provisioned`, { source, target });
  return { source, target };
}

function provisionRepoLocalPlanrSkills(repo) {
  const loop = provisionRepoLocalSkill(repo, "planr-loop");
  provisionRepoLocalSkill(repo, "planr-work");
  provisionRepoLocalSkill(repo, "planr-review");
  const sourceText = readFileSync(loop.source, "utf8");
  assertOk(
    !sourceText.includes("dispatch through the routing skill"),
    "canonical repo planr-loop skill still contains stale routing-skill instruction",
    { source: loop.source },
  );
  assertOk(
    sourceText.includes("Pick packets expose provider-neutral `routing.profile`; they do not expose a host-owned `routing.agent_type`"),
    "canonical repo planr-loop skill does not state the neutral routing.profile handoff contract",
    { source: loop.source },
  );
  assertOk(
    sourceText.includes("dispatch that profile identifier as the host-native role/`agent_type`"),
    "canonical repo planr-loop skill does not map matching external profile ids to native agent_type",
    { source: loop.source },
  );
  assertOk(
    sourceText.includes("The `spawn_agent` tool call itself must include `agent_type` set exactly to the matching `routing.profile`"),
    "canonical repo planr-loop skill does not require Codex spawn_agent args to carry native agent_type",
    { source: loop.source },
  );
  assertOk(
    sourceText.includes("If no matching repository role exists, keep the host's default dispatch contract"),
    "canonical repo planr-loop skill does not preserve default dispatch without a matching role",
    { source: loop.source },
  );
}

function createPlanrLoopContract(repo) {
  const project = parseJson(stripPlanrNoise(run(planrBin, ["--json", "project", "init", "Host Oracle Project", "--client", "codex"], { cwd: repo }).stdout), "planr project init");
  const plan = parseJson(stripPlanrNoise(run(planrBin, ["--json", "plan", "new", "Host oracle loop", "--platform", "cli"], { cwd: repo }).stdout), "planr plan new").plan;
  const map = parseJson(stripPlanrNoise(run(planrBin, ["--json", "map", "build", "--from", plan.id], { cwd: repo }).stdout), "planr map build");
  assertEqual(map.created.length, 1, "minimal Planr loop item count");
  const created = map.created[0];
  const itemDescription = [
    "Tiny CLI/README smoke for the cross-product oracle.",
    `Use $planr-work on this exact item ${created.id}.`,
    "Append one short evidence line to README.md.",
    "Run the replayable smoke command: grep -q \"Planr loop oracle smoke\" README.md.",
    `Before planr done/review, record live verification with: planr log add --item ${created.id} --kind verification --summary \"live verification on cli: README smoke line found\" --cmd \"grep -q 'Planr loop oracle smoke' README.md\".`,
    "Then run planr done with --review and stop after requesting review.",
  ].join(" ");
  const item = parseJson(stripPlanrNoise(run(planrBin, [
    "--json",
    "item",
    "update",
    created.id,
    "--description",
    itemDescription,
    "--work-type",
    "code",
  ], { cwd: repo }).stdout), "planr item update").item;
  assertEqual(item.plan_path, plan.path, "minimal Planr item plan path");
  assertEqual(item.status, "ready", "minimal Planr item status");
  parseJson(stripPlanrNoise(run(planrBin, [
    "--json",
    "context",
    "add",
    `GOAL CONTRACT ${plan.id}: DONE when every in-scope map item is closed with log evidence, all reviews are closed with verdict complete, no open approvals remain, and a replayable --kind verification log exists for item ${item.id}. Iteration budget: 1. Driver entry is exactly $planr-loop. The driver must not implement directly. The generated .codex/config.toml roles are available to this Codex run, so routing.profile is not advisory and the driver must not claim role binding is unavailable. A default-role maker/checker spawn without agent_type is invalid and must not be waited on. The driver must dispatch maker with a spawn_agent tool call whose arguments include agent_type exactly model_routing_terra_high, fork_turns exactly none, task_name exactly maker, and message exactly "Use $planr-work on item ${item.id}. Stop after requesting review."; then wait for it. The driver must then dispatch reviewer with a spawn_agent tool call whose arguments include agent_type exactly model_routing_sol_high, fork_turns exactly none, task_name exactly reviewer, and message exactly "Use $planr-review on item ${item.id}. Close the review with a verdict."; then wait for it. Maker must use $planr-work through the native role, add the README smoke evidence, run grep, log --kind verification, then done --review. Reviewer must use $planr-review through the native role and close the review with a verdict.`,
    "--item",
    item.id,
    "--tag",
    "goal-contract",
  ], { cwd: repo }).stdout), "planr context add goal-contract");
  receipt("fresh Planr loop contract created", { project: project.project.id, plan: plan.id, item: item.id });
  return { project: project.project, plan, item };
}

function compileAndApply(switchloomBin, repo) {
  const bundle = join(tempRoot, "balanced-planr-codex.json");
  run(switchloomBin, ["compile", "balanced", "--host", "codex-openai", "--integration", "planr", "--output", bundle]);
  const inspect = parseJson(run(switchloomBin, ["inspect", bundle]).stdout, "switchloom inspect");
  assertEqual(inspect.integration, "planr", "bundle integration");
  assertEqual(inspect.valid, true, "bundle validity");
  assertEqual(inspect.artifact_count, 9, "bundle artifact count");

  const apply = parseJson(run(switchloomBin, ["apply", bundle, "--repository", repo, "--yes"]).stdout, "switchloom apply");
  assertEqual(apply.bundle_id, EXPECTED.bundleId, "applied bundle id");
  const paths = apply.artifacts.map((artifact) => artifact.path).sort();
  for (const path of [
    ".planr/agents.toml",
    ".planr/policy.toml",
    ".codex/config.toml",
    ".codex/agents/model-routing-terra-high.toml",
    ".codex/agents/model-routing-sol-high.toml",
  ]) {
    assertOk(paths.includes(path), "apply did not emit required artifact", { path, paths });
    assertOk(existsSync(join(repo, path)), "applied artifact missing on disk", { path });
  }
  receipt("external apply emitted repo-local Planr and Codex declarations", { repo, artifactCount: paths.length });
  return { bundle, appliedArtifacts: paths };
}

function assertPlanrConsumes(repo) {
  assertOk(existsSync(planrBin), "cleaned Planr binary missing; run cargo build --bin planr", { planrBin });
  const agents = parseJson(stripPlanrNoise(run(planrBin, ["--json", "agents", "list"], { cwd: repo }).stdout), "planr agents list");
  const policy = parseJson(stripPlanrNoise(run(planrBin, ["--json", "policy", "check"], { cwd: repo }).stdout), "planr policy check");
  assertOk(agents.registry.profiles.model_routing_terra_high, "Planr did not consume maker profile");
  assertOk(agents.registry.profiles.model_routing_sol_high, "Planr did not consume reviewer profile");
  assertEqual(agents.registry.profiles.model_routing_terra_high.model, "gpt-5.6-terra", "maker requested model");
  assertEqual(agents.registry.profiles.model_routing_terra_high.effort, "high", "maker requested effort");
  assertEqual(policy.ok, true, "Planr policy check");
  receipt("Planr consumes external declarations", { profiles: Object.keys(agents.registry.profiles).length, policy: policy.policy_id });
}

function stripPlanrNoise(stdout) {
  return stdout.replace(/^Not privileged.*\n/gm, "");
}

function codexProjectTrustOverride(repo) {
  const escapedRepo = repo.replace(/\\/g, "\\\\").replace(/"/g, "\\\"");
  return `projects."${escapedRepo}".trust_level="trusted"`;
}

function sandboxRegexEscape(path) {
  return path.replace(/[|\\{}()[\]^$+*?.]/g, "\\$&");
}

function sandboxString(path) {
  return path.replace(/\\/g, "\\\\").replace(/"/g, "\\\"");
}

function protectedUserConfigEntries() {
  return [...USER_CONFIG_SENTINELS, ...EXTRA_PROTECTED_WRITE_FILES].flatMap((relativePath) => {
    const path = join(homedir(), relativePath);
    const dir = dirname(path);
    const base = basename(path);
    const dotTemp = join(dir, `.${base}.tmp`);
    return [
      { kind: "literal", path },
      { kind: "literal", path: `${path}.tmp` },
      { kind: "literal", path: dotTemp },
      { kind: "regex", path: `^${sandboxRegexEscape(path)}[.].*` },
      { kind: "regex", path: `^${sandboxRegexEscape(join(dir, `.${base}`))}[.].*` },
    ];
  });
}

function protectedWriteEntries() {
  return [
    ...protectedUserConfigEntries(),
    ...EXTRA_PROTECTED_WRITE_DIRS.map((relativePath) => ({
      kind: "subpath",
      path: join(homedir(), relativePath),
    })),
    { kind: "subpath", path: switchloomSourceRoot() },
  ];
}

function codexOuterSandboxProfile(repo) {
  const profilePath = join(tempRoot, "codex-live-no-global-config-write.sb");
  const protectedEntries = protectedWriteEntries();
  const protectedPaths = [...new Set(protectedEntries
    .filter((entry) => entry.kind !== "regex")
    .map((entry) => entry.path))];
  const denyEntries = protectedEntries.map((entry) => {
    if (entry.kind === "subpath") {
      return `  (subpath "${sandboxString(entry.path)}")`;
    }
    return (
      entry.kind === "regex"
        ? `  (regex #"${sandboxString(entry.path)}")`
        : `  (literal "${sandboxString(entry.path)}")`
    );
  });
  writeFileSync(profilePath, [
    "(version 1)",
    "(allow default)",
    "(allow file-write*",
    `  (subpath "${sandboxString(repo)}")`,
    ")",
    "(deny file-write*",
    ...denyEntries,
    ")",
    "",
  ].join("\n"));
  return {
    allowedRepo: repo,
    externallySandboxed: true,
    profilePath,
    protectedPaths,
  };
}

function runCodexLiveCommand(args, options) {
  if (platform() !== "darwin") {
    return {
      result: run("codex", args, options),
      sandbox: {
        allowedRepo: options.cwd,
        externallySandboxed: false,
        profilePath: null,
        protectedPaths: [],
      },
    };
  }
  const sandbox = options.sandbox || codexOuterSandboxProfile(options.cwd || root);
  return {
    result: run("/usr/bin/sandbox-exec", ["-f", sandbox.profilePath, "codex", ...args], options),
    sandbox,
  };
}

const CODEX_MULTI_AGENT_V2_METADATA_CONFIG = [
  "--config",
  "features.multi_agent_v2.hide_spawn_agent_metadata=false",
  "--config",
  "features.multi_agent_v2.tool_namespace=\"planr_agents\"",
];

function recordCodexHostDiagnostics(repo, sandbox = null) {
  const version = run("codex", ["--version"], { cwd: repo }).stdout.trim();
  const features = run("codex", [
    ...CODEX_MULTI_AGENT_V2_METADATA_CONFIG,
    "features",
    "list",
  ], { cwd: repo }).stdout;
  const multiAgentV2 = features
    .split(/\n/)
    .map((line) => line.trim().split(/\s+/))
    .find((columns) => columns[0] === "multi_agent_v2");
  receipt("Codex host diagnostics captured without forcing multi_agent_v2", {
    version,
    externally_sandboxed: sandbox?.externallySandboxed ?? false,
    denied_protected_paths: sandbox?.protectedPaths || [],
    allowed_repo: sandbox?.allowedRepo || repo,
    multi_agent_v2: {
      stage: multiAgentV2?.slice(1, -1).join(" ") || null,
      enabled: multiAgentV2?.at(-1) || null,
    },
    config: CODEX_MULTI_AGENT_V2_METADATA_CONFIG.filter((_, index) => index % 2 === 1),
  });
}

function requestedOnlyRouteAuditPayload() {
  return {
    requested: {
      profile: "model_routing_terra_high",
      role: "model_routing_terra_high",
      client: "codex",
      agent_type: { value: "model_routing_terra_high", enforcement: "requested_only", evidence: "binding" },
      model: { value: "gpt-5.6-terra", enforcement: "requested_only", evidence: "policy" },
      effort: { value: "high", enforcement: "requested_only", evidence: "policy" },
      context_fork: { value: { mode: "none" }, enforcement: "requested_only", evidence: "policy" },
    },
    resolved: {
      profile: "model_routing_terra_high",
      role: "model_routing_terra_high",
      client: "codex",
      agent_type: { value: "model_routing_terra_high", enforcement: "verified", evidence: "binding" },
      model: { value: "gpt-5.6-terra", enforcement: "verified", evidence: "binding" },
      effort: { value: "high", enforcement: "verified", evidence: "binding" },
      context_fork: { value: { mode: "none" }, enforcement: "verified", evidence: "binding" },
    },
    effective: {
      profile: "model_routing_terra_high",
      role: "model_routing_terra_high",
      client: "codex",
      agent_type: { value: "model_routing_terra_high", enforcement: "requested_only", evidence: "binding" },
      model: { value: "gpt-5.6-terra", enforcement: "requested_only", evidence: "policy" },
      effort: { value: "high", enforcement: "requested_only", evidence: "policy" },
      context_fork: { value: { mode: "none" }, enforcement: "requested_only", evidence: "policy" },
    },
    transition: { kind: "initial", reason: "requested-only negative test", evidence: ["policy"] },
    policy: { id: "balanced", version: "1.0.0" },
    binding: { id: "codex-openai", version: "2.0.0" },
    metering: {
      wall_time_seconds: { value: 1, confidence: "trusted" },
      tool_calls: { value: 1, confidence: "trusted" },
      tokens: { value: null, confidence: "unavailable" },
      credits_micros: { value: null, confidence: "unavailable" },
    },
  };
}

function assertRequestedOnlyAuditShape(routeAudit) {
  assertOk(existsSync(routeAudit), "requested-only route audit missing", { routeAudit });
  const audit = parseJson(readFileSync(routeAudit, "utf8"), "requested-only route audit");
  for (const key of ["agent_type", "model", "effort", "context_fork"]) {
    assertEqual(
      audit.effective?.[key]?.enforcement,
      "requested_only",
      `requested-only audit effective ${key} enforcement`,
    );
  }
  return audit;
}

function assertRequestedOnlyRejectedFromAudit(routeAudit) {
  const itemId = process.env.PLANR_ROUTE_AUDIT_ITEM || "i-prove-published-cross-product-ro-e103";
  assertRequestedOnlyAuditShape(routeAudit);
  const result = run(planrBin, ["log", "add", "--item", itemId, "--summary", "negative", "--route-audit", routeAudit], {
    cwd: root,
    allowFailure: true,
  });
  assertOk(result.status !== 0, "requested-only route audit unexpectedly accepted", result);
  assertOk(
    `${result.stdout}\n${result.stderr}`.includes("cannot use requested_only as effective execution proof"),
    "requested-only rejection did not cite effective proof",
    result,
  );
  receipt("requested-only metadata rejected as effective proof", { itemId, routeAudit });
}

function assertRequestedOnlyRejected() {
  const routeAudit = join(tempRoot, "requested-only-route-audit.json");
  writeFileSync(routeAudit, JSON.stringify({
    ...requestedOnlyRouteAuditPayload(),
  }, null, 2));
  assertRequestedOnlyRejectedFromAudit(routeAudit);
}

function runCodexNoAuth(repo) {
  const noauthHome = join(tempRoot, "noauth-home");
  const noauthCodex = join(tempRoot, "noauth-codex");
  const evidencePath = join(tempRoot, "codex-noauth-result.json");
  mkdirSync(noauthHome, { recursive: true });
  mkdirSync(noauthCodex, { recursive: true });
  const result = run("codex", ["exec", "--json", "--ephemeral", "--skip-git-repo-check", "Return noauth-ok"], {
    cwd: repo,
    env: { HOME: noauthHome, CODEX_HOME: noauthCodex },
    allowFailure: true,
    timeoutMs: 45_000,
    maxBuffer: 20 * 1024 * 1024,
  });
  const authError = classifyCodexAuthError(result);
  writeFileSync(evidencePath, JSON.stringify(noAuthEvidence(result, authError), null, 2));
  assertOk(result.status !== 0, "missing-auth Codex run unexpectedly succeeded", result);
  assertOk(authError, "missing-auth Codex run did not fail with authentication evidence", result);
  receipt("missing Codex authentication fails closed", {
    evidencePath,
    evidenceSha256: hashFile(evidencePath, "sha256"),
    authError,
  });
}

function classifyCodexAuthError(result) {
  const output = `${result.stdout}\n${result.stderr}`;
  if (output.includes("401 Unauthorized")) {
    return "401 Unauthorized";
  }
  if (output.includes("Missing bearer or basic authentication")) {
    return "Missing bearer or basic authentication";
  }
  return null;
}

function noAuthEvidence(result, authError) {
  return {
    command: result.command,
    cwd: result.cwd,
    status: result.status,
    signal: result.signal,
    timedOut: result.timedOut,
    authError,
    stdoutSha256: hashText(result.stdout),
    stderrSha256: hashText(result.stderr),
    stdoutTail: result.stdout.slice(-2000),
    stderrTail: result.stderr.slice(-2000),
  };
}

function listRolloutFiles(dir = join(homedir(), ".codex/sessions"), files = []) {
  if (!existsSync(dir)) {
    return files;
  }
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      listRolloutFiles(path, files);
    } else if (entry.isFile() && entry.name.endsWith(".jsonl")) {
      const stat = statSync(path);
      files.push({ path, size: stat.size, mtimeMs: stat.mtimeMs });
    }
  }
  return files;
}

function rolloutFilesForReplay(publicParentThreadId, outputPath) {
  const liveStat = statSync(outputPath);
  const windowMs = 60 * 60 * 1000;
  const matching = listRolloutFiles().filter((file) => {
    if (Math.abs(file.mtimeMs - liveStat.mtimeMs) > windowMs || file.size > 10 * 1024 * 1024) {
      return false;
    }
    const text = readFileSync(file.path, "utf8");
    return text.includes(publicParentThreadId)
      || text.includes("model_routing_terra_high")
      || text.includes("model_routing_sol_high");
  });
  assertOk(matching.length > 0, "replay could not find persisted rollout files", { publicParentThreadId });
  return matching;
}

function snapshotRollouts() {
  return new Map(listRolloutFiles().map((file) => [file.path, file]));
}

function changedRollouts(before) {
  return listRolloutFiles().filter((file) => {
    const previous = before.get(file.path);
    return !previous || previous.size !== file.size || previous.mtimeMs !== file.mtimeMs;
  });
}

function parseJsonlEvents(stdout, label) {
  const events = [];
  for (const line of stdout.split(/\n+/).filter(Boolean)) {
    const trimmed = line.trim();
    if (!trimmed.startsWith("{")) {
      continue;
    }
    try {
      events.push(JSON.parse(trimmed));
    } catch (error) {
      fail(`${label} contained invalid JSONL`, { error: error.message, line });
    }
  }
  return events;
}

function parseRollout(path) {
  return { path, events: parseJsonlEvents(readFileSync(path, "utf8"), path) };
}

function payloadOf(event) {
  return event?.payload || event?.item || event || {};
}

function sessionMeta(rollout) {
  return rollout.events.find((event) => event?.type === "session_meta")?.payload || {};
}

function threadIdFromMeta(meta) {
  return meta.thread_id || meta.id || meta.session_id || meta.source?.subagent?.thread_spawn?.thread_id || null;
}

function parentThreadIdFromMeta(meta) {
  return meta.parent_thread_id || meta.source?.subagent?.thread_spawn?.parent_thread_id || null;
}

function agentRoleFromMeta(meta) {
  return meta.agent_role
    || meta.source?.subagent?.thread_spawn?.agent_role
    || null;
}

function turnContext(rollout) {
  return rollout.events.find((event) => event?.type === "turn_context")?.payload || {};
}

function functionCallPayload(event) {
  const payload = payloadOf(event);
  if (payload?.type === "function_call") {
    return payload;
  }
  if (payload?.item?.type === "function_call") {
    return payload.item;
  }
  return null;
}

function parseFunctionArguments(call) {
  if (!call?.arguments) {
    return {};
  }
  if (typeof call.arguments === "string") {
    return parseJson(call.arguments, `function arguments for ${call.name}`);
  }
  return call.arguments;
}

function spawnFunctionCalls(parentRollout) {
  return parentRollout.events
    .map(functionCallPayload)
    .filter((call) => call?.name === "spawn_agent" || call?.name === "collaboration.spawn_agent")
    .map((call) => ({ call, args: parseFunctionArguments(call) }));
}

function hasHostCompletion(rollout) {
  return rollout.events.some((event) => {
    const payload = payloadOf(event);
    return payload?.type === "task_complete"
      || payload?.type === "turn_completed"
      || payload?.phase === "final_answer"
      || payload?.item?.phase === "final_answer"
      || (event?.type === "event_msg" && event?.payload?.type === "task_complete")
      || (event?.type === "event_msg" && event?.payload?.phase === "final_answer");
  });
}

function rolloutText(rollout) {
  return readFileSync(rollout.path, "utf8");
}

function userMessages(rollout) {
  return rollout.events.flatMap((event) => {
    const payload = payloadOf(event);
    if (payload?.type === "user_message" && typeof payload.message === "string") {
      return [payload.message];
    }
    return [];
  });
}

function commandExecutionEvents(rollout) {
  return rollout.events
    .map((event, index) => ({ index, payload: payloadOf(event) }))
    .filter(({ payload }) => payload?.type === "command_execution");
}

function customToolCallOutputText(payload) {
  if (typeof payload?.output === "string") {
    return payload.output;
  }
  if (Array.isArray(payload?.output)) {
    return payload.output.map((entry) => {
      if (typeof entry === "string") {
        return entry;
      }
      if (typeof entry?.text === "string") {
        return entry.text;
      }
      return JSON.stringify(entry);
    }).join("");
  }
  return "";
}

function successfulCustomToolOutputAfter(rollout, callId, callIndex) {
  return rollout.events
    .map((event, index) => ({ index, payload: payloadOf(event) }))
    .find(({ index, payload }) => (
      index > callIndex
      && payload?.type === "custom_tool_call_output"
      && payload.call_id === callId
      && customToolCallOutputText(payload).startsWith("Script completed")
    ));
}

function extractJsonObjectAt(source, start) {
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let index = start; index < source.length; index += 1) {
    const char = source[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (char === "\\") {
      escaped = inString;
      continue;
    }
    if (char === "\"") {
      inString = !inString;
      continue;
    }
    if (inString) {
      continue;
    }
    if (char === "{") {
      depth += 1;
    } else if (char === "}") {
      depth -= 1;
      if (depth === 0) {
        return source.slice(start, index + 1);
      }
    }
  }
  return null;
}

function execCommandInputs(input) {
  const commands = [];
  const marker = "tools.exec_command(";
  let offset = 0;
  while (offset < input.length) {
    const callStart = input.indexOf(marker, offset);
    if (callStart < 0) {
      break;
    }
    const objectStart = input.indexOf("{", callStart + marker.length);
    if (objectStart < 0) {
      break;
    }
    const objectText = extractJsonObjectAt(input, objectStart);
    if (!objectText) {
      break;
    }
    try {
      const args = JSON.parse(objectText);
      if (typeof args.cmd === "string") {
        commands.push(args.cmd);
      }
    } catch {
      // Ignore non-JSON tool source; evidence must come from the decoded exec_command args.
    }
    offset = objectStart + objectText.length;
  }
  return commands;
}

function completedExecCommandEvidence(rollout) {
  const commandExecutions = commandExecutionEvents(rollout)
    .filter(({ payload }) => payload.status === "completed" && payload.exit_code === 0)
    .map(({ index, payload }) => ({ index, command: payload.command || "" }));
  const customExecCalls = rollout.events.flatMap((event, index) => {
    const payload = payloadOf(event);
    if (
      payload?.type === "custom_tool_call"
      && payload.name === "exec"
      && payload.status === "completed"
      && typeof payload.call_id === "string"
      && typeof payload.input === "string"
      && successfulCustomToolOutputAfter(rollout, payload.call_id, index)
    ) {
      return execCommandInputs(payload.input).map((command) => ({ index, command }));
    }
    return [];
  });
  return [...commandExecutions, ...customExecCalls].sort((a, b) => a.index - b.index);
}

function childToolCallFragments(rollout) {
  return rollout.events.flatMap((event) => {
    const payload = payloadOf(event);
    const fragments = [];
    if (payload?.type === "command_execution" && typeof payload.command === "string") {
      fragments.push(payload.command);
    }
    if (payload?.item?.type === "command_execution" && typeof payload.item.command === "string") {
      fragments.push(payload.item.command);
    }
    const call = functionCallPayload(event);
    if (call) {
      if (typeof call.name === "string") {
        fragments.push(call.name);
      }
      if (typeof call.arguments === "string") {
        fragments.push(call.arguments);
      } else if (call.arguments) {
        fragments.push(JSON.stringify(call.arguments));
      }
    }
    if (payload?.type === "custom_tool_call") {
      if (typeof payload.name === "string") {
        fragments.push(payload.name);
      }
      if (typeof payload.input === "string") {
        fragments.push(payload.input);
      } else if (payload.input) {
        fragments.push(JSON.stringify(payload.input));
      }
    }
    return fragments;
  });
}

function normalizedSkillReadEvidence(fragment, absoluteSkillPath, relativeSkillPath) {
  const normalized = fragment.replace(/\\/g, "/");
  const absolute = absoluteSkillPath.replace(/\\/g, "/");
  return /\b(?:bat|cat|head|less|nl|rg|sed|tail)\b/.test(normalized)
    && (normalized.includes(absolute) || normalized.includes(relativeSkillPath));
}

function assertParentUsedRepoLocalPlanrLoop(parent, repo) {
  const text = rolloutText(parent);
  assertOk(!existsSync(join(repo, "AGENTS.md")), "cross-product oracle must not generate AGENTS.md prompt workarounds", {
    repo,
  });
  const repoSkillRoot = join(repo, ".codex/skills");
  assertOk(text.includes(`= \`${repoSkillRoot}\``) || text.includes(`= "${repoSkillRoot}"`), "parent rollout did not include repo-local skill root", {
    parent: parent.path,
    repoSkillRoot,
  });
  assertOk(text.includes("(file: r0/planr-loop/SKILL.md)"), "parent rollout did not resolve planr-loop through the repo-local skill root", {
    parent: parent.path,
  });
  assertOk(text.includes(".codex/skills/planr-loop/SKILL.md"), "parent rollout did not show a repo-local planr-loop file read", {
    parent: parent.path,
  });
  assertOk(!text.includes("/Users/kregenrek/.agents/skills/planr-loop/SKILL.md"), "parent rollout used global stale planr-loop skill", {
    parent: parent.path,
  });
  assertOk(!text.includes("dispatch through the routing skill"), "parent rollout included stale routing-skill instruction", {
    parent: parent.path,
  });
  assertOk(!text.includes("Host Routing Oracle Instructions"), "parent rollout used generated AGENTS.md prompt workaround", {
    parent: parent.path,
  });
  assertOk(text.includes("Pick packets expose provider-neutral `routing.profile`; they do not expose a host-owned `routing.agent_type`"), "parent rollout did not include neutral routing.profile handoff contract", {
    parent: parent.path,
  });
  assertOk(text.includes("dispatch that profile identifier as the host-native role/`agent_type`"), "parent rollout did not include profile-as-agent_type handoff contract", {
    parent: parent.path,
  });
  assertOk(text.includes("Each iteration follows the Planr stage protocols"), "parent rollout did not include cleaned planr-loop iteration text", {
    parent: parent.path,
  });
  const messages = userMessages(parent);
  assertOk(messages.length > 0, "parent rollout had no user messages", { parent: parent.path });
  for (const message of messages) {
    assertEqual(message, "$planr-loop", "parent user entry");
  }
}

function assertMakerLoggedVerification(makerRollout) {
  const commands = completedExecCommandEvidence(makerRollout);
  const evidence = findVerificationBeforeDone(commands);
  assertOk(evidence, "maker verification log was not recorded before planr done for the same item", {
    maker: makerRollout.path,
    commands: commands.map(({ command }) => command),
  });
}

function extractPlanrCommandOption(command, option) {
  const match = command.match(new RegExp(`(?:^|\\s)${sandboxRegexEscape(option)}(?:=|\\s+)(["']?)([^\\s"']+)\\1`));
  return match?.[2] || null;
}

function planrVerificationItem(command) {
  if (!/\bplanr\s+log\s+add\b/.test(command) || !/(?:^|\s)--kind(?:=|\s+)(["']?)verification\1(?:\s|$)/.test(command)) {
    return null;
  }
  return extractPlanrCommandOption(command, "--item");
}

function planrDoneItem(command) {
  const match = command.match(/\bplanr\s+done\s+(["']?)([^\s"']+)\1/);
  return match?.[2] || null;
}

function findVerificationBeforeDone(commands) {
  const verifications = commands.flatMap(({ index, command }) => {
    const item = planrVerificationItem(command || "");
    return item ? [{ index, command, item }] : [];
  });
  const dones = commands.flatMap(({ index, command }) => {
    const item = planrDoneItem(command || "");
    return item ? [{ index, command, item }] : [];
  });
  for (const verification of verifications) {
    const done = dones.find((candidate) => candidate.item === verification.item && verification.index < candidate.index);
    if (done) {
      return { verification, done };
    }
  }
  return null;
}

function rolloutLineCommands(rollout) {
  let offset = 0;
  return rolloutText(rollout).split(/\n/).map((line) => {
    const entry = { index: offset, command: line };
    offset += line.length + 1;
    return entry;
  });
}

function assertMakerReplayLoggedVerification(makerRollout) {
  const structuredCommands = completedExecCommandEvidence(makerRollout);
  const structuredEvidence = findVerificationBeforeDone(structuredCommands);
  if (structuredEvidence) {
    return;
  }
  const replayEvidence = findVerificationBeforeDone(rolloutLineCommands(makerRollout));
  assertOk(replayEvidence, "maker replay verification log was not recorded before planr done for the same item", {
    maker: makerRollout.path,
    structuredCommands: structuredCommands.map(({ command }) => command),
    fallback: "line-level transcript text",
  });
}

function assertChildUsedExpectedSkill(rollout, expectedSkill, repo) {
  const text = rolloutText(rollout);
  const absoluteSkillPath = join(repo, ".codex/skills", expectedSkill, "SKILL.md");
  const relativeSkillPath = `.codex/skills/${expectedSkill}/SKILL.md`;
  const globalSkillPathPattern = new RegExp(
    `${sandboxRegexEscape(homedir()).replace(/\\\\/g, "/")}/(?:\\.agents|\\.codex)(?:/[^\\s"']*)?/skills/${sandboxRegexEscape(expectedSkill)}/SKILL\\.md`,
  );
  const normalizedText = text.replace(/\\/g, "/");
  assertOk(!globalSkillPathPattern.test(normalizedText), `child rollout used global ${expectedSkill} skill`, {
    rollout: rollout.path,
    expectedSkill,
  });
  const toolCallFragments = childToolCallFragments(rollout);
  const readEvidence = toolCallFragments.filter((fragment) => (
    normalizedSkillReadEvidence(fragment, absoluteSkillPath, relativeSkillPath)
  ));
  assertOk(
    readEvidence.length > 0,
    `child rollout did not read repo-local ${expectedSkill} skill`,
    {
      rollout: rollout.path,
      expectedSkill,
      acceptedPaths: [absoluteSkillPath, relativeSkillPath],
      toolCallFragments,
    },
  );
}

function hasReplayReviewerAuditEvidence(rollout) {
  const text = rolloutText(rollout);
  return text.includes("PLANR_WORKER_ID=checker-reviewer planr --json pick --work-type review")
    && text.includes("grep -q 'Planr loop oracle smoke' README.md")
    && text.includes("SMOKE_EXIT=0")
    && text.includes("The packet matches the assigned target and includes both completion and replayable verification logs");
}

function assertSpawnArguments(spawns, expected) {
  const matches = spawns.filter(({ args }) => args.agent_type === expected.role);
  assertEqual(matches.length, 1, `${expected.label} spawn function-call count`);
  const { args } = matches[0];
  assertEqual(args.agent_type, expected.role, `${expected.label} native agent_type`);
  assertEqual(args.fork_turns, "none", `${expected.label} fork_turns`);
  assertOk(typeof args.message === "string" && args.message.length > 0, `${expected.label} spawn message missing`, args);
  return args;
}

function assertChildRollout(childRollouts, parentThreadId, expected) {
  const matches = childRollouts.filter((rollout) => agentRoleFromMeta(sessionMeta(rollout)) === expected.role);
  assertEqual(matches.length, 1, `${expected.label} child rollout count`);
  const rollout = matches[0];
  const meta = sessionMeta(rollout);
  const context = turnContext(rollout);
  assertEqual(parentThreadIdFromMeta(meta), parentThreadId, `${expected.label} parent_thread_id`);
  assertEqual(agentRoleFromMeta(meta), expected.role, `${expected.label} session_meta agent_role`);
  assertEqual(context.model, expected.model, `${expected.label} effective model`);
  assertEqual(context.effort, expected.effort, `${expected.label} effective effort`);
  assertOk(
    hasHostCompletion(rollout) || (expected.allowReplayReviewerAuditEvidence && hasReplayReviewerAuditEvidence(rollout)),
    `${expected.label} child did not complete`,
    { path: rollout.path },
  );
  assertChildUsedExpectedSkill(rollout, expected.skill, expected.repo);
  return { path: rollout.path, threadId: threadIdFromMeta(meta), role: expected.role, rollout };
}

function isDirectChildWithRole(rollout, parentThreadId, expectedRoles) {
  const meta = sessionMeta(rollout);
  return parentThreadIdFromMeta(meta) === parentThreadId
    && expectedRoles.has(agentRoleFromMeta(meta));
}

function assertCodexLiveHostEvidence(result, outputPath, changedFiles, repo) {
  const combined = `${result.stderr}\n${result.stdout}`;
  assertOk(!combined.includes("collab spawn failed"), "Codex host reported a collab spawn failure", { outputPath });
  const publicEvents = parseJsonlEvents(result.stdout, "Codex public JSONL");
  const publicParentThreadId = publicEvents.find((event) => event?.type === "thread.started")?.thread_id;
  assertOk(publicParentThreadId, "Codex public JSONL did not expose parent thread id", { outputPath });
  const rollouts = changedFiles.map((file) => parseRollout(file.path));
  const parentCandidates = rollouts.filter((rollout) => {
    const meta = sessionMeta(rollout);
    return threadIdFromMeta(meta) === publicParentThreadId && !parentThreadIdFromMeta(meta);
  });
  assertEqual(parentCandidates.length, 1, "parent rollout count");
  const parent = parentCandidates[0];
  assertParentUsedRepoLocalPlanrLoop(parent, repo);
  const parentThreadId = threadIdFromMeta(sessionMeta(parent));
  assertOk(parentThreadId, "parent rollout missing thread id", { path: parent.path, meta: sessionMeta(parent) });
  const spawns = spawnFunctionCalls(parent);
  assertEqual(spawns.length, 2, "parent spawn_agent function-call count");
  assertSpawnArguments(spawns, {
    label: "maker",
    role: "model_routing_terra_high",
  });
  assertSpawnArguments(spawns, {
    label: "reviewer",
    role: "model_routing_sol_high",
  });
  const expectedRoles = new Set(["model_routing_terra_high", "model_routing_sol_high"]);
  const directChildrenWithOtherRoles = rollouts.filter((rollout) => {
    const meta = sessionMeta(rollout);
    const role = agentRoleFromMeta(meta);
    return parentThreadIdFromMeta(meta) === parentThreadId && role && !expectedRoles.has(role);
  });
  assertEqual(directChildrenWithOtherRoles.length, 0, "unexpected direct child rollout role count");
  const hostGeneratedDirectChildren = rollouts.filter((rollout) => {
    const meta = sessionMeta(rollout);
    return parentThreadIdFromMeta(meta) === parentThreadId && !agentRoleFromMeta(meta);
  });
  for (const rollout of hostGeneratedDirectChildren) {
    const meta = sessionMeta(rollout);
    assertOk(meta.source?.subagent?.other || !meta.source?.subagent?.thread_spawn, "unrouted direct child used thread_spawn metadata", {
      path: rollout.path,
      meta,
    });
  }
  const childRollouts = rollouts.filter((rollout) => isDirectChildWithRole(rollout, parentThreadId, expectedRoles));
  assertEqual(childRollouts.length, 2, "child rollout count");
  const maker = assertChildRollout(childRollouts, parentThreadId, {
    label: "maker",
    role: "model_routing_terra_high",
    model: "gpt-5.6-terra",
    effort: "high",
    skill: "planr-work",
    repo,
  });
  const reviewer = assertChildRollout(childRollouts, parentThreadId, {
    label: "reviewer",
    role: "model_routing_sol_high",
    model: "gpt-5.6-sol",
    effort: "high",
    skill: "planr-review",
    repo,
    allowReplayReviewerAuditEvidence: Boolean(replayRoot),
  });
  if (replayRoot) {
    assertMakerReplayLoggedVerification(maker.rollout);
  } else {
    assertMakerLoggedVerification(maker.rollout);
  }
  assertOk(maker.path !== reviewer.path, "maker and reviewer child rollout files must be distinct", { maker, reviewer });
  return {
    parent_rollout: parent.path,
    maker_rollout: maker.path,
    reviewer_rollout: reviewer.path,
    maker_thread_id: maker.threadId,
    reviewer_thread_id: reviewer.threadId,
    maker_role: "model_routing_terra_high",
    reviewer_role: "model_routing_sol_high",
    maker_model: "gpt-5.6-terra",
    reviewer_model: "gpt-5.6-sol",
    maker_effort: "high",
    reviewer_effort: "high",
    fork_turns_all_used: false,
  };
}

function runCodexLive(repo, contract) {
  const prompt = "$planr-loop";
  const outputPath = join(tempRoot, "codex-live.jsonl");
  const beforeRollouts = snapshotRollouts();
  const beforeLiveConfig = snapshotUserConfigSentinels();
  const sandbox = platform() === "darwin"
    ? codexOuterSandboxProfile(repo)
    : {
        allowedRepo: repo,
        externallySandboxed: false,
        profilePath: null,
        protectedPaths: [],
      };
  recordCodexHostDiagnostics(repo, sandbox);
  const sandboxArgs = platform() === "darwin"
    ? ["--dangerously-bypass-approvals-and-sandbox"]
    : ["--sandbox", "workspace-write"];
  const { result } = runCodexLiveCommand([
    "exec",
    "-C",
    repo,
    "--config",
    codexProjectTrustOverride(repo),
    ...CODEX_MULTI_AGENT_V2_METADATA_CONFIG,
    ...sandboxArgs,
    "--json",
    "--",
    prompt,
  ], {
    cwd: repo,
    sandbox,
    allowFailure: true,
    timeoutMs: 360_000,
    maxBuffer: 50 * 1024 * 1024,
  });
  writeFileSync(outputPath, `${result.stderr}${result.stdout}`);
  assertUserConfigSentinelsUnchanged(beforeLiveConfig);
  assertEqual(result.status, 0, "authenticated Codex live run status");
  const changed = changedRollouts(beforeRollouts);
  const hostEvidence = assertCodexLiveHostEvidence(result, outputPath, changed, repo);
  receipt("authenticated Codex executed routed maker and reviewer", { outputPath, hostEvidence });
}

function assertRetainedMissingAuthEvidence() {
  const noauthRoot = join(tempRoot, "noauth-codex");
  const evidencePath = join(tempRoot, "codex-noauth-result.json");
  assertOk(existsSync(noauthRoot), "retained replay root missing noauth Codex home", { noauthRoot });
  assertOk(existsSync(evidencePath), "retained replay root missing no-auth command result", { evidencePath });
  const evidence = parseJson(readFileSync(evidencePath, "utf8"), "retained no-auth command result");
  assertOk(evidence.status !== 0, "retained no-auth command unexpectedly succeeded", evidence);
  assertOk(
    evidence.authError === "401 Unauthorized" || evidence.authError === "Missing bearer or basic authentication",
    "retained no-auth result lacks authentication error",
    evidence,
  );
  receipt("missing Codex authentication failure evidence retained", {
    noauthRoot,
    evidencePath,
    evidenceSha256: hashFile(evidencePath, "sha256"),
    authError: evidence.authError,
  });
}

function assertRetainedUninstallAndUnroutedPlanr(repo, contract) {
  const switchloomBin = join(tempRoot, "switchloom-package/package/npm/native", nativeTarget(), "model-routing");
  assertOk(existsSync(switchloomBin), "retained replay root missing extracted Switchloom binary", { switchloomBin });
  const bundle = join(tempRoot, "balanced-planr-codex.json");
  const expectedPaths = [
    ".codex/agents/model-routing-luna-xhigh.toml",
    ".codex/agents/model-routing-sol-high.toml",
    ".codex/agents/model-routing-sol-medium.toml",
    ".codex/agents/model-routing-sol-ultra.toml",
    ".codex/agents/model-routing-terra-high.toml",
    ".codex/agents/model-routing-terra-medium.toml",
    ".codex/config.toml",
    ".planr/agents.toml",
    ".planr/policy.toml",
  ].sort();
  if (!existsSync(join(repo, ".model-routing/manifest.json"))) {
    assertOk(existsSync(bundle), "retained replay root missing applied bundle for reinstall", { bundle });
    const apply = parseJson(run(switchloomBin, ["apply", bundle, "--repository", repo, "--yes"]).stdout, "switchloom replay apply");
    assertArrayEqual(apply.artifacts.map((artifact) => artifact.path).sort(), expectedPaths, "replay apply artifact set");
  }
  const unmanaged = [".codex/agents/user-local.toml", ".planr/user-note.txt"];
  for (const path of unmanaged) {
    const full = join(repo, path);
    mkdirSync(dirname(full), { recursive: true });
    writeFileSync(full, "managed_by = \"user\"\n");
  }
  const uninstall = parseJson(
    run(switchloomBin, ["uninstall", "--repository", repo]).stdout,
    "switchloom replay uninstall",
  );
  const removedPaths = uninstall.artifacts.map((artifact) => artifact.path).sort();
  assertArrayEqual(removedPaths, expectedPaths, "replay uninstall removed artifact set");
  for (const artifact of uninstall.artifacts) {
    assertEqual(artifact.status, "removed", `managed artifact ${artifact.path} replay uninstall status`);
  }
  for (const path of expectedPaths) {
    assertOk(!existsSync(join(repo, path)), "managed artifact still exists after replay uninstall", { path });
  }
  for (const path of unmanaged) {
    assertOk(existsSync(join(repo, path)), "replay uninstall removed unmanaged file", { path });
  }
  const agents = parseJson(stripPlanrNoise(run(planrBin, ["--json", "agents", "check"], { cwd: repo }).stdout), "replay unrouted planr agents check");
  assertEqual(agents.ok, true, "replay unrouted Planr agents check");
  assertEqual(agents.reason, "missing", "replay unrouted Planr missing registry reason");
  assertPlanAuditHolds(repo, contract);
  receipt("retained Switchloom uninstall and unrouted Planr checks passed");
}

function completeRetainedReviewIfNeeded(repo, contract) {
  const audit = parseJson(stripPlanrNoise(run(planrBin, ["--json", "plan", "audit", contract.plan.id], { cwd: repo }).stdout), "replay pre-close plan audit");
  if (audit.holds) {
    receipt("retained Planr audit already holds", { plan: contract.plan.id });
    return;
  }
  const review = parseJson(stripPlanrNoise(run(planrBin, ["--json", "trace", "item", "i-review-build-first-slice-8f04"], { cwd: repo }).stdout), "retained review trace");
  assertEqual(review.item.status, "picked", "retained review status before replay close");
  assertEqual(review.item.worker_id, "checker-reviewer", "retained review worker");
  run(planrBin, [
    "review",
    "close",
    "i-review-build-first-slice-8f04",
    "--verdict",
    "complete",
    "--reviewer",
    "checker-reviewer",
    "--close-target",
  ], { cwd: repo });
  receipt("retained Planr review closed from replayed reviewer evidence", { review: "i-review-build-first-slice-8f04" });
}

function retainedGoalContract(repo) {
  const result = parseJson(
    stripPlanrNoise(run(planrBin, ["--json", "context", "list", "--tag", "goal-contract"], { cwd: repo }).stdout),
    "retained goal contract context list",
  );
  const contracts = (result.contexts || []).filter((context) => (
    context?.kind === "goal-contract"
    && typeof context.item_id === "string"
    && typeof context.content === "string"
    && context.content.startsWith("GOAL CONTRACT ")
  ));
  assertEqual(contracts.length, 1, "retained replay goal-contract context count");
  const [contract] = contracts;
  const planMatch = contract.content.match(/^GOAL CONTRACT\s+([^:\s]+):/);
  assertOk(planMatch, "retained replay goal-contract content did not expose a plan id", {
    context: contract,
  });
  receipt("retained Planr goal contract derived", {
    plan: planMatch[1],
    item: contract.item_id,
    context: contract.id,
  });
  return { plan: { id: planMatch[1] }, item: { id: contract.item_id } };
}

function runReplayFromRetainedRoot() {
  const beforeReplayConfig = snapshotUserConfigSentinels();
  const beforeSwitchloomSource = snapshotSwitchloomSourceWorktree();
  const repo = join(tempRoot, "fresh-repo");
  assertOk(existsSync(repo), "retained replay root missing fresh repo", { repo });
  const outputPath = join(tempRoot, "codex-live.jsonl");
  assertOk(existsSync(outputPath), "retained replay root missing codex-live.jsonl", { outputPath });
  const tarball = retainedPublicTarball();
  assertEqual(hashFile(tarball, "sha1"), EXPECTED.shasum, "retained tarball sha1");
  assertEqual(integrity(tarball), EXPECTED.integrity, "retained tarball sha512 integrity");
  const replayOutput = readFileSync(outputPath, "utf8");
  const publicEvents = parseJsonlEvents(replayOutput, "retained Codex public JSONL");
  const publicParentThreadId = publicEvents.find((event) => event?.type === "thread.started")?.thread_id;
  assertOk(publicParentThreadId, "retained Codex public JSONL did not expose parent thread id", { outputPath });
  const changedFiles = rolloutFilesForReplay(publicParentThreadId, outputPath);
  const hostEvidence = assertCodexLiveHostEvidence({ status: 0, stdout: replayOutput, stderr: "" }, outputPath, changedFiles, repo);
  receipt("replayed authenticated Codex routed maker and reviewer", { outputPath, hostEvidence });
  const contract = retainedGoalContract(repo);
  completeRetainedReviewIfNeeded(repo, contract);
  assertPlanAuditHolds(repo, contract);
  assertRetainedUninstallAndUnroutedPlanr(repo, contract);
  assertRetainedMissingAuthEvidence();
  assertRequestedOnlyRejectedFromAudit(join(tempRoot, "requested-only-route-audit.json"));
  assertUserConfigSentinelsUnchanged(beforeReplayConfig);
  assertSwitchloomSourceWorktreeUnchanged(beforeSwitchloomSource);
  assertNoDuplicateModelSelectionOwnership();
  const receiptPath = join(tempRoot, "replay-receipt.json");
  const payload = { ok: true, mode: "replay", tempRoot, receipts };
  writeFileSync(receiptPath, JSON.stringify(payload, null, 2));
  receipt("replay receipt written", { receiptPath });
  process.stdout.write(JSON.stringify({ ...payload, receiptPath }, null, 2));
}

function assertPlanAuditHolds(repo, contract) {
  const audit = parseJson(stripPlanrNoise(run(planrBin, ["--json", "plan", "audit", contract.plan.id], { cwd: repo }).stdout), "planr plan audit");
  assertEqual(audit.holds, true, "fresh Planr audit holds");
  const clauses = new Map((audit.clauses || []).map((clause) => [clause.clause, clause]));
  for (const name of ["items_settled", "reviews_complete", "approvals_clear", "verification_logged"]) {
    assertEqual(clauses.get(name)?.pass, true, `fresh Planr audit clause ${name}`);
  }
  assertOk((clauses.get("verification_logged")?.logs || []).length > 0, "fresh Planr audit missing verification logs", audit);
  receipt("fresh Planr audit holds with verification evidence", {
    plan: contract.plan.id,
    item: contract.item.id,
    clauses: audit.clauses.map((clause) => clause.clause),
  });
}

function assertUninstallAndUnroutedPlanr(switchloomBin, repo, appliedArtifacts, contract) {
  const unmanaged = [
    ".codex/agents/user-local.toml",
    ".planr/user-note.txt",
  ];
  for (const path of unmanaged) {
    const full = join(repo, path);
    mkdirSync(dirname(full), { recursive: true });
    writeFileSync(full, "managed_by = \"user\"\n");
  }
  const uninstall = parseJson(run(switchloomBin, ["uninstall", "--repository", repo]).stdout, "switchloom uninstall");
  const removedPaths = uninstall.artifacts.map((artifact) => artifact.path).sort();
  assertArrayEqual(removedPaths, appliedArtifacts, "uninstall removed artifact set");
  for (const artifact of uninstall.artifacts) {
    assertEqual(artifact.status, "removed", `managed artifact ${artifact.path} uninstall status`);
  }
  for (const path of appliedArtifacts) {
    assertOk(!existsSync(join(repo, path)), "managed artifact still exists after uninstall", { path });
  }
  for (const path of unmanaged) {
    assertOk(existsSync(join(repo, path)), "uninstall removed unmanaged file", { path });
  }
  const agents = parseJson(stripPlanrNoise(run(planrBin, ["--json", "agents", "check"], { cwd: repo }).stdout), "unrouted planr agents check");
  assertEqual(agents.ok, true, "unrouted Planr agents check");
  assertEqual(agents.reason, "missing", "unrouted Planr missing registry reason");
  assertPlanAuditHolds(repo, contract);
  receipt("Switchloom uninstall removes only managed files and unrouted Planr still works");
}

function assertNoDuplicateModelSelectionOwnership() {
  run("cargo", ["test", "--test", "routing_ownership"]);
  receipt("canonical routing ownership regression passed");
}

let beforeConfig;

try {
  if (replayRoot) {
    runReplayFromRetainedRoot();
  } else {
    beforeConfig = snapshotUserConfigSentinels();
    const beforeSwitchloomSource = snapshotSwitchloomSourceWorktree();
    const tarball = packageTarball();
    const switchloomBin = extractTarball(tarball);
    const repo = createFreshRepo();
    const { bundle, appliedArtifacts } = compileAndApply(switchloomBin, repo);
    provisionRepoLocalPlanrSkills(repo);
    const contract = createPlanrLoopContract(repo);
    assertPlanrConsumes(repo);
    assertRequestedOnlyRejected();
    runCodexNoAuth(repo);
    runCodexLive(repo, contract);
    assertPlanAuditHolds(repo, contract);
    assertUninstallAndUnroutedPlanr(switchloomBin, repo, appliedArtifacts, contract);
    assertUserConfigSentinelsUnchanged(beforeConfig);
    assertSwitchloomSourceWorktreeUnchanged(beforeSwitchloomSource);
    assertNoDuplicateModelSelectionOwnership();
    receipt("cross-product oracle complete", { tempRoot, bundle });
    const receiptPath = join(tempRoot, "oracle-receipt.json");
    receipt("oracle receipt written", { receiptPath });
    const payload = { ok: true, mode: "live", tempRoot, receipts };
    writeFileSync(receiptPath, JSON.stringify(payload, null, 2));
    process.stdout.write(JSON.stringify({ ...payload, receiptPath }, null, 2));
  }
} catch (error) {
  process.stderr.write(`\n[fail] ${error.message}\n`);
  if (error.detail) {
    process.stderr.write(`${JSON.stringify(error.detail, null, 2)}\n`);
  }
  process.stderr.write(JSON.stringify({ ok: false, tempRoot, receipts }, null, 2));
  process.exit(1);
}
