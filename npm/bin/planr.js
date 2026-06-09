#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(here, "..", "..");
const candidates = [
  process.env.PLANR_NATIVE_BIN,
  path.join(packageRoot, "target", "release", "planr"),
  path.join(packageRoot, "target", "debug", "planr"),
  path.join(packageRoot, "dist", "planr-1.0.0", "planr"),
].filter(Boolean);

const binary = candidates.find(candidate => fs.existsSync(candidate));

if (!binary) {
  console.error("Planr native binary was not found.");
  console.error("Build it with: cargo build --release");
  console.error("Or set PLANR_NATIVE_BIN=/absolute/path/to/planr");
  process.exit(127);
}

const result = spawnSync(binary, process.argv.slice(2), {
  stdio: "inherit",
  env: process.env,
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 0);
