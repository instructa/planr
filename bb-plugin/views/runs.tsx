// Runs: one per commit that changed task statuses. A trend of stacked bars,
// the run list grouped by day, and the changes of the selected run. Narrow
// panels show the list, then the selected run with a back button.
import { useEffect, useRef } from "react";
import { Button } from "@/components/ui/button";
import { Icon } from "@/components/ui/icon";
import { cn } from "@/lib/utils";
import {
  dayKey,
  hm,
  markOf,
  PROGRESS,
  runKey,
  runSummary,
  statusLabel,
  statusShort,
  type BoardRun,
  type BoardView,
} from "@/lib/board";
import { isBoardKey, useElementWidth } from "@/hooks/use-width";
import { ChangeCells, StatusMark, statusBg, Tip } from "./marks";
import { Empty, EmptyState, GroupHeader, ROW_FOCUS, TaskRow, idWidth } from "./parts";

const SPLIT_MIN = 760;
const CHANGE_ORDER = [
  "new",
  "done",
  "review",
  "in-progress",
  "open",
  "decision",
  "blocked",
  "dropped",
  "removed",
];

function Trend({
  view,
  selected,
  width,
  onSelect,
}: {
  view: BoardView;
  selected: number;
  width: number;
  onSelect: (i: number) => void;
}) {
  const runs = view.data.runs;
  const max = Math.max(1, ...runs.map((r) => r.total));
  const days: Array<{ day: string; n: number }> = [];
  for (const r of runs) {
    const day = dayKey(r.date);
    const last = days.at(-1);
    if (last?.day === day) last.n++;
    else days.push({ day, n: 1 });
  }
  return (
    <div className="shrink-0 border-b border-border-hairline px-3.5 pb-2 pt-3">
      <div className="flex h-14 items-end gap-0.5" role="group" aria-label="Task counts per run">
        {runs.map((r, i) => (
          <Tip
            key={`${runKey(r)}-${i}`}
            content={
              <span className="block space-y-0.5">
                <span className="block font-medium">
                  {dayKey(r.date)} {hm(r.date)} ·{" "}
                  <span className="font-mono">{r.commit ?? "uncommitted"}</span>
                </span>
                <span className="block">{r.subject}</span>
                <span className="block opacity-80">
                  {PROGRESS.filter((s) => r.counts[s])
                    .map((s) => `${statusShort(s)} ${r.counts[s]}`)
                    .join(" · ")}
                </span>
              </span>
            }
          >
            <button
              type="button"
              aria-pressed={i === selected}
              aria-label={`Run ${dayKey(r.date)} ${hm(r.date)}: ${r.subject}`}
              onClick={() => onSelect(i)}
              className={cn(
                "flex h-full min-w-0.5 flex-1 flex-col-reverse gap-px rounded-[2px] outline-offset-2",
                "hover:opacity-80",
                "focus-visible:outline focus-visible:outline-1 focus-visible:outline-ring",
                i === selected && "outline outline-1 outline-foreground",
              )}
            >
              {PROGRESS.filter((s) => r.counts[s]).map((s) => (
                <span
                  key={s}
                  className={cn("w-full shrink-0 first:rounded-b-[2px]", statusBg(s))}
                  style={{ height: `calc(${(r.counts[s] / max) * 100}% - 1px)` }}
                />
              ))}
            </button>
          </Tip>
        ))}
      </div>
      <div className="mt-1 flex text-[10px] tabular-nums text-subtle-foreground" aria-hidden>
        {days.map((d, i) => (
          <span
            key={`${d.day}-${i}`}
            className="min-w-0 overflow-hidden whitespace-nowrap"
            style={{ flex: d.n }}
          >
            {/* A day label needs about 40px; narrower days stay unlabeled. */}
            {width === 0 || (d.n / runs.length) * width >= 40 ? d.day : ""}
          </span>
        ))}
      </div>
    </div>
  );
}

