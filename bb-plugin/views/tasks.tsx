// Tasks: preset chips, search, goal/track/status filters, a list grouped by
// status (or sorted flat), and the task detail beside it or, when narrow, in
// its place.
import { useEffect, useMemo, useRef, type ReactNode } from "react";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Icon, type IconName } from "@/components/ui/icon";
import { cn } from "@/lib/utils";
import {
  dayKey,
  goalBuilt,
  goalMark,
  goalTasks,
  markOf,
  matchesFilters,
  NO_FILTERS,
  runAt,
  runLabel,
  SORT_LABEL,
  STATUS_ORDER,
  statusLabel,
  type BoardGoal,
  type BoardRun,
  type BoardTask,
  type BoardView,
  type MainFilter,
  type TaskFilters,
  type TaskSort,
} from "@/lib/board";
import type { BoardActions } from "@/hooks/use-actions";
import { isBoardKey, useElementWidth } from "@/hooks/use-width";
import { StatusMark, Tag } from "./marks";
import { EmptyState, FOCUS_RING, GoalActionButton, Row, idWidth } from "./parts";
import { TaskDetail } from "./task-detail";

const SPLIT_MIN = 820;

function chipClass(active: boolean) {
  return cn(
    "flex h-6 shrink-0 items-center gap-1.5 rounded-md border px-2.5 text-xs max-md:pointer-coarse:h-8",
    FOCUS_RING,
    active
      ? "border-border bg-secondary font-medium text-foreground"
      : "border-transparent text-muted-foreground hover:bg-state-hover hover:text-foreground",
  );
}

function menuChipClass(active: boolean) {
  return cn(
    "flex h-6 shrink-0 items-center gap-1.5 rounded-md border px-2.5 text-xs max-md:pointer-coarse:h-8",
    FOCUS_RING,
    active
      ? "border-border bg-secondary text-foreground"
      : "border-dashed border-border text-muted-foreground hover:border-input hover:text-foreground",
  );
}

