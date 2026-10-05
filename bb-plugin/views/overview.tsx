// Overview: summary, goal queue, the approver's Needs-you queue, recent runs, up next,
// bottlenecks and tracks. Narrow panels stack in that order; wide ones split into
// a work column (goals, Needs you) and an activity column.
import { useState } from "react";
import { Icon } from "@/components/ui/icon";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import {
  dayKey,
  goalBuilt,
  goalMark,
  hm,
  leverText,
  markOf,
  PROGRESS,
  runAt,
  runLabel,
  runSummary,
  statusLabel,
  type BoardGoal,
  type BoardRun,
  type BoardView,
  type MainFilter,
} from "@/lib/board";
import type { BoardActions } from "@/hooks/use-actions";
import { ChangeCells, StatusMark, StatusStrip, Tip, progressItems } from "./marks";
import {
  CheckTaskButton,
  Empty,
  FOCUS_RING,
  GoalActionButton,
  GoalRow,
  GroupHeader,
  MoreButton,
  MoreRow,
  Section,
  TaskRow,
  idWidth,
} from "./parts";

export interface OverviewNav {
  openTask: (id: string) => void;
  openRun: (run: BoardRun) => void;
  filter: (main: MainFilter) => void;
  filterGoal: (name: string) => void;
  filterTrack: (track: string) => void;
  showRuns: () => void;
}

const CHECK_ROWS = 5;
const LIST_ROWS = 8;
const RUN_ROWS = 6;
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

