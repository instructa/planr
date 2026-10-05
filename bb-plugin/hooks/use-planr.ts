// Data hooks of the planr board. One request in flight per query; realtime
// signals and reconnects queue a single refetch, and late responses for an older
// project are dropped, so the board never shows another project's data.
import { useCallback, useEffect, useRef, useState } from "react";
import {
  experimental_usePluginId,
  useBbContext,
  useRealtime,
  useRealtimeConnectionState,
  useRpc,
} from "@get-bb/plugin-sdk/app";
import { PLANR_CHANGED, type BoardResult, type rpcContract, type Session } from "../contract";

export type PlanrRpc = ReturnType<typeof usePlanrRpc>;
export const usePlanrRpc = () => useRpc<typeof rpcContract>();

export const errorText = (cause: unknown) => (cause instanceof Error ? cause.message : String(cause));

/**
 * Calls `onReconnect` when the realtime socket comes back after a drop. Plugin
 * signals are not replayed, so durable state must be refetched then. The first
 * connection is skipped because the mount fetch already covers it.
 */
function useReconnect(onReconnect: () => void) {
  const state = useRealtimeConnectionState();
  const previous = useRef(state);
  const established = useRef(state !== "connecting");
  const callback = useRef(onReconnect);
  callback.current = onReconnect;
  useEffect(() => {
    if (state === "reconnecting") established.current = true;
    if (state === "connected" && previous.current !== "connected") {
      if (established.current) callback.current();
      established.current = true;
    }
    previous.current = state;
  }, [state]);
}

interface Query<T> {
  data: T | undefined;
  error: string | null;
  loading: boolean;
  refetch: () => void;
}

/** A fetch keyed by `key`; refetches coalesce while one is in flight. */
function useQuery<T>(key: string | null, fetcher: () => Promise<T>): Query<T> {
  const [state, setState] = useState<{ key: string | null; data: T | undefined; error: string | null }>({
    key,
    data: undefined,
    error: null,
  });
  const [loading, setLoading] = useState(key !== null);
  const fetcherRef = useRef(fetcher);
  fetcherRef.current = fetcher;
  const keyRef = useRef(key);
  keyRef.current = key;
  const inFlight = useRef(false);
  const queued = useRef(false);

  const run = useCallback(() => {
    const requested = keyRef.current;
    if (requested === null) return;
    if (inFlight.current) {
      queued.current = true;
      return;
    }
    inFlight.current = true;
    setLoading(true);
    fetcherRef
      .current()
      .then(
        (data) => {
          if (keyRef.current === requested) setState({ key: requested, data, error: null });
        },
        (cause: unknown) => {
          if (keyRef.current !== requested) return;
          // Keep the last good data of the same key; a failed refresh only adds an error.
          setState((s) => ({
            key: requested,
            data: s.key === requested ? s.data : undefined,
            error: errorText(cause),
          }));
        },
      )
      .finally(() => {
        inFlight.current = false;
        if (queued.current || keyRef.current !== requested) {
          queued.current = false;
          run();
          return;
        }
        setLoading(false);
      });
  }, []);

  useEffect(() => {
    setState((s) => (s.key === key ? s : { key, data: undefined, error: null }));
    if (key === null) {
      setLoading(false);
      return;
    }
    run();
  }, [key, run]);

  const current = state.key === key;
  return {
    data: current ? state.data : undefined,
    error: current ? state.error : null,
    loading: key !== null && (loading || !current),
    refetch: run,
  };
}

/** Coalesces bursts of signals (a host watch can fire many) into one call. */
function useBatched(callback: () => void, ms = 120) {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const ref = useRef(callback);
  ref.current = callback;
  useEffect(
    () => () => {
      if (timer.current !== null) clearTimeout(timer.current);
    },
    [],
  );
  return useCallback(() => {
    if (timer.current !== null) return;
    timer.current = setTimeout(() => {
      timer.current = null;
      ref.current();
    }, ms);
  }, [ms]);
}

