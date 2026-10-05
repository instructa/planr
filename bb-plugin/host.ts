import {
  experimental_defineHostEntry,
  type ExperimentalHostRpcContext,
  type ExperimentalHostWatchSubscription,
} from "@get-bb/plugin-sdk";
import { execFile, type ChildProcess } from "node:child_process";
import { promisify } from "node:util";
import { createHash } from "node:crypto";
import { existsSync, lstatSync, readFileSync, realpathSync } from "node:fs";
import { isAbsolute, join, relative, resolve, sep } from "node:path";
import { buildIndex, config, findRoot, nextWork, rewrite } from "../skills/planr/scripts/lib/graph.mjs";
import { boardData } from "../skills/planr/scripts/lib/board.mjs";
import { git as readGit, runs } from "../skills/planr/scripts/lib/runs.mjs";
import defaults from "../skills/planr/assets/templates/planr.config.json" with { type: "json" };
import { hostContract, hostSignals } from "./host-contract.ts";
import type { Action, ActionResult, RepoState } from "./contract.ts";

type Context = ExperimentalHostRpcContext<typeof hostSignals>;
const exec = promisify(execFile);
const queues = new Map<string, Promise<unknown>>();
const children = new Set<ChildProcess>();
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

async function serial<T>(key: string, action: () => T | Promise<T>): Promise<T> {
  const previous = queues.get(key) ?? Promise.resolve();
  const pending = previous.catch(() => undefined).then(action);
  queues.set(key, pending);
  try {
    return await pending;
  } finally {
    if (queues.get(key) === pending) {
      queues.delete(key);
    }
  }
}

async function gitBytes(root: string, ...args: string[]) {
  const pending = exec("git", ["-C", root, ...args], {
    encoding: "buffer",
    maxBuffer: 64 << 20,
    timeout: 120_000,
    env: { ...process.env, GIT_TERMINAL_PROMPT: "0" },
  });
  children.add(pending.child);
  try {
    return (await pending).stdout;
  } finally {
    children.delete(pending.child);
  }
}

async function git(root: string, ...args: string[]) {
  return (await gitBytes(root, ...args)).toString("utf8").trim();
}

function planningRoot(path: string) {
  if (!isAbsolute(path)) {
    throw Error("Planning root must be an absolute directory.");
  }
  const root = realpathSync(path);
  if (!lstatSync(root).isDirectory()) {
    throw Error("Planning root must be a directory.");
  }
  try {
    if (findRoot(root) === root) {
      return root;
    }
  } catch {
    // The engine's CLI hint does not apply to a folder selected in the board.
  }
  throw Error("This folder is not a planr planning repo (no tasks/ folder or planr.config.json found).");
}

function snapshot(path: string) {
  const root = planningRoot(path);
  const settings = config(root, defaults);
  const hashes: Record<string, string> = {};
  const data = buildIndex(root, settings, file => {
    const bytes = readFileSync(confined(root, relative(root, file)));
    hashes[relative(root, file)] = createHash("sha256").update(bytes).digest("hex");
    return bytes.toString("utf8");
  });
  return { root, settings, hashes, ...data };
}

function confined(root: string, file: string) {
  const path = resolve(root, file);
  const actual = realpathSync(path);
  const part = relative(root, actual);
  if (part === ".." || part.startsWith(`..${sep}`) || isAbsolute(part) || lstatSync(path).isSymbolicLink()) {
    throw Error(`File outside planning root or symbolic link: ${file}`);
  }
  return path;
}

export const fileSha = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