function RunList({
  view,
  selected,
  onSelect,
}: {
  view: BoardView;
  selected: number | null;
  onSelect: (i: number) => void;
}) {
  const runs = view.data.runs;
  const refs = useRef(new Map<number, HTMLButtonElement>());
  useEffect(() => {
    if (selected !== null) refs.current.get(selected)?.scrollIntoView({ block: "nearest" });
  }, [selected]);
  const ordered = runs.map((run, i) => ({ run, i })).reverse();
  const groups: Array<{ day: string; items: typeof ordered }> = [];
  for (const item of ordered) {
    const day = dayKey(item.run.date);
    const last = groups.at(-1);
    if (last?.day === day) last.items.push(item);
    else groups.push({ day, items: [item] });
  }
  return (
    <div>
      {groups.map((g) => (
        <section key={g.day} aria-label={g.day === view.today ? "Today" : g.day}>
          <div
            className={cn(
              "sticky top-0 z-10 flex h-8 items-center gap-2 border-b border-border-hairline bg-background",
              "px-3.5 text-xs font-semibold",
            )}
          >
            {g.day === view.today ? "Today" : g.day}
            <span className="font-normal tabular-nums text-subtle-foreground">{g.items.length}</span>
          </div>
          {g.items.map(({ run, i }) => (
            <button
              key={`${runKey(run)}-${i}`}
              ref={(el) => {
                if (el) refs.current.set(i, el);
                else refs.current.delete(i);
              }}
              type="button"
              onClick={() => onSelect(i)}
              aria-current={i === selected ? "true" : undefined}
              className={cn(
                "flex w-full min-w-0 flex-col gap-1 border-b border-border-hairline px-3.5 py-2 text-left",
                "hover:bg-state-hover",
                ROW_FOCUS,
                i === selected && "bg-state-active hover:bg-state-active",
              )}
            >
              <span className="flex w-full min-w-0 items-baseline gap-2">
                <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{hm(run.date)}</span>
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
          ))}
        </section>
      ))}
    </div>
  );
}

function RunChanges({
  view,
  run,
  index,
  onOpenTask,
  onShowInTasks,
}: {
  view: BoardView;
  run: BoardRun;
  index: number;
  onOpenTask: (id: string) => void;
  onShowInTasks: (index: number) => void;
}) {
  const groups = new Map<string, BoardRun["changes"]>();
  for (const c of run.changes) {
    const key = c.from === null ? "new" : c.to;
    groups.set(key, [...(groups.get(key) ?? []), c]);
  }
  return (
    <div className="min-w-0">
      <div className="space-y-1 border-b border-border-hairline px-3.5 py-3">
        <h2 className="text-sm font-semibold [text-wrap:pretty]">{run.subject}</h2>
        <p className="text-xs tabular-nums text-muted-foreground">
          {dayKey(run.date)} {hm(run.date)} ·{" "}
          <span className="font-mono">{run.commit ?? "uncommitted (git status)"}</span> · {run.total} tasks
        </p>
        {run.changes.length > 0 ? (
          <Button
            variant="outline"
            size="sm"
            className="mt-1.5 h-7 gap-1.5 text-xs"
            onClick={() => onShowInTasks(index)}
          >
            Show {run.changes.length} in Tasks
            <Icon name="ArrowRight" className="size-3" />
          </Button>
        ) : null}
      </div>
      {run.initial ? (
        <Empty>Baseline: the first commit of the planning repository.</Empty>
      ) : run.changes.length === 0 ? (
        <Empty>No status changed in this run.</Empty>
      ) : (
        CHANGE_ORDER.filter((k) => groups.has(k)).map((k) => {
          const changes = groups.get(k) ?? [];
          const idCh = idWidth(changes.map((c) => c.id));
          return (
            <div
              key={k}
              role="group"
              aria-label={k === "new" ? "Added" : `To ${statusLabel(k).toLowerCase()}`}
            >
              <GroupHeader
                label={k === "new" ? "Added" : `To ${statusLabel(k).toLowerCase()}`}
                count={changes.length}
                icon={<StatusMark kind={k === "new" ? "new" : markOf(k)} className="size-3" />}
              />
              {changes.map((c) => {
                const task = view.byId.get(c.id);
                const meta =
                  c.from === null
                    ? `New, ${statusLabel(c.to).toLowerCase()}`
                    : `${statusLabel(c.from)} → ${statusLabel(c.to)}`;
                return task ? (
                  <TaskRow key={c.id} task={task} idCh={idCh} meta={meta} onOpen={() => onOpenTask(c.id)} />
                ) : (
                  <div
                    key={c.id}
                    className={cn(
                      "flex h-[34px] items-center gap-2 border-b border-border-hairline px-3.5",
                      "last:border-b-0",
                    )}
                  >
                    <StatusMark kind="removed" />
                    <span
                      className="shrink-0 font-mono text-xs text-subtle-foreground"
                      style={{ width: idCh }}
                    >
                      {c.id}
                    </span>
                    <span className="min-w-0 truncate text-sm text-muted-foreground">
                      No longer in the index
                    </span>
                  </div>
                );
              })}
            </div>
          );
        })
      )}
    </div>
  );
}

