// The Planr panel: resolves the project (host hint, route, last choice or
// picker), loads the board and routes between Overview, Runs and Tasks.
import { cn } from "@/lib/utils";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useBbNavigate, type PluginNavPanelProps } from "@get-bb/plugin-sdk/app";
import { Button } from "@/components/ui/button";
import { TooltipProvider } from "@/components/ui/tooltip";
import {
  boardView,
  matchesFilters,
  NO_FILTERS,
  parseBoard,
  runKey,
  type BoardGoal,
  type BoardRun,
  type MainFilter,
  type TaskFilters,
} from "@/lib/board";
import { buildSubPath, PANEL_PATH, parseRoute, type BoardViewName, type PanelRoute } from "@/lib/route";
import { useActionAccess, useBoardActions } from "@/hooks/use-actions";
import { useBoard, useLastProject, useProjects, useSession, type PlanrProject } from "@/hooks/use-planr";
import type { Session } from "../contract";
import { isBoardKey } from "@/hooks/use-width";
import { CloseGoalDialog } from "./close-goal";
import { Overview, type OverviewNav } from "./overview";
import { EmptyState, LoadingBoard, RefreshFailedNotice } from "./parts";
import { RunsView } from "./runs";
import { MappingSetup, ProjectChooser, useHosts } from "./setup";
import { TasksView } from "./tasks";
import { ProjectPicker, RefreshButton, RepoStatus, SessionStatus, useRepoRefresh, ViewNav } from "./topbar";

type Go = (next: Partial<PanelRoute>, replace?: boolean) => void;

/** Optional host extension: a project id handed to plugin pages. Detected at runtime. */
function hostProjectId(props: PluginNavPanelProps): string | null {
  const value: unknown = Reflect.get(props, "experimental_projectId");
  return typeof value === "string" && value !== "" ? value : null;
}

function Bar({ children }: { children: React.ReactNode }) {
  return (
    <header
      className={cn(
        "flex h-11 shrink-0 items-center gap-2 border-b border-border-hairline bg-background px-2.5 text-sm",
      )}
    >
      {children}
    </header>
  );
}

