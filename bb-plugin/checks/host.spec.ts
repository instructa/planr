import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import type { ExperimentalHostWatchListener } from "@get-bb/plugin-sdk";
import { createFakePluginHost } from "@get-bb/plugin-sdk/testing";
import { experimental_createHostEntryHarness } from "@get-bb/plugin-sdk/testing/host";
import host, { apply, fileSha } from "../host.ts";
import plugin from "../server.ts";
import { boardResultSchema, PLANR_CHANGED } from "../contract.ts";
import { hostContract } from "../host-contract.ts";

const temporary = resolve(import.meta.dirname, "../../artifacts/tmp");
mkdirSync(temporary, { recursive: true });
const task = "---\nid: A\ntitle: Task A\nstatus: review\ntrack: test\ndepends: []\n---\nBody.\n";
const goal = "---\ntitle: Goal G\nstate: draft\nstream: S1\nscope: [A, B]\n---\nGoal body.\n";
const git = (root: string, ...args: string[]) => execFileSync("git", ["-C", root, ...args], {
  encoding: "utf8",
  stdio: ["ignore", "pipe", "pipe"],
}).trim();

function identity(root: string) {
  git(root, "config", "user.name", "Planr Test");
  git(root, "config", "user.email", "planr-test@example.invalid");
}

function fixture(t: TestContext, remote = true) {
  const base = mkdtempSync(join(temporary, "host-"));
  const root = join(base, "main");
  const origin = join(base, "origin.git");
  mkdirSync(root);
  git(root, "init", "-b", "main");
  identity(root);
  mkdirSync(join(root, "tasks"));
  mkdirSync(join(root, "plans"));
  writeFileSync(join(root, "tasks/A.md"), task);
  writeFileSync(join(root, "tasks/B.md"), task.replace(/A/g, "B")
    .replace("review", "open").replace("depends: []", "depends: []\nowner: S1"));
  writeFileSync(join(root, "plans/g-goal.md"), goal);
  writeFileSync(join(root, "plans/closed-goal.md"), goal.replace("draft", "review"));
  git(root, "add", ".");
  git(root, "commit", "-m", "Initial planning");
  if (remote) {
    mkdirSync(origin);
    git(origin, "init", "--bare", "-b", "main");
    git(root, "remote", "add", "origin", origin);
    git(root, "push", "-u", "origin", "main");
  }
  t.after(() => rmSync(base, { recursive: true, force: true }));
  return { root, base, origin, ui: `${root}-planr-ui` };
}

function mainState(root: string) {
  return {
    head: git(root, "rev-parse", "HEAD"),
    status: git(root, "status", "--porcelain"),
    task: readFileSync(join(root, "tasks/A.md"), "utf8"),
    goal: readFileSync(join(root, "plans/g-goal.md"), "utf8"),
  };
}

function rejectWithRace(root: string, base: string, changeTarget: boolean) {
  const peer = join(base, "peer");
  git(base, "clone", join(base, "origin.git"), peer);
  identity(peer);
  const race = join(base, "race.mjs");
  const file = changeTarget ? "tasks/A.md" : "notes.md";
  writeFileSync(race, `
import { existsSync, writeFileSync, readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
const marker = ${JSON.stringify(join(base, "raced"))};
if (!existsSync(marker)) {
  writeFileSync(marker, 'once');
  const file = ${JSON.stringify(join(peer, file))};
  const original = ${changeTarget ? "readFileSync(file, 'utf8')" : "''"};
  const content = original + ${JSON.stringify(changeTarget ? 'Changed remotely.\n' : 'Other file.\n')};
  writeFileSync(file, content);
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('GIT_')));
  const git = (...args) => execFileSync('git', ['-C', ${JSON.stringify(peer)}, ...args], { env });
  git('add', '.');
  git('commit', '-m', 'Concurrent change');
  git('push', 'origin', 'main');
}
`);
  const quote = (text: string) => `'${text.replace(/'/g, "'\\''")}'`;
  writeFileSync(join(root, ".git/hooks/pre-push"), `#!/bin/sh\n${quote(process.execPath)} ${quote(race)}\n`, {
    mode: 0o755,
  });
}