export function RunsView({
  view,
  item,
  onSelect,
  onBack,
  onOpenTask,
  onShowInTasks,
}: {
  view: BoardView;
  item: string | null;
  onSelect: (run: BoardRun | null, replace?: boolean) => void;
  onBack: () => void;
  onOpenTask: (id: string) => void;
  onShowInTasks: (index: number) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const width = useElementWidth(ref);
  const split = width === 0 || width >= SPLIT_MIN;
  const runs = view.data.runs;
  const found = item === null ? -1 : runs.findIndex((r) => runKey(r) === item);
  const selected = found >= 0 ? found : split && runs.length > 0 ? runs.length - 1 : null;

  const stateRef = useRef({ selected, runs, onSelect });
  stateRef.current = { selected, runs, onSelect };
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!isBoardKey(event)) return;
      const dir = ({ j: 1, ArrowDown: 1, k: -1, ArrowUp: -1 } as Record<string, number>)[event.key];
      const { selected: current, runs: list, onSelect: select } = stateRef.current;
      if (dir === undefined || list.length === 0) return;
      event.preventDefault();
      const from = current ?? list.length;
      const next = Math.max(0, Math.min(list.length - 1, from - dir));
      select(list[next], true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (runs.length === 0) {
    return (
      <div ref={ref} className="h-full">
        <EmptyState icon="Clock" title="No runs yet" description="No commit has changed a task status yet." />
      </div>
    );
  }
  const run = selected === null ? null : runs[selected];
  const select = (i: number) => onSelect(runs[i]);
  return (
    <div ref={ref} className="flex h-full min-h-0 flex-col">
      <Trend view={view} selected={selected ?? -1} width={width} onSelect={select} />
      {split ? (
        <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,0.95fr)_minmax(0,1.05fr)]">
          <div className="min-h-0 overflow-y-auto border-r border-border-hairline">
            <RunList view={view} selected={selected} onSelect={select} />
          </div>
          <div className="min-h-0 overflow-y-auto">
            {run !== null && selected !== null ? (
              <RunChanges
                view={view}
                run={run}
                index={selected}
                onOpenTask={onOpenTask}
                onShowInTasks={onShowInTasks}
              />
            ) : null}
          </div>
        </div>
      ) : run !== null && selected !== null ? (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="flex h-9 items-center border-b border-border-hairline px-2">
            <Button variant="ghost" size="sm" className="h-7 gap-1 px-2 text-xs" onClick={onBack}>
              <Icon name="ChevronLeft" className="size-3.5" />
              All runs
            </Button>
          </div>
          <RunChanges
            view={view}
            run={run}
            index={selected}
            onOpenTask={onOpenTask}
            onShowInTasks={onShowInTasks}
          />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <RunList view={view} selected={null} onSelect={select} />
        </div>
      )}
    </div>
  );
}
