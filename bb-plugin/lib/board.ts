// Board data as the engine produces it (docs/contract.md, "Board data").
// ready, openDeps, waitingOn, unlocks and goal come from the engine
// (skills/planr/scripts/lib/graph.mjs owns those rules). This module only reads,
// sorts and groups them for presentation; it never derives readiness or unlocks.

export type TaskStatus = "open" | "in-progress" | "review" | "blocked" | "decision" | "done" | "dropped";

/** Task statuses plus the two marks runs need: a removed task and a new one. */
export type MarkKind = TaskStatus | "removed" | "new";

export interface BoardTask {
  id: string;
  title: string;
  status: string;
  track: string;
  depends: string[];
  owner: string | null;
  file: string;
  openDeps: string[];
  ready: boolean;
  waitingOn: string[];
  unlocks: number;
  goal: string[];
  history: Array<{ run: number; from: string | null; to: string }>;
  lastRun: number | null;
}

export interface BoardGoal {
  name: string;
  file: string;
  title: string | null;
  state: string;
  stream: string | null;
  budget: string | null;
  thread: string | null;
  scope: string[];
  done: number;
  review: number;
  total: number;
  ready: string[];
  waiting: string[];
}

export interface BoardRun {
  commit: string | null;
  date: string;
  subject: string;
  counts: Record<string, number>;
  total: number;
  initial: boolean;
  changes: Array<{ id: string; from: string | null; to: string }>;
}

export interface BoardData {
  project: string;
  commit: string;
  dirty: boolean;
  generatedAt: string;
  tasks: BoardTask[];
  goals: BoardGoal[];
  runs: BoardRun[];
  warnings: string[];
}

const list = <T>(value: unknown): T[] => (Array.isArray(value) ? (value as T[]) : []);
const text = (value: unknown, fallback = ""): string => (typeof value === "string" ? value : fallback);
const count = (value: unknown): number => (Number.isFinite(Number(value)) ? Number(value) : 0);
const strings = (value: unknown): string[] => list<unknown>(value).map(String);

/** Reads the engine payload defensively; values are coerced, never recomputed. */
export function parseBoard(raw: Record<string, unknown>): BoardData {
  return {
    project: text(raw.project, "planning"),
    commit: text(raw.commit),
    dirty: raw.dirty === true,
    generatedAt: text(raw.generatedAt, new Date().toISOString()),
    tasks: list<Record<string, unknown>>(raw.tasks).map((t) => ({
      id: text(t.id),
      title: text(t.title),
      status: text(t.status, "open"),
      track: text(t.track, "none"),
      depends: strings(t.depends),
      owner: typeof t.owner === "string" ? t.owner : null,
      file: text(t.file),
      openDeps: strings(t.openDeps),
      ready: t.ready === true,
      waitingOn: strings(t.waitingOn),
      unlocks: count(t.unlocks),
      goal: strings(t.goal),
      history: list<Record<string, unknown>>(t.history).map((h) => ({
        run: count(h.run),
        from: typeof h.from === "string" ? h.from : null,
        to: text(h.to),
      })),
      lastRun: typeof t.lastRun === "number" ? t.lastRun : null,
    })),
    goals: list<Record<string, unknown>>(raw.goals).map((g) => ({
      name: text(g.name),
      file: text(g.file),
      title: typeof g.title === "string" ? g.title : null,
      state: text(g.state, "draft"),
      stream: typeof g.stream === "string" ? g.stream : null,
      budget: typeof g.budget === "string" ? g.budget : null,
      thread: typeof g.thread === "string" ? g.thread : null,
      scope: strings(g.scope),
      done: count(g.done),
      review: count(g.review),
      total: count(g.total),
      ready: strings(g.ready),
      waiting: strings(g.waiting),
    })),
    runs: list<Record<string, unknown>>(raw.runs).map((r) => ({
      commit: typeof r.commit === "string" ? r.commit : null,
      date: text(r.date),
      subject: text(r.subject),
      counts: Object.fromEntries(
        Object.entries((r.counts ?? {}) as Record<string, unknown>).map(([k, v]) => [k, count(v)]),
      ),
      total: count(r.total),
      initial: r.initial === true,
      changes: list<Record<string, unknown>>(r.changes).map((c) => ({
        id: text(c.id),
        from: typeof c.from === "string" ? c.from : null,
        to: text(c.to),
      })),
    })),
    warnings: strings(raw.warnings),
  };
}

