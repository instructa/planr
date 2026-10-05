// The standalone dependency-free engine is JavaScript; its wire data stays opaque to the server.
declare module "*.mjs" {
  export function config(root: string, defaults?: object): {
    tasksDir: string;
    goalsDir: string;
    boardOut: string;
    goalSuffix: string;
  };
  export function findRoot(start: string): string;
  export function buildIndex(
    root: string,
    settings?: ReturnType<typeof config>,
    read?: (file: string) => string,
  ): {
    index: {
      tasks: Array<Record<string, unknown> & {
        id: string;
        file: string;
        status: string;
      }>;
      goals: Array<Record<string, unknown> & {
        name: string;
        file: string;
        state: string;
        stream: string | null;
      }>;
      counts: Record<string, number>;
      warnings: string[];
    };
    errors: string[];
  };
  export function nextWork(
    index: ReturnType<typeof buildIndex>["index"],
    goals: ReturnType<typeof buildIndex>["index"]["goals"],
    stream?: string,
  ): {
    ready: Array<Record<string, unknown>>;
    waiting: Array<Record<string, unknown>>;
  };
  export function rewrite(file: string, fields: Record<string, string>): void;
  export function boardData(
    root: string,
    index: ReturnType<typeof buildIndex>["index"],
    settings: ReturnType<typeof config>,
  ): Record<string, unknown>;
  export function runs(
    root: string,
    index: ReturnType<typeof buildIndex>["index"],
    settings: ReturnType<typeof config>,
  ): { warnings: string[] };
  export function git(root: string, args: string[]): string;
}
