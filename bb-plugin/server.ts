import type { BbPluginApi, PluginCliContext } from "@get-bb/plugin-sdk";
import { mappingSchema, PLANR_CHANGED, rpcContract, type Mapping } from "./contract.ts";
import { hostContract, hostSignals } from "./host-contract.ts";
import { approvalSession, parseApprovers, requestIdentity } from "./identity.ts";

export { rpcContract } from "./contract.ts";
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

// Bound growing collections by UTF-8 bytes, preserving whole lines and a truncation notice.
function output(lines: string[]) {
  const selected: string[] = [];
  let bytes = 0;
  for (const line of lines) {
    const text = line.replace(/[\r\n]/g, " ").slice(0, 1000);
    bytes += Buffer.byteLength(text) + 1;
    if (bytes > 16_000) {
      selected.push("planr: output truncated; open Planr in the sidebar");
      break;
    }
    selected.push(text);
  }
  return selected.join("\n") + "\n";
}

function argumentsFor(argv: string[], context: PluginCliContext) {
  const [command, ...args] = argv;
  if (!["next", "check", "board"].includes(command)) {
    throw Error("Use next, check or board.");
  }
  const options: Record<string, string> = {};
  for (let i = 0; i < args.length; i += 2) {
    const key = args[i].replace(/^--/, "");
    const allowed = command === "next" ? ["project", "goal", "stream"] : ["project"];
    if (!args[i].startsWith("--") || !allowed.includes(key) || options[key] || !args[i + 1]) {
      throw Error(`Invalid option: ${args[i]}`);
    }
    if (args[i + 1].startsWith("--")) {
      throw Error(`Missing value for ${args[i]}`);
    }
    options[key] = args[i + 1];
  }
  const projectId = options.project || context.projectId;
  if (!projectId) {
    throw Error("No project context; pass --project <projectId>.");
  }
  if (command === "next" && !!options.goal === !!options.stream) {
    throw Error("next requires exactly one of --goal or --stream.");
  }
  return { command, projectId, goal: options.goal, stream: options.stream };
}

