// Building blocks shared by the board views, styled like bb's own Tasks plugin:
// hairline rows, sticky group headers, token colors only.
import { useId, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Icon, type IconName } from "@/components/ui/icon";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import {
  GOAL_ASK,
  goalBuilt,
  goalMark,
  markOf,
  statusLabel,
  type BoardGoal,
  type BoardTask,
} from "@/lib/board";
import type { BoardActions } from "@/hooks/use-actions";
import { StatusMark, Tag, Tip } from "./marks";

export const FOCUS_RING = "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring";
export const ROW_FOCUS =
  "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring";

/** Width of an ID column: the longest ID it shows, at most 14ch and 30% of the row. */
export const idWidth = (ids: string[]) =>
  `min(${Math.min(14, Math.max(6, ...ids.map((id) => id.length)))}ch, 30%)`;

export function Section({
  title,
  meta,
  action,
  children,
  className,
  labelledBy,
}: {
  title: string;
  meta?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
  labelledBy?: string;
}) {
  return (
    <section
      aria-labelledby={labelledBy}
      className={cn("min-w-0 overflow-hidden rounded-lg border border-border bg-card", className)}
    >
      <header className="flex h-10 items-center gap-2 border-b border-border-hairline px-3.5">
        <h2 id={labelledBy} className="shrink-0 text-sm font-semibold">
          {title}
        </h2>
        {meta !== undefined ? (
          <span className="min-w-0 truncate text-xs tabular-nums text-subtle-foreground">{meta}</span>
        ) : null}
        {action !== undefined ? <span className="ml-auto flex shrink-0 items-center">{action}</span> : null}
      </header>
      {children}
    </section>
  );
}

/** A quiet "All" style link button in section and group headers. */
export function MoreButton({
  label,
  onClick,
  ariaLabel,
}: {
  label: string;
  onClick: () => void;
  ariaLabel?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={ariaLabel}
      className={cn(
        "flex h-6 items-center gap-1 rounded-md px-1.5 text-xs text-muted-foreground hover:bg-state-hover",
        "hover:text-foreground",
        FOCUS_RING,
      )}
    >
      {label}
      <Icon name="ArrowRight" className="size-3" />
    </button>
  );
}

export function GroupHeader({
  label,
  count,
  action,
  icon,
}: {
  label: string;
  count: number;
  action?: ReactNode;
  icon?: ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex h-8 items-center gap-2 border-b border-border-hairline bg-muted/40 px-3.5 text-xs font-medium",
        "text-muted-foreground",
      )}
    >
      {icon}
      <span>{label}</span>
      <span className="tabular-nums text-subtle-foreground">{count}</span>
      {action !== undefined ? <span className="ml-auto">{action}</span> : null}
    </div>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <p className="px-3.5 py-4 text-sm text-muted-foreground">{children}</p>;
}

/** The centered state bb's own pages use for empty and setup screens. */
export function EmptyState({
  icon,
  title,
  description,
  children,
}: {
  icon: IconName;
  title: string;
  description?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="flex min-h-full flex-col items-center justify-center gap-3 p-6 text-center">
      <div className="flex size-10 items-center justify-center rounded-md bg-secondary text-muted-foreground">
        <Icon name={icon} className="size-5" />
      </div>
      <div className="max-w-md space-y-1">
        <p className="text-sm font-medium">{title}</p>
        {description !== undefined ? <p className="text-sm text-muted-foreground">{description}</p> : null}
      </div>
      {children}
    </div>
  );
}

export function LoadingBoard() {
  return (
    <div className="space-y-3 px-3.5 py-3" aria-busy="true" aria-label="Loading board">
      <Skeleton className="h-24 w-full" />
      <Skeleton className="h-36 w-full" />
      <div className="space-y-2">
        {Array.from({ length: 6 }, (_, i) => (
          <div key={i} className="flex h-[34px] items-center gap-2">
            <Skeleton className="size-3.5 rounded-full" />
            <Skeleton className="h-3 w-14" />
            <Skeleton className="h-3 w-3/5" />
          </div>
        ))}
      </div>
    </div>
  );
}

/** Shown above a board whose last refetch failed; the last good board stays visible. */
export function RefreshFailedNotice({ error, onRetry }: { error: string; onRetry: () => void }) {
  return (
    <div
      role="status"
      className={cn(
        "flex shrink-0 items-center gap-2 border-b border-border-hairline bg-muted/40 px-3.5 py-1.5",
        "text-xs",
      )}
    >
      <Icon name="AlertTriangle" className="size-3.5 shrink-0 text-warning" />
      <span className="min-w-0 flex-1 truncate text-muted-foreground" title={error}>
        Refresh failed, showing the last loaded board: {error}
      </span>
      <Button size="sm" variant="ghost" className="h-6 px-2 text-xs" onClick={onRetry}>
        Retry
      </Button>
    </div>
  );
}