function repoState(root: string, hostId: string): RepoState {
  const divergence = readGit(root, ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"]);
  const [ahead, behind] = divergence ? divergence.split(/\s+/).map(Number) : [0, 0];
  return {
    root,
    hostId,
    branch: readGit(root, ["symbolic-ref", "--short", "HEAD"]) || null,
    head: readGit(root, ["rev-parse", "HEAD"]) || null,
    dirty: !!readGit(root, ["status", "--porcelain"]),
    ahead,
    behind,
    hasRemote: !!readGit(root, ["remote"]),
  };
}

async function repoKey(root: string) {
  return realpathSync(await git(root, "rev-parse", "--path-format=absolute", "--git-common-dir"));
}

function readRepo<T>(path: string, action: (root: string) => T): Promise<T> {
  const root = planningRoot(path);
  const common = readGit(root, ["rev-parse", "--path-format=absolute", "--git-common-dir"]);
  return serial(common ? realpathSync(common) : root, () => action(root));
}

async function writeCheckout(root: string) {
  const top = realpathSync(await git(root, "rev-parse", "--show-toplevel"));
  if (!readGit(root, ["remote"])) {
    return { path: top, planning: root, hasRemote: false, target: "", destination: "" };
  }
  const branch = await git(root, "symbolic-ref", "--short", "HEAD");
  const path = `${top}-planr-ui`;
  if (!readGit(root, ["remote", "get-url", "origin"])) {
    throw Error("Board actions require origin when a remote is configured.");
  }
  const target = await git(root, "rev-parse", "--symbolic-full-name", "@{upstream}");
  if (!target.startsWith("refs/remotes/origin/")) {
    throw Error("The planning branch must track origin before applying board actions.");
  }
  const destination = target.slice("refs/remotes/origin/".length);
  if (!existsSync(path)) {
    await git(root, "worktree", "add", "--detach", path, `refs/heads/${branch}`);
  }
  if (lstatSync(path).isSymbolicLink() || realpathSync(path) !== realpathSync(
    await git(path, "rev-parse", "--show-toplevel")
  )) {
    throw Error("Plugin worktree must be a separate, real checkout.");
  }
  if (await repoKey(path) !== await repoKey(root)) {
    throw Error("Plugin worktree belongs to another repository.");
  }
  if (readGit(path, ["symbolic-ref", "-q", "HEAD"])) {
    throw Error("Plugin worktree must be detached.");
  }
  if (await git(path, "status", "--porcelain")) {
    throw Error("Plugin worktree is dirty; refusing to reset it.");
  }
  await git(path, "fetch", "origin");
  if (await git(path, "status", "--porcelain")) {
    throw Error("Plugin worktree changed during fetch; refusing to reset it.");
  }
  await git(path, "reset", "--hard", target);
  return { path, planning: join(path, relative(top, root)), hasRemote: true, target, destination };
}

function failure(reason: Extract<ActionResult, { ok: false }>["reason"], text: string): ActionResult {
  return { ok: false, reason, message: text };
}

async function unchanged(root: string, file: string, expectedSha256: string) {
  if (await git(root, "status", "--porcelain", "--untracked-files=all", "--ignored", "--", file)) {
    return false;
  }
  try {
    return fileSha(confined(root, file)) === expectedSha256;
  } catch {
    return false;
  }
}

async function refresh(root: string) {
  if (!repoState(root, "").hasRemote) {
    return { ok: true, message: "no remote; nothing to fetch" };
  }
  await git(root, "fetch", "origin");
  if (await git(root, "status", "--porcelain")) {
    return { ok: true, message: "Fetched; checkout is dirty, so pull was skipped." };
  }
  await git(root, "pull", "--ff-only");
  return { ok: true, message: null };
}

export async function apply(
  rootPath: string, action: Action, expectedSha256: string, approvedBy?: string,
): Promise<ActionResult> {
  try {
    const root = planningRoot(rootPath);
    return await serial(await repoKey(root), async () => {
      const task = action.kind === "check-task";
      const id = task ? action.id : action.goal;
      const select = (index: ReturnType<typeof buildIndex>["index"]) => task
        ? index.tasks.find(item => item.id === id)
        : index.goals.find(item => item.name === id);
      const source = select(snapshot(root).index);
      if (source && !await unchanged(root, source.file, expectedSha256)) {
        return failure("conflict", "Main checkout file changed or has local edits; refresh before applying.");
      }
      const ui = await writeCheckout(root);
      const data = snapshot(ui.planning);
      const record = select(data.index);
      if (!record) {
        return failure("invalid-transition", `Unknown ${task ? "task" : "goal"}: ${id}`);
      }
      const file = confined(ui.planning, record.file);
      const changed = relative(ui.path, file);
      if (!await unchanged(root, source?.file ?? record.file, expectedSha256)) {
        return failure("conflict", "Main checkout file changed or has local edits; refresh before applying.");
      }
      if (data.hashes[record.file] !== expectedSha256 || fileSha(file) !== expectedSha256) {
        return failure("conflict", "File changed; refresh before applying.");
      }
      if (data.errors.length) {
        return failure("host-error", data.errors.join("; "));
      }
      const current = task ? record.status : record.state;
      const expected = action.kind === "approve-goal" ? "draft" : "review";
      if (current !== expected) {
        return failure("invalid-transition", `Expected ${expected}, found ${current}.`);
      }
      const attribution = approvedBy ? `\nApproved-by: ${approvedBy}` : "";
      const commitMessage = `planr: ${action.kind} ${id}\n\nApproved-via: planr-ui${attribution}`;
      rewrite(file, {
        [task ? "status" : "state"]: action.kind === "approve-goal" ? "approved" : "done",
      });
      const writtenSha = fileSha(file);
      try {
        await git(ui.path, "commit", "-m", commitMessage, "--", changed);
      } catch (error) {
        // Undo only our one-file edit if a commit fails; never clean a previously dirty worktree.
        if (fileSha(file) === writtenSha) {
          await git(ui.path, "restore", "--source=HEAD", "--staged", "--worktree", "--", changed);
        }
        return failure("host-error", message(error));
      }
      let commit = await git(ui.path, "rev-parse", "HEAD");
      if (!ui.hasRemote) {
        return { ok: true, commit, pushed: false, note: "no remote; committed locally" };
      }
      try {
        await git(ui.path, "push", "origin", `HEAD:refs/heads/${ui.destination}`);
      } catch {
        try {
          await git(ui.path, "fetch", "origin");
          const upstreamBytes = await gitBytes(ui.path, "show", `${ui.target}:${changed}`);
          const sha = createHash("sha256").update(upstreamBytes).digest("hex");
          if (sha !== expectedSha256) {
            return failure("push-rejected", "Remote file changed; refresh and retry.");
          }
          await git(ui.path, "rebase", "--merge", ui.target);
          await git(ui.path, "push", "origin", `HEAD:refs/heads/${ui.destination}`);
          commit = await git(ui.path, "rev-parse", "HEAD");
        } catch (error) {
          // Never resolve conflicts; leave the dedicated worktree clean for the next reviewed action.
          if (existsSync(join(await git(ui.path, "rev-parse", "--absolute-git-dir"), "rebase-merge"))) {
            await git(ui.path, "rebase", "--abort");
          }
          return failure("push-rejected", message(error));
        }
      }
      let note: string | null;
      try {
        note = (await refresh(root)).message;
      } catch (error) {
        note = `Pushed; checkout refresh failed: ${message(error)}`;
      }
      return { ok: true, commit, pushed: true, note };
    });
  } catch (error) {
    return failure("host-error", message(error));
  }
}

interface Watch {
  root: string;
  seen: number;
  signature: string;
  subscriptions: ExperimentalHostWatchSubscription[];
  context: Context;
}
const watches = new Map<string, Watch>();
let expiry: ReturnType<typeof setInterval> | undefined;

async function unwatch(key: string) {
  const watch = watches.get(key);
  watches.delete(key);
  await Promise.all(watch?.subscriptions.map(sub => sub.dispose()) ?? []);
}

async function watch(key: string, path: string, context: Context) {
  const root = planningRoot(path);
  const settings = config(root, defaults);
  const directories = [settings.tasksDir, settings.goalsDir];
  const signature = JSON.stringify(directories.map(dir => [dir, existsSync(join(root, dir))]));
  const previous = watches.get(key);
  if (previous?.root === root && previous.signature === signature) {
    previous.seen = Date.now();
    previous.context = context;
    return;
  }
  await unwatch(key);
  const entry: Watch = { root, signature, seen: Date.now(), subscriptions: [], context };
  watches.set(key, entry);
  try {
    for (const dir of directories) {
      if (!existsSync(join(root, dir))) {
        continue;
      }
      entry.subscriptions.push(await context.experimental_watch({
        rootPath: confined(root, dir),
        debounceMs: 200,
        maxWaitMs: 500,
      }, () => entry.context.experimental_emitSignal("changed", { root })));
    }
    // Also observe root metadata and creation of an initially absent tasks/goals directory.
    entry.subscriptions.push(await context.experimental_watch({
      rootPath: root,
      ignoredPaths: ["node_modules/**", ".planr/board.html"],
      debounceMs: 200,
      maxWaitMs: 500,
    }, event => {
      if (event.kind !== "changed" || event.changes.some(change =>
        ["planr.config.json", ".git/HEAD", ...directories].includes(change.path)
      )) {
        return entry.context.experimental_emitSignal("changed", { root });
      }
    }));
  } catch (error) {
    await unwatch(key);
    throw error;
  }
  if (!expiry) {
    expiry = setInterval(() => {
      for (const [id, value] of watches) {
        if (Date.now() - value.seen > 30_000) {
          void serial(`watch:${id}`, () => unwatch(id)).catch(() => undefined);
        }
      }
    }, 5000);
    expiry.unref();
  }
}

export default experimental_defineHostEntry({
  contract: hostContract,
  experimental_signals: hostSignals,
  handlers: {
    board: async ({ root: path, hostId }) => {
      try {
        return await readRepo(path, selected => {
          const { root, index, errors, settings, hashes } = snapshot(selected);
          if (errors.length) {
            return { status: "error" as const, message: errors.join("; ") };
          }
          return {
            status: "ok" as const,
            data: boardData(root, index, settings),
            repo: repoState(root, hostId),
            fileSha: hashes,
          };
        });
      } catch (error) {
        return { status: "error" as const, message: message(error) };
      }
    },
    check: ({ root: path }) => readRepo(path, selected => {
      const { root, index, errors, settings } = snapshot(selected);
      const historyWarnings = errors.length ? [] : runs(root, index, settings).warnings;
      return { root, errors, warnings: [...index.warnings, ...historyWarnings], counts: index.counts };
    }),
    next: ({ root: path, goal, stream }) => readRepo(path, selected => {
      const { index, errors } = snapshot(selected);
      if (!!goal === !!stream) {
        throw Error("Choose exactly one of goal or stream.");
      }
      const goals = goal ? index.goals.filter(item => item.name === goal)
        : index.goals.filter(item => item.stream === stream && ["running", "approved"].includes(item.state));
      if (goal && !goals.length) {
        throw Error(`Unknown goal: ${goal}`);
      }
      const selectedStream = stream ?? goals[0]?.stream ?? undefined;
      return { ...nextWork(index, goals, selectedStream), errors, warnings: index.warnings };
    }),
    refresh: async ({ root: path }, context) => {
      try {
        const root = planningRoot(path);
        const result = await serial(await repoKey(root), () => refresh(root));
        await context.experimental_emitSignal("changed", { root });
        return result;
      } catch (error) {
        return { ok: false, message: message(error) };
      }
    },
    apply: async ({ root, action, expectedSha256, approvedBy }, context) => {
      const result = await apply(root, action, expectedSha256, approvedBy);
      if (result.ok) {
        try {
          await context.experimental_emitSignal("changed", { root: planningRoot(root) });
        } catch (error) {
          const notice = `Change notification failed: ${message(error)}`;
          result.note = [result.note, notice].filter(Boolean).join("; ");
        }
      }
      return result;
    },
    watch: async ({ root, key }, context) => {
      await serial(`watch:${key}`, () => watch(key, root, context));
      return null;
    },
    unwatch: async ({ key }) => {
      await serial(`watch:${key}`, () => unwatch(key));
      return null;
    },
  },
  dispose: async () => {
    if (expiry) {
      clearInterval(expiry);
    }
    expiry = undefined;
    for (const child of children) {
      child.kill("SIGTERM");
    }
    await Promise.all([...watches.keys()].map(key => serial(`watch:${key}`, () => unwatch(key))));
  },
});
