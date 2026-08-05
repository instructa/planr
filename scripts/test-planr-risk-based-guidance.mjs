#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (name) => readFileSync(path.join(root, "plugins/planr/skills", name, "SKILL.md"), "utf8");
const readRel = (name) => readFileSync(path.join(root, name), "utf8");
const planrBin = process.env.PLANR_BIN ?? path.join(root, "target/debug/planr");
const shippedSkillRoot = path.join(root, "plugins/planr/skills");
const forbiddenBlanketReplay = [
  /reviewer reruns (?:it|the logged verification evidence)/iu,
  /rerun the\s+logged verification commands/iu,
  /replay the logged evidence/iu,
  /all logged verification commands/iu,
  /every logged verification/iu,
];
const forbiddenUnconditionalReview = /planr done[^\n]*(?:\\\n[^\n]*){0,8}\s+--review(?![\w-])/iu;
const forbiddenLegacyDoneReview = /(?:planr\s+)?done\s+--review(?![\w-])|planr done[^\n]*(?:\\\n[^\n]*){0,8}\s+--review(?![\w-])/iu;
const forbiddenSyntheticRepair = /<fix-item-id>|Planr creates? (?:a |the )?fix item|creates? (?:a |the )?follow-up-review item/iu;

const shippedSkillAssets = readdirSync(shippedSkillRoot)
  .map((name) => [name, path.join(shippedSkillRoot, name, "SKILL.md")])
  .filter(([, skillPath]) => existsSync(skillPath))
  .map(([name, skillPath]) => [name, readFileSync(skillPath, "utf8")]);

function teachesOutcomeSettlement(contents) {
  return /(?:^|\n)planr done\s+<|Plain `planr done`/u.test(contents);
}

function assertCanonicalOutcomeSettlement(name, contents) {
  assert.match(contents, /work_packet\.kind/u, `${name} must branch on typed work-packet kind`);
  assert.match(contents, /work_packet\.transition|FeatureRun transition/u, `${name} must branch on typed settlement transition`);
  assert.match(contents, /`done --next` is the standard settlement path inside an authorized compatible maker run|Plain `planr done`/u, `${name} must teach fused settlement for compatible runs or plain done for standalone work`);
  assert.match(contents, /--escalate <reason>/u, `${name} must limit intentional review override to structured escalation`);
  assert.match(contents, /--escalation-ref/u, `${name} must require an escalation reference`);
  assert.match(contents, /--escalation-explanation/u, `${name} must require an escalation explanation`);
}

const goal = read("planr-goal");
const loop = read("planr-loop");
const work = read("planr-work");
const review = read("planr-review");
const web = read("planr-verify-web");
const taskGraph = read("planr-task-graph");
const roleAssets = [
  ["claude-reviewer", readRel("plugins/planr/agents/planr-reviewer.md")],
  ["cursor-reviewer", readRel("plugins/planr/skills/planr-loop/agents/planr-reviewer.md")],
  ["pi-reviewer", readRel("plugins/planr/agents/pi/planr-reviewer.md")],
  ["claude-worker", readRel("plugins/planr/agents/planr-worker.md")],
  ["cursor-worker", readRel("plugins/planr/skills/planr-loop/agents/planr-worker.md")],
  ["pi-worker", readRel("plugins/planr/agents/pi/planr-worker.md")],
  ["host-dispatch", readRel("plugins/planr/skills/planr-loop/references/host-dispatch.md")],
];

function runPlanr(args, cwd) {
  const command = existsSync(planrBin)
    ? { bin: planrBin, args }
    : { bin: "cargo", args: ["run", "--manifest-path", path.join(root, "Cargo.toml"), "--quiet", "--", ...args] };
  const result = spawnSync(command.bin, command.args, { cwd, encoding: "utf8" });
  assert.equal(result.status, 0, `${command.bin} ${command.args.join(" ")} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`);
  return result.stdout;
}

function relativeFiles(dir) {
  const out = [];
  const visit = (current) => {
    for (const entry of readdirSync(current)) {
      const absolute = path.join(current, entry);
      const stat = statSync(absolute);
      if (stat.isDirectory()) {
        visit(absolute);
      } else if (stat.isFile()) {
        out.push(path.relative(dir, absolute));
      }
    }
  };
  visit(dir);
  return out.sort();
}