function MenuChip({
  icon,
  label,
  value,
  children,
  align = "start",
}: {
  icon: IconName;
  label: string;
  value: string | null;
  children: ReactNode;
  align?: "start" | "end";
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button type="button" className={menuChipClass(value !== null)}>
          <Icon name={icon} className="size-3" />
          {label}
          {value !== null ? (
            <span className="max-w-40 truncate font-medium @max-md:max-w-24">{value}</span>
          ) : null}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align={align} className="max-h-80 min-w-48 overflow-y-auto" mobileTitle={label}>
        {children}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function FilterBar({
  view,
  filters,
  setFilters,
  shown,
  searchRef,
  onEnter,
}: {
  view: BoardView;
  filters: TaskFilters;
  setFilters: (next: TaskFilters) => void;
  shown: number;
  searchRef: React.RefObject<HTMLInputElement | null>;
  onEnter: () => void;
}) {
  const { data, baseRun } = view;
  const main = filters.main;
  const presets: Array<[MainFilter, string, number]> = [
    ["all", "All", data.tasks.length],
    ["needs", "Needs you", view.needsYou.length],
    ["check", "To check", view.toCheck.length],
    ["ready", "Up next", view.upNext.length],
    ["changed", baseRun ? `Moved since ${dayKey(baseRun.date)}` : "Moved", view.changedSinceBase.size],
  ];
  if (main.startsWith("run:")) {
    const index = Number(main.slice(4));
    const run = data.runs[index];
    if (run) {
      presets.push([
        main,
        `Run ${runLabel(view, run)}`,
        data.tasks.filter((t) => t.history.some((h) => h.run === index)).length,
      ]);
    }
  }
  const status = main.startsWith("status:") ? main.slice(7) : null;
  const goalChoices = view.queued.filter((g) => goalTasks(view, g.name).length > 0);
  const active = main !== "all" || filters.goal !== null || filters.track !== null || filters.query !== "";
  const toggleMain = (key: MainFilter) =>
    setFilters({ ...filters, main: filters.main === key ? "all" : key });
  return (
    <div className="shrink-0 border-b border-border-hairline">
      <div
        role="group"
        aria-label="Presets"
        className={cn(
          "flex items-center gap-1 overflow-x-auto px-3.5 pt-2 [scrollbar-width:none]",
          "[&::-webkit-scrollbar]:hidden",
        )}
      >
        {presets.map(([key, label, n]) => (
          <button
            key={key}
            type="button"
            aria-pressed={main === key}
            onClick={() => toggleMain(key)}
            className={chipClass(main === key)}
          >
            {label}
            <span className="tabular-nums text-subtle-foreground">{n}</span>
          </button>
        ))}
      </div>
      <div className="flex items-center gap-1.5 px-3.5 py-2">
        <div
          className={cn(
            "flex min-w-0 flex-1 items-center gap-1.5 overflow-x-auto [scrollbar-width:none]",
            "[&::-webkit-scrollbar]:hidden",
          )}
        >
          <label className="relative flex h-6 w-36 shrink-0 items-center @xl:w-48">
            <Icon
              name="Search"
              className="pointer-events-none absolute left-2 size-3 text-subtle-foreground"
            />
            <input
              ref={searchRef}
              type="search"
              value={filters.query}
              onChange={(e) => setFilters({ ...filters, query: e.target.value })}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  onEnter();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  if (filters.query) setFilters({ ...filters, query: "" });
                  else e.currentTarget.blur();
                }
              }}
              placeholder="Filter tasks"
              aria-label="Filter tasks by ID or title"
              aria-keyshortcuts="/"
              className={cn(
                "h-full w-full rounded-md border border-border bg-background pl-6 pr-2 text-xs",
                "text-foreground placeholder:text-subtle-foreground",
                "focus-visible:border-input focus-visible:outline-none focus-visible:ring-1",
                "focus-visible:ring-ring",
              )}
            />
          </label>
          {goalChoices.length > 0 || filters.goal !== null ? (
            <MenuChip icon="Workflow" label="Goal" value={filters.goal}>
              {goalChoices.map((g) => (
                <DropdownMenuCheckboxItem
                  key={g.name}
                  checked={filters.goal === g.name}
                  onCheckedChange={(checked) =>
                    setFilters({ ...filters, goal: checked === true ? g.name : null })
                  }
                >
                  <span className="flex min-w-0 items-center gap-2">
                    <StatusMark kind={goalMark(g)} className="size-3" />
                    <span className="font-mono text-xs">{g.name}</span>
                    <span className="ml-auto pl-3 text-xs tabular-nums text-subtle-foreground">
                      {goalTasks(view, g.name).length}
                    </span>
                  </span>
                </DropdownMenuCheckboxItem>
              ))}
            </MenuChip>
          ) : null}
          <MenuChip icon="FilterHorizontal" label="Track" value={filters.track}>
            {view.tracks.map((track) => (
              <DropdownMenuCheckboxItem
                key={track}
                checked={filters.track === track}
                onCheckedChange={(checked) =>
                  setFilters({ ...filters, track: checked === true ? track : null })
                }
              >
                <span className="flex min-w-0 flex-1 items-center gap-2">
                  {track}
                  <span className="ml-auto pl-3 text-xs tabular-nums text-subtle-foreground">
                    {data.tasks.filter((t) => t.track === track).length}
                  </span>
                </span>
              </DropdownMenuCheckboxItem>
            ))}
          </MenuChip>
          <MenuChip icon="Circle" label="Status" value={status === null ? null : statusLabel(status)}>
            {view.statuses.map((s) => (
              <DropdownMenuCheckboxItem
                key={s}
                checked={status === s}
                onCheckedChange={(checked) =>
                  setFilters({ ...filters, main: checked === true ? `status:${s}` : "all" })
                }
              >
                <span className="flex min-w-0 flex-1 items-center gap-2">
                  <StatusMark kind={markOf(s)} className="size-3" />
                  {statusLabel(s)}
                  <span className="ml-auto pl-3 text-xs tabular-nums text-subtle-foreground">
                    {data.tasks.filter((t) => t.status === s).length}
                  </span>
                </span>
              </DropdownMenuCheckboxItem>
            ))}
          </MenuChip>
          {active ? (
            <button
              type="button"
              onClick={() => setFilters({ ...NO_FILTERS, sort: filters.sort })}
              className={cn(
                "flex h-6 shrink-0 items-center gap-1 rounded-md border border-dashed border-border px-2.5",
                "text-xs text-muted-foreground hover:border-input hover:text-foreground",
                FOCUS_RING,
              )}
            >
              <Icon name="X" className="size-3" />
              Clear
            </button>
          ) : null}
        </div>
        <MenuChip
          icon="Sort"
          label="Sort"
          value={filters.sort === "status" ? null : SORT_LABEL[filters.sort]}
          align="end"
        >
          {(Object.keys(SORT_LABEL) as TaskSort[]).map((key) => (
            <DropdownMenuCheckboxItem
              key={key}
              checked={filters.sort === key}
              onCheckedChange={(checked) => {
                if (checked === true) setFilters({ ...filters, sort: key });
              }}
            >
              {SORT_LABEL[key]}
            </DropdownMenuCheckboxItem>
          ))}
        </MenuChip>
        <span
          className="hidden shrink-0 whitespace-nowrap text-xs tabular-nums text-subtle-foreground @md:inline"
          aria-live="polite"
        >
          {shown} {shown === 1 ? "task" : "tasks"}
        </span>
      </div>
    </div>
  );
}

