import assert from "node:assert/strict";
import { test } from "node:test";
import { createFakePluginHost, makeThreadResponse } from "@get-bb/plugin-sdk/testing";
import plugin from "../server.ts";
import { PLANR_CHANGED } from "../contract.ts";

const project = {
  id: "project-one",
  name: "Planning Test",
  kind: "standard" as const,
  createdAt: 0,
  updatedAt: 0,
  gitRemoteUrl: null,
  sources: [{ id: "source-one", hostId: "host-one", path: "/planning", projectId: "project-one",
    type: "local_path" as const, isDefault: true, createdAt: 0, updatedAt: 0 }],
};

test("thread lookup: personal, deleted and missing return null; no host calls", async t => {
  const lookups: string[] = [];
  const { bb, harness } = createFakePluginHost({
    pluginId: "planr",
    sdk: {
      threads: {
        get: async ({ threadId }) => {
          if (threadId === "missing") {
            throw Error("Thread not found");
          }
          return makeThreadResponse({
            id: threadId,
            projectId: threadId === "no-project" ? "" : threadId,
            deletedAt: threadId === "deleted" ? 1 : null,
          });
        },
      },
      projects: {
        get: async ({ projectId }) => {
          lookups.push(projectId);
          if (projectId === "missing-project") {
            throw Error("Project not found");
          }
          return { ...project, id: projectId, kind: projectId === "personal" ? "personal" : "standard" };
        },
      },
    },
  });
  plugin(bb);
  t.after(() => harness.lifecycle.dispose());
  for (const threadId of ["standard", "personal", "no-project", "deleted", "missing", "missing-project"]) {
    assert.deepEqual(await harness.behavior.callRpc("thread_project_get", { threadId }), {
      projectId: threadId === "standard" ? "standard" : null,
    });
  }
  assert.deepEqual(lookups, ["standard", "personal", "missing-project"]);
  assert.equal(harness.inspection.experimental_hostRpcCalls.length, 0);
});

test("mapping is host validated before storing; signals and worker exits invalidate and re-arm", async t => {
  let valid = false;
  const { bb, harness } = createFakePluginHost({
    pluginId: "planr",
    sdk: { projects: { get: async () => project, list: async () => [project] } },
    experimental_callHostRpc: async call => {
      if (call.method === "check") {
        if (!valid) {
          throw Error("Planning root not found.");
        }
        return {
          root: "/canonical/planning",
          errors: [],
          warnings: ["A: without owner"],
          counts: { open: 2 },
        };
      }
      if (call.method === "watch" || call.method === "unwatch") {
        return null;
      }
      throw Error(`Unexpected method: ${call.method}`);
    },
  });
  plugin(bb);
  t.after(() => harness.lifecycle.dispose());
  const input = { projectId: project.id, mapping: { hostId: "host-one", root: "/planning" } };
  const rejected = await harness.behavior.callRpc("mapping_set", input);
  assert.deepEqual(rejected, { ok: false, message: "Planning root not found." });
  assert.equal(await bb.storage.kv.get(`mapping:${project.id}`), undefined);
  assert.equal(harness.inspection.experimental_hostRpcCalls.length, 1);
  valid = true;
  assert.deepEqual(await harness.behavior.callRpc("mapping_set", input), { ok: true, message: null });
  assert.deepEqual(await bb.storage.kv.get(`mapping:${project.id}`), {
    hostId: "host-one", root: "/canonical/planning",
  });
  const signals = harness.inspection.realtimeSignals;
  const initial = signals.length;
  await harness.behavior.experimental_emitHostSignal("other-host", "changed", {
    root: "/canonical/planning",
  });
  await harness.behavior.experimental_emitHostSignal("host-one", "changed", { root: "/other-root" });
  assert.equal(signals.length, initial);
  await harness.behavior.experimental_emitHostSignal("host-one", "changed", {
    root: "/canonical/planning",
  });
  assert.deepEqual(signals.at(-1), { channel: PLANR_CHANGED, payload: { projectId: project.id } });
  await harness.behavior.experimental_emitHostWorkerExit("host-one");
  assert.equal(harness.inspection.experimental_hostRpcCalls.at(-1)?.method, "watch");
  assert.equal(signals.length, initial + 2);
  assert.equal(harness.inspection.registrations.agentTools.length, 0);
});

test("CLI resolves project context or --project, refuses writes and bounds warning output", async t => {
  const { bb, harness } = createFakePluginHost({
    pluginId: "planr",
    experimental_callHostRpc: async call => {
      if (call.method === "check") {
        return { root: "/planning", errors: [], warnings: Array(300).fill("x".repeat(300)), counts: {} };
      }
      if (call.method === "next") {
        return { ready: [{ id: "A", title: "Next task", unlocks: 2 }],
        waiting: [], errors: [], warnings: [] };
      }
      if (call.method === "unwatch") {
        return null;
      }
      throw Error(`Unexpected method: ${call.method}`);
    },
  });
  plugin(bb);
  t.after(() => harness.lifecycle.dispose());
  await bb.storage.kv.set(`mapping:${project.id}`, { hostId: "host-one", root: "/planning" });
  const absent = await harness.behavior.runCli(["check"]);
  assert.equal(absent.exitCode, 1);
  assert.match(absent.stderr, /--project/);
  const check = await harness.behavior.runCli(["check"], { projectId: project.id });
  assert.equal(check.exitCode, 0, check.stderr);
  assert.ok(Buffer.byteLength(check.stdout) < 16_100);
  assert.match(check.stdout, /truncated/);
  const next = await harness.behavior.runCli(["next", "--stream", "S1", "--project", project.id]);
  assert.equal(next.exitCode, 0, next.stderr);
  assert.match(next.stdout, /ready A: Next task/);
  assert.equal(harness.inspection.experimental_hostRpcCalls.at(-1)?.hostId, "host-one");
  const before = harness.inspection.experimental_hostRpcCalls.length;
  const write = await harness.behavior.runCli(["set", "A", "done"], { projectId: project.id });
  assert.equal(write.exitCode, 1);
  assert.equal(harness.inspection.experimental_hostRpcCalls.length, before);
});

test("folder picker falls back to the requested host when no client host is supplied", async t => {
  const picked: { hostId: string; clientHostId: string }[] = [];
  const { bb, harness } = createFakePluginHost({
    pluginId: "planr",
    sdk: {
      hosts: {
        pickFolder: async args => {
          picked.push(args);
          return { path: "/picked/planning" };
        },
      },
    },
  });
  plugin(bb);
  t.after(() => harness.lifecycle.dispose());
  for (const clientHostId of [null, "client-host"]) {
    const result = await harness.behavior.callRpc("mapping_pick_folder", {
      projectId: "project", hostId: "requested-host", clientHostId,
    });
    assert.deepEqual(result, { path: "/picked/planning" });
    assert.deepEqual(picked.at(-1), {
      hostId: "requested-host", clientHostId: clientHostId ?? "requested-host",
    });
  }
});
