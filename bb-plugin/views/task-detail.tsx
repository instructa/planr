// Task detail: facts, notes, dependencies, dependents and status history.
import type { ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Icon } from "@/components/ui/icon";
import { cn } from "@/lib/utils";
import {
  dayKey,
  goalMark,
  hm,
  markOf,
  runAt,
  statusLabel,
  taskWarnings,
  type BoardRun,
  type BoardTask,
  type BoardView,
} from "@/lib/board";
import type { BoardActions } from "@/hooks/use-actions";
import { StatusMark, Tag } from "./marks";
import { CheckTaskButton, Empty, FOCUS_RING, GroupHeader, TaskRow, idWidth } from "./parts";

export function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[6.5rem_minmax(0,1fr)] items-baseline gap-2 py-1 text-sm">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="min-w-0 break-words">{children}</dd>
    </div>
  );
}

function Note({ label, children }: { label: string; children: ReactNode }) {
  return (
    <p
      className={cn(
        "rounded-md border border-border-hairline bg-muted/40 px-3 py-2 text-xs text-muted-foreground",
      )}
    >
      <span className="mr-1.5 font-medium text-foreground">{label}</span>
      {children}
    </p>
  );
}

const linkClass = cn(
  "rounded-sm underline decoration-border underline-offset-4 hover:decoration-current",
  FOCUS_RING,
);