function Board({
  projectId,
  route,
  go,
  projects,
  showPicker,
  onChooseProject,
  refetchProjects,
  session,
  refetchSession,
}: {
  projectId: string;
  route: PanelRoute;
  go: Go;
  projects: PlanrProject[] | undefined;
  showPicker: boolean;
  onChooseProject: (projectId: string) => void;
  refetchProjects: () => void;
  /** undefined while loading or when the server does not answer; actions then stay offered. */
  session: Session | undefined;
  refetchSession: () => void;
}) {
  const board = useBoard(projectId);
  const result = board.data;
  const ok = result?.status === "ok" ? result : null;
  const view = useMemo(() => (ok === null ? null : boardView(parseBoard(ok.data))), [ok]);
  const repo = useRepoRefresh(projectId, board.refetch);
  const access = useActionAccess(session, refetchSession);
  const actions = useBoardActions(projectId, ok?.fileSha ?? null, board.refetch, repo, access);
  const hosts = useHosts();
  const [filters, setFilters] = useState<TaskFilters>(NO_FILTERS);
  const [closing, setClosing] = useState<BoardGoal | null>(null);
  const project = projects?.find((p) => p.id === projectId) ?? null;

  const viewRef = useRef(view);
  viewRef.current = view;
  const filtersRef = useRef(filters);
  filtersRef.current = filters;

  const nav: OverviewNav = useMemo(() => {
    const toTasks = (next: Partial<TaskFilters>) => {
      setFilters({ ...NO_FILTERS, sort: filtersRef.current.sort, ...next });
      go({ view: "tasks", item: null });
    };
    return {
      openTask: (id) => {
        const v = viewRef.current;
        const task = v?.byId.get(id);
        if (v && task && !matchesFilters(v, filtersRef.current, task)) {
          setFilters({ ...NO_FILTERS, sort: filtersRef.current.sort });
        }
        go({ view: "tasks", item: id });
      },
      openRun: (run: BoardRun) => go({ view: "runs", item: runKey(run) }),
      filter: (main: MainFilter) => toTasks({ main }),
      filterGoal: (goal) => toTasks({ goal }),
      filterTrack: (track) => toTasks({ track }),
      showRuns: () => go({ view: "runs", item: null }),
    };
  }, [go]);

  // 1 2 3 switch views, as on the standalone board.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (viewRef.current === null || !isBoardKey(event)) return;
      const target = ({ "1": "overview", "2": "runs", "3": "tasks" } as Record<string, BoardViewName>)[
        event.key
      ];
      if (target === undefined) return;
      event.preventDefault();
      go({ view: target, item: null });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go]);

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
      <div className="h-full overflow-y-auto">
        <MappingSetup
          projectId={projectId}
          projectName={project?.name ?? null}
          onConnected={() => {
            board.refetch();
            refetchProjects();
          }}
        />
      </div>
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
  } else if (route.view === "runs") {
    body = (
      <RunsView
        view={view}
        item={route.item}
        onSelect={(run, replace) => go({ view: "runs", item: run === null ? null : runKey(run) }, replace)}
        onBack={() => go({ view: "runs", item: null })}
        onOpenTask={nav.openTask}
        onShowInTasks={(index) => nav.filter(`run:${index}`)}
      />
    );
  } else if (route.view === "tasks") {
    body = (
      <TasksView
        view={view}
        filters={filters}
        setFilters={setFilters}
        selectedId={route.item}
        onSelect={(id, replace) => go({ view: "tasks", item: id }, replace)}
        onOpenRun={nav.openRun}
        actions={actions}
        onCloseGoal={setClosing}
      />
    );
  } else {
    body = (
      <div className="h-full overflow-y-auto">
        <Overview view={view} nav={nav} actions={actions} onCloseGoal={setClosing} />
      </div>
    );
  }

  return (
    <div className="@container flex h-full min-h-0 flex-col bg-background text-foreground">
      <Bar>
        {showPicker ? (
          <ProjectPicker projects={projects} current={projectId} onChoose={onChooseProject} />
        ) : (
          <span className="min-w-0 truncate px-1 font-semibold">{project?.name ?? ""}</span>
        )}
        {view !== null ? (
          <ViewNav
            view={route.view}
            counts={{ runs: view.data.runs.length, tasks: view.data.tasks.length }}
            onChange={(v) => go({ view: v, item: null })}
          />
        ) : null}
        <span className="ml-auto" />
        {ok !== null && view !== null ? (
          <>
            {session !== undefined ? <SessionStatus session={session} /> : null}
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
      </Bar>
      {board.error !== null && result !== undefined ? (
        <RefreshFailedNotice error={board.error} onRetry={board.refetch} />
      ) : null}
      <div className="min-h-0 flex-1">{body}</div>
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

export function PlanrPage(props: PluginNavPanelProps) {
  const route = parseRoute(props.subPath);
  const hostProject = hostProjectId(props);
  const navigate = useBbNavigate();
  const projects = useProjects();
  const session = useSession();
  const [lastProject, remember] = useLastProject();
  const projectId = hostProject ?? route.projectId;

  const routeRef = useRef(route);
  routeRef.current = route;
  const go: Go = useCallback(
    (next, replace) => {
      const subPath = buildSubPath({ ...routeRef.current, ...next });
      navigate.toPluginPanel(PANEL_PATH, { subPath, replace: replace === true });
    },
    [navigate],
  );

  useEffect(() => {
    if (hostProject !== null) {
      if (route.projectId !== hostProject) go({ projectId: hostProject, view: "overview", item: null }, true);
      return;
    }
    if (
      route.projectId === null &&
      lastProject !== null &&
      projects.data?.some((p) => p.id === lastProject)
    ) {
      go({ projectId: lastProject, view: "overview", item: null }, true);
    }
  }, [go, hostProject, lastProject, projects.data, route.projectId]);

  useEffect(() => {
    if (projectId !== null && projects.data?.some((p) => p.id === projectId)) remember(projectId);
  }, [projectId, projects.data, remember]);

  const choose = (id: string) => go({ projectId: id, view: "overview", item: null });

  return (
    <TooltipProvider delayDuration={300}>
      {projectId === null ? (
        <div
          className={cn(
            "@container flex h-full min-h-0 flex-col overflow-y-auto bg-background text-foreground",
          )}
        >
          <ProjectChooser projects={projects.data} error={projects.error} onChoose={choose} />
        </div>
      ) : (
        <Board
          key={projectId}
          projectId={projectId}
          route={route}
          go={go}
          projects={projects.data}
          showPicker={hostProject === null}
          onChooseProject={choose}
          refetchProjects={projects.refetch}
          session={session.data}
          refetchSession={session.refetch}
        />
      )}
    </TooltipProvider>
  );
}
