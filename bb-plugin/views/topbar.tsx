// The board's own bar under bb's title bar: project, view switch, repo state.
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Icon } from "@/components/ui/icon";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn, formatHomePathForDisplay } from "@/lib/utils";
import type { IconName } from "@/components/ui/icon";
import { dayKey, hm } from "@/lib/board";
import type { BoardViewName } from "@/lib/route";
import type { RepoState, Session } from "../contract";
import { errorText, usePlanrRpc, type PlanrProject } from "@/hooks/use-planr";
import { FOCUS_RING } from "./parts";
import { Tip } from "./marks";

export function ProjectPicker({
  projects,
  current,
  onChoose,
}: {
  projects: PlanrProject[] | undefined;
  current: string;
  onChoose: (projectId: string) => void;
}) {
  const name =
    projects?.find((p) => p.id === current)?.name ?? (projects === undefined ? "" : "Unknown project");
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          aria-label={`Project: ${name || "loading"}. Switch project`}
          className="h-7 min-w-0 max-w-[45cqw] shrink gap-1 px-2 text-sm font-semibold"
        >
          <span className="truncate">{name || "Project"}</span>
          <Icon name="ChevronDown" className="size-3.5 shrink-0 text-muted-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-56" mobileTitle="Switch project">
        <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">Projects</DropdownMenuLabel>
        {(projects ?? []).map((p) => (
          <DropdownMenuItem key={p.id} onSelect={() => onChoose(p.id)} className="gap-2">
            <Icon name="Check" className={cn("size-3.5", p.id === current ? "opacity-100" : "opacity-0")} />
            <span className="min-w-0 flex-1 truncate">{p.name}</span>
            {p.mapped ? null : <span className="text-xs text-subtle-foreground">Not set up</span>}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

const VIEW_LABEL: Record<BoardViewName, string> = { overview: "Overview", runs: "Runs", tasks: "Tasks" };

export function ViewNav({
  view,
  counts,
  onChange,
}: {
  view: BoardViewName;
  counts: Partial<Record<BoardViewName, number>>;
  onChange: (view: BoardViewName) => void;
}) {
  return (
    <nav aria-label="Board views" className="flex shrink-0 items-center rounded-md bg-muted p-0.5">
      {(["overview", "runs", "tasks"] as const).map((v, i) => (
        <button
          key={v}
          type="button"
          onClick={() => onChange(v)}
          aria-current={view === v ? "page" : undefined}
          aria-keyshortcuts={String(i + 1)}
          className={cn(
            "flex items-center gap-1.5 rounded-sm px-2.5 py-0.5 text-xs max-md:pointer-coarse:py-1.5",
            FOCUS_RING,
            view === v
              ? "bg-background text-foreground shadow-2xs"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {VIEW_LABEL[v]}
          {counts[v] !== undefined ? (
            <span className="hidden tabular-nums text-subtle-foreground @2xl:inline">{counts[v]}</span>
          ) : null}
        </button>
      ))}
    </nav>
  );
}

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex min-w-0 items-baseline gap-3 text-xs">
      <dt className="w-20 shrink-0 text-muted-foreground">{label}</dt>
      <dd className="min-w-0 flex-1 break-words">{children}</dd>
    </div>
  );
}

/** Branch, sync and working-tree state of the checkout the board reads from. */
export function RepoStatus({
  repo,
  lastCommit,
  hostName,
  loadedAt,
  refreshing,
  onRefresh,
}: {
  repo: RepoState;
  lastCommit: string;
  hostName: string | null;
  loadedAt: number | null;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  const [hash, date] = lastCommit.split(" · ");
  const head = repo.head ?? hash ?? "";
  const sync = !repo.hasRemote
    ? "No remote"
    : repo.behind === 0 && repo.ahead === 0
      ? "Up to date with origin"
      : [repo.behind > 0 && `${repo.behind} behind`, repo.ahead > 0 && `${repo.ahead} ahead`]
          .filter(Boolean)
          .join(" · ");
  const summary = [
    repo.branch ?? "detached",
    repo.behind > 0 ? `${repo.behind} behind origin` : null,
    repo.ahead > 0 ? `${repo.ahead} ahead of origin` : null,
    repo.dirty ? "uncommitted changes" : null,
  ]
    .filter(Boolean)
    .join(", ");
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          aria-label={`Planning repository: ${summary}. Details`}
          className="h-7 min-w-0 shrink gap-1.5 px-2 text-xs font-normal text-muted-foreground"
        >
          <Icon name="GitBranch" className="size-3.5 shrink-0" />
          <span className="hidden truncate @lg:inline">{repo.branch ?? "detached"}</span>
          {head ? (
            <span className="hidden font-mono text-subtle-foreground @3xl:inline">{head.slice(0, 7)}</span>
          ) : null}
          {repo.behind > 0 ? <span className="tabular-nums text-attention">↓{repo.behind}</span> : null}
          {repo.ahead > 0 ? <span className="tabular-nums">↑{repo.ahead}</span> : null}
          {repo.dirty ? (
            <span className="flex items-center gap-1">
              <span aria-hidden className="size-1.5 rounded-full bg-warning" />
              <span className="hidden @3xl:inline">modified</span>
            </span>
          ) : null}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 p-0" mobileTitle="Planning repository">
        <div className="space-y-3 p-3.5">
          <div className="min-w-0">
            <p className="text-sm font-semibold">Planning repository</p>
            <p className="mt-0.5 break-all font-mono text-xs text-muted-foreground">
              {formatHomePathForDisplay(repo.root)}
            </p>
          </div>
          <dl className="space-y-1.5">
            <Fact label="Host">{hostName ?? <span className="font-mono">{repo.hostId}</span>}</Fact>
            <Fact label="Branch">{repo.branch ?? "detached HEAD"}</Fact>
            <Fact label="Last commit">
              <span className="font-mono">{head.slice(0, 7)}</span>
              {date ? (
                <span className="text-muted-foreground">
                  {" "}
                  · {dayKey(date)} {hm(date)}
                </span>
              ) : null}
            </Fact>
            <Fact label="Origin">{sync}</Fact>
            <Fact label="Checkout">{repo.dirty ? "Uncommitted changes" : "Clean"}</Fact>
          </dl>
        </div>
        <div className="flex items-center gap-2 border-t border-border-hairline px-3.5 py-2.5">
          <p className="min-w-0 flex-1 text-xs text-subtle-foreground">
            {loadedAt !== null ? `Board loaded ${hm(new Date(loadedAt).toISOString())}` : ""}
          </p>
          <Button
            size="sm"
            variant="outline"
            className="h-7 gap-1.5 text-xs"
            disabled={refreshing}
            onClick={onRefresh}
          >
            <Icon
              name={refreshing ? "Spinner" : "RotateCcw"}
              className={cn("size-3.5", refreshing && "animate-spin")}
            />
            {refreshing ? "Refreshing…" : "Refresh from origin"}
          </Button>
        </div>
        <p className="border-t border-border-hairline px-3.5 py-2 text-xs text-subtle-foreground">
          Refresh fetches origin and fast-forwards the checkout only when it is clean.
        </p>
      </PopoverContent>
    </Popover>
  );
}

/** repo_refresh with a toast for its outcome; the board refetches afterwards. */
export function useRepoRefresh(projectId: string, onDone: () => void) {
  const rpc = usePlanrRpc();
  const [refreshing, setRefreshing] = useState(false);
  const refresh = async () => {
    if (refreshing) return;
    setRefreshing(true);
    try {
      const result = await rpc.call("repo_refresh", { projectId });
      if (result.ok)
        toast.success("Planning repository refreshed", { description: result.message ?? undefined });
      else
        toast.warning("Refresh incomplete", {
          description: result.message ?? "The checkout was not updated.",
        });
    } catch (cause) {
      toast.error("Refresh failed", { description: errorText(cause) });
    } finally {
      setRefreshing(false);
      onDone();
    }
  };
  return { refreshing, refresh };
}

export function RefreshButton({ refreshing, onRefresh }: { refreshing: boolean; onRefresh: () => void }) {
  return (
    <Tip content="Refresh from origin" side="bottom">
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="size-7 shrink-0 text-muted-foreground hover:text-foreground max-md:pointer-coarse:size-9"
        aria-label="Refresh from origin"
        aria-busy={refreshing}
        disabled={refreshing}
        onClick={onRefresh}
      >
        <Icon name="RotateCcw" className={cn("size-3.5", refreshing && "animate-spin")} />
      </Button>
    </Tip>
  );
}

function describeSession(session: Session): { icon: IconName; short: string; title: string; warn: boolean } {
  const p = session.principal;
  if (session.mode === "enforced" && p !== null) {
    const name = p.displayName ?? p.subject;
    return { icon: "UserRound", short: name, title: `Signed in as ${name}`, warn: !session.canApprove };
  }
  if (session.mode === "enforced") {
    return { icon: "Lock", short: "Not signed in", title: "Not signed in", warn: true };
  }
  if (session.mode === "unavailable") {
    return {
      icon: "Lock",
      short: "Approvals disabled",
      title: "Approvals disabled: no identity listener",
      warn: true,
    };
  }
  return {
    icon: "Info",
    short: "By convention",
    title: "Approvals by convention (no identity on this server)",
    warn: false,
  };
}

/** Who the server sees as the caller, and whether Approve, Close and Mark checked are allowed. */
export function SessionStatus({ session }: { session: Session }) {
  const d = describeSession(session);
  const p = session.principal;
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          aria-label={`${d.title}. Details`}
          className="h-7 min-w-0 shrink gap-1.5 px-2 text-xs font-normal text-muted-foreground"
        >
          <Icon name={d.icon} className={cn("size-3.5 shrink-0", d.warn && "text-warning")} />
          <span className="hidden max-w-36 truncate @2xl:inline">{d.short}</span>
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 p-0" mobileTitle="Approvals">
        <div className="space-y-3 p-3.5">
          <p className="text-sm font-semibold">{d.title}</p>
          {p !== null ? (
            <dl className="space-y-1.5">
              <Fact label="Identity">
                <span className="break-all font-mono">
                  {p.issuer}:{p.subject}
                </span>
              </Fact>
              <Fact label="Kind">{p.kind}</Fact>
            </dl>
          ) : null}
          <p className="text-xs text-muted-foreground">
            {session.canApprove
              ? session.mode === "convention"
                ? "Approve, Close and Mark checked exist only in this board, not in the CLI or for agents. " +
                  "This server cannot tell who clicks."
                : "You can approve and close goals and mark tasks checked."
              : (session.reason ?? "Approve, Close and Mark checked are not allowed for you.")}
          </p>
        </div>
      </PopoverContent>
    </Popover>
  );
}