function Summary({
  view,
  nav,
  onWarnings,
}: {
  view: BoardView;
  nav: OverviewNav;
  onWarnings: React.ReactNode;
}) {
  const { data, baseRun } = view;
  const shown = PROGRESS.filter(
    (s) => s !== "dropped" && (data.tasks.some((t) => t.status === s) || data.runs.some((r) => r.counts[s])),
  );
  const countOf = (s: string) => data.tasks.filter((t) => t.status === s).length;
  return (
    <section aria-label="Summary" className="min-w-0 rounded-lg border border-border bg-card px-3.5 py-3">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span className="tabular-nums">
          <b className="font-semibold text-foreground">{data.tasks.length}</b> tasks ·{" "}
          <b className="font-semibold text-foreground">{view.tracks.length}</b> tracks
        </span>
        {baseRun !== null ? (
          <span className="tabular-nums">
            Δ since {dayKey(baseRun.date)} ·{" "}
            <b className="font-semibold text-foreground">{view.changedSinceBase.size}</b> moved
          </span>
        ) : null}
        <span className="ml-auto">{onWarnings}</span>
      </div>
      <StatusStrip items={progressItems(data.tasks)} className="mt-2.5 h-2" />
      <div className="mt-2 grid grid-cols-3 gap-1 @xl:grid-cols-6" role="group" aria-label="Tasks by status">
        {shown.map((s) => {
          const n = countOf(s);
          const delta = baseRun === null ? null : n - (baseRun.counts[s] ?? 0);
          const sign = delta !== null && delta > 0 ? "+" : "";
          const deltaText = delta ? `, ${sign}${delta} since ${dayKey(baseRun?.date ?? "")}` : "";
          return (
            <button
              key={s}
              type="button"
              onClick={() => nav.filter(`status:${s}`)}
              aria-label={`${n} ${statusLabel(s)}${deltaText}. Show in Tasks`}
              className={cn(
                "flex min-w-0 flex-col items-start gap-1 rounded-md px-2 py-1.5 text-left",
                "hover:bg-state-hover",
                FOCUS_RING,
              )}
            >
              <span className="flex items-baseline gap-1.5">
                <span className="text-lg font-semibold leading-none tabular-nums">{n}</span>
                {delta !== null ? (
                  <span
                    className={cn(
                      "text-xs tabular-nums",
                      delta ? "font-medium text-foreground" : "text-subtle-foreground",
                    )}
                  >
                    {delta > 0 ? "+" : delta < 0 ? "−" : "±"}
                    {Math.abs(delta)}
                  </span>
                ) : null}
              </span>
              <span className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                <StatusMark kind={markOf(s)} className="size-3" />
                <span className="truncate">{statusLabel(s)}</span>
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}

export function WarningsButton({ view, onOpenTask }: { view: BoardView; onOpenTask: (id: string) => void }) {
  const warnings = view.data.warnings;
  if (warnings.length === 0) return null;
  const groups = new Map<string, Array<string | null>>();
  for (const w of warnings) {
    const m = w.match(/^(\S+?):\s+(.+)$/);
    const [key, message] = m ? [m[1], m[2]] : [null, w];
    groups.set(message, [...(groups.get(message) ?? []), key]);
  }
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={cn(
            "flex items-center gap-1 rounded-md px-1.5 py-0.5 hover:bg-state-hover hover:text-foreground",
            FOCUS_RING,
          )}
        >
          <Icon name="AlertTriangle" className="size-3.5 text-warning" />
          <span className="tabular-nums">{plural(warnings.length, "warning")}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        className="max-h-[min(70vh,520px)] w-[min(420px,calc(100vw-20px))] overflow-y-auto p-0"
        mobileTitle="Warnings"
      >
        <p
          className={cn(
            "sticky top-0 border-b border-border-hairline bg-popover px-3.5 py-2.5 text-sm font-semibold",
          )}
        >
          Warnings{" "}
          <span className="text-xs font-normal tabular-nums text-subtle-foreground">{warnings.length}</span>
        </p>
        <ul className="divide-y divide-border-hairline">
          {[...groups].map(([message, keys]) => (
            <li key={message} className="space-y-1.5 px-3.5 py-2.5">
              <p className="text-sm">
                {message} <span className="text-xs tabular-nums text-subtle-foreground">{keys.length}</span>
              </p>
              {keys.some((k) => k !== null) ? (
                <p className="flex flex-wrap gap-x-2 gap-y-1 font-mono text-xs">
                  {keys.map((k, i) =>
                    k === null ? null : view.byId.has(k) ? (
                      <button
                        key={`${k}-${i}`}
                        type="button"
                        onClick={() => onOpenTask(k)}
                        className={cn(
                          "rounded-sm text-muted-foreground underline decoration-border underline-offset-4",
                          "hover:text-foreground",
                          FOCUS_RING,
                        )}
                      >
                        {k}
                      </button>
                    ) : (
                      <span key={`${k}-${i}`} className="text-muted-foreground">
                        {k}
                      </span>
                    ),
                  )}
                </p>
              ) : null}
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  );
}

export function goalNote(g: BoardGoal) {
  return [
    g.review && `${g.review} to check`,
    g.ready.length && `${g.ready.length} ready`,
    g.waiting.length && `${g.waiting.length} waiting`,
  ]
    .filter(Boolean)
    .join(" · ");
}

function GoalTile({ view, goal, onOpen }: { view: BoardView; goal: BoardGoal; onOpen: () => void }) {
  const scope = goal.scope.flatMap((id) => view.byId.get(id) ?? []);
  const note = goalNote(goal);
  const built = `${goalBuilt(goal)}/${goal.total} built`;
  const counts = [
    built,
    `${goal.done} done`,
    `${goal.review} to check`,
    `${goal.ready.length} ready`,
    `${goal.waiting.length} waiting`,
  ].join(" · ");
  const label = `Goal ${goal.name}, ${goal.state}, ${built}. Show its tasks`;
  const tip = (
    <span className="block space-y-0.5">
      <span className="block font-medium">{goal.title ?? goal.name}</span>
      <span className="block opacity-80">
        {[goal.state, goal.stream, goal.budget, goal.thread].filter(Boolean).join(" · ")}
      </span>
      {goal.state === "draft" ? <span className="block">Draft: waiting for your approval.</span> : null}
      {goal.state === "review" ? (
        <span className="block">Review: over budget or waiting for your sign-off.</span>
      ) : null}
      <span className="block opacity-80">{counts}</span>
    </span>
  );
  return (
    <Tip content={tip}>
      <button
        type="button"
        onClick={onOpen}
        aria-label={label}
        className={cn(
          "flex min-w-0 flex-col gap-1 rounded-md px-2 py-1.5 text-left hover:bg-state-hover",
          FOCUS_RING,
        )}
      >
        <span className="flex w-full min-w-0 items-center gap-1.5 text-xs">
          <StatusMark kind={goalMark(goal)} className="size-3" />
          <span className="font-medium">{goal.state}</span>
          <span className="min-w-0 flex-1 truncate text-subtle-foreground">
            {[goal.stream, goal.budget].filter(Boolean).join(" · ")}
          </span>
          <span className="shrink-0 tabular-nums text-muted-foreground">
            {goalBuilt(goal)}/{goal.total}
          </span>
        </span>
        <span className="flex w-full min-w-0 items-baseline gap-2">
          <span className="shrink-0 font-mono text-xs">{goal.name}</span>
          <span className="min-w-0 truncate text-sm text-muted-foreground">{goal.title ?? ""}</span>
        </span>
        <span className="flex w-full min-w-0 items-center gap-2 text-xs text-subtle-foreground">
          <StatusStrip items={progressItems(scope)} label={`${goal.name} · `} className="flex-1" />
          {note ? <span className="shrink-0 tabular-nums">{note}</span> : null}
        </span>
      </button>
    </Tip>
  );
}

function GoalQueue({ view, nav }: { view: BoardView; nav: OverviewNav }) {
  const goals = view.data.goals;
  const of = (s: string) => goals.filter((g) => g.state === s);
  const counts = ["running", "approved", "draft", "review"]
    .filter((s) => of(s).length)
    .map((s) => `${of(s).length} ${s}`)
    .join(" · ");
  const rest = (["legacy", "done"] as const).filter((s) => of(s).length);
  return (
    <Section
      title="Goals"
      labelledBy="planr-goals"
      meta={counts || undefined}
      action={
        rest.length ? (
          <span className="flex items-center gap-1 text-xs text-subtle-foreground">
            {rest.map((s) => (
              <Tip
                key={s}
                content={
                  <span className="block space-y-0.5">
                    {of(s).map((g) => (
                      <span key={g.name} className="block">
                        <b className="font-medium">{g.name}</b> <span className="opacity-80">{g.file}</span>
                      </span>
                    ))}
                  </span>
                }
              >
                <button
                  type="button"
                  className={cn(
                    "rounded-md px-1.5 py-0.5 tabular-nums hover:bg-state-hover hover:text-foreground",
                    FOCUS_RING,
                  )}
                >
                  +{of(s).length} {s}
                </button>
              </Tip>
            ))}
          </span>
        ) : undefined
      }
    >
      {view.queued.length > 0 ? (
        <div className="grid grid-cols-1 gap-x-3 gap-y-0.5 p-1.5 @md:grid-cols-2 @6xl:grid-cols-3">
          {view.queued.map((g) => (
            <GoalTile key={g.name} view={view} goal={g} onOpen={() => nav.filterGoal(g.name)} />
          ))}
        </div>
      ) : (
        <Empty>
          {goals.length ? "No goal is running, approved or waiting for approval." : "No goal files yet."}
        </Empty>
      )}
    </Section>
  );
}

/** Where Needs you rows lead. Without `filter` (the thread side panel) lists expand in place. */
export type NeedsYouNav = Pick<OverviewNav, "openTask" | "filterGoal"> & Partial<Pick<OverviewNav, "filter">>;

export function NeedsYou({
  view,
  nav,
  actions,
  onCloseGoal,
  goalOpenLabel,
}: {
  view: BoardView;
  nav: NeedsYouNav;
  actions: BoardActions;
  onCloseGoal: (goal: BoardGoal) => void;
  /** Accessible name of what a goal row opens; the full board filters Tasks by the goal. */
  goalOpenLabel?: string;
}) {
  const { askGoals, needsYou, toCheck } = view;
  const [allChecks, setAllChecks] = useState(false);
  const filter = nav.filter;
  const checkRows = allChecks ? toCheck : toCheck.slice(0, CHECK_ROWS);
  const idCh = idWidth([
    ...askGoals.map((g) => g.name),
    ...needsYou.map((t) => t.id),
    ...checkRows.map((t) => t.id),
  ]);
  const count =
    [
      askGoals.length && plural(askGoals.length, "goal"),
      needsYou.length && `${needsYou.length} to decide`,
      toCheck.length && `${toCheck.length} to check`,
    ]
      .filter(Boolean)
      .join(" · ") || "0";
  const empty = askGoals.length + needsYou.length + toCheck.length === 0;
  return (
    <Section title="Needs you" labelledBy="planr-needs-you" meta={count}>
      {empty ? (
        <Empty>
          Nothing waits for you: no goal to approve or close, no decision, nothing blocked or to check.
        </Empty>
      ) : null}
      {askGoals.length > 0 ? (
        <div role="group" aria-label="Goals">
          <GroupHeader label="Goals" count={askGoals.length} />
          {askGoals.map((g) => (
            <GoalRow
              key={g.name}
              goal={g}
              idCh={idCh}
              onOpen={() => nav.filterGoal(g.name)}
              openLabel={goalOpenLabel}
              trailing={<GoalActionButton goal={g} actions={actions} onClose={onCloseGoal} />}
            />
          ))}
        </div>
      ) : null}
      {needsYou.length > 0 ? (
        <div role="group" aria-label="To decide">
          <GroupHeader
            label="To decide"
            count={needsYou.length}
            action={
              filter ? (
                <MoreButton label="All" ariaLabel="All tasks to decide" onClick={() => filter("needs")} />
              ) : undefined
            }
          />
          {needsYou.map((t) => (
            <TaskRow key={t.id} task={t} idCh={idCh} meta={leverText(t)} onOpen={() => nav.openTask(t.id)} />
          ))}
        </div>
      ) : null}
      {toCheck.length > 0 ? (
        <div role="group" aria-label="To check">
          <GroupHeader
            label="To check"
            count={toCheck.length}
            action={
              filter ? (
                <MoreButton label="All" ariaLabel="All tasks to check" onClick={() => filter("check")} />
              ) : undefined
            }
          />
          {checkRows.map((t) => {
            const run = runAt(view, t);
            return (
              <TaskRow
                key={t.id}
                task={t}
                idCh={idCh}
                meta={run ? runLabel(view, run) : ""}
                onOpen={() => nav.openTask(t.id)}
                trailing={<CheckTaskButton task={t} actions={actions} compact />}
              />
            );
          })}
          {toCheck.length > checkRows.length ? (
            filter ? (
              <MoreRow
                label={`${toCheck.length - CHECK_ROWS} more in Tasks`}
                onClick={() => filter("check")}
              />
            ) : (
              <MoreRow
                label={`Show ${toCheck.length - checkRows.length} more`}
                icon="ChevronDown"
                onClick={() => setAllChecks(true)}
              />
            )
          ) : null}
        </div>
      ) : null}
    </Section>
  );
}

function RunRow({ view, run, onOpen }: { view: BoardView; run: BoardRun; onOpen: () => void }) {
  return (
    <button
      type="button"
      onClick={onOpen}
      className={cn(
        "flex w-full min-w-0 flex-col gap-1 border-b border-border-hairline px-3.5 py-2 text-left",
        "last:border-b-0 hover:bg-state-hover",
        "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring",
      )}
    >
      <span className="flex w-full min-w-0 items-baseline gap-2">
        <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{runLabel(view, run)}</span>
        <span className="shrink-0 font-mono text-xs text-subtle-foreground">
          {run.commit ?? "uncommitted"}
        </span>
        <span className="min-w-0 truncate text-sm">{run.subject}</span>
      </span>
      <span className="flex min-w-0 items-center gap-2 text-xs text-subtle-foreground">
        <ChangeCells changes={run.changes} />
        <span className="truncate">{runSummary(run)}</span>
      </span>
    </button>
  );
}

export function Overview({
  view,
  nav,
  actions,
  onCloseGoal,
}: {
  view: BoardView;
  nav: OverviewNav;
  actions: BoardActions;
  onCloseGoal: (goal: BoardGoal) => void;
}) {
  const { data } = view;
  const recent = [...data.runs]
    .reverse()
    .filter((r) => !r.initial)
    .slice(0, RUN_ROWS);
  const upNext = view.upNext.slice(0, LIST_ROWS);
  const levers = view.levers.slice(0, LIST_ROWS);
  const upNextCh = idWidth(upNext.map((t) => t.id));
  const leversCh = idWidth(levers.map((t) => t.id));
  const since = view.baseRun ? ` since ${dayKey(view.baseRun.date)}` : "";
  const movedMeta = `${view.changedSinceBase.size} tasks moved${since}`;
  return (
    <div
      className={cn(
        "mx-auto flex w-full max-w-[1600px] flex-col gap-3 px-3.5 py-3 @5xl:grid",
        "@5xl:grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)] @5xl:items-start",
      )}
    >
      <div className="@5xl:col-span-2">
        <Summary
          view={view}
          nav={nav}
          onWarnings={<WarningsButton view={view} onOpenTask={nav.openTask} />}
        />
      </div>
      <div className="flex min-w-0 flex-col gap-3">
        <GoalQueue view={view} nav={nav} />
        <NeedsYou view={view} nav={nav} actions={actions} onCloseGoal={onCloseGoal} />
      </div>
      <div className="flex min-w-0 flex-col gap-3">
        <Section
          title="Up next"
          labelledBy="planr-up-next"
          meta={`${view.upNext.length} ready`}
          action={
            view.upNext.length > 0 ? (
              <MoreButton label="All" ariaLabel="All ready tasks" onClick={() => nav.filter("ready")} />
            ) : undefined
          }
        >
          {upNext.length > 0 ? (
            upNext.map((t) => (
              <TaskRow
                key={t.id}
                task={t}
                idCh={upNextCh}
                meta={[t.goal[0], leverText(t)].filter(Boolean).join(" · ")}
                onOpen={() => nav.openTask(t.id)}
              />
            ))
          ) : (
            <Empty>No open task has all of its dependencies done.</Empty>
          )}
        </Section>
        <Section
          title="Recent runs"
          labelledBy="planr-recent-runs"
          meta={movedMeta}
          action={<MoreButton label="All" ariaLabel="All runs" onClick={nav.showRuns} />}
        >
          {recent.length > 0 ? (
            recent.map((r) => (
              <RunRow key={`${r.commit}-${r.date}`} view={view} run={r} onOpen={() => nav.openRun(r)} />
            ))
          ) : (
            <Empty>Only the baseline commit so far.</Empty>
          )}
        </Section>
        <Section
          title="Bottlenecks"
          labelledBy="planr-bottlenecks"
          meta="unfinished tasks that block the most"
        >
          {levers.length > 0 ? (
            levers.map((t) => (
              <TaskRow
                key={t.id}
                task={t}
                idCh={leversCh}
                meta={leverText(t)}
                onOpen={() => nav.openTask(t.id)}
              />
            ))
          ) : (
            <Empty>No unfinished task holds others up.</Empty>
          )}
        </Section>
        <Section title="Tracks" labelledBy="planr-tracks" meta={`${view.tracks.length}`}>
          <ul className="divide-y divide-border-hairline">
            {view.tracks.map((track) => {
              const ts = data.tasks.filter((t) => t.track === track);
              const done = ts.filter((t) => t.status === "done").length;
              return (
                <li key={track}>
                  <button
                    type="button"
                    onClick={() => nav.filterTrack(track)}
                    aria-label={`Track ${track}, ${done} of ${ts.length} done. Show its tasks`}
                    className={cn(
                      "grid w-full grid-cols-[minmax(4rem,9rem)_minmax(0,1fr)_auto] items-center gap-3",
                      "px-3.5 py-2 text-left hover:bg-state-hover",
                      "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset",
                      "focus-visible:ring-ring",
                    )}
                  >
                    <span className="truncate text-sm">{track}</span>
                    <StatusStrip items={progressItems(ts)} label={`${track} · `} />
                    <span className="text-xs tabular-nums text-subtle-foreground">
                      {done}/{ts.length}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        </Section>
      </div>
      <p className="text-xs text-subtle-foreground @5xl:col-span-2">
        Built from <span className="font-mono">{data.commit.split(" · ")[0] || "the working tree"}</span>
        {data.dirty ? " with uncommitted task changes" : ""} at {dayKey(data.generatedAt)}{" "}
        {hm(data.generatedAt)}
      </p>
    </div>
  );
}
