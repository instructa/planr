// The Planr board in a thread's side panel: the board of the thread's project, compact.
// Needs you, the goal queue and Up next with the same actions, gating and toasts as the
// full board; task and goal rows open a detail inside the panel, Esc or Back returns.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useBbNavigate, type PluginThreadPanelProps } from "@get-bb/plugin-sdk/app";
import { Button } from "@/components/ui/button";
import { Icon } from "@/components/ui/icon";
import { TooltipProvider } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import {
  boardView,
  dayKey,
  goalBuilt,
  goalMark,
  hm,
  leverText,
  parseBoard,
  runKey,
  statusLabel,
  type BoardGoal,
  type BoardView,
} from "@/lib/board";
import { buildSubPath, PANEL_PATH, type PanelRoute } from "@/lib/route";
import { useActionAccess, useBoardActions, type BoardActions } from "@/hooks/use-actions";
import { useBoard, useProjects, useSession, useThreadProject } from "@/hooks/use-planr";
import { CloseGoalDialog } from "./close-goal";
import { StatusMark, StatusStrip, Tip, progressItems } from "./marks";
import { NeedsYou, WarningsButton, goalNote } from "./overview";
import {
  Empty,
  EmptyState,
  FOCUS_RING,
  GoalActionButton,
  GroupHeader,
  LoadingBoard,
  MoreRow,
  RefreshFailedNotice,
  Row,
  Section,
  TaskRow,
  idWidth,
} from "./parts";
import { useHosts } from "./setup";
import { Fact, TaskDetail } from "./task-detail";
import { RefreshButton, RepoStatus, SessionStatus, useRepoRefresh } from "./topbar";

type Detail = { kind: "task"; id: string } | { kind: "goal"; name: string };

const UP_NEXT_ROWS = 8;

/** Opens the full Planr page; navigating there leaves the thread. */
function useOpenFullBoard() {
  const navigate = useBbNavigate();
  return useCallback(
    (route: PanelRoute) => navigate.toPluginPanel(PANEL_PATH, { subPath: buildSubPath(route) }),
    [navigate],
  );
}

const overviewOf = (projectId: string | null): PanelRoute => ({ projectId, view: "overview", item: null });

function GoalQueueRow({
  view,
  goal,
  idCh,
  onOpen,
}: {
  view: BoardView;
  goal: BoardGoal;
  idCh: string;
  onOpen: () => void;
}) {
  const scope = goal.scope.flatMap((id) => view.byId.get(id) ?? []);
  const built = `${goalBuilt(goal)}/${goal.total}`;
  return (
    <Row
      label={`Goal ${goal.name}: ${goal.title ?? goal.name}, ${goal.state}, ${built} built. Show details`}
      onOpen={onOpen}
    >
      <StatusMark kind={goalMark(goal)} />
      <span
        className="shrink-0 truncate font-mono text-xs text-subtle-foreground"
        style={{ width: idCh }}
        title={goal.name}
      >
        {goal.name}
      </span>
      <span className="min-w-0 flex-1 truncate text-sm" title={goal.title ?? goal.name}>
        {goal.title ?? goal.name}
      </span>
      <span className="hidden shrink-0 text-xs text-subtle-foreground @md:inline">{goal.state}</span>
      <StatusStrip
        items={progressItems(scope)}
        label={`${goal.name} · `}
        className="hidden w-12 shrink-0 @sm:flex"
      />
      <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{built}</span>
    </Row>
  );
}

function GoalQueue({ view, onOpenGoal }: { view: BoardView; onOpenGoal: (name: string) => void }) {
  const goals = view.data.goals;
  const of = (s: string) => goals.filter((g) => g.state === s).length;
  const counts = ["running", "approved", "draft", "review"]
    .filter((s) => of(s))
    .map((s) => `${of(s)} ${s}`)
    .join(" · ");
  const idCh = idWidth(view.queued.map((g) => g.name));
  return (
    <Section title="Goals" labelledBy="planr-panel-goals" meta={counts || undefined}>
      {view.queued.length > 0 ? (
        view.queued.map((g) => (
          <GoalQueueRow key={g.name} view={view} goal={g} idCh={idCh} onOpen={() => onOpenGoal(g.name)} />
        ))
      ) : (
        <Empty>
          {goals.length ? "No goal is running, approved or waiting for approval." : "No goal files yet."}
        </Empty>
      )}
    </Section>
  );
}

