#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(here, "..", "..");

function platformTarget() {
  const osName = { darwin: "darwin", linux: "linux" }[os.platform()];
  const arch = { arm64: "arm64", x64: "x86_64" }[os.arch()];
  if (!osName || !arch) {
    return null;
  }
  return `${osName}-${arch}`;
}

const target = platformTarget();
const candidates = [
  process.env.PLANR_NATIVE_BIN,
  // Published package: per-platform binaries bundled at release time.
  target && path.join(here, "..", "native", target, "planr"),
  // Repository checkout: local cargo builds.
  path.join(packageRoot, "target", "release", "planr"),
  path.join(packageRoot, "target", "debug", "planr"),
].filter(Boolean);

const binary = candidates.find(candidate => fs.existsSync(candidate));

if (!binary) {
  if (!target) {
    console.error(`Planr has no native binary for ${os.platform()}-${os.arch()}.`);
    console.error("Supported platforms: darwin-arm64, darwin-x86_64, linux-x86_64, linux-arm64.");
  } else {
    console.error("Planr native binary was not found.");
    console.error("Build it with: cargo build --release");
    console.error("Or set PLANR_NATIVE_BIN=/absolute/path/to/planr");
  }
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
