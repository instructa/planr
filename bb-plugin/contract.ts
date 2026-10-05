// Shared contract of the planr bb plugin. The UI, server and host code against it;
// change it here first.
//
// Layers (see README.md):
// - app.tsx (UI) talks to server.ts over `rpcContract` and listens on PLANR_CHANGED.
// - server.ts owns the project → planning-repo mapping (bb.storage.kv), the CLI and
//   the policy of which human actions exist. It never touches files itself.
// - host.ts runs on the machine that holds the planning repo. It is the only code that
//   reads files, runs git and calls the planr engine (../skills/planr/scripts/lib/*.mjs).
//   It owns the write protocol (own worktree, origin-first compare-and-set, push).
// - Readiness, unlocks, validation and board data come only from the engine.
import { defineRpcContract } from "@get-bb/plugin-sdk";
import { z } from "zod";

/** Realtime channel; payload `{ projectId }`. Published after a host watch signal or a write. */
export const PLANR_CHANGED = "planr-changed";

/** Where a project's planning repo lives. Stored in kv under `mapping:<projectId>`. */
export const mappingSchema = z.object({
  hostId: z.string().min(1),
  /** Absolute path of the planning root (directory with tasks/ or planr.config.json). */
  root: z.string().min(1),
});
export type Mapping = z.infer<typeof mappingSchema>;

/** Git state of the configured (main) checkout the board reads from. */
export const repoStateSchema = z.object({
  root: z.string(),
  hostId: z.string(),
  branch: z.string().nullable(),
  head: z.string().nullable(),
  dirty: z.boolean(),
  ahead: z.number().int(),
  behind: z.number().int(),
  hasRemote: z.boolean(),
});
export type RepoState = z.infer<typeof repoStateSchema>;

/**
 * Human actions. Allowed transitions are fixed here and re-checked on the host:
 * - approve-goal: goal state draft → approved
 * - close-goal:   goal state review → done
 * - check-task:   task status review → done (an approver's manual check)
 * There is no CLI or agent-tool path to these actions.
 */
export const actionSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("approve-goal"), goal: z.string().min(1) }),
  z.object({ kind: z.literal("close-goal"), goal: z.string().min(1) }),
  z.object({ kind: z.literal("check-task"), id: z.string().min(1) }),
]);
export type Action = z.infer<typeof actionSchema>;

export const actionResultSchema = z.union([
  z.object({
    ok: z.literal(true),
    commit: z.string(),
    pushed: z.boolean(),
    /** e.g. "no remote; committed locally" */
    note: z.string().nullable(),
  }),
  z.object({
    ok: z.literal(false),
    reason: z.enum(["conflict", "invalid-transition", "push-rejected", "not-mapped", "host-error", "forbidden"]),
    message: z.string(),
  }),
]);
export type ActionResult = z.infer<typeof actionResultSchema>;

/**
 * Board payload. `data` is the engine's BoardData (docs/contract.md, "Board data")
 * passed through unchanged; the UI reads derived fields from it and never recomputes them.
 * `fileSha` maps every task/goal file path (relative to root) to the sha256 the board was
 * rendered from; actions send the one they act on as `expectedSha256`.
 */
export const boardResultSchema = z.discriminatedUnion("status", [
  z.object({ status: z.literal("unmapped") }),
  z.object({
    status: z.literal("ok"),
    data: z.record(z.string(), z.unknown()),
    repo: repoStateSchema,
    fileSha: z.record(z.string(), z.string()),
  }),
  z.object({ status: z.literal("error"), message: z.string() }),
]);
export type BoardResult = z.infer<typeof boardResultSchema>;

/**
 * Who may click human actions. Where bb provides `bb.server.experimental_requestPrincipal()`
 * (for example a fork with a trusted identity listener), it identifies the caller by issuer + subject;
 * upstream bb has no request identity.
 * - "enforced": identity API with an identity listener. Actions need a principal of kind "person" whose
 *   `issuer:subject` is in the `approvers` setting (empty setting = any person).
 * - "unavailable": identity API present but no identity listener → actions are refused.
 * - "convention": upstream bb. Actions are UI-only by convention (no CLI/agent path).
 * Known limit: agents on a device signed in as the user carry that user's identity.
 */
export const principalSchema = z.object({
  kind: z.enum(["person", "machine", "local"]),
  issuer: z.string().min(1),
  subject: z.string().min(1),
  displayName: z.string().nullable(),
});
export type Principal = z.infer<typeof principalSchema>;

export const sessionSchema = z.object({
  mode: z.enum(["enforced", "unavailable", "convention"]),
  principal: principalSchema.nullable(),
  canApprove: z.boolean(),
  /** Why actions are not allowed, user-facing; null when canApprove. */
  reason: z.string().nullable(),
});
export type Session = z.infer<typeof sessionSchema>;

export const rpcContract = defineRpcContract({
  /**
   * Project of a thread, for the side-panel view (threadPanelAction gets only a threadId).
   * The UI prefers useBbContext().projectId and falls back to this.
   */
  thread_project_get: {
    input: z.object({ threadId: z.string().min(1) }),
    output: z.object({ projectId: z.string().nullable() }),
  },
  /** The caller's identity and whether human actions are allowed for them. */
  session_get: {
    input: z.null(),
    output: sessionSchema,
  },
  /** bb projects with their mapping state; used by the project picker (upstream bb has no implicit project on plugin pages). */
  projects_list: {
    input: z.null(),
    output: z.object({
      projects: z.array(z.object({ id: z.string(), name: z.string(), mapped: z.boolean() })),
    }),
  },
  /** Current mapping plus suggestions from the project's sources (hostId + path). */
  mapping_get: {
    input: z.object({ projectId: z.string() }),
    output: z.object({
      mapping: mappingSchema.nullable(),
      suggestions: z.array(z.object({ hostId: z.string(), hostName: z.string().nullable(), path: z.string() })),
    }),
  },
  /** Validates on the host (planr check must find a planning root) before storing. */
  mapping_set: {
    input: z.object({ projectId: z.string(), mapping: mappingSchema }),
    output: z.object({ ok: z.boolean(), message: z.string().nullable() }),
  },
  mapping_pick_folder: {
    input: z.object({ projectId: z.string(), hostId: z.string(), clientHostId: z.string().nullable() }),
    output: z.object({ path: z.string().nullable() }),
  },
  board_get: {
    input: z.object({ projectId: z.string() }),
    output: boardResultSchema,
  },
  /** fetch; pull --ff-only only when the main checkout is clean. */
  repo_refresh: {
    input: z.object({ projectId: z.string() }),
    output: z.object({ ok: z.boolean(), message: z.string().nullable() }),
  },
  action_apply: {
    input: z.object({ projectId: z.string(), action: actionSchema, expectedSha256: z.string().min(1) }),
    output: actionResultSchema,
  },
});