function UpNext({ view, onOpenTask }: { view: BoardView; onOpenTask: (id: string) => void }) {
  const [all, setAll] = useState(false);
  const rows = all ? view.upNext : view.upNext.slice(0, UP_NEXT_ROWS);
  const idCh = idWidth(rows.map((t) => t.id));
  return (
    <Section title="Up next" labelledBy="planr-panel-up-next" meta={`${view.upNext.length} ready`}>
      {rows.length > 0 ? (
        <>
          {rows.map((t) => (
            <TaskRow
              key={t.id}
              task={t}
              idCh={idCh}
              meta={[t.goal[0], leverText(t)].filter(Boolean).join(" · ")}
              onOpen={() => onOpenTask(t.id)}
            />
          ))}
          {view.upNext.length > rows.length ? (
            <MoreRow
              label={`Show ${view.upNext.length - rows.length} more`}
              icon="ChevronDown"
              onClick={() => setAll(true)}
            />
          ) : null}
        </>
      ) : (
        <Empty>No open task has all of its dependencies done.</Empty>
      )}
    </Section>
  );
}

const linkClass = cn(
  "rounded-sm underline decoration-border underline-offset-4 hover:decoration-current",
  FOCUS_RING,
);

const GOAL_ACTION_NOTE: Record<string, string> = {
  draft: "Sets the goal from draft to approved.",
  review: "Sets the goal from review to done.",
};

