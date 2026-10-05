// Panel routes: subPath = <projectId>[/<view>[/<item>]].
// item is a task ID in Tasks and a run key (commit or "worktree") in Runs.
export const PANEL_PATH = "board";

export type BoardViewName = "overview" | "runs" | "tasks";
export const VIEWS: BoardViewName[] = ["overview", "runs", "tasks"];

export interface PanelRoute {
  projectId: string | null;
  view: BoardViewName;
  item: string | null;
}

const isView = (value: string | undefined): value is BoardViewName =>
  value !== undefined && (VIEWS as string[]).includes(value);

const decode = (part: string | undefined): string | null => {
  if (part === undefined || part === "") return null;
  try {
    return decodeURIComponent(part);
  } catch {
    return null;
  }
};

export function parseRoute(subPath: string): PanelRoute {
  const [project, view, ...rest] = subPath.split("/");
  return {
    projectId: decode(project),
    view: isView(view) ? view : "overview",
    item: isView(view) && view !== "overview" ? decode(rest.join("/")) : null,
  };
}

export function buildSubPath(route: PanelRoute): string {
  if (route.projectId === null) return "";
  const parts = [encodeURIComponent(route.projectId)];
  if (route.view !== "overview" || route.item !== null) parts.push(route.view);
  if (route.item !== null && route.view !== "overview") parts.push(encodeURIComponent(route.item));
  return parts.join("/");
}