function GoalBanner({
  goal,
  actions,
  onCloseGoal,
  onClear,
}: {
  goal: BoardGoal;
  actions: BoardActions;
  onCloseGoal: (goal: BoardGoal) => void;
  onClear: () => void;
}) {
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-border-hairline bg-muted/40 px-3.5 py-2">
      <StatusMark kind={goalMark(goal)} />
      <span className="min-w-0 flex-1">
        <span className="flex min-w-0 items-baseline gap-2">
          <span className="shrink-0 font-mono text-xs">{goal.name}</span>
          <span className="min-w-0 truncate text-sm font-medium">{goal.title ?? ""}</span>
        </span>
        <span className="block truncate text-xs text-muted-foreground">
          {[goal.state, goal.stream, goal.budget, `${goalBuilt(goal)}/${goal.total} built`]
            .filter(Boolean)
            .join(" · ")}
          {goal.state === "draft" ? " · waiting for your approval" : ""}
          {goal.state === "review" ? " · waiting for your sign-off" : ""}
        </span>
      </span>
      <GoalActionButton goal={goal} actions={actions} onClose={onCloseGoal} />
      <button
        type="button"
        onClick={onClear}
        aria-label={`Clear goal filter ${goal.name}`}
        className={cn(
          "flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground",
          "hover:bg-state-hover hover:text-foreground",
          FOCUS_RING,
        )}
      >
        <Icon name="X" className="size-3.5" />
      </button>
    </div>
  );
}

