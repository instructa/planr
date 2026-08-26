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
  if (!osName || !arch) return null;
  return `${osName}-${arch}`;
}

function repositoryBuildCandidates(binaryName) {
  if (!fs.existsSync(path.join(packageRoot, "Cargo.toml"))) return [];
  const metadata = spawnSync(
    "cargo",
    ["metadata", "--no-deps", "--format-version", "1"],
    { cwd: packageRoot, encoding: "utf8" },
  );
  if (metadata.status !== 0) return [];
  try {
    const targetDirectory = JSON.parse(metadata.stdout).target_directory;
    if (typeof targetDirectory !== "string" || targetDirectory.length === 0) return [];
    return [
      path.join(targetDirectory, "release", binaryName),
      path.join(targetDirectory, "debug", binaryName),
    ];
  } catch {
    return [];
  }
}

export function runNative({ binaryName, overrideEnvironment, productName }) {
  const target = platformTarget();
  const packagedCandidates = [
    process.env[overrideEnvironment],
    target && path.join(here, "..", "native", target, binaryName),
  ].filter(Boolean);
  const binary = packagedCandidates.find(candidate => fs.existsSync(candidate))
    ?? repositoryBuildCandidates(binaryName).find(candidate => fs.existsSync(candidate));

  if (!binary) {
    if (!target) {
      console.error(`${productName} has no native binary for ${os.platform()}-${os.arch()}.`);
      console.error("Supported platforms: darwin-arm64, darwin-x86_64, linux-x86_64, linux-arm64.");
    } else {
      console.error(`${productName} native binary was not found.`);
      console.error(`Build it with: cargo build --release --bin ${binaryName}`);
      console.error(`Or set ${overrideEnvironment}=/absolute/path/to/${binaryName}`);
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
}
