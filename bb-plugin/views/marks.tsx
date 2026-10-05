// Status marks, strips and change cells. Shape carries the status (readable
// without color); color comes only from host status tokens.
import type { ReactNode } from "react";
import { cn } from "@/lib/utils";
import { PROGRESS, statusLabel, type BoardTask, type MarkKind } from "@/lib/board";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

const TEXT: Record<MarkKind, string> = {
  open: "text-subtle-foreground",
  "in-progress": "text-attention",
  review: "text-timeline-accent",
  done: "text-success",
  blocked: "text-destructive",
  decision: "text-warning",
  dropped: "text-subtle-foreground",
  removed: "text-subtle-foreground",
  new: "text-muted-foreground",
};
const BG: Record<string, string> = {
  open: "bg-subtle-foreground/60",
  "in-progress": "bg-attention",
  review: "bg-timeline-accent",
  done: "bg-success",
  blocked: "bg-destructive",
  decision: "bg-warning",
  dropped: "bg-subtle-foreground/40",
  removed: "bg-subtle-foreground/40",
  new: "bg-muted-foreground",
};
export const statusBg = (s: string) => (Object.hasOwn(BG, s) ? BG[s] : BG.open);

/** 14×14 status mark; the cut-out glyphs use the page background. */
export function StatusMark({ kind, className }: { kind: MarkKind; className?: string }) {
  const ring = (dash?: string) => (
    <circle
      cx="7"
      cy="7"
      r="5.4"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeDasharray={dash}
    />
  );
  const cut = "var(--background)";
  return (
    <svg viewBox="0 0 14 14" aria-hidden className={cn("size-3.5 shrink-0", TEXT[kind], className)}>
      {kind === "open" ? ring() : null}
      {kind === "in-progress" ? (
        <>
          {ring()}
          <path d="M7 7 L7 3.2 A3.8 3.8 0 0 1 7 10.8 Z" fill="currentColor" />
        </>
      ) : null}
      {kind === "review" ? (
        <>
          {ring()}
          <circle cx="7" cy="7" r="2.4" fill="currentColor" />
        </>
      ) : null}
      {kind === "done" ? (
        <>
          <circle cx="7" cy="7" r="6" fill="currentColor" />
          <path
            d="M4.4 7.2 l1.8 1.8 3.4-3.8"
            fill="none"
            stroke={cut}
            strokeWidth="1.5"
            strokeLinecap="round"
          />
        </>
      ) : null}
      {kind === "blocked" ? (
        <>
          <circle cx="7" cy="7" r="6" fill="currentColor" />
          <rect x="3.9" y="6.2" width="6.2" height="1.6" rx="0.8" fill={cut} />
        </>
      ) : null}
      {kind === "decision" ? (
        <>
          <rect
            x="2.5"
            y="2.5"
            width="9"
            height="9"
            rx="1.8"
            transform="rotate(45 7 7)"
            fill="currentColor"
          />
          <rect x="6.25" y="3.9" width="1.5" height="3.7" rx="0.75" fill={cut} />
          <circle cx="7" cy="9.5" r="0.9" fill={cut} />
        </>
      ) : null}
      {kind === "dropped" ? (
        <>
          {ring()}
          <path d="M3.8 10.2 L10.2 3.8" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
        </>
      ) : null}
      {kind === "removed" ? ring("1.8 2") : null}
      {kind === "new" ? (
        <path d="M7 3 v8 M3 7 h8" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      ) : null}
    </svg>
  );
}

/** Horizontal strip of status segments, width proportional to count. */
export function StatusStrip({
  items,
  label = "",
  className,
}: {
  items: Array<[string, number]>;
  label?: string;
  className?: string;
}) {
  const shown = items.filter(([, n]) => n > 0);
  const total = shown.reduce((sum, [, n]) => sum + n, 0) || 1;
  const summary = shown.map(([s, n]) => `${statusLabel(s)} ${n}`).join(", ");
  return (
    <span
      role="img"
      aria-label={`${label}${summary || "no tasks"}`}
      className={cn("flex h-1.5 min-w-0 gap-0.5 overflow-hidden rounded-full bg-muted", className)}
    >
      {shown.map(([s, n]) => (
        <span
          key={s}
          title={`${label}${statusLabel(s)} · ${n} (${Math.round((n / total) * 100)}%)`}
          className={cn("h-full min-w-0.5", statusBg(s))}
          style={{ flexGrow: n, flexBasis: 0 }}
        />
      ))}
    </span>
  );
}

export const progressItems = (tasks: BoardTask[]): Array<[string, number]> =>
  PROGRESS.map((s) => [s, tasks.filter((t) => t.status === s).length]);

/** One cell per changed task, colored by target status; new tasks are neutral. */
export function ChangeCells({
  changes,
  max = 16,
}: {
  changes: Array<{ from: string | null; to: string }>;
  max?: number;
}) {
  return (
    <span className="flex shrink-0 items-center gap-0.5" aria-hidden>
      {changes.slice(0, max).map((c, i) => (
        <span key={i} className={cn("h-2.5 w-1 rounded-[1px]", c.from === null ? BG.new : statusBg(c.to))} />
      ))}
      {changes.length > max ? <span className="ml-0.5">+{changes.length - max}</span> : null}
    </span>
  );
}

/** Small neutral tag after a title ("ready", "goal"). */
export function Tag({ children }: { children: ReactNode }) {
  return (
    <span
      className={cn(
        "shrink-0 rounded-md border border-border px-1.5 text-[11px] leading-4 text-muted-foreground",
      )}
    >
      {children}
    </span>
  );
}

/** Tooltip wrapper for an element that already is focusable. */
export function Tip({
  content,
  children,
  side = "top",
}: {
  content: ReactNode;
  children: ReactNode;
  side?: "top" | "bottom" | "left" | "right";
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side={side} className="max-w-72">
        {content}
      </TooltipContent>
    </Tooltip>
  );
}