test("human actions commit one line, push with trailer and fast-forward a clean main checkout", async t => {
  const { root, origin, ui } = fixture(t);
  for (const [action, file, expected] of [
    [{ kind: "approve-goal", goal: "g" }, "plans/g-goal.md", goal.replace("draft", "approved")],
    [{ kind: "close-goal", goal: "closed" }, "plans/closed-goal.md", goal.replace("draft", "done")],
    [{ kind: "check-task", id: "A" }, "tasks/A.md", task.replace("review", "done")],
  ] as const) {
    const result = await apply(root, action, fileSha(join(root, file)));
    if (!result.ok) {
      throw Error(result.message);
    }
    assert.equal(result.pushed, true);
    assert.equal(git(origin, "rev-parse", "main"), result.commit);
    assert.equal(readFileSync(join(ui, file), "utf8"), expected);
    assert.equal(git(ui, "show", "--format=", "--name-only", result.commit), file);
    assert.match(git(ui, "log", "-1", "--format=%B"), /Approved-via: planr-ui/);
    assert.doesNotMatch(git(ui, "log", "-1", "--format=%B"), /^Approved-by:/m);
    assert.equal(git(ui, "status", "--porcelain"), "");
    assert.equal(git(root, "rev-parse", "HEAD"), result.commit);
    assert.equal(readFileSync(join(root, file), "utf8"), expected);
    assert.equal(git(root, "status", "--porcelain"), "");
  }
});

test("host apply appends the server-approved identity trailer and rejects multiline attribution", async t => {
  const { root, origin } = fixture(t);
  const harness = experimental_createHostEntryHarness(host);
  t.after(() => harness.experimental_dispose());
  const input = {
    root, action: { kind: "check-task", id: "A" } as const,
    expectedSha256: fileSha(join(root, "tasks/A.md")), approvedBy: "tailscale:ada@github",
  };
  const before = mainState(root);
  for (const approvedBy of ["a\nApproved-by: b", "a\n", "a\r", "a\0"]) {
    await assert.rejects(harness.experimental_call("apply", { ...input, approvedBy }));
  }
  assert.deepEqual(mainState(root), before);
  const result = await harness.experimental_call("apply", input);
  if (!result.ok) {
    throw Error(result.message);
  }
  assert.equal(git(origin, "rev-parse", "main"), result.commit);
  assert.equal(git(root, "log", "-1", "--format=%B"), [
    "planr: check-task A", "", "Approved-via: planr-ui", "Approved-by: tailscale:ada@github",
  ].join("\n"));
});

test("offline actions advance main cumulatively and preserve other staged and unstaged files", async t => {
  const { root, ui } = fixture(t, false);
  writeFileSync(join(root, "notes.md"), "Initial notes.\n");
  git(root, "add", "notes.md");
  git(root, "commit", "-m", "Add notes");
  writeFileSync(join(root, "notes.md"), "Staged notes.\n");
  git(root, "add", "notes.md");
  writeFileSync(join(root, "tasks/B.md"), readFileSync(join(root, "tasks/B.md")) + "Local edit.\n");
  writeFileSync(join(root, "scratch.txt"), "Untracked work.\n");
  const before = mainState(root);
  const staged = git(root, "diff", "--cached", "--binary");
  const unstaged = git(root, "diff", "--binary");
  let previous = before.head;
  for (const [action, file, expected] of [
    [{ kind: "check-task", id: "A" }, "tasks/A.md", task.replace("review", "done")],
    [{ kind: "approve-goal", goal: "g" }, "plans/g-goal.md", goal.replace("draft", "approved")],
  ] as const) {
    const result = await apply(root, action, fileSha(join(root, file)));
    if (!result.ok) {
      throw Error(result.message);
    }
    assert.equal(result.pushed, false);
    assert.match(result.note!, /no remote/);
    assert.equal(git(root, "rev-parse", "HEAD"), result.commit);
    assert.equal(git(root, "rev-parse", "HEAD^"), previous);
    assert.equal(git(root, "show", "--format=", "--name-only", result.commit), file);
    assert.match(git(root, "log", "-1", "--format=%B"), /Approved-via: planr-ui/);
    assert.equal(readFileSync(join(root, file), "utf8"), expected);
    assert.equal(git(root, "status", "--porcelain"), before.status);
    assert.equal(git(root, "diff", "--cached", "--binary"), staged);
    assert.equal(git(root, "diff", "--binary"), unstaged);
    previous = result.commit;
  }
  assert.equal(readFileSync(join(root, "tasks/A.md"), "utf8"), task.replace("review", "done"));
  assert.equal(readFileSync(join(root, "scratch.txt"), "utf8"), "Untracked work.\n");
  assert.equal(existsSync(ui), false);
});