function ListRow({
  view,
  task,
  idCh,
  selected,
  related,
  onOpen,
  rowRef,
}: {
  view: BoardView;
  task: BoardTask;
  idCh: string;
  selected: boolean;
  related: boolean;
  onOpen: () => void;
  rowRef: (el: HTMLDivElement | null) => void;
}) {
  const run = runAt(view, task);
  const moved = view.changedSinceBase.has(task.id);
  return (
    <Row
      label={`${task.id}: ${task.title}, ${statusLabel(task.status)}`}
      onOpen={onOpen}
      selected={selected}
      related={related}
      rowRef={rowRef}
    >
      <span className="relative flex shrink-0 items-center">
        <StatusMark kind={markOf(task.status)} />
        {moved ? (
          <span
            title={`Moved since ${view.baseRun ? dayKey(view.baseRun.date) : "the start"}`}
            className={cn(
              "absolute -right-0.5 -top-0.5 size-1.5 rounded-full bg-foreground ring-2 ring-background",
            )}
          />
        ) : null}
      </span>
      <span
        className="shrink-0 truncate font-mono text-xs text-subtle-foreground"
        style={{ width: idCh }}
        title={task.id}
      >
        {task.id}
      </span>
      <span className="flex min-w-0 flex-1 items-center gap-1.5">
        <span className="min-w-0 truncate text-sm" title={task.title}>
          {task.title}
        </span>
        {task.goal.length > 0 ? (
          <span className="hidden @2xl:inline-flex">
            <Tag>goal</Tag>
          </span>
        ) : null}
        {task.ready ? (
          <span className="hidden @2xl:inline-flex">
            <Tag>ready</Tag>
          </span>
        ) : null}
      </span>
      <span className="hidden w-24 shrink-0 truncate text-xs text-subtle-foreground @3xl:block">
        {task.track}
      </span>
      <span
        className={cn(
          "hidden w-14 shrink-0 text-right text-xs tabular-nums @md:block",
          task.waitingOn.length >= 3 ? "font-medium text-foreground" : "text-subtle-foreground",
        )}
        title={task.waitingOn.length ? `Blocks ${task.waitingOn.length} open tasks` : undefined}
      >
        {task.waitingOn.length ? `blocks ${task.waitingOn.length}` : ""}
      </span>
      <span
        className="w-11 shrink-0 text-right text-xs tabular-nums text-subtle-foreground"
        title={run ? `Moved ${dayKey(run.date)}` : undefined}
      >
        {run ? dayKey(run.date) : ""}
      </span>
    </Row>
  );
}

