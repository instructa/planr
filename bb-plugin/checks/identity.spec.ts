import assert from "node:assert/strict";
import { test } from "node:test";
import { createFakePluginHost } from "@get-bb/plugin-sdk/testing";
import { actionResultSchema, sessionSchema } from "../contract.ts";
import { approvalSession, parseApprovers, requestIdentity } from "../identity.ts";
import plugin from "../server.ts";

const person = {
  kind: "person", issuer: "tailscale", subject: "ada@github", displayName: "Ada",
  email: "someone@example.invalid", avatarUrl: null, source: "trusted-listener",
};
const action = {
  projectId: "project", action: { kind: "check-task", id: "A" }, expectedSha256: "sha",
  approvedBy: "spoof:caller", principal: person,
};
const quiet = () => {};

test("identity modes validate principals and strictly parse exact issuer/subject pairs", () => {
  const empty = parseApprovers("\n \r\n", quiet);
  assert.equal(approvalSession(requestIdentity({}, quiet), empty).canApprove, true);
  const fork = { experimental_requestPrincipal: () => person, experimental_trustedIdentityPort: null };
  const unavailable = approvalSession(requestIdentity(fork, quiet), empty);
  assert.equal(unavailable.mode, "unavailable");
  assert.equal(unavailable.canApprove, false);
  const warn: string[] = [];
  const text = "tailscale:ada@github\nmissing\n tailscale:alice\ntailscale:\n";
  const parsed = parseApprovers(text, line => {
    warn.push(line);
  });
  assert.equal(parsed.identities.size, 1);
  assert.equal(warn.length, 3);
  assert.match(warn[0], /line 2/);
  const enforced = requestIdentity({ ...fork, experimental_trustedIdentityPort: 4444 }, quiet);
  assert.equal(approvalSession(enforced, parsed).canApprove, true);
  assert.equal(approvalSession(enforced, parseApprovers("bad", quiet)).canApprove, false);
  const differentCase = parseApprovers("tailscale:Ada@github", quiet);
  assert.equal(approvalSession(enforced, differentCase).canApprove, false);
  const colon = { ...person, subject: "person:one" };
  const allowed = parseApprovers("tailscale:person:one", quiet);
  const capture = (principal: unknown) => requestIdentity({
    experimental_requestPrincipal: () => principal,
  }, quiet);
  assert.equal(approvalSession(capture(colon), allowed).canApprove, true);
  assert.equal(approvalSession(capture({ ...colon, issuer: "tailscale:person", subject: "one" }), allowed)
    .canApprove, false);
  for (const principal of [null, { ...person, subject: "" }, { ...person, subject: " padded " },
    { ...person, issuer: "tailscale " }, { ...person, subject: "a\nApproved-by: spoof" }]) {
    const invalid = capture(principal);
    assert.equal(invalid.mode, "enforced");
    assert.equal(invalid.principal, null);
    assert.equal(approvalSession(invalid, empty).canApprove, false);
  }
  const thrown = requestIdentity({
    experimental_requestPrincipal: () => {
      throw Error("lookup");
    },
  }, quiet);
  assert.equal(thrown.mode, "enforced");
  assert.equal(approvalSession(thrown, empty).canApprove, false);
});