test("offline stale SHA and staged, unstaged or untracked target files cause conflict", async t => {
  const { root } = fixture(t, false);
  const file = join(root, "tasks/A.md");
  const action = { kind: "check-task", id: "A" } as const;
  const before = mainState(root);
  const stale = await apply(root, action, "old-sha");
  assert.equal(stale.ok ? "success" : stale.reason, "conflict");
  assert.deepEqual(mainState(root), before);
  for (const staged of [false, true]) {
    writeFileSync(file, task + "Local change.\n");
    if (staged) {
      git(root, "add", "tasks/A.md");
    }
    const state = mainState(root);
    const result = await apply(root, action, fileSha(file));
    assert.equal(result.ok ? "success" : result.reason, "conflict");
    assert.deepEqual(mainState(root), state);
    git(root, "restore", "--staged", "--worktree", "tasks/A.md");
  }
  const untracked = join(root, "tasks/C.md");
  writeFileSync(untracked, task.replace(/A/g, "C"));
  const state = mainState(root);
  const result = await apply(root, { kind: "check-task", id: "C" }, fileSha(untracked));
  assert.equal(result.ok ? "success" : result.reason, "conflict");
  assert.deepEqual(mainState(root), state);
  assert.equal(readFileSync(untracked, "utf8"), task.replace(/A/g, "C"));
});

test("remote actions reject local target edits and changed or missing main files", async t => {
  const { root, origin, ui } = fixture(t);
  const file = join(root, "tasks/A.md");
  const originalSha = fileSha(file);
  const head = git(origin, "rev-parse", "main");
  writeFileSync(file, task + "Manual local edit.\n");
  for (const staged of [false, true]) {
    if (staged) {
      git(root, "add", "tasks/A.md");
    }
    for (const sha of [originalSha, fileSha(file)]) {
      const before = mainState(root);
      const result = await apply(root, { kind: "check-task", id: "A" }, sha);
      assert.equal(result.ok ? "success" : result.reason, "conflict");
      assert.deepEqual(mainState(root), before);
      assert.equal(git(origin, "rev-parse", "main"), head);
      assert.equal(existsSync(ui), false);
    }
  }
  git(root, "commit", "-m", "Commit local target edit");
  const before = mainState(root);
  const result = await apply(root, { kind: "check-task", id: "A" }, originalSha);
  assert.equal(result.ok ? "success" : result.reason, "conflict");
  assert.deepEqual(mainState(root), before);
  assert.equal(git(origin, "rev-parse", "main"), head);
  assert.equal(existsSync(ui), false);
  rmSync(file);
  const status = git(root, "status", "--porcelain");
  const deleted = await apply(root, { kind: "check-task", id: "A" }, originalSha);
  assert.equal(deleted.ok ? "success" : deleted.reason, "conflict");
  assert.equal(existsSync(file), false);
  assert.equal(git(root, "status", "--porcelain"), status);
  assert.equal(git(root, "rev-parse", "HEAD"), before.head);
  assert.equal(git(origin, "rev-parse", "main"), head);
  assert.equal(git(ui, "status", "--porcelain"), "");
});

