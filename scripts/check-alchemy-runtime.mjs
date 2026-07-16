#!/usr/bin/env node
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const MINIMUM_ALCHEMY_NODE_MAJOR = 22;

export function assertAlchemyRuntime(version = process.versions.node) {
  const major = Number.parseInt(String(version).split(".", 1)[0], 10);
  if (!Number.isSafeInteger(major) || major < MINIMUM_ALCHEMY_NODE_MAJOR) {
    throw new Error(
      `Cloudflare deployment requires Node.js ${MINIMUM_ALCHEMY_NODE_MAJOR} or newer; current runtime is ${version}. ` +
        "The published Planr CLI remains compatible with Node.js 18 or newer.",
    );
  }
  return major;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const major = assertAlchemyRuntime();
    console.log(`Alchemy deployment runtime ready (Node.js ${major})`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