export function TasksView({
  view,
  filters,
  setFilters,
  selectedId,
  onSelect,
  onOpenRun,
  actions,
  onCloseGoal,
}: {
  view: BoardView;
  filters: TaskFilters;
  setFilters: (next: TaskFilters) => void;
  selectedId: string | null;
  onSelect: (id: string | null, replace?: boolean) => void;
  onOpenRun: (run: BoardRun) => void;
  actions: BoardActions;
  onCloseGoal: (goal: BoardGoal) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const width = useElementWidth(ref);
  const split = width === 0 || width >= SPLIT_MIN;
  const searchRef = useRef<HTMLInputElement>(null);
  const rows = useRef(new Map<string, HTMLDivElement>());

  const list = useMemo(
    () => view.data.tasks.filter((t) => matchesFilters(view, filters, t)).sort(view.sorts[filters.sort]),
    [view, filters],
  );
  const groups = useMemo(() => {
    if (filters.sort !== "status") return [{ status: null as string | null, tasks: list }];
    const known = STATUS_ORDER.map((s) => ({
      status: s as string | null,
      tasks: list.filter((t) => t.status === s),
    }));
    const other = list.filter((t) => !(STATUS_ORDER as string[]).includes(t.status));
    return [...known, { status: "other", tasks: other }].filter((g) => g.tasks.length > 0);
  }, [list, filters.sort]);
  const ordered = useMemo(() => groups.flatMap((g) => g.tasks), [groups]);
  const idCh = idWidth(list.map((t) => t.id));
  const selected = selectedId === null ? null : (view.byId.get(selectedId) ?? null);
  const related = new Set(selected ? [...selected.depends, ...selected.waitingOn] : []);
  const goal = filters.goal === null ? null : (view.goalByName.get(filters.goal) ?? null);

  useEffect(() => {
    if (selectedId !== null) rows.current.get(selectedId)?.scrollIntoView({ block: "nearest" });
  }, [selectedId, split]);

  const stateRef = useRef({ ordered, selectedId, filters, onSelect, setFilters });
  stateRef.current = { ordered, selectedId, filters, onSelect, setFilters };
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!isBoardKey(event)) return;
      const s = stateRef.current;
      if (event.key === "/") {
        event.preventDefault();
        searchRef.current?.focus();
        return;
      }
      if (event.key === "Escape") {
        if (s.selectedId !== null) s.onSelect(null);
        else if (
          s.filters.main !== "all" ||
          s.filters.goal !== null ||
          s.filters.track !== null ||
          s.filters.query
        ) {
          s.setFilters({ ...NO_FILTERS, sort: s.filters.sort });
        }
        return;
      }
      const dir = ({ j: 1, ArrowDown: 1, k: -1, ArrowUp: -1 } as Record<string, number>)[event.key];
      if (dir === undefined || s.ordered.length === 0) return;
      event.preventDefault();
      const i = s.ordered.findIndex((t) => t.id === s.selectedId);
      const next = i < 0 ? 0 : Math.max(0, Math.min(s.ordered.length - 1, i + dir));
      s.onSelect(s.ordered[next].id, true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const toggle = (id: string) => onSelect(selectedId === id ? null : id);
  const detail =
    selectedId !== null ? (
      <TaskDetail
        view={view}
        task={selected}
        missingId={selectedId}
        actions={actions}
        closeLabel={split ? "close" : "back"}
        onClose={() => onSelect(null)}
        onOpenTask={(id) => {
          const task = view.byId.get(id);
          if (task && !matchesFilters(view, filters, task)) setFilters({ ...NO_FILTERS, sort: filters.sort });
          onSelect(id);
        }}
        onOpenRun={onOpenRun}
        onFilterGoal={(name) => {
          setFilters({ ...NO_FILTERS, sort: filters.sort, goal: name });
        }}
      />
    ) : null;

  const listBody =
    list.length === 0 ? (
      <EmptyState
        icon="Search"
        title="No task matches"
        description="Change or clear the filters to see more tasks."
      >
        <button
          type="button"
          onClick={() => setFilters({ ...NO_FILTERS, sort: filters.sort })}
          className={cn(
            "rounded-md border border-border px-2.5 py-1 text-xs hover:bg-state-hover",
            FOCUS_RING,
          )}
        >
          Clear filters
        </button>
      </EmptyState>
    ) : (
      groups.map((g) => (
        <section key={g.status ?? "all"} aria-label={g.status === null ? "Tasks" : statusLabel(g.status)}>
          {g.status !== null ? (
            <div
              className={cn(
                "sticky top-0 z-20 flex items-center gap-2 border-b border-border-hairline bg-background",
                "px-3.5 pb-1.5 pt-2.5 text-sm font-semibold",
              )}
            >
              <StatusMark kind={markOf(g.status)} />
              {g.status === "other" ? "Other" : statusLabel(g.status)}
              <span className="text-xs font-normal tabular-nums text-subtle-foreground">
                {g.tasks.length}
              </span>
            </div>
          ) : null}
          {g.tasks.map((t) => (
            <ListRow
              key={t.id}
              view={view}
              task={t}
              idCh={idCh}
              selected={t.id === selectedId}
              related={related.has(t.id)}
              onOpen={() => toggle(t.id)}
              rowRef={(el) => {
                if (el) rows.current.set(t.id, el);
                else rows.current.delete(t.id);
              }}
            />
          ))}
        </section>
      ))
    );

  return (
    <div ref={ref} className="flex h-full min-h-0 flex-col">
      {split || selectedId === null ? (
        <>
          <FilterBar
            view={view}
            filters={filters}
            setFilters={setFilters}
            shown={list.length}
            searchRef={searchRef}
            onEnter={() => {
              if (ordered.length > 0) {
                onSelect(ordered[0].id);
                searchRef.current?.blur();
              }
            }}
          />
          {goal !== null ? (
            <GoalBanner
              goal={goal}
              actions={actions}
              onCloseGoal={onCloseGoal}
              onClear={() => setFilters({ ...filters, goal: null })}
            />
          ) : null}
        </>
      ) : null}
      <div className="flex min-h-0 flex-1">
        {split || selectedId === null ? (
          <div className="@container min-h-0 min-w-0 flex-1 overflow-y-auto">{listBody}</div>
        ) : null}
        {detail !== null ? (
          <aside
            aria-label="Task detail"
            className={cn(
              "@container min-h-0 overflow-y-auto",
              split ? "w-[clamp(320px,40%,460px)] shrink-0 border-l border-border-hairline" : "flex-1",
            )}
          >
            {detail}
          </aside>
        ) : null}
      </div>
    </div>
  );
}