for (const dirty of [false, true]) {
  test(`successful pushes refresh main before signaling (${dirty ? "dirty" : "clean"})`, async t => {
    const { root, origin } = fixture(t);
    if (dirty) {
      writeFileSync(join(root, "tasks/B.md"), readFileSync(join(root, "tasks/B.md")) + "Other edit.\n");
      git(root, "add", "tasks/B.md");
      writeFileSync(join(root, "scratch.txt"), "Untracked work.\n");
    }
    const before = mainState(root);
    const staged = git(root, "diff", "--cached", "--binary");
    const harness = experimental_createHostEntryHarness(host);
    t.after(() => harness.experimental_dispose());
    const result = await harness.experimental_call("apply", {
      root, action: { kind: "check-task", id: "A" }, expectedSha256: fileSha(join(root, "tasks/A.md")),
    });
    if (!result.ok) {
      throw Error(result.message);
    }
    assert.equal(result.pushed, true);
    assert.equal(git(origin, "rev-parse", "main"), result.commit);
    assert.deepEqual(harness.experimental_getSignals(), [{ signal: "changed", payload: { root } }]);
    const board = await harness.experimental_call("board", { root, hostId: "local" });
    if (board.status !== "ok") {
      throw Error("Expected refreshed board");
    }
    assert.equal(board.repo.behind, dirty ? 1 : 0);
    assert.equal(board.repo.ahead, 0);
    assert.equal(board.repo.dirty, dirty);
    if (dirty) {
      assert.match(result.note!, /dirty.*skipped/);
      assert.deepEqual(mainState(root), before);
      assert.equal(git(root, "diff", "--cached", "--binary"), staged);
      assert.equal(readFileSync(join(root, "scratch.txt"), "utf8"), "Untracked work.\n");
    } else {
      assert.equal(board.repo.head, result.commit);
      assert.equal(readFileSync(join(root, "tasks/A.md"), "utf8"), task.replace("review", "done"));
    }
  });
}

test("a pushed action stays successful when a divergent main checkout cannot fast-forward", async t => {
  const { root, origin } = fixture(t);
  writeFileSync(join(root, "notes.md"), "Local committed work.\n");
  git(root, "add", "notes.md");
  git(root, "commit", "-m", "Local work");
  const before = mainState(root);
  const harness = experimental_createHostEntryHarness(host);
  t.after(() => harness.experimental_dispose());
  const result = await harness.experimental_call("apply", {
    root, action: { kind: "check-task", id: "A" }, expectedSha256: fileSha(join(root, "tasks/A.md")),
  });
  if (!result.ok) {
    throw Error(result.message);
  }
  assert.equal(result.pushed, true);
  assert.equal(git(origin, "rev-parse", "main"), result.commit);
  assert.match(result.note!, /checkout refresh failed/);
  assert.deepEqual(mainState(root), before);
  const board = await harness.experimental_call("board", { root, hostId: "local" });
  if (board.status !== "ok") {
    throw Error("Expected board");
  }
  assert.equal(board.repo.ahead, 1);
  assert.equal(board.repo.behind, 1);
  assert.equal(board.repo.dirty, false);
  assert.equal(harness.experimental_getSignals().length, 1);
});

test("mapping a non-planning folder returns board copy instead of the engine CLI hint", async t => {
  const { root, base } = fixture(t, false);
  const hostHarness = experimental_createHostEntryHarness(host);
  const { bb, harness } = createFakePluginHost({
    pluginId: "planr",
    sdk: {
      projects: {
        get: async () => ({
          id: "project", name: "Test", kind: "standard", createdAt: 0, updatedAt: 0,
          gitRemoteUrl: null, sources: [],
        }),
      },
    },
    experimental_callHostRpc: call =>
      hostHarness.experimental_call("check", hostContract.check.input.parse(call.input)),
  });
  plugin(bb);
  t.after(async () => {
    await harness.lifecycle.dispose();
    await hostHarness.experimental_dispose();
  });
  for (const folder of [join(base, "not-planning"), join(root, "not-planning")]) {
    mkdirSync(folder);
    const result = await harness.behavior.callRpc("mapping_set", {
      projectId: "project", mapping: { hostId: "local", root: folder },
    });
    assert.deepEqual(result, {
      ok: false,
      message: "This folder is not a planr planning repo (no tasks/ folder or planr.config.json found).",
    });
    assert.equal(await bb.storage.kv.get("mapping:project"), undefined);
  }
});