function readMarkdownCorpus(...dirs) {
  const corpus = [];
  for (const dir of dirs) {
    for (const relative of relativeFiles(dir)) {
      if (!/[.](?:md|mdx)$/u.test(relative)) continue;
      corpus.push([path.relative(root, path.join(dir, relative)), readFileSync(path.join(dir, relative), "utf8")]);
    }
  }
  assert.ok(corpus.length > 0, "documentation corpus must not be empty");
  return corpus;
}

function installedHostAssets() {
  const workspace = mkdtempSync(path.join(os.tmpdir(), "planr-risk-guidance-"));
  try {
    runPlanr(["--json", "project", "init", "Risk Guidance", "--client", "codex"], workspace);
    runPlanr(["install", "codex", "--force", "--no-mcp", "--json"], workspace);
    for (const client of ["claude", "cursor", "grok", "pi"]) {
      runPlanr(["install", client, "--force", "--no-mcp", "--no-hooks", "--json"], workspace);
    }
    const installed = relativeFiles(workspace)
      .filter((relative) => /(?:^|\/)(?:agents|skills)\//u.test(relative) || relative === ".codex/hooks/planr-codex-stop.sh")
      .map((relative) => [relative, readFileSync(path.join(workspace, relative), "utf8")]);
    assert.ok(installed.some(([relative]) => relative === ".claude/agents/planr-reviewer.md"));
    assert.ok(installed.some(([relative]) => relative === ".cursor/agents/planr-reviewer.md"));
    assert.ok(installed.some(([relative]) => relative === ".grok/agents/planr-reviewer.md"));
    assert.ok(installed.some(([relative]) => relative === ".pi/agents/planr-reviewer.md"));
    assert.ok(installed.some(([relative]) => relative === ".cursor/skills/planr-loop/SKILL.md"));
    assert.ok(installed.some(([relative]) => relative === ".grok/skills/planr-loop/SKILL.md"));
    assert.ok(installed.some(([relative]) => relative === ".pi/skills/planr-loop/SKILL.md"));
    assert.ok(installed.some(([relative]) => relative === ".codex/hooks/planr-codex-stop.sh"));
    return installed;
  } finally {
    rmSync(workspace, { recursive: true, force: true });
  }
}

assert.match(goal, /small coherent change is one implementation item plus one signal-bearing independent review/u);
assert.match(goal, /versioned verification policy and source-bound receipt runner/u);
assert.match(loop, /cheap, missing, failing, or explicitly high-risk evidence/u);
assert.match(loop, /maker never leases its own ReviewGate/u);
assert.match(loop, /Keep one active write item/u);
assert.match(loop, /compatible same-plan maker run/u);
assert.match(loop, /Stop when settlement opens a material ReviewGate, ownership or scope becomes incompatible, work blocks, the pick is empty, or a budget boundary is reached/u);
assert.match(loop, /atomically rolls the internal three-outcome ExecutionBatch/u);
assert.match(loop, /compact durable handoff/u);
assert.match(loop, /branches only on the returned typed `work_packet`/u);
assert.match(loop, /mode: "finding_repair"/u);
assert.match(loop, /kind: "hold"` stops/u);
assert.match(loop, /fresh verification-only worker with a verifier identity distinct from the maker/u);
assert.match(loop, /canonical Evidence `SOURCE_PATHS` digest/u);
assert.match(loop, /source mismatch inside the Evidence transaction records a failed non-covering attempt and zero trusted receipts/u);
assert.match(loop, /after repair the coordinator re-freezes, then a leased verifier reruns readiness/u);
assert.match(loop, /selectively rerunning only invalidated Evidence/u);
assert.match(loop, /Keep exactly one final independent product ReviewGate/u);
assert.match(work, /npm run verification:run -- --receipt/u);
assert.match(work, /receipt path, digest, source revision, selected profile\/gates/u);
assert.match(work, /`done --next` is the standard settlement path inside an authorized compatible maker run/u);
assert.match(work, /Branch only on `work_packet.kind`/u);
assert.match(work, /work_packet.execution_state/u);
assert.match(work, /mode: "finding_repair"/u);
assert.match(work, /Do not call `planr done` for a finding-repair packet that has no item/u);
assert.match(work, /Intentional escalation beyond computed materiality uses `--escalate <reason>`/u);
assert.match(work, /committed `.planr\/policy.toml`/u);
assert.match(work, /must not run Evidence readiness, collection, or an opportunistic browser\/live smoke while later compatible implementation outcomes can still change the source/u);
assert.match(work, /keep the same maker through stable source freeze/iu);
assert.match(work, /durably caps each internal ExecutionBatch at three outcomes and rolls it for the same maker inside `done --next`/u);
assert.match(work, /fresh verification-only worker first leases `planr pick --plan <plan-id> --work-type verification --json`/u);
assert.match(work, /executes only `readiness\.run_index\.repository_path`/u);
assert.match(work, /planr evidence run` enforces the frozen source digest inside the real Evidence transaction before any trusted receipt commit/u);
assert.match(work, /product-source mutation records a failed non-covering attempt and zero new trusted receipts/u);
assert.match(work, /write Planr runtime state, receipts, logs, and artifacts/u);
assert.match(work, /coordinator re-freezes and a leased verifier reruns readiness before selectively rerunning only invalidated Evidence/u);
assert.match(work, /Product findings route back to the responsible maker/u);
assert.doesNotMatch(work, forbiddenUnconditionalReview, "planr-work must not teach unconditional maker --review");
assert.match(review, /npm run verification:verify -- --receipt/u);
assert.match(review, /Receipt validation does not replace judgment/u);
assert.match(review, /Never manufacture independence by changing identities/u);
assert.match(review, /Continue only when `work_packet.kind` is `review_gate`/u);
assert.match(review, /canonical `execution_state`/u);
assert.match(loop, /Do not run readiness or evidence runs before every mutable slice/u);
assert.match(loop, /Batch compatible implementation work first, settle source/u);
assert.match(loop, /does not implement or inspect product source/u);
assert.match(loop, /including default-role dispatch when no generated role exists, set `fork_turns: "none"`/u);
assert.match(loop, /Spawn each maker or checker role once/u);
assert.match(loop, /Use one completion-length `wait_agent`/u);
assert.match(loop, /`timeout_ms: 3600000`/u);
assert.match(loop, /do not implement a 60-second polling loop/u);
assert.match(loop, /Call `list_agents`, another `wait_agent`, or recovery checks only after that wait times out, the role reports lost state, or explicit user steering/u);
assert.match(loop, /route review findings back to that same maker with `followup_task`/u);
assert.match(loop, /after accepted re-review reuse that maker again/u);
assert.match(loop, /internal transition is transparent to the driver and does not wake the root/u);
assert.match(loop, /Do not spawn a replacement maker unless the host reports the original unavailable/u);
assert.match(web, /approved deployment decision before the deploy begins/u);
assert.match(web, /does not automatically trigger another full build or gate replay/u);
assert.match(web, /Continue only when `work_packet.kind` is `verification`/u);
assert.match(web, /Product source is read-only/u);
assert.match(web, /Do not call `planr done`/u);
assert.match(web, /do not choose a different unregistered tool or downgrade the observation/u);
assert.doesNotMatch(web, /drop to the next tier|one-off headless|scriptable fallback/iu);
assertCanonicalOutcomeSettlement("planr-task-graph", taskGraph);

for (const [name, contents] of shippedSkillAssets) {
  assert.doesNotMatch(contents, forbiddenLegacyDoneReview, `${name} must not teach unsupported done --review`);
  if (teachesOutcomeSettlement(contents)) assertCanonicalOutcomeSettlement(name, contents);
}

for (const [name, contents] of [["loop", loop], ["review", review], ["web", web]]) {
  assert.doesNotMatch(contents, forbiddenBlanketReplay[0], `${name} must not require unconditional replay`);
}

for (const [name, contents] of roleAssets) {
  for (const pattern of forbiddenBlanketReplay) {
    assert.doesNotMatch(contents, pattern, `${name} must not require blanket replay`);
  }
  assert.doesNotMatch(contents, forbiddenSyntheticRepair, `${name} must not dispatch synthetic fix items`);
}

for (const [name, contents] of roleAssets.filter(([name]) => name.endsWith("reviewer"))) {
  assert.match(contents, /selectively replay only cheap, missing, failing, or explicitly\s+high-risk evidence/u, `${name} must carry selective replay language`);
  assert.match(contents, /work_packet.kind: "review_gate"/u, `${name} must consume typed ReviewGate packets`);
  assert.match(contents, /canonical\s+`execution_state`/u, `${name} must consume canonical execution state`);
}

for (const [name, contents] of roleAssets.filter(([name]) => name.endsWith("worker"))) {
  assert.match(contents, /compatible\s+same-plan maker run/u, `${name} must carry compatible same-plan run language`);
  assert.match(contents, /keep one worker identity/u, `${name} must keep a stable maker identity`);
  assert.match(contents, /internal three-outcome ExecutionBatch atomically/u, `${name} must keep the durable internal cap without a host stop`);
  assert.match(contents, /compact durable handoff/u, `${name} must require compact durable handoff`);
  assert.match(contents, /fresh\s+verification-only worker first leases/u, `${name} must hand frozen-source Evidence to a leased verifier`);
  assert.match(contents, /readiness\.run_index\.repository_path/u, `${name} must name the sole executable readiness path`);
  assert.match(contents, /SOURCE_PATHS digest/u, `${name} must enforce verifier source freeze through canonical Evidence scope`);
  assert.match(contents, /zero\s+new trusted receipts/u, `${name} must fail closed before receipt acceptance`);
  assert.match(contents, /do not run binding Evidence\s+readiness, collection,\s+or an opportunistic live smoke for a\s+mutable implementation item/u, `${name} must defer binding Evidence until source freeze`);
  assert.match(contents, /`planr done --next` inside the\s+authorized run/u, `${name} must use fused settlement inside a compatible run`);
  assert.match(contents, /work_packet.kind/u, `${name} must consume typed outcome packets`);
  assert.match(contents, /mode` is `finding_repair`|mode: "finding_repair"/u, `${name} must repair findings on the typed packet`);
  assert.match(contents, /No review or fix map item exists/u, `${name} must hard-cut synthetic repair items`);
  assert.doesNotMatch(contents, forbiddenUnconditionalReview, `${name} must not require unconditional --review`);
}

const hostDispatch = roleAssets.find(([name]) => name === "host-dispatch")[1];
assert.match(hostDispatch, /compatible same-plan maker run/u);
assert.match(hostDispatch, /default Codex role is the correct fallback, still set `fork_turns: "none"`/u);
assert.match(hostDispatch, /wait once with `wait_agent\(\{ timeout_ms: 3600000 \}\)`/u);
assert.match(hostDispatch, /Codex maker continuity example/u);
assert.match(hostDispatch, /`planr done --next` atomically settles the outcome, rolls a capped internal ExecutionBatch/u);
assert.doesNotMatch(
  hostDispatch,
  /First run `planr run batch roll|If the maker stops because the typed settlement transition is `batch_cap_reached`/u,
  "host dispatch must not preserve the manual batch-cap continuation choreography",
);
assert.match(hostDispatch, /After the checker accepts the same review gate/u);
assert.match(hostDispatch, /no fix item is created/u);
assert.doesNotMatch(hostDispatch, forbiddenSyntheticRepair);
assert.match(hostDispatch, /compatible same-plan maker run/u);
assert.match(hostDispatch, /Your first command is planr pick --plan <plan-id> --work-type verification --json/u);
assert.match(loop, /Its first command is `planr pick --plan <plan-id> --work-type verification --json`/u);
assert.match(web, /This typed pick is the verifier's first action/u);
assert.match(web, /only executable Evidence input/u);
assert.match(web, /<exact-readiness\.run_index\.repository_path>/u);
assert.doesNotMatch(web, /readiness-run-index-path|<sealed-digest>|run-feature\.json/u);
assert.match(hostDispatch, /canonical Evidence SOURCE_PATHS digest/u);
assert.match(hostDispatch, /source mismatch fails before trusted receipt commit with zero new receipts/u);
assert.match(roleAssets.find(([name]) => name === "host-dispatch")[1], /Verifier prompts carry the same canonical Evidence SOURCE_PATHS digest requirement as Codex/u);
assert.match(roleAssets.find(([name]) => name === "host-dispatch")[1], /planr evidence run`, which rejects source-mismatched receipts transactionally/u);

const installedAssets = installedHostAssets();
const installedSkillAssets = installedAssets.filter(([relative]) => /(?:^|\/)skills\/planr(?:-[^/]+)?\/SKILL[.]md$/u.test(relative));
const shippedSkillNames = new Set(shippedSkillAssets.map(([name]) => name));
const installedSkillNames = new Set(installedSkillAssets.map(([relative]) => relative.match(/skills\/(planr(?:-[^/]+)?)\/SKILL[.]md$/u)?.[1]).filter(Boolean));
for (const name of shippedSkillNames) {
  assert.ok(installedSkillNames.has(name), `installer mapping must ship ${name}`);
}
for (const [relative, contents] of installedSkillAssets) {
  assert.doesNotMatch(contents, forbiddenLegacyDoneReview, `${relative} must not teach unsupported done --review`);
  if (teachesOutcomeSettlement(contents)) assertCanonicalOutcomeSettlement(relative, contents);
}
for (const [relative, contents] of installedAssets) {
  for (const pattern of forbiddenBlanketReplay) {
    assert.doesNotMatch(contents, pattern, `${relative} must not require blanket replay`);
  }
}
for (const [relative, contents] of installedAssets.filter(([relative]) => /planr-reviewer[.](?:md|toml)$/u.test(relative))) {
  assert.match(contents, /selectively replay only cheap, missing, failing, or explicitly\s+high-risk evidence/u, `${relative} must carry selective replay language`);
  assert.match(contents, /work_packet.kind: "review_gate"/u, `${relative} must consume typed ReviewGate packets`);
}
for (const [relative, contents] of installedAssets.filter(([relative]) => /planr-worker[.](?:md|toml)$/u.test(relative))) {
  assert.match(contents, /compatible\s+same-plan maker run/u, `${relative} must carry compatible same-plan run language`);
  assert.match(contents, /keep one worker identity/u, `${relative} must keep a stable maker identity`);
  assert.match(contents, /internal three-outcome ExecutionBatch atomically/u, `${relative} must preserve the internal cap without a host stop`);
  assert.match(contents, /compact durable handoff/u, `${relative} must require compact durable handoff`);
  assert.match(contents, /`planr done --next` inside the\s+authorized run/u, `${relative} must use fused settlement inside a compatible run`);
  assert.match(contents, /SOURCE_PATHS digest/u, `${relative} must enforce verifier source freeze through canonical Evidence scope`);
  assert.match(contents, /work_packet.kind/u, `${relative} must consume typed outcome packets`);
  assert.match(contents, /No review or fix map item exists/u, `${relative} must hard-cut synthetic repair items`);
  assert.doesNotMatch(contents, forbiddenUnconditionalReview, `${relative} must not require unconditional --review`);
}
for (const [relative, contents] of installedAssets.filter(([relative]) => /planr-loop\/SKILL[.]md$/u.test(relative))) {
  assert.match(contents, /compatible same-plan maker run/u, `${relative} must carry compatible same-plan run language`);
  assert.match(contents, /cheap, missing, failing, or explicitly-high-risk evidence|cheap, missing, failing, or explicitly high-risk evidence/u, `${relative} must carry selective replay language`);
  assert.match(contents, /Do not run readiness or evidence runs before every mutable slice/u, `${relative} must avoid mutable-slice Evidence churn`);
  assert.match(contents, /including default-role dispatch when no generated role exists, set `fork_turns: "none"`/u, `${relative} must hard-cut Codex full-history forks`);
  assert.match(contents, /Spawn each maker or checker role once/u, `${relative} must forbid spawn retry churn`);
  assert.match(contents, /Use one completion-length `wait_agent`/u, `${relative} must forbid polling churn`);
  assert.match(contents, /`timeout_ms: 3600000`/u, `${relative} must use a true completion-length wait`);
  assert.match(contents, /does not implement or inspect product source/u, `${relative} must keep the driver out of product source`);
  assert.match(contents, /route review findings back to that same maker with `followup_task`/u, `${relative} must reuse the maker for review findings`);
  assert.match(contents, /after accepted re-review reuse that maker again/u, `${relative} must reuse the maker after accepted re-review`);
  assert.match(contents, /Its first command is `planr pick --plan <plan-id> --work-type verification --json`/u, `${relative} must hand frozen-source Evidence to a leased verifier`);
  assert.match(contents, /readiness\.run_index\.repository_path/u, `${relative} must name the sole executable readiness path`);
  assert.match(contents, /SOURCE_PATHS.*source digest|SOURCE_PATHS digest/u, `${relative} must enforce verifier source freeze through canonical Evidence scope`);
  assert.match(contents, /selectively rerun(?:s|ning) only invalidated Evidence/u, `${relative} must require selective invalidated Evidence reruns`);
  assert.match(contents, /branches only on the returned typed `work_packet`/u, `${relative} must dispatch typed work packets`);
}

const installedStop = installedAssets.find(([relative]) => relative === ".codex/hooks/planr-codex-stop.sh")?.[1];
assert.ok(installedStop, "Codex install must include the canonical Stop hook");
assert.match(installedStop, /hook owns no workflow policy/u);
assert.match(installedStop, /planr --json stop --input/u);

const evidenceDoc = readRel("apps/docs/content/docs/reference/evidence.mdx");
assert.match(evidenceDoc, /Dogfood and benchmark reruns must choose a committed repository policy profile/u);
assert.match(evidenceDoc, /missing `.planr\/policy.toml` is safe but intentionally conservative/u);
assert.match(evidenceDoc, /<exact-readiness\.run_index\.repository_path>/u);
assert.doesNotMatch(evidenceDoc, /<sealed-digest>|run-feature\.json/u);
const dailyLoopDoc = readRel("apps/docs/content/docs/guides/daily-worker-loop.mdx");
assert.match(dailyLoopDoc, /atomically rolls each internal three-outcome ExecutionBatch/u);
assert.match(dailyLoopDoc, /fresh verification-only worker whose first action is `planr pick --plan <plan-id> --work-type verification --json`/u);
assert.match(dailyLoopDoc, /execute only `readiness\.run_index\.repository_path`/u);
assert.match(dailyLoopDoc, /`planr evidence run` checks the frozen source inside the Evidence transaction before trusted receipt commit/u);
assert.match(dailyLoopDoc, /zero new trusted receipts/u);
assert.match(dailyLoopDoc, /Preserve exactly one final independent product review/u);
assert.match(dailyLoopDoc, /work_packet.transition: "review_gate"/u);
const workPacketDoc = readRel("apps/docs/content/docs/guides/feature-run-work-packets.mdx");
for (const heading of [
  "## 1. Normal compatible outcome",
  "## 2. Protected-risk interrupt",
  "## 3. Frozen-source verification",
  "## 4. Finding and repair on one ReviewGate",
  "## 5. Budget hold",
  "## 6. Missing verification capability",
]) {
  assert.match(workPacketDoc, new RegExp(heading.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&"), "u"), `missing canonical example ${heading}`);
}
assert.match(workPacketDoc, /--escalate protected-risk-discovered/u);
assert.match(workPacketDoc, /--work-type verification/u);
assert.match(workPacketDoc, /<exact-readiness\.run_index\.repository_path>/u);
assert.doesNotMatch(workPacketDoc, /<sealed-digest>|run-feature\.json/u);
assert.match(workPacketDoc, /mode: "finding_repair"/u);
assert.match(workPacketDoc, /work_packet.kind: "hold"/u);
assert.match(workPacketDoc, /status: "blocked"/u);
assert.match(workPacketDoc, /Do not call `planr done` from this packet/u);
assert.doesNotMatch(workPacketDoc, forbiddenSyntheticRepair);
assert.match(readRel("src/evidence/policy.rs"), /pub\(crate\) const SOURCE_PATHS/u);
assert.match(readRel("src/evidence/execution.rs"), /recorded failed attempt .* without trusted receipt/u);

for (const [relative, contents] of readMarkdownCorpus(path.join(root, "apps/docs/content"), path.join(root, "docs"))) {
  for (const pattern of forbiddenBlanketReplay) {
    assert.doesNotMatch(contents, pattern, `${relative} must not document blanket replay`);
  }
  assert.doesNotMatch(contents, forbiddenUnconditionalReview, `${relative} must not document unconditional maker --review`);
}

process.stdout.write(`planr risk-based guidance contract: ok (${shippedSkillAssets.length} shipped skills, 7 source role/dispatch assets, ${installedSkillAssets.length} installed skills, ${installedAssets.length} installed assets, docs corpus)\n`);
