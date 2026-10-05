// Human board actions: approve a draft goal, close a review goal, mark a review
// task checked. Each sends the sha256 of the file as the board last saw it; the
// host refuses when the file changed since. Nothing is shown as done before the
// host answers, and the board refetches after every answer.
import { useCallback, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import type { Action, ActionResult, Session } from "../contract";
import { errorText, usePlanrRpc } from "./use-planr";

export interface BoardActionTarget {
  action: Action;
  /** Task or goal file path relative to the planning root, the key into fileSha. */
  file: string;
}

const VERB: Record<Action["kind"], { doing: string; done: string; failed: string }> = {
  "approve-goal": { doing: "Approving", done: "Goal approved", failed: "Goal not approved" },
  "close-goal": { doing: "Closing", done: "Goal closed", failed: "Goal not closed" },
  "check-task": { doing: "Saving", done: "Marked checked", failed: "Not marked checked" },
};

const subject = (action: Action) => (action.kind === "check-task" ? action.id : action.goal);
const shortCommit = (commit: string) => commit.slice(0, 7);

function report(action: Action, result: ActionResult) {
  const verb = VERB[action.kind];
  const name = subject(action);
  if (result.ok) {
    if (result.pushed) {
      toast.success(`${verb.done}: ${name}`, {
        description: `Committed ${shortCommit(result.commit)} and pushed to origin.`,
      });
    } else {
      toast.warning(`${verb.done}: ${name} (not pushed)`, {
        description: `Commit ${shortCommit(result.commit)} stays local: ${result.note ?? "not pushed"}.`,
      });
    }
    return;
  }
  switch (result.reason) {
    case "conflict":
      toast.warning(`${verb.failed}: ${name} changed`, {
        description: `${result.message} Nothing was written.`,
      });
      return;
    case "push-rejected":
      toast.error(`${verb.failed}: push rejected`, {
        description: `${result.message} Origin did not accept the change.`,
      });
      return;
    case "invalid-transition":
      toast.error(`${verb.failed}: ${name}`, { description: result.message });
      return;
    case "not-mapped":
      toast.error(`${verb.failed}: no planning repository`, { description: result.message });
      return;
    case "host-error":
      toast.error(`${verb.failed}: host error`, { description: result.message });
      return;
    case "forbidden":
      toast.error(`${verb.failed}: not allowed`, { description: result.message });
      return;
  }
}

/**
 * Who may take human actions, from session_get. While the session is unknown the
 * actions stay offered; the server stays the authority and refuses with "forbidden".
 */
export function useActionAccess(session: Session | undefined, onForbidden: () => void) {
  const blocked =
    session === undefined || session.canApprove
      ? null
      : (session.reason ?? "Approve, Close and Mark checked are not allowed for you.");
  return useMemo(() => ({ blocked, onForbidden }), [blocked, onForbidden]);
}

export function useBoardActions(
  projectId: string,
  fileSha: Record<string, string> | null,
  refetch: () => void,
  repo: { refreshing: boolean; refresh: () => Promise<void> },
  access: {
    /** Why human actions are not allowed for this caller; null when they are (or unknown). */
    blocked: string | null;
    /** Called after the server refused with "forbidden"; the session may have changed. */
    onForbidden: () => void;
  },
) {
  const rpc = usePlanrRpc();
  const [running, setRunning] = useState<ReadonlySet<string>>(() => new Set());
  // Applied actions whose file the board still shows unchanged: the host pushed
  // the change, but the main checkout the board reads has not pulled it yet.
  const [applied, setApplied] = useState<ReadonlyMap<string, { commit: string; sha: string }>>(
    () => new Map(),
  );
  // After an answer the button stays disabled until the next board payload
  // replaces the one the action was based on (or a few seconds pass).
  const [settling, setSettling] = useState<ReadonlyMap<string, Record<string, string> | null>>(
    () => new Map(),
  );
  // The ref guards double clicks that land before the disabled state renders.
  const inFlight = useRef(new Set<string>());

  const apply = useCallback(
    async ({ action, file }: BoardActionTarget) => {
      if (inFlight.current.has(file)) return;
      if (access.blocked !== null) {
        toast.error(`${VERB[action.kind].failed}: not allowed`, { description: access.blocked });
        return;
      }
      const basis = fileSha;
      const expectedSha256 = basis?.[file];
      if (expectedSha256 === undefined) {
        toast.error(`${VERB[action.kind].failed}: ${subject(action)}`, {
          description: `${file} is not in the loaded board. Refresh and try again.`,
        });
        refetch();
        return;
      }
      inFlight.current.add(file);
      setRunning((s) => new Set(s).add(file));
      try {
        const result = await rpc.call("action_apply", { projectId, action, expectedSha256 });
        report(action, result);
        if (!result.ok && result.reason === "forbidden") access.onForbidden();
        if (result.ok)
          setApplied((m) => new Map(m).set(file, { commit: result.commit, sha: expectedSha256 }));
      } catch (cause) {
        toast.error(`${VERB[action.kind].failed}: ${subject(action)}`, { description: errorText(cause) });
      } finally {
        inFlight.current.delete(file);
        setSettling((m) => new Map(m).set(file, basis));
        setRunning((s) => {
          const next = new Set(s);
          next.delete(file);
          return next;
        });
        setTimeout(() => {
          setSettling((m) => {
            if (m.get(file) !== basis) return m;
            const next = new Map(m);
            next.delete(file);
            return next;
          });
        }, 8000);
        refetch();
      }
    },
    [access, fileSha, projectId, refetch, rpc],
  );

  const isPending = (file: string) =>
    running.has(file) || (settling.has(file) && settling.get(file) === fileSha);
  const isRunning = (file: string) => running.has(file);
  /** The commit of an applied action while the board still shows the file as it was before. */
  const appliedCommit = (file: string) => {
    const entry = applied.get(file);
    return entry !== undefined && fileSha?.[file] === entry.sha ? entry.commit : null;
  };
  return {
    apply,
    isPending,
    isRunning,
    appliedCommit,
    blocked: access.blocked,
    refreshing: repo.refreshing,
    refreshRepo: () => void repo.refresh(),
  };
}

export type BoardActions = ReturnType<typeof useBoardActions>;