for (const remote of [false, true]) {
  test(`nested planning roots use git-relative paths (${remote ? "remote" : "offline"})`, async t => {
    const { root, origin, ui } = fixture(t, remote);
    const planning = join(root, "planning");
    mkdirSync(planning);
    git(root, "mv", "tasks", "planning/tasks");
    git(root, "mv", "plans", "planning/plans");
    git(root, "commit", "-m", "Nest planning root");
    if (remote) {
      git(root, "push");
    }
    const file = "planning/tasks/A.md";
    const result = await apply(planning, { kind: "check-task", id: "A" }, fileSha(join(root, file)));
    if (!result.ok) {
      throw Error(result.message);
    }
    assert.equal(result.pushed, remote);
    const checkout = remote ? ui : root;
    assert.equal(readFileSync(join(checkout, file), "utf8"), task.replace("review", "done"));
    assert.equal(git(checkout, "show", "--format=", "--name-only", result.commit), file);
    assert.equal(existsSync(`${planning}-planr-ui`), false);
    assert.equal(git(root, "status", "--porcelain"), "");
    if (remote) {
      assert.equal(git(origin, "rev-parse", "main"), result.commit);
      assert.equal(git(root, "rev-parse", "HEAD"), result.commit);
      assert.equal(readFileSync(join(root, file), "utf8"), task.replace("review", "done"));
    } else {
      assert.equal(git(root, "rev-parse", "HEAD"), result.commit);
      assert.equal(existsSync(ui), false);
    }
  });
}

test("stale SHA and invalid transitions write nothing; dirty plugin worktrees are refused", async t => {
  const { root, ui, origin } = fixture(t);
  const before = mainState(root);
  const head = git(origin, "rev-parse", "main");
  const conflict = await apply(root, { kind: "check-task", id: "A" }, "old-sha");
  assert.equal(conflict.ok ? "success" : conflict.reason, "conflict");
  const invalid = await apply(root, { kind: "check-task", id: "B" }, fileSha(join(root, "tasks/B.md")));
  assert.equal(invalid.ok ? "success" : invalid.reason, "invalid-transition");
  writeFileSync(join(root, ".git/hooks/pre-commit"), "#!/bin/sh\nexit 1\n", { mode: 0o755 });
  const refused = await apply(root, { kind: "check-task", id: "A" }, fileSha(join(root, "tasks/A.md")));
  assert.equal(refused.ok ? "success" : refused.reason, "host-error");
  assert.equal(git(ui, "status", "--porcelain"), "");
  assert.equal(readFileSync(join(ui, "tasks/A.md"), "utf8"), task);
  writeFileSync(join(ui, "tasks/A.md"), task + "Local worktree edit.\n");
  const dirty = await apply(root, { kind: "check-task", id: "A" }, fileSha(join(root, "tasks/A.md")));
  assert.equal(dirty.ok ? "success" : dirty.reason, "host-error");
  if (!dirty.ok) {
    assert.match(dirty.message, /dirty/);
  }
  assert.match(readFileSync(join(ui, "tasks/A.md"), "utf8"), /Local worktree edit/);
  assert.equal(git(origin, "rev-parse", "main"), head);
  assert.deepEqual(mainState(root), before);
});

