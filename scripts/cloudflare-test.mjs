#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { assertAlchemyRuntime } from "./check-alchemy-runtime.mjs";

export function cloudflareTestSteps(action) {
  if (action === "deploy") {
    return [
      ["pnpm", ["site:check"]],
      ["pnpm", ["exec", "alchemy", "deploy", "--stage", "test"]],
    ];
  }
  if (action === "destroy") {
    return [["pnpm", ["exec", "alchemy", "destroy", "--stage", "test"]]];
  }
  throw new Error("usage: node scripts/cloudflare-test.mjs <deploy|destroy>");
}

export function runCloudflareTest(action, spawn = spawnSync) {
  assertAlchemyRuntime();
  for (const [command, args] of cloudflareTestSteps(action)) {
    const result = spawn(command, args, { stdio: "inherit", env: process.env });
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error(`${command} ${args.join(" ")} exited with status ${result.status}`);
    }
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    runCloudflareTest(process.argv[2]);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