const signalProject = (payload: unknown): string | null => {
  if (typeof payload !== "object" || payload === null) return null;
  const value: unknown = Reflect.get(payload, "projectId");
  return typeof value === "string" ? value : null;
};

export interface PlanrProject {
  id: string;
  name: string;
  mapped: boolean;
}

export function useProjects(): Query<PlanrProject[]> {
  const rpc = usePlanrRpc();
  const query = useQuery("projects", async () => (await rpc.call("projects_list", null)).projects);
  const refetch = useBatched(query.refetch);
  // Mapping changes arrive on the same channel; the list carries `mapped`.
  useRealtime(PLANR_CHANGED, refetch);
  useReconnect(query.refetch);
  return query;
}

export function useBoard(projectId: string | null): Query<BoardResult> & { loadedAt: number | null } {
  const rpc = usePlanrRpc();
  const query = useQuery(projectId, () => rpc.call("board_get", { projectId: projectId ?? "" }));
  const [loadedAt, setLoadedAt] = useState<number | null>(null);
  useEffect(() => {
    if (query.data !== undefined) setLoadedAt(Date.now());
  }, [query.data]);
  const refetch = useBatched(query.refetch);
  useRealtime(PLANR_CHANGED, (payload) => {
    const signalled = signalProject(payload);
    if (signalled === null || signalled === projectId) refetch();
  });
  useReconnect(query.refetch);
  return { ...query, loadedAt };
}

/**
 * The caller's identity and whether human actions are allowed. Loaded with
 * the board and again after a realtime reconnect; the server stays the authority
 * and refuses with "forbidden" whatever the UI shows.
 */
export function useSession(): Query<Session> {
  const rpc = usePlanrRpc();
  const query = useQuery("session", () => rpc.call("session_get", null));
  useReconnect(query.refetch);
  return query;
}

/** bb's placeholder project of projectless threads; it never has a planning repository. */
const PERSONAL_PROJECT_ID = "proj_personal";

/**
 * The project of the thread a side panel belongs to: the route's project while the
 * route shows that thread, else thread_project_get. `projectId` is undefined while
 * unknown and null for a thread without a project.
 */
export function useThreadProject(threadId: string) {
  const context = useBbContext();
  const fromRoute =
    context.projectId !== null && (context.threadId === null || context.threadId === threadId)
      ? context.projectId
      : null;
  const rpc = usePlanrRpc();
  const query = useQuery(fromRoute === null ? `thread:${threadId}` : null, async () => {
    try {
      return (await rpc.call("thread_project_get", { threadId })).projectId;
    } catch (cause) {
      // A server without the method (older plugin build) answers unknown_method.
      if (Reflect.get(Object(cause), "code") === "unknown_method") {
        throw new Error("This Planr server cannot look up a thread's project yet.");
      }
      throw cause;
    }
  });
  const projectId = fromRoute ?? query.data;
  return {
    projectId: projectId === PERSONAL_PROJECT_ID ? null : projectId,
    error: fromRoute === null ? query.error : null,
    refetch: query.refetch,
  };
}

export function useMapping(projectId: string) {
  const rpc = usePlanrRpc();
  return useQuery(`mapping:${projectId}`, () => rpc.call("mapping_get", { projectId }));
}

/** The last chosen project, kept per plugin id so a renamed copy does not share it. */
export function useLastProject(): [string | null, (projectId: string) => void] {
  const pluginId = experimental_usePluginId();
  const storageKey = `${pluginId}:last-project`;
  const read = () => {
    try {
      return window.localStorage.getItem(storageKey);
    } catch {
      return null;
    }
  };
  const [value, setValue] = useState<string | null>(read);
  const remember = useCallback(
    (projectId: string) => {
      setValue(projectId);
      try {
        window.localStorage.setItem(storageKey, projectId);
      } catch {
        // Storage can be unavailable (private mode); the choice then lasts for this page only.
      }
    },
    [storageKey],
  );
  return [value, remember];
}