for (const changed of [false, true]) {
  const outcome = changed ? "aborts for changed SHA" : "rebases and retries";
  test(`push rejection refetches and ${outcome}`, async t => {
    const { root, base, origin, ui } = fixture(t);
    rejectWithRace(root, base, changed);
    const before = mainState(root);
    const result = await apply(root, { kind: "check-task", id: "A" }, fileSha(join(root, "tasks/A.md")));
    assert.equal(readFileSync(join(base, "raced"), "utf8"), "once");
    assert.equal(result.ok, !changed);
    if (changed) {
      if (result.ok) {
        throw Error("Expected push rejection");
      }
      assert.equal(result.reason, "push-rejected");
      assert.match(git(origin, "show", "main:tasks/A.md"), /status: review/);
    } else {
      if (!result.ok) {
        throw Error(result.message);
      }
      assert.equal(git(origin, "rev-parse", "main"), result.commit);
      assert.equal(git(origin, "show", "main:notes.md"), "Other file.");
      assert.match(git(origin, "show", "main:tasks/A.md"), /status: done/);
    }
    assert.equal(git(ui, "status", "--porcelain"), "");
    if (changed) {
      assert.deepEqual(mainState(root), before);
    } else if (result.ok) {
      assert.equal(git(root, "rev-parse", "HEAD"), result.commit);
      assert.equal(readFileSync(join(root, "tasks/A.md"), "utf8"), task.replace("review", "done"));
    }
  });
}

test("host wire data uses the engine and emits changes; refresh skips a dirty main checkout", async t => {
  const { root } = fixture(t);
  writeFileSync(join(root, "plans/g-goal.md"), goal.replace("draft", "approved"));
  git(root, "commit", "-am", "Approve fixture goal");
  git(root, "push");
  const harness = experimental_createHostEntryHarness(host);
  t.after(() => harness.experimental_dispose());
  const board = await harness.experimental_call("board", { root, hostId: "host-local" });
  assert.equal(board.status, "ok");
  if (board.status !== "ok") {
    throw Error("Expected board");
  }
  assert.equal(board.repo.hostId, "host-local");
  assert.equal(board.fileSha["tasks/A.md"], fileSha(join(root, "tasks/A.md")));
  assert.equal((board.data.tasks as unknown[]).length, 2);
  assert.equal((await harness.experimental_call("check", { root })).errors.length, 0);
  const next = await harness.experimental_call("next", { root, goal: "g" });
  assert.equal(next.ready.length, 1);
  assert.equal((await harness.experimental_call("next", { root, stream: "S1" })).ready.length, 1);
  const result = await harness.experimental_call("apply", {
    root, action: { kind: "check-task", id: "A" }, expectedSha256: board.fileSha["tasks/A.md"],
  });
  assert.equal(result.ok, true);
  writeFileSync(join(root, "tasks/A.md"), task + "Local body.\n");
  const refresh = await harness.experimental_call("refresh", { root });
  assert.match(refresh.message!, /dirty/);
  assert.match(readFileSync(join(root, "tasks/A.md"), "utf8"), /Local body/);
  assert.ok(harness.experimental_getSignals().length >= 2);
});

test("origin-first SHA comparison rejects a changed remote file even when main is unchanged", async t => {
  const { root, origin, base } = fixture(t);
  const before = mainState(root);
  const sha = fileSha(join(root, "tasks/A.md"));
  const peer = join(base, "peer");
  git(base, "clone", origin, peer);
  identity(peer);
  writeFileSync(join(peer, "tasks/A.md"), task + "Changed on origin.\n");
  git(peer, "add", ".");
  git(peer, "commit", "-m", "Remote edit");
  git(peer, "push");
  const head = git(origin, "rev-parse", "main");
  const result = await apply(root, { kind: "check-task", id: "A" }, sha);
  assert.equal(result.ok ? "success" : result.reason, "conflict");
  assert.equal(git(origin, "rev-parse", "main"), head);
  assert.deepEqual(mainState(root), before);
});

