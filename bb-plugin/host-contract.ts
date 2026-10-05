import { defineRpcContract } from "@get-bb/plugin-sdk";
import { z } from "zod";
import { actionSchema, actionResultSchema, boardResultSchema } from "./contract.ts";

const root = z.object({ root: z.string().min(1) });
const checked = z.object({
  root: z.string(),
  errors: z.array(z.string()),
  warnings: z.array(z.string()),
  counts: z.record(z.string(), z.number()),
});
export const hostSignals = { changed: { payload: root } };
export const hostContract = defineRpcContract({
  board: {
    input: root.extend({ hostId: z.string() }),
    output: boardResultSchema,
  },
  check: { input: root, output: checked },
  next: {
    input: root.extend({ goal: z.string().optional(), stream: z.string().optional() }),
    output: z.object({
      ready: z.array(z.record(z.string(), z.unknown())),
      waiting: z.array(z.record(z.string(), z.unknown())),
      errors: z.array(z.string()),
      warnings: z.array(z.string()),
    }),
  },
  refresh: { input: root, output: z.object({ ok: z.boolean(), message: z.string().nullable() }) },
  apply: {
    input: root.extend({
      action: actionSchema,
      expectedSha256: z.string().min(1),
      approvedBy: z.string().min(1).refine(value => !/[\x00-\x1f\x7f]/.test(value)).optional(),
    }),
    output: actionResultSchema,
  },
  watch: { input: root.extend({ key: z.string() }), output: z.null() },
  unwatch: { input: z.object({ key: z.string() }), output: z.null() },
});