/** A goal inside the panel: its facts, the human action it waits for and its scope. */
function GoalDetail({
  view,
  goal,
  missingName,
  actions,
  onBack,
  onOpenTask,
  onCloseGoal,
}: {
  view: BoardView;
  goal: BoardGoal | null;
  missingName: string;
  actions: BoardActions;
  onBack: () => void;
  onOpenTask: (id: string) => void;
  onCloseGoal: (goal: BoardGoal) => void;
}) {
  const navigate = useBbNavigate();
  const header = (
    <div
      className={cn(
        "sticky top-0 z-10 flex h-10 shrink-0 items-center gap-2 border-b border-border-hairline",
        "bg-background px-3.5",
      )}
    >
      <Button variant="ghost" size="icon" className="-ml-1.5 size-7" aria-label="Back (Esc)" onClick={onBack}>
        <Icon name="ChevronLeft" className="size-4" />
      </Button>
      {goal !== null ? <StatusMark kind={goalMark(goal)} /> : null}
      <span className="min-w-0 truncate font-mono text-xs">{goal?.name ?? missingName}</span>
      {goal !== null ? <span className="shrink-0 text-xs text-muted-foreground">{goal.state}</span> : null}
    </div>
  );
  if (goal === null) {
    return (
      <div className="flex min-h-full flex-col">
        {header}
        <Empty>This goal is not in the board anymore.</Empty>
      </div>
    );
  }
  const scope = goal.scope.flatMap((id) => view.byId.get(id) ?? []).sort(view.sorts.status);
  const missing = goal.scope.filter((id) => !view.byId.has(id));
  const idCh = idWidth(goal.scope);
  const note = goalNote(goal);
  const thread = goal.thread;
  return (
    <div className="flex min-h-full flex-col">
      {header}
      <div className="space-y-3 px-3.5 py-3">
        <h2 className="text-base font-semibold leading-snug [text-wrap:pretty]">{goal.title ?? goal.name}</h2>
        {Object.hasOwn(GOAL_ACTION_NOTE, goal.state) ? (
          <div className="flex flex-wrap items-center gap-2">
            <GoalActionButton goal={goal} actions={actions} onClose={onCloseGoal} />
            <span className="text-xs text-muted-foreground">
              {actions.blocked ?? GOAL_ACTION_NOTE[goal.state]}
            </span>
          </div>
        ) : null}
        <dl>
          <Fact label="Built">
            <span className="tabular-nums">
              {goalBuilt(goal)} of {goal.total}
            </span>
            {note ? <span className="text-muted-foreground"> · {note}</span> : null}
          </Fact>
          {goal.stream ? <Fact label="Stream">{goal.stream}</Fact> : null}
          {goal.budget ? <Fact label="Budget">{goal.budget}</Fact> : null}
          {thread ? (
            <Fact label="Thread">
              {thread.startsWith("thr_") ? (
                <button
                  type="button"
                  className={cn("font-mono text-xs", linkClass)}
                  aria-label={`Open thread ${thread}`}
                  onClick={() => navigate.toThread(thread)}
                >
                  {thread}
                </button>
              ) : (
                <span className="font-mono text-xs">{thread}</span>
              )}
            </Fact>
          ) : null}
          <Fact label="File">
            <span className="break-all font-mono text-xs">{goal.file}</span>
          </Fact>
        </dl>
        <StatusStrip items={progressItems(scope)} label={`${goal.name} · `} className="h-2" />
      </div>
      <div role="group" aria-label="Tasks" className="pb-3">
        <GroupHeader label="Tasks" count={goal.scope.length} />
        {scope.length > 0 || missing.length > 0 ? (
          <>
            {scope.map((t) => (
              <TaskRow
                key={t.id}
                task={t}
                idCh={idCh}
                meta={statusLabel(t.status)}
                onOpen={() => onOpenTask(t.id)}
              />
            ))}
            {missing.map((id) => (
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
          <Empty>The goal names no tasks.</Empty>
        )}
      </div>
    </div>
  );
}

function PanelBoard({ projectId }: { projectId: string }) {
  const board = useBoard(projectId);
  const projects = useProjects();
  const session = useSession();
  const result = board.data;
  const ok = result?.status === "ok" ? result : null;
  const view = useMemo(() => (ok === null ? null : boardView(parseBoard(ok.data))), [ok]);
  const repo = useRepoRefresh(projectId, board.refetch);
  const access = useActionAccess(session.data, session.refetch);
  const actions = useBoardActions(projectId, ok?.fileSha ?? null, board.refetch, repo, access);
  const hosts = useHosts();
  const openFull = useOpenFullBoard();
  const [closing, setClosing] = useState<BoardGoal | null>(null);
  const name = projects.data?.find((p) => p.id === projectId)?.name ?? "";

  // Details stack over the board, which stays mounted (and keeps its scroll) underneath.
  const [stack, setStack] = useState<Detail[]>([]);
  const detail = stack.at(-1) ?? null;
  const stackRef = useRef(stack);
  stackRef.current = stack;
  const opener = useRef<HTMLElement | null>(null);
  const detailRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const push = useCallback((next: Detail) => {
    if (stackRef.current.length === 0) {
      const active = document.activeElement;
      opener.current = active instanceof HTMLElement ? active : null;
    }
    setStack((s) => [...s, next]);
  }, []);
  const back = useCallback(() => setStack((s) => s.slice(0, -1)), []);
  const openTask = useCallback((id: string) => push({ kind: "task", id }), [push]);
  const openGoal = useCallback((name: string) => push({ kind: "goal", name }), [push]);
  const needsNav = useMemo(() => ({ openTask, filterGoal: openGoal }), [openTask, openGoal]);

  useEffect(() => {
    if (detail !== null) {
      detailRef.current?.focus({ preventScroll: true });
      return;
    }
    if (opener.current?.isConnected) opener.current.focus();
    opener.current = null;
  }, [detail]);

  // Esc goes back, but only for keys pressed inside the panel (not in popovers or the thread).
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key !== "Escape" || event.defaultPrevented || stackRef.current.length === 0) return;
    if (!(event.target instanceof Node) || rootRef.current?.contains(event.target) !== true) return;
    event.preventDefault();
    event.stopPropagation();
    back();
  };

  const fullRoute: PanelRoute =
    detail?.kind === "task" ? { projectId, view: "tasks", item: detail.id } : overviewOf(projectId);

  let body: React.ReactNode;
  if (result === undefined) {
    body =
      board.error !== null ? (
        <EmptyState icon="AlertTriangle" title="The board could not be loaded" description={board.error}>
          <Button size="sm" variant="outline" className="h-7" onClick={board.refetch}>
            Try again
          </Button>
        </EmptyState>
      ) : (
        <LoadingBoard />
      );
  } else if (result.status === "unmapped") {
    body = (
      <EmptyState
        icon="Workflow"
        title="No planning repository connected"
        description={
          `Connect ${name || "this project"} to its planning repository in Planr to see its board here.`
        }
      >
        <Button size="sm" className="h-7" onClick={() => openFull(overviewOf(projectId))}>
          Set up in Planr
        </Button>
      </EmptyState>
    );
  } else if (result.status === "error" || view === null) {
    body = (
      <EmptyState
        icon="AlertTriangle"
        title="The board could not be built"
        description={result.status === "error" ? result.message : "Unexpected board payload."}
      >
        <Button size="sm" variant="outline" className="h-7" onClick={board.refetch}>
          Try again
        </Button>
      </EmptyState>
    );
  } else {
    body = (
      <div className="flex flex-col gap-2.5 p-2.5">
        <NeedsYou
          view={view}
          nav={needsNav}
          actions={actions}
          onCloseGoal={setClosing}
          goalOpenLabel="Show details"
        />
        <GoalQueue view={view} onOpenGoal={openGoal} />
        <UpNext view={view} onOpenTask={openTask} />
        <footer className="flex flex-wrap items-center gap-x-2 gap-y-1 px-1 text-xs text-subtle-foreground">
          <span>
            Built from{" "}
            <span className="font-mono">{view.data.commit.split(" · ")[0] || "the working tree"}</span>
            {view.data.dirty ? " with uncommitted task changes" : ""} at {dayKey(view.data.generatedAt)}{" "}
            {hm(view.data.generatedAt)}
          </span>
          <span className="ml-auto">
            <WarningsButton view={view} onOpenTask={openTask} />
          </span>
        </footer>
      </div>
    );
  }

  return (
    <div ref={rootRef} className="flex h-full min-h-0 flex-col" onKeyDown={onKeyDown}>
      <header className="flex h-10 shrink-0 items-center gap-1 border-b border-border-hairline px-2">
        <span className="min-w-0 flex-1 truncate px-1 text-sm font-semibold" title={name}>
          {name}
        </span>
        {ok !== null && view !== null ? (
          <>
            {session.data !== undefined ? <SessionStatus session={session.data} /> : null}
            <RepoStatus
              repo={ok.repo}
              lastCommit={view.data.commit}
              hostName={hosts.find((h) => h.id === ok.repo.hostId)?.name ?? null}
              loadedAt={board.loadedAt}
              refreshing={repo.refreshing}
              onRefresh={() => void repo.refresh()}
            />
            <RefreshButton refreshing={repo.refreshing} onRefresh={() => void repo.refresh()} />
          </>
        ) : null}
        <Tip content="Open full board" side="bottom">
          <Button
            variant="ghost"
            size="sm"
            aria-label="Open full board"
            className={cn(
              "h-7 shrink-0 gap-1 px-2 text-xs font-normal text-muted-foreground hover:text-foreground",
            )}
            onClick={() => openFull(fullRoute)}
          >
            <Icon name="ArrowUpRight" className="size-3.5" />
            <span className="hidden @md:inline">Full board</span>
          </Button>
        </Tip>
      </header>
      {board.error !== null && result !== undefined ? (
        <RefreshFailedNotice error={board.error} onRetry={board.refetch} />
      ) : null}
      <div className="relative min-h-0 flex-1">
        <div className="isolate h-full overflow-y-auto" inert={detail !== null}>
          {body}
        </div>
        {detail !== null && view !== null ? (
          <div
            key={`${stack.length}:${detail.kind}:${detail.kind === "task" ? detail.id : detail.name}`}
            ref={detailRef}
            tabIndex={-1}
            role="region"
            aria-label={detail.kind === "task" ? `Task ${detail.id}` : `Goal ${detail.name}`}
            className="absolute inset-0 z-10 overflow-y-auto bg-background outline-none"
          >
            {detail.kind === "task" ? (
              <TaskDetail
                view={view}
                task={view.byId.get(detail.id) ?? null}
                missingId={detail.id}
                actions={actions}
                closeLabel="back"
                backLabel="Back (Esc)"
                onClose={back}
                onOpenTask={openTask}
                onOpenRun={(run) => openFull({ projectId, view: "runs", item: runKey(run) })}
                onFilterGoal={openGoal}
              />
            ) : (
              <GoalDetail
                view={view}
                goal={view.goalByName.get(detail.name) ?? null}
                missingName={detail.name}
                actions={actions}
                onBack={back}
                onOpenTask={openTask}
                onCloseGoal={setClosing}
              />
            )}
          </div>
        ) : null}
      </div>
      <CloseGoalDialog
        goal={closing}
        hasRemote={ok?.repo.hasRemote ?? false}
        actions={actions}
        onOpenChange={(open) => {
          if (!open) setClosing(null);
        }}
      />
    </div>
  );
}

/** Before the board: the thread's project is still unknown, missing or not readable. */
function ProjectState({ threadId }: { threadId: string }) {
  const project = useThreadProject(threadId);
  const openFull = useOpenFullBoard();
  if (project.projectId !== undefined && project.projectId !== null) {
    return <PanelBoard key={project.projectId} projectId={project.projectId} />;
  }
  const openPlanr = (
    <Button size="sm" variant="outline" className="h-7" onClick={() => openFull(overviewOf(null))}>
      Open Planr
    </Button>
  );
  if (project.projectId === null) {
    return (
      <EmptyState
        icon="Workflow"
        title="This thread has no project"
        description="Planr shows the board of a project's planning repository."
      >
        {openPlanr}
      </EmptyState>
    );
  }
  if (project.error !== null) {
    return (
      <EmptyState icon="AlertTriangle" title="The thread's project is unknown" description={project.error}>
        <div className="flex flex-wrap justify-center gap-2">
          <Button size="sm" variant="outline" className="h-7" onClick={project.refetch}>
            Try again
          </Button>
          {openPlanr}
        </div>
      </EmptyState>
    );
  }
  return <LoadingBoard />;
}

export function ThreadBoardPanel({ threadId }: PluginThreadPanelProps) {
  return (
    <TooltipProvider delayDuration={300}>
      <div className="@container h-full min-h-0 overflow-y-auto bg-background text-foreground">
        <ProjectState threadId={threadId} />
      </div>
    </TooltipProvider>
  );
}