test("concurrent actions on a repo are serialized without dropping either change", async t => {
  const { root, origin } = fixture(t);
  const results = await Promise.all([
    apply(root, { kind: "approve-goal", goal: "g" }, fileSha(join(root, "plans/g-goal.md"))),
    apply(root, { kind: "check-task", id: "A" }, fileSha(join(root, "tasks/A.md"))),
  ]);
  assert.ok(results.every(result => result.ok));
  assert.match(git(origin, "show", "main:plans/g-goal.md"), /state: approved/);
  assert.match(git(origin, "show", "main:tasks/A.md"), /status: done/);
  assert.equal(git(root, "rev-parse", "HEAD"), git(origin, "rev-parse", "main"));
  assert.equal(readFileSync(join(root, "tasks/A.md"), "utf8"), task.replace("review", "done"));
  assert.equal(readFileSync(join(root, "plans/g-goal.md"), "utf8"), goal.replace("draft", "approved"));
});

test("native watches debounce, heartbeat reuses subscriptions and unwatch disposes them", async t => {
  const { root } = fixture(t, false);
  const listeners = new Map<string, (event: { kind: "rescan-required" }) => void | Promise<void>>();
  let disposals = 0;
  const harness = experimental_createHostEntryHarness(host, {
    experimental_watch: (options, listener) => {
      assert.equal(options.debounceMs, 200);
      listeners.set(options.rootPath, listener);
      return { dispose: async () => { disposals++; } };
    },
  });
  t.after(() => harness.experimental_dispose());
  await harness.experimental_call("watch", { root, key: "project" });
  await harness.experimental_call("watch", { root, key: "project" });
  assert.equal(listeners.size, 3);
  await listeners.get(join(root, "tasks"))!({ kind: "rescan-required" });
  assert.deepEqual(harness.experimental_getSignals().at(-1), { signal: "changed", payload: { root } });
  await harness.experimental_call("unwatch", { key: "project" });
  assert.equal(disposals, 3);
});

test("native watch changes reach server realtime through the SDK host and plugin harnesses", async t => {
  const { root } = fixture(t, false);
  const listeners = new Map<string, ExperimentalHostWatchListener>();
  const hostHarness = experimental_createHostEntryHarness(host, {
    experimental_watch: (options, listener) => {
      listeners.set(options.rootPath, listener);
      return { dispose: async () => { listeners.delete(options.rootPath); } };
    },
  });
  const { bb, harness } = createFakePluginHost({
    pluginId: "planr",
    experimental_callHostRpc: async call => {
      switch (call.method) {
        case "board":
          return hostHarness.experimental_call("board", hostContract.board.input.parse(call.input));
        case "watch":
          return hostHarness.experimental_call("watch", hostContract.watch.input.parse(call.input));
        case "unwatch":
          return hostHarness.experimental_call("unwatch", hostContract.unwatch.input.parse(call.input));
        default:
          throw Error(`Unexpected host call: ${call.method}`);
      }
    },
  });
  plugin(bb);
  t.after(async () => {
    await harness.lifecycle.dispose();
    await hostHarness.experimental_dispose();
  });
  await bb.storage.kv.set("mapping:project", { hostId: "local-host", root });
  await bb.storage.kv.set("mapping:other-host", { hostId: "other-host", root });
  await bb.storage.kv.set("mapping:other-root", { hostId: "local-host", root: join(root, "elsewhere") });
  const board = boardResultSchema.parse(
    await harness.behavior.callRpc("board_get", { projectId: "project" })
  );
  assert.equal(board.status, "ok");
  assert.equal(listeners.size, 3);
  writeFileSync(join(root, "tasks/A.md"), task + "Watched body edit.\n");
  await listeners.get(join(root, "tasks"))!({
    kind: "changed", changes: [{ path: "A.md", type: "update" }],
  });
  const signals = hostHarness.experimental_getSignals();
  assert.deepEqual(signals, [{ signal: "changed", payload: { root } }]);
  // Bridge actual validated host emissions as the daemon would; do not invent a changed payload.
  for (const signal of signals) {
    await harness.behavior.experimental_emitHostSignal("local-host", signal.signal, signal.payload);
  }
  assert.deepEqual(harness.inspection.realtimeSignals, [
    { channel: PLANR_CHANGED, payload: { projectId: "project" } },
  ]);
});