export function TaskDetail({
  view,
  task,
  missingId,
  actions,
  onClose,
  closeLabel,
  onOpenTask,
  onOpenRun,
  onFilterGoal,
  backLabel = "Back to tasks (Esc)",
}: {
  view: BoardView;
  task: BoardTask | null;
  missingId: string;
  actions: BoardActions;
  onClose: () => void;
  closeLabel: "back" | "close";
  onOpenTask: (id: string) => void;
  onOpenRun: (run: BoardRun) => void;
  onFilterGoal: (name: string) => void;
  backLabel?: string;
}) {
  const header = (
    <div
      className={cn(
        "sticky top-0 z-10 flex h-10 shrink-0 items-center gap-2 border-b border-border-hairline",
        "bg-background px-3.5",
      )}
    >
      {closeLabel === "back" ? (
        <Button
          variant="ghost"
          size="icon"
          className="-ml-1.5 size-7"
          aria-label={backLabel}
          onClick={onClose}
        >
          <Icon name="ChevronLeft" className="size-4" />
        </Button>
      ) : null}
      {task !== null ? (
        <>
          <StatusMark kind={markOf(task.status)} />
          <span className="font-mono text-xs">{task.id}</span>
          <span className="text-xs text-muted-foreground">{statusLabel(task.status)}</span>
        </>
      ) : (
        <span className="font-mono text-xs">{missingId}</span>
      )}
      {closeLabel === "close" ? (
        <Button
          variant="ghost"
          size="icon"
          className="ml-auto size-7 text-muted-foreground hover:text-foreground"
          aria-label="Close detail (Esc)"
          onClick={onClose}
        >
          <Icon name="X" className="size-3.5" />
        </Button>
      ) : null}
    </div>
  );
  if (task === null) {
    return (
      <div className="flex min-h-full flex-col">
        {header}
        <Empty>This task is not in the board anymore.</Empty>
      </div>
    );
  }
  const run = runAt(view, task);
  const warnings = taskWarnings(view.data, task.id);
  const deps = task.depends.flatMap((id) => view.byId.get(id) ?? []);
  const dependents = task.waitingOn.flatMap((id) => view.byId.get(id) ?? []);
  const missingDeps = task.depends.filter((id) => !view.byId.has(id));
  const idCh = idWidth([...task.depends, ...task.waitingOn]);
  return (
    <div className="flex min-h-full flex-col">
      {header}
      <div className="space-y-3 px-3.5 py-3">
        <h2 className="text-base font-semibold leading-snug [text-wrap:pretty]">{task.title}</h2>
        {task.status === "review" ? (
          <div className="flex flex-wrap items-center gap-2">
            <CheckTaskButton task={task} actions={actions} variant="default" />
            <span className="text-xs text-muted-foreground">
              {actions.blocked ?? "Sets the task to done after your manual check."}
            </span>
          </div>
        ) : null}
        <dl>
          <Fact label="Track">{task.track}</Fact>
          {task.owner ? <Fact label="Owner">{task.owner}</Fact> : null}
          {task.goal.length > 0 ? (
            <Fact label="Goal">
              <span className="flex flex-wrap gap-x-2 gap-y-0.5">
                {task.goal.map((name) => {
                  const goal = view.goalByName.get(name);
                  return (
                    <button
                      key={name}
                      type="button"
                      className={cn("inline-flex items-center gap-1", linkClass)}
                      onClick={() => onFilterGoal(name)}
                      aria-label={`Show tasks of goal ${name}`}
                    >
                      {goal ? <StatusMark kind={goalMark(goal)} className="size-3" /> : null}
                      <span className="font-mono text-xs">{name}</span>
                    </button>
                  );
                })}
              </span>
            </Fact>
          ) : null}
          {task.unlocks > 0 ? <Fact label="Unlocks">{task.unlocks} when satisfied</Fact> : null}
          {run !== null ? (
            <Fact label="Moved">
              <button type="button" className={linkClass} onClick={() => onOpenRun(run)}>
                {dayKey(run.date)} {hm(run.date)}
              </button>
            </Fact>
          ) : null}
          <Fact label="File">
            <span className="break-all font-mono text-xs">{task.file}</span>
          </Fact>
          {task.ready ? (
            <Fact label="Tags">
              <span className="flex gap-1">
                <Tag>ready</Tag>
              </span>
            </Fact>
          ) : null}
        </dl>
        {task.status === "review" || warnings.length > 0 ? (
          <div className="space-y-1.5">
            {task.status === "review" ? (
              <Note label="Check">Built and tested by agents; waiting for your manual check.</Note>
            ) : null}
            {warnings.map((w) => (
              <Note key={w} label="Warning">
                {w}
              </Note>
            ))}
          </div>
        ) : null}
      </div>
      <div role="group" aria-label="Needs">
        <GroupHeader
          label="Needs"
          count={task.depends.length}
          action={
            task.openDeps.length > 0 ? (
              <span className="font-normal">{task.openDeps.length} not done</span>
            ) : undefined
          }
        />
        {deps.length > 0 || missingDeps.length > 0 ? (
          <>
            {deps.map((d) => (
              <TaskRow
                key={d.id}
                task={d}
                idCh={idCh}
                meta={statusLabel(d.status)}
                onOpen={() => onOpenTask(d.id)}
              />
            ))}
            {missingDeps.map((id) => (
              <p
                key={id}
                className={cn(
                  "flex h-[34px] items-center gap-2 border-b border-border-hairline px-3.5 text-sm",
                  "text-muted-foreground",
                )}
              >
                <StatusMark kind="removed" />
                <span className="font-mono text-xs">{id}</span> unknown task
              </p>
            ))}
          </>
        ) : (
          <Empty>No dependencies.</Empty>
        )}
      </div>
      <div role="group" aria-label="Waiting on this">
        <GroupHeader
          label="Waiting on this"
          count={task.waitingOn.length}
          action={
            task.unlocks > 0 ? <span className="font-normal">{task.unlocks} ready once done</span> : undefined
          }
        />
        {dependents.length > 0 ? (
          dependents.map((d) => (
            <TaskRow
              key={d.id}
              task={d}
              idCh={idCh}
              meta={statusLabel(d.status)}
              onOpen={() => onOpenTask(d.id)}
            />
          ))
        ) : (
          <Empty>Nothing open depends on it.</Empty>
        )}
      </div>
      <div role="group" aria-label="History" className="pb-3">
        <GroupHeader label="History" count={task.history.length} />
        {task.history.length > 0 ? (
          <ol>
            {[...task.history].reverse().map((h, i) => {
              const at = view.data.runs[h.run];
              return (
                <li
                  key={`${h.run}-${i}`}
                  className={cn(
                    "flex h-8 items-center gap-3 border-b border-border-hairline px-3.5 text-xs",
                    "last:border-b-0",
                  )}
                >
                  <span className="w-20 shrink-0 tabular-nums text-muted-foreground">
                    {at ? `${dayKey(at.date)} ${hm(at.date)}` : "·"}
                  </span>
                  <span className="flex min-w-0 flex-1 items-center gap-1.5">
                    <StatusMark kind={h.from === null ? "new" : markOf(h.from)} className="size-3" />
                    {h.from === null ? "New" : statusLabel(h.from)}
                    <Icon name="ArrowRight" className="size-3 text-subtle-foreground" />
                    <StatusMark kind={markOf(h.to)} className="size-3" />
                    {statusLabel(h.to)}
                  </span>
                  {at ? (
                    <button
                      type="button"
                      title={at.subject}
                      onClick={() => onOpenRun(at)}
                      className={cn(
                        "shrink-0 font-mono text-subtle-foreground hover:text-foreground",
                        linkClass,
                      )}
                    >
                      {at.commit ?? "uncommitted"}
                    </button>
                  ) : null}
                </li>
              );
            })}
          </ol>
        ) : (
          <Empty>Unchanged since the first commit.</Empty>
        )}
      </div>
    </div>
  );
}
