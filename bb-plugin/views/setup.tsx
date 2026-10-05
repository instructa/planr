// Before the board: choosing a project (upstream bb gives plugin pages none)
// and connecting a project to its planning repository.
import { useEffect, useId, useMemo, useState, type FormEvent } from "react";
import { useSdk } from "@get-bb/plugin-sdk/app";
import { Button } from "@/components/ui/button";
import { Icon } from "@/components/ui/icon";
import { Input } from "@/components/ui/input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn, formatHomePathForDisplay } from "@/lib/utils";
import { errorText, useMapping, usePlanrRpc, type PlanrProject } from "@/hooks/use-planr";
import { EmptyState, FOCUS_RING } from "./parts";

export interface HostOption {
  id: string;
  name: string;
  connected: boolean;
}

/** bb hosts for the folder picker; empty when the list is not available. */
export function useHosts(): HostOption[] {
  const sdk = useSdk();
  const [hosts, setHosts] = useState<HostOption[]>([]);
  useEffect(() => {
    let live = true;
    sdk.hosts.list().then(
      (list) => {
        if (live)
          setHosts(list.map((h) => ({ id: h.id, name: h.name, connected: h.status === "connected" })));
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [sdk]);
  return hosts;
}

export function ProjectChooser({
  projects,
  error,
  onChoose,
}: {
  projects: PlanrProject[] | undefined;
  error: string | null;
  onChoose: (projectId: string) => void;
}) {
  if (projects === undefined) {
    return error !== null ? (
      <EmptyState icon="AlertTriangle" title="Projects could not be loaded" description={error} />
    ) : (
      <EmptyState icon="ListTodo" title="Loading projects…" />
    );
  }
  if (projects.length === 0) {
    return (
      <EmptyState
        icon="ListTodo"
        title="No projects yet"
        description="Create a project in bb, then connect it to its planning repository here."
      />
    );
  }
  return (
    <EmptyState
      icon="ListTodo"
      title="Choose a project"
      description="Planr shows the task board of a project's planning repository."
    >
      <ul
        className={cn(
          "mt-1 w-full max-w-sm divide-y divide-border-hairline overflow-hidden rounded-lg border",
          "border-border bg-card text-left",
        )}
      >
        {projects.map((p) => (
          <li key={p.id}>
            <button
              type="button"
              onClick={() => onChoose(p.id)}
              className={cn(
                "flex w-full items-center gap-2 px-3.5 py-2.5 text-left text-sm hover:bg-state-hover",
                FOCUS_RING,
              )}
            >
              <span className="min-w-0 flex-1 truncate font-medium">{p.name}</span>
              <span className="text-xs text-subtle-foreground">{p.mapped ? "Connected" : "Not set up"}</span>
              <Icon name="ChevronRight" className="size-3.5 text-muted-foreground" />
            </button>
          </li>
        ))}
      </ul>
    </EmptyState>
  );
}

const code = (text: string) => <code className="font-mono text-xs">{text}</code>;

export function MappingSetup({
  projectId,
  projectName,
  onConnected,
}: {
  projectId: string;
  projectName: string | null;
  onConnected: () => void;
}) {
  const rpc = usePlanrRpc();
  const mapping = useMapping(projectId);
  const hosts = useHosts();
  const suggestions = mapping.data?.suggestions ?? [];
  const hostOptions = useMemo(() => {
    const byId = new Map<string, HostOption>();
    for (const s of suggestions)
      byId.set(s.hostId, { id: s.hostId, name: s.hostName ?? s.hostId, connected: true });
    for (const h of hosts) byId.set(h.id, h);
    return [...byId.values()];
  }, [hosts, suggestions]);
  const [hostId, setHostId] = useState<string | null>(null);
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState<"pick" | "save" | null>(null);
  const [savingPath, setSavingPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pathId = useId();
  const errorId = useId();
  const host = hostId ?? suggestions[0]?.hostId ?? hostOptions.find((h) => h.connected)?.id ?? null;

  const connect = async (targetHost: string, root: string) => {
    if (busy !== null) return;
    setBusy("save");
    setSavingPath(root);
    setError(null);
    try {
      const result = await rpc.call("mapping_set", { projectId, mapping: { hostId: targetHost, root } });
      if (result.ok) onConnected();
      else setError(result.message ?? `${root} is not a planning repository.`);
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(null);
      setSavingPath(null);
    }
  };

  const pick = async () => {
    if (host === null || busy !== null) return;
    setBusy("pick");
    setError(null);
    try {
      const result = await rpc.call("mapping_pick_folder", { projectId, hostId: host, clientHostId: null });
      if (result.path !== null) setPath(result.path);
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(null);
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const root = path.trim();
    if (root === "" || host === null) return;
    void connect(host, root);
  };

  return (
    <div className="mx-auto flex w-full max-w-lg flex-col gap-5 px-4 py-10">
      <div className="flex flex-col items-center gap-3 text-center">
        <div
          className={cn(
            "flex size-10 items-center justify-center rounded-md bg-secondary text-muted-foreground",
          )}
        >
          <Icon name="FolderPlus" className="size-5" />
        </div>
        <div className="space-y-1">
          <h2 className="text-sm font-medium">
            Connect {projectName ?? "this project"} to its planning repository
          </h2>
          <p className="text-sm text-muted-foreground">
            The planning repository is the folder with {code("tasks/")} or {code("planr.config.json")}. Planr
            reads the board from it.
          </p>
        </div>
      </div>

      {suggestions.length > 0 ? (
        <section
          aria-label="Suggested folders"
          className="overflow-hidden rounded-lg border border-border bg-card"
        >
          <p
            className={cn(
              "border-b border-border-hairline px-3.5 py-2 text-xs font-medium text-muted-foreground",
            )}
          >
            From this project's sources
          </p>
          <ul className="divide-y divide-border-hairline">
            {suggestions.map((s) => (
              <li key={`${s.hostId}:${s.path}`} className="flex items-center gap-2 px-3.5 py-2">
                <Icon name="Folder" className="size-3.5 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-mono text-xs" title={s.path}>
                    {formatHomePathForDisplay(s.path)}
                  </span>
                  <span className="block truncate text-xs text-subtle-foreground">
                    {s.hostName ?? s.hostId}
                  </span>
                </span>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 text-xs"
                  disabled={busy !== null}
                  aria-busy={savingPath === s.path || undefined}
                  onClick={() => void connect(s.hostId, s.path)}
                >
                  {savingPath === s.path ? <Icon name="Spinner" className="size-3.5 animate-spin" /> : null}
                  Use
                </Button>
              </li>
            ))}
          </ul>
        </section>
      ) : mapping.loading ? (
        <p className="text-center text-xs text-subtle-foreground">Looking for planning folders…</p>
      ) : null}

      <form onSubmit={submit} className="space-y-2" aria-describedby={error !== null ? errorId : undefined}>
        <label htmlFor={pathId} className="block text-xs font-medium text-muted-foreground">
          Planning folder
        </label>
        {hostOptions.length > 1 ? (
          <div className="flex items-center gap-2 text-xs">
            <span className="text-muted-foreground">Host</span>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 gap-1.5 px-2 text-xs font-normal"
                  aria-label={`Host: ${hostOptions.find((h) => h.id === host)?.name ?? "none"}. Change host`}
                >
                  {hostOptions.find((h) => h.id === host)?.name ?? "Choose host"}
                  <Icon name="ChevronDown" className="size-3 text-muted-foreground" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="min-w-48" mobileTitle="Host">
                {hostOptions.map((h) => (
                  <DropdownMenuItem key={h.id} onSelect={() => setHostId(h.id)} className="gap-2">
                    <Icon
                      name="Check"
                      className={cn("size-3.5", h.id === host ? "opacity-100" : "opacity-0")}
                    />
                    <span className="min-w-0 flex-1 truncate">{h.name}</span>
                    {h.connected ? null : <span className="text-xs text-subtle-foreground">offline</span>}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        ) : null}
        <div className="flex items-center gap-2">
          <Input
            id={pathId}
            value={path}
            onChange={(e) => setPath(e.target.value)}
            placeholder="/path/to/planning-repo"
            spellCheck={false}
            aria-invalid={error !== null || undefined}
            className="h-8 font-mono text-xs"
          />
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-8 shrink-0 text-xs"
            disabled={host === null || busy !== null}
            onClick={() => void pick()}
          >
            {busy === "pick" ? (
              <Icon name="Spinner" className="size-3.5 animate-spin" />
            ) : (
              <Icon name="Folder" className="size-3.5" />
            )}
            Choose…
          </Button>
        </div>
        {error !== null ? (
          <p id={errorId} role="alert" className="text-xs text-destructive">
            {error}
          </p>
        ) : null}
        {mapping.error !== null ? <p className="text-xs text-destructive">{mapping.error}</p> : null}
        <div className="flex justify-end pt-1">
          <Button
            type="submit"
            size="sm"
            className="h-8"
            disabled={path.trim() === "" || host === null || busy !== null}
          >
            {busy === "save" && savingPath === path.trim() ? (
              <Icon name="Spinner" className="size-3.5 animate-spin" />
            ) : null}
            Connect
          </Button>
        </div>
      </form>
    </div>
  );
}