export const STATUS_LABEL: Record<string, string> = {
  blocked: "Blocked",
  decision: "Decision",
  "in-progress": "In progress",
  review: "Review",
  open: "Open",
  done: "Done",
  dropped: "Dropped",
  removed: "Removed",
};
const STATUS_SHORT: Record<string, string> = {
  blocked: "blocked",
  decision: "decide",
  "in-progress": "wip",
  review: "review",
  open: "open",
  done: "done",
  dropped: "dropped",
  removed: "removed",
};
/** Strip and count order: finished work first. */
export const PROGRESS: TaskStatus[] = [
  "done",
  "review",
  "in-progress",
  "open",
  "decision",
  "blocked",
  "dropped",
];
/** List order: what needs attention first. */
export const STATUS_ORDER: TaskStatus[] = [
  "blocked",
  "decision",
  "in-progress",
  "review",
  "open",
  "done",
  "dropped",
];
const RANK: Record<string, number> = Object.fromEntries(STATUS_ORDER.map((s, i) => [s, i]));
/** Goal states reuse task marks: running works, approved waits to start, draft waits on an approver. */
const GOAL_MARK: Record<string, MarkKind> = {
  running: "in-progress",
  approved: "open",
  draft: "decision",
  review: "review",
  done: "done",
  legacy: "removed",
};
/** Goals waiting on an approver and what they do with them. */
export const GOAL_ASK: Record<string, string> = { draft: "approve", review: "close" };

export const statusLabel = (s: string) => (Object.hasOwn(STATUS_LABEL, s) ? STATUS_LABEL[s] : s);
export const statusShort = (s: string | null) =>
  s === null ? "new" : Object.hasOwn(STATUS_SHORT, s) ? STATUS_SHORT[s] : s;
export const rank = (s: string) => (Object.hasOwn(RANK, s) ? RANK[s] : STATUS_ORDER.length);
export const markOf = (s: string): MarkKind => (Object.hasOwn(STATUS_LABEL, s) ? (s as MarkKind) : "open");
export const goalMark = (g: BoardGoal): MarkKind =>
  Object.hasOwn(GOAL_MARK, g.state) ? GOAL_MARK[g.state] : "open";
/** Built work counts review: those tasks are finished except for the manual check. */
export const goalBuilt = (g: BoardGoal) => g.done + g.review;

export const dayKey = (iso: string) => {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return `${String(d.getDate()).padStart(2, "0")}.${String(d.getMonth() + 1).padStart(2, "0")}.`;
};
export const hm = (iso: string) => {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleTimeString("de-AT", { hour: "2-digit", minute: "2-digit" });
};
export const idCompare = (a: { id: string }, b: { id: string }) =>
  a.id.localeCompare(b.id, undefined, { numeric: true });
const byLever = (a: BoardTask, b: BoardTask) =>
  b.unlocks - a.unlocks || b.waitingOn.length - a.waitingOn.length;
export const leverText = (t: BoardTask) =>
  [t.unlocks && `unlocks ${t.unlocks}`, t.waitingOn.length && `blocks ${t.waitingOn.length}`]
    .filter(Boolean)
    .join(" · ");

export const PRESETS = ["all", "needs", "check", "ready", "changed"] as const;
export type Preset = (typeof PRESETS)[number];
/** One exclusive filter: a preset, a status or a run index. */
export type MainFilter = Preset | `status:${string}` | `run:${number}`;
export type TaskSort = "status" | "id" | "track" | "waiting" | "moved";
export const SORT_LABEL: Record<TaskSort, string> = {
  status: "Status",
  id: "ID",
  track: "Track",
  waiting: "Blocks most",
  moved: "Recently moved",
};
export interface TaskFilters {
  main: MainFilter;
  goal: string | null;
  track: string | null;
  query: string;
  sort: TaskSort;
}
export const NO_FILTERS: TaskFilters = {
  main: "all",
  goal: null,
  track: null,
  query: "",
  sort: "status",
};

/** Presentation groups of one board payload, computed once per payload. */
export interface BoardView {
  data: BoardData;
  byId: Map<string, BoardTask>;
  goalByName: Map<string, BoardGoal>;
  /** Blocked and decision tasks, ranked by unlocks, then blocks, then status. */
  needsYou: BoardTask[];
  /** Ready tasks; those in a running or approved goal first. */
  upNext: BoardTask[];
  /** Review tasks, most recently moved first. */
  toCheck: BoardTask[];
  /** Unfinished tasks that hold others up. Review and done already satisfy dependents. */
  levers: BoardTask[];
  /** Draft goals (approve) and review goals (close). */
  askGoals: BoardGoal[];
  /** Goals with a tile: running, approved, draft, review. */
  queued: BoardGoal[];
  tracks: string[];
  statuses: TaskStatus[];
  today: string;
  baseRun: BoardRun | null;
  baseIndex: number;
  changedSinceBase: Set<string>;
  sorts: Record<TaskSort, (a: BoardTask, b: BoardTask) => number>;
}