/** A quiet full-width row under a list: "N more in Tasks", "Show N more". */
export function MoreRow({
  label,
  icon = "ArrowRight",
  onClick,
}: {
  label: string;
  icon?: IconName;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex h-8 w-full items-center gap-1 px-3.5 text-xs text-muted-foreground",
        "hover:bg-state-hover hover:text-foreground",
        FOCUS_RING,
      )}
    >
      {label}
      <Icon name={icon} className="size-3" />
    </button>
  );
}

/** A row that opens something, with optional trailing controls that stay clickable. */
export function Row({
  label,
  onOpen,
  selected,
  related,
  children,
  trailing,
  className,
  rowRef,
}: {
  label: string;
  onOpen: () => void;
  selected?: boolean;
  related?: boolean;
  children: ReactNode;
  trailing?: ReactNode;
  className?: string;
  rowRef?: (el: HTMLDivElement | null) => void;
}) {
  return (
    <div
      ref={rowRef}
      className={cn(
        "relative flex h-[34px] min-w-0 items-center gap-2 border-b border-border-hairline px-3.5 text-left",
        "last:border-b-0 hover:bg-state-hover",
        related && "bg-muted/60",
        selected && "bg-state-active hover:bg-state-active",
        className,
      )}
    >
      <button
        type="button"
        aria-label={label}
        aria-current={selected ? "true" : undefined}
        onClick={onOpen}
        className={cn("absolute inset-0 rounded-none", ROW_FOCUS)}
      />
      {children}
      {trailing !== undefined ? (
        <span className="relative z-10 flex shrink-0 items-center">{trailing}</span>
      ) : null}
    </div>
  );
}