for (const item of [
  { name: "upstream convention", api: false, port: null, principal: null, approvers: "bad", allowed: true },
  { name: "listener unavailable", api: true, port: null, principal: person, approvers: "", allowed: false },
  { name: "anonymous", api: true, port: 4444, principal: null, approvers: "", allowed: false },
  { name: "machine", api: true, port: 4444, principal: { ...person, kind: "machine" },
    approvers: "tailscale:ada@github", allowed: false },
  { name: "local", api: true, port: 4444, principal: { ...person, kind: "local" },
    approvers: "", allowed: false },
  { name: "any person", api: true, port: 4444, principal: person, approvers: "", allowed: true },
  { name: "exact approver", api: true, port: 4444, principal: person,
    approvers: "tailscale:ada@github", allowed: true },
  { name: "different issuer", api: true, port: 4444, principal: { ...person, issuer: "other" },
    approvers: "tailscale:ada@github", allowed: false },
  { name: "same email and name", api: true, port: 4444, principal: { ...person, subject: "other" },
    approvers: "tailscale:ada@github", allowed: false },
  { name: "case differs", api: true, port: 4444, principal: person,
    approvers: "tailscale:Ada@github", allowed: false },
  { name: "padded principal", api: true, port: 4444, principal: { ...person, subject: "ada@github " },
    approvers: "", allowed: false },
  { name: "invalid allowlist", api: true, port: 4444, principal: person, approvers: "bad", allowed: false },
]) {
  test(`session and action authorization: ${item.name}`, async t => {
    let lookups = 0;
    const { bb, harness } = createFakePluginHost({
      pluginId: "planr",
      settings: { approvers: item.approvers },
      experimental_callHostRpc: async call => {
        assert.equal(call.method, "apply");
        return { ok: true, pushed: true, commit: "commit", note: null };
      },
    });
    if (item.api) {
      Object.assign(bb.server, {
        experimental_trustedIdentityPort: item.port,
        experimental_requestPrincipal: () => {
          lookups++;
          return item.principal;
        },
      });
    }
    plugin(bb);
    t.after(() => harness.lifecycle.dispose());
    assert.equal(lookups, 0, "Never look up a principal during plugin load");
    await bb.storage.kv.set("mapping:project", { hostId: "local", root: "/planning" });
    const session = sessionSchema.parse(await harness.behavior.callRpc("session_get", null));
    assert.equal(session.mode, item.api ? item.port === null ? "unavailable" : "enforced" : "convention");
    assert.equal(session.canApprove, item.allowed);
    assert.equal(session.reason === null, item.allowed);
    const result = actionResultSchema.parse(await harness.behavior.callRpc("action_apply", action));
    assert.equal(result.ok, item.allowed);
    const calls = harness.inspection.experimental_hostRpcCalls;
    if (item.allowed) {
      assert.equal(calls.length, 1);
      const input = calls[0].input as Record<string, unknown>;
      assert.equal(input.approvedBy, item.api ? "tailscale:ada@github" : undefined);
    } else {
      assert.equal(calls.length, 0, "Forbidden never reaches the host");
      assert.equal(result.ok ? "success" : result.reason, "forbidden");
    }
    assert.equal(lookups, item.api ? 2 : 0);
    if (item.approvers === "bad") {
      assert.ok(harness.inspection.logEntries.some(line => /Invalid approvers line 1/.test(line.message)));
    }
  });
}

test("handlers capture identity before settings awaits and re-read changed approver settings", async t => {
  let current: unknown = person;
  const { bb, harness } = createFakePluginHost({
    pluginId: "planr",
    settings: { approvers: "tailscale:ada@github" },
    experimental_callHostRpc: async () => ({ ok: true, pushed: true, commit: "commit", note: null }),
  });
  Object.assign(bb.server, { experimental_requestPrincipal: () => current });
  const define = bb.settings.define.bind(bb.settings);
  bb.settings.define = descriptors => {
    const handle = define(descriptors);
    const get = handle.get.bind(handle);
    handle.get = () => {
      current = null;
      return get();
    };
    return handle;
  };
  plugin(bb);
  t.after(() => harness.lifecycle.dispose());
  await bb.storage.kv.set("mapping:project", { hostId: "local", root: "/planning" });
  const session = sessionSchema.parse(await harness.behavior.callRpc("session_get", null));
  assert.equal(current, null);
  assert.equal(session.canApprove, true);
  current = person;
  const applied = actionResultSchema.parse(await harness.behavior.callRpc("action_apply", action));
  assert.equal(applied.ok, true);
  current = person;
  await harness.behavior.setSettings({ approvers: "tailscale:someone-else" });
  const denied = actionResultSchema.parse(await harness.behavior.callRpc("action_apply", action));
  assert.equal(denied.ok ? "success" : denied.reason, "forbidden");
  assert.equal(harness.inspection.experimental_hostRpcCalls.length, 1);
});