export function boardView(data: BoardData): BoardView {
  const { tasks, goals, runs } = data;
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const goalByName = new Map(goals.map((g) => [g.name, g]));
  const inActiveGoal = (t: BoardTask) =>
    t.goal.some((n) => ["running", "approved"].includes(goalByName.get(n)?.state ?? ""));
  const today = dayKey(runs.at(-1)?.date ?? data.generatedAt);
  let baseIndex = -1;
  for (let i = runs.length - 1; i >= 0; i--) {
    if (dayKey(runs[i].date) !== today) {
      baseIndex = i;
      break;
    }
  }
  const trackCount = new Map<string, number>();
  for (const t of tasks) trackCount.set(t.track, (trackCount.get(t.track) ?? 0) + 1);
  const statusThenLever = (a: BoardTask, b: BoardTask) =>
    rank(a.status) - rank(b.status) || byLever(a, b) || idCompare(a, b);
  return {
    data,
    byId,
    goalByName,
    needsYou: tasks
      .filter((t) => t.status === "blocked" || t.status === "decision")
      .sort((a, b) => byLever(a, b) || rank(a.status) - rank(b.status) || idCompare(a, b)),
    upNext: tasks
      .filter((t) => t.ready)
      .sort((a, b) => Number(inActiveGoal(b)) - Number(inActiveGoal(a)) || byLever(a, b) || idCompare(a, b)),
    toCheck: tasks
      .filter((t) => t.status === "review")
      .sort((a, b) => (b.lastRun ?? -1) - (a.lastRun ?? -1) || idCompare(a, b)),
    levers: tasks
      .filter((t) => !["review", "done", "dropped"].includes(t.status) && t.waitingOn.length > 0)
      .sort((a, b) => b.waitingOn.length - a.waitingOn.length || b.unlocks - a.unlocks || idCompare(a, b)),
    askGoals: goals.filter((g) => Object.hasOwn(GOAL_ASK, g.state)),
    queued: goals.filter((g) => ["running", "approved", "draft", "review"].includes(g.state)),
    tracks: [...trackCount.keys()].sort((a, b) => (trackCount.get(b) ?? 0) - (trackCount.get(a) ?? 0)),
    statuses: PROGRESS.filter((s) => tasks.some((t) => t.status === s)),
    today,
    baseRun: baseIndex >= 0 ? runs[baseIndex] : null,
    baseIndex,
    changedSinceBase: new Set(tasks.filter((t) => t.history.some((h) => h.run > baseIndex)).map((t) => t.id)),
    sorts: {
      status: statusThenLever,
      id: idCompare,
      track: (a, b) => a.track.localeCompare(b.track) || statusThenLever(a, b),
      waiting: (a, b) => b.waitingOn.length - a.waitingOn.length || statusThenLever(a, b),
      moved: (a, b) => (b.lastRun ?? -1) - (a.lastRun ?? -1) || idCompare(a, b),
    },
  };
}

export const runLabel = (view: BoardView, r: BoardRun) =>
  `${dayKey(r.date) === view.today ? "" : `${dayKey(r.date)} `}${hm(r.date)}`;
export const runAt = (view: BoardView, t: BoardTask) =>
  t.lastRun === null ? null : (view.data.runs[t.lastRun] ?? null);
export const goalTasks = (view: BoardView, name: string) =>
  view.data.tasks.filter((t) => t.goal.includes(name));

export function matchesFilters(view: BoardView, f: TaskFilters, t: BoardTask): boolean {
  const m = f.main;
  if (m === "needs" && !(t.status === "blocked" || t.status === "decision")) return false;
  if (m === "check" && t.status !== "review") return false;
  if (m === "ready" && !t.ready) return false;
  if (m === "changed" && !view.changedSinceBase.has(t.id)) return false;
  if (m.startsWith("status:") && t.status !== m.slice(7)) return false;
  if (m.startsWith("run:") && !t.history.some((h) => h.run === Number(m.slice(4)))) return false;
  if (f.track !== null && t.track !== f.track) return false;
  if (f.goal !== null && !t.goal.includes(f.goal)) return false;
  const q = f.query.trim().toLowerCase();
  if (q && !(t.id.toLowerCase().includes(q) || t.title.toLowerCase().includes(q))) return false;
  return true;
}

export function runSummary(r: BoardRun): string {
  const news = r.changes.filter((c) => c.from === null).length;
  const done = r.changes.filter((c) => c.from !== null && c.to === "done").length;
  const moved = r.changes.length - done - news;
  return (
    [done && `${done} done`, news && `${news} new`, moved && `${moved} moved`].filter(Boolean).join(" · ") ||
    (r.initial ? `baseline · ${r.total} tasks` : "no status change")
  );
}

/** Index warnings that start with the task's ID ("ID: message"). */
export function taskWarnings(data: BoardData, id: string): string[] {
  return data.warnings.filter((w) => w.startsWith(`${id}:`)).map((w) => w.slice(id.length + 1).trim());
}

/** Stable key of a run for routes: its commit, or "worktree" for uncommitted changes. */
export const runKey = (r: BoardRun) => r.commit ?? "worktree";
