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
const packagedCandidates = [
  process.env.PLANR_NATIVE_BIN,
  // Published package: per-platform binaries bundled at release time.
  target && path.join(here, "..", "native", target, "planr"),
].filter(Boolean);

function repositoryBuildCandidates() {
  if (!fs.existsSync(path.join(packageRoot, "Cargo.toml"))) {
    return [];
  }
  const metadata = spawnSync(
    "cargo",
    ["metadata", "--no-deps", "--format-version", "1"],
    { cwd: packageRoot, encoding: "utf8" },
  );
  if (metadata.status !== 0) {
    return [];
  }
  try {
    const targetDirectory = JSON.parse(metadata.stdout).target_directory;
    if (typeof targetDirectory !== "string" || targetDirectory.length === 0) {
      return [];
    }
    return [
      path.join(targetDirectory, "release", "planr"),
      path.join(targetDirectory, "debug", "planr"),
    ];
  } catch {
    return [];
  }
}

const binary = packagedCandidates.find(candidate => fs.existsSync(candidate))
  ?? repositoryBuildCandidates().find(candidate => fs.existsSync(candidate));

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