export function TaskRow({
  task,
  meta,
  idCh,
  onOpen,
  selected,
  related,
  trailing,
  moved,
  showTags = false,
  rowRef,
}: {
  task: BoardTask;
  meta?: ReactNode;
  idCh: string;
  onOpen: () => void;
  selected?: boolean;
  related?: boolean;
  trailing?: ReactNode;
  moved?: boolean;
  showTags?: boolean;
  rowRef?: (el: HTMLDivElement | null) => void;
}) {
  return (
    <Row
      label={`${task.id}: ${task.title}, ${statusLabel(task.status)}`}
      onOpen={onOpen}
      selected={selected}
      related={related}
      trailing={trailing}
      rowRef={rowRef}
    >
      <span className="relative flex shrink-0 items-center">
        <StatusMark kind={markOf(task.status)} />
        {moved ? (
          <span
            title="Moved since the last day's baseline"
            className="absolute -right-0.5 -top-0.5 size-1.5 rounded-full bg-foreground ring-2 ring-card"
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
        {showTags && task.goal.length > 0 ? (
          <span className="hidden @xl:inline-flex">
            <Tag>goal</Tag>
          </span>
        ) : null}
        {showTags && task.ready ? (
          <span className="hidden @xl:inline-flex">
            <Tag>ready</Tag>
          </span>
        ) : null}
      </span>
      {meta !== undefined && meta !== "" ? (
        <span
          className={cn(
            "hidden min-w-0 max-w-[45%] shrink truncate text-right text-xs tabular-nums",
            "text-subtle-foreground @sm:block",
          )}
        >
          {meta}
        </span>
      ) : null}
    </Row>
  );
}

export function ActionButton({
  icon,
  label,
  pendingLabel,
  pending,
  running,
  onClick,
  blocked = null,
  compact = false,
  variant = "outline",
}: {
  icon: IconName;
  label: string;
  pendingLabel: string;
  pending: boolean;
  running: boolean;
  onClick: () => void;
  /** Why the caller may not take this action; the button stays focusable and explains it. */
  blocked?: string | null;
  compact?: boolean;
  variant?: "outline" | "default";
}) {
  const reasonId = useId();
  const button = (
    <Button
      type="button"
      variant={variant}
      size="sm"
      disabled={pending}
      aria-disabled={blocked !== null || undefined}
      aria-describedby={blocked !== null ? reasonId : undefined}
      aria-busy={running || undefined}
      aria-label={compact ? (running ? pendingLabel : label) : undefined}
      onClick={blocked !== null ? undefined : onClick}
      className={cn(
        "h-6 gap-1 px-2 text-xs",
        variant === "outline" && "bg-background",
        blocked !== null && "cursor-not-allowed opacity-50 hover:bg-background",
      )}
    >
      <Icon name={running ? "Spinner" : icon} className={cn("size-3", running && "animate-spin")} />
      <span className={cn(compact && "hidden @lg:inline")}>{running ? pendingLabel : label}</span>
      {blocked !== null ? (
        <span id={reasonId} className="sr-only">
          {blocked}
        </span>
      ) : null}
    </Button>
  );
  return blocked !== null ? <Tip content={blocked}>{button}</Tip> : button;
}

/**
 * Shown instead of an action that the host already committed and pushed while the
 * board, read from the main checkout, still shows the old file. It refreshes the checkout.
 */
function AppliedButton({
  commit,
  actions,
  compact,
}: {
  commit: string;
  actions: BoardActions;
  compact?: boolean;
}) {
  const label = "Refresh to update";
  return (
    <Tip content={`Pushed as ${commit.slice(0, 7)}. The checkout the board reads is behind origin.`}>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        disabled={actions.refreshing}
        aria-busy={actions.refreshing || undefined}
        aria-label={`Pushed as ${commit.slice(0, 7)}. ${label}`}
        onClick={actions.refreshRepo}
        className="h-6 gap-1 px-2 text-xs text-muted-foreground"
      >
        <Icon
          name={actions.refreshing ? "Spinner" : "RotateCcw"}
          className={cn("size-3", actions.refreshing && "animate-spin")}
        />
        <span className={cn(compact && "hidden @lg:inline")}>{label}</span>
      </Button>
    </Tip>
  );
}

/** Approve (draft) or Close (review) for a goal; Close asks first. */
export function GoalActionButton({
  goal,
  actions,
  onClose,
  compact,
}: {
  goal: BoardGoal;
  actions: BoardActions;
  onClose: (goal: BoardGoal) => void;
  compact?: boolean;
}) {
  const commit = actions.appliedCommit(goal.file);
  if (commit !== null && (goal.state === "draft" || goal.state === "review")) {
    return <AppliedButton commit={commit} actions={actions} compact={compact} />;
  }
  if (goal.state === "draft") {
    return (
      <ActionButton
        icon="Check"
        label="Approve"
        pendingLabel="Approving…"
        pending={actions.isPending(goal.file)}
        running={actions.isRunning(goal.file)}
        compact={compact}
        blocked={actions.blocked}
        onClick={() =>
          void actions.apply({ action: { kind: "approve-goal", goal: goal.name }, file: goal.file })
        }
      />
    );
  }
  if (goal.state === "review") {
    return (
      <ActionButton
        icon="CircleCheck"
        label="Close"
        pendingLabel="Closing…"
        pending={actions.isPending(goal.file)}
        running={actions.isRunning(goal.file)}
        compact={compact}
        blocked={actions.blocked}
        onClick={() => onClose(goal)}
      />
    );
  }
  return null;
}

export function CheckTaskButton({
  task,
  actions,
  compact,
  variant,
}: {
  task: BoardTask;
  actions: BoardActions;
  compact?: boolean;
  variant?: "outline" | "default";
}) {
  if (task.status !== "review") return null;
  const commit = actions.appliedCommit(task.file);
  if (commit !== null) return <AppliedButton commit={commit} actions={actions} compact={compact} />;
  return (
    <ActionButton
      icon="Check"
      label="Mark checked"
      pendingLabel="Saving…"
      pending={actions.isPending(task.file)}
      running={actions.isRunning(task.file)}
      compact={compact}
      variant={variant}
      blocked={actions.blocked}
      onClick={() => void actions.apply({ action: { kind: "check-task", id: task.id }, file: task.file })}
    />
  );
}

export function GoalRow({
  goal,
  idCh,
  onOpen,
  trailing,
  openLabel = "Show its tasks",
}: {
  goal: BoardGoal;
  idCh: string;
  onOpen: () => void;
  trailing?: ReactNode;
  /** What opening the row does, for its accessible name. */
  openLabel?: string;
}) {
  const ask = Object.hasOwn(GOAL_ASK, goal.state) ? GOAL_ASK[goal.state] : goal.state;
  return (
    <Row
      label={`Goal ${goal.name}: ${goal.title ?? goal.name}, ${goal.state}, to ${ask}. ${openLabel}`}
      onOpen={onOpen}
      trailing={trailing}
    >
      <StatusMark kind={goalMark(goal)} />
      <span
        className="shrink-0 truncate font-mono text-xs text-subtle-foreground"
        style={{ width: idCh }}
        title={goal.name}
      >
        {goal.name}
      </span>
      <span className="flex min-w-0 flex-1 items-center gap-1.5">
        <span className="min-w-0 truncate text-sm" title={goal.title ?? goal.name}>
          {goal.title ?? goal.name}
        </span>
      </span>
      <span className="hidden shrink-0 text-xs tabular-nums text-subtle-foreground @sm:block">
        {goalBuilt(goal)}/{goal.total} built
      </span>
    </Row>
  );
}
