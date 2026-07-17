#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function option(name, fallback) {
  const index = process.argv.indexOf(name);
  return index === -1 ? fallback : process.argv[index + 1];
}

const routingBin = resolve(packageRoot, option("--routing-bin", "../target/release/planr-routing"));
const catalog = resolve(packageRoot, "website/data/catalog.json");

function run(args) {
  const result = spawnSync(routingBin, args, {
    cwd: packageRoot,
    stdio: "inherit",
    env: process.env,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${routingBin} ${args.join(" ")} exited with status ${result.status}`);
  }
}

try {
  run(["catalog", "build", "--output", catalog]);
  run(["catalog", "verify", catalog]);
  console.log("regenerated 20 experimental routing compositions from package-owned sources");
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
}