export default function plugin(bb: BbPluginApi) {
  const settings = bb.settings.define({
    approvers: {
      type: "string",
      label: "Approvers",
      description: "One issuer:subject per line. Empty allows any identified person.",
      experimental_multiline: true,
      default: "",
    },
  });
  const identityServer = bb.server as typeof bb.server & Parameters<typeof requestIdentity>[0];
  const warn = (message: string) => bb.log.warn(message);
  let previousApprovers: string | undefined;
  let approvers = parseApprovers("", warn);

  async function session(identity: ReturnType<typeof requestIdentity>) {
    const values = await settings.get();
    if (values.approvers !== previousApprovers) {
      approvers = parseApprovers(values.approvers, warn);
      previousApprovers = values.approvers;
    }
    return approvalSession(identity, approvers);
  }

  const host = bb.hosts.experimental_client({ contract: hostContract, experimental_signals: hostSignals });
  const watched = new Map<string, Mapping>();
  let disposed = false;

  async function mapping(projectId: string) {
    const stored = mappingSchema.safeParse(await bb.storage.kv.get(`mapping:${projectId}`));
    return stored.success ? stored.data : null;
  }

  async function mappings() {
    const keys = await bb.storage.kv.list("mapping:");
    return Promise.all(keys.map(async key => ({
      projectId: key.slice(8),
      mapping: await mapping(key.slice(8)),
    })));
  }

  async function arm(projectId: string, selected: Mapping) {
    if (disposed) {
      return;
    }
    const old = watched.get(projectId);
    watched.set(projectId, selected);
    if (old && (old.hostId !== selected.hostId || old.root !== selected.root)) {
      try {
        await host.call("unwatch", { key: projectId }, { hostId: old.hostId });
      } catch (error) {
        bb.log.warn(`Watch cleanup unavailable: ${message(error)}`);
      }
    }
    await host.call("watch", { key: projectId, root: selected.root }, { hostId: selected.hostId });
  }

  async function changed(hostId: string, root?: string, rearm = false) {
    for (const item of await mappings()) {
      const selected = item.mapping;
      if (!selected || selected.hostId !== hostId || (root && selected.root !== root)) {
        continue;
      }
      bb.realtime.publish(PLANR_CHANGED, { projectId: item.projectId });
      if (rearm) {
        try {
          await arm(item.projectId, selected);
        } catch (error) {
          bb.log.warn(`Watch unavailable: ${message(error)}`);
        }
      }
    }
  }

  const offSignal = host.experimental_onSignal("changed", ({ hostId, payload }) =>
    changed(hostId, payload.root)
  );
  const offExit = host.experimental_onWorkerExit(({ hostId }) => changed(hostId, undefined, true));
  const heartbeat = setInterval(() => {
    for (const [projectId, selected] of watched) {
      void arm(projectId, selected).catch(error => bb.log.warn(`Watch unavailable: ${message(error)}`));
    }
  }, 10_000);
  heartbeat.unref();

  async function board(projectId: string) {
    const selected = await mapping(projectId);
    if (!selected) {
      return { status: "unmapped" as const };
    }
    try {
      const result = await host.call("board", selected, { hostId: selected.hostId });
      try {
        await arm(projectId, selected);
      } catch (error) {
        bb.log.warn(`Watch unavailable: ${message(error)}`);
      }
      return result;
    } catch (error) {
      return { status: "error" as const, message: message(error) };
    }
  }

  bb.rpc.register(rpcContract, {
    thread_project_get: async ({ threadId }) => {
      try {
        const thread = await bb.sdk.threads.get({ threadId });
        if (thread.deletedAt != null || !thread.projectId) {
          return { projectId: null };
        }
        const project = await bb.sdk.projects.get({ projectId: thread.projectId });
        return { projectId: project.kind === "personal" ? null : thread.projectId };
      } catch {
        return { projectId: null };
      }
    },
    session_get: async () => {
      const identity = requestIdentity(identityServer, warn);
      return session(identity);
    },
    projects_list: async () => {
      const projects = await bb.sdk.projects.list({ includePersonal: true });
      return { projects: await Promise.all(projects.map(async item => ({
        id: item.id, name: item.name, mapped: !!await mapping(item.id),
      }))) };
    },
    mapping_get: async ({ projectId }) => {
      const project = await bb.sdk.projects.get({ projectId });
      const suggestions = await Promise.all(project.sources.map(async source => {
        let hostName: string | null = null;
        try {
          hostName = (await bb.sdk.hosts.get({ hostId: source.hostId })).name;
        } catch {
          // Offline hosts still provide a usable source suggestion.
        }
        return { hostId: source.hostId, hostName, path: source.path };
      }));
      return { mapping: await mapping(projectId), suggestions };
    },
    mapping_set: async ({ projectId, mapping: selected }) => {
      try {
        await bb.sdk.projects.get({ projectId });
        const checked = await host.call("check", { root: selected.root }, { hostId: selected.hostId });
        if (checked.errors.length) {
          return { ok: false, message: checked.errors.join("; ") };
        }
        const valid = { ...selected, root: checked.root };
        await bb.storage.kv.set(`mapping:${projectId}`, valid);
        try {
          await arm(projectId, valid);
        } catch (error) {
          bb.log.warn(`Watch unavailable: ${message(error)}`);
        }
        bb.realtime.publish(PLANR_CHANGED, { projectId });
        return { ok: true, message: null };
      } catch (error) {
        return { ok: false, message: message(error) };
      }
    },
    mapping_pick_folder: async ({ hostId, clientHostId }) => {
      const result = await bb.sdk.hosts.pickFolder({ hostId, clientHostId: clientHostId ?? hostId });
      return { path: result.path };
    },
    board_get: ({ projectId }) => board(projectId),
    repo_refresh: async ({ projectId }) => {
      const selected = await mapping(projectId);
      if (!selected) {
        return { ok: false, message: "Project has no planning mapping." };
      }
      try {
        return await host.call("refresh", { root: selected.root }, { hostId: selected.hostId });
      } catch (error) {
        return { ok: false, message: message(error) };
      }
    },
    action_apply: async ({ projectId, action, expectedSha256 }) => {
      const identity = requestIdentity(identityServer, warn);
      const allowed = await session(identity);
      if (!allowed.canApprove) {
        return { ok: false as const, reason: "forbidden" as const, message: allowed.reason! };
      }
      const selected = await mapping(projectId);
      if (!selected) {
        return {
        ok: false as const,
        reason: "not-mapped" as const,
        message: "Project has no mapping.",
      };
      }
      try {
        const approvedBy = allowed.mode === "enforced" && allowed.principal
          ? `${allowed.principal.issuer}:${allowed.principal.subject}` : undefined;
        return await host.call("apply", {
          root: selected.root, action, expectedSha256, ...(approvedBy ? { approvedBy } : {}),
        }, {
          hostId: selected.hostId,
          timeoutMs: 300_000,
        });
      } catch (error) {
        return { ok: false as const, reason: "host-error" as const, message: message(error) };
      }
    },
  });

  bb.cli.register({
    name: "planr",
    summary: "Read planning readiness and validation.",
    commands: [
      {
        name: "next",
        summary: "Ready and waiting work",
        usage: "bb planr next --goal g|--stream s [--project p]",
      },
      { name: "check", summary: "Validate planning files", usage: "bb planr check [--project p]" },
      { name: "board", summary: "Planning summary", usage: "bb planr board [--project p]" },
    ],
    async run(argv, context) {
      try {
        const { command, projectId, goal, stream } = argumentsFor(argv, context);
        const selected = await mapping(projectId);
        if (!selected) {
          throw Error("Project has no planning mapping; open Planr in the sidebar.");
        }
        const options = { hostId: selected.hostId, signal: context.signal };
        if (command === "board") {
          const result = await board(projectId);
          if (result.status !== "ok") {
            throw Error(result.status === "error" ? result.message : "Not mapped.");
          }
          const tasks = result.data.tasks as unknown[];
          const goals = result.data.goals as unknown[];
          return { exitCode: 0, stdout: output([
            `planr: ${tasks.length} tasks, ${goals.length} goals; branch ${result.repo.branch ?? "none"}`,
            "Open Planr in the sidebar.",
          ]) };
        }
        if (command === "check") {
          const checked = await host.call("check", { root: selected.root }, options);
          return { exitCode: checked.errors.length ? 1 : 0, stdout: output([
            ...checked.errors.map(text => `planr: error: ${text}`),
            ...checked.warnings.map(text => `planr: warning: ${text}`),
            `planr: ${checked.errors.length} errors, ${checked.warnings.length} warnings`,
          ]) };
        }
        const query = { root: selected.root, ...(goal ? { goal } : { stream: stream! }) };
        const next = await host.call("next", query, options);
        return { exitCode: next.errors.length ? 1 : 0, stdout: output([
          ...next.errors.map(text => `planr: error: ${text}`),
          ...next.warnings.map(text => `planr: warning: ${text}`),
          ...next.ready.map(task => `ready ${task.id}: ${task.title} (${task.unlocks} unlocks)`),
          ...next.waiting.map(task => `waiting ${task.id}: ${task.title} (${task.status})`),
        ]) };
      } catch (error) {
        return { exitCode: 1, stderr: output([`planr: ${message(error)}`]) };
      }
    },
  });

  bb.onDispose(async () => {
    disposed = true;
    clearInterval(heartbeat);
    offSignal();
    offExit();
    await Promise.allSettled([...watched].map(([key, selected]) =>
      host.call("unwatch", { key }, { hostId: selected.hostId })
    ));
    watched.clear();
  });
}
