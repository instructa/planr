#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function option(name, fallback) {
  const index = process.argv.indexOf(name);
  if (index === -1) return fallback;
  const value = process.argv[index + 1];
  if (!value) throw new Error(`missing value for ${name}`);
  return value;
}

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(result.stderr.trim() || result.stdout.trim() || `${command} exited ${result.status}`);
  }
  return result.stdout;
}

function digest(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function artifact(path, kind) {
  const absolute = resolve(repositoryRoot, path);
  return `[[entries.artifacts]]
path = "${path}"
kind = "${kind}"
sha256 = "${digest(absolute)}"
size_bytes = ${statSync(absolute).size}`;
}

const atUnix = Number(option("--at-unix", "1784160000"));
if (!Number.isSafeInteger(atUnix)) throw new Error("--at-unix must be an integer");
const configuredPlanr = option("--planr-bin", "target/release/planr");
const planrBin = isAbsolute(configuredPlanr)
  ? configuredPlanr
  : resolve(repositoryRoot, configuredPlanr);
const scratch = resolve(repositoryRoot, `.planr/catalog-generation-${process.pid}`);
const reportDir = relative(repositoryRoot, scratch);
const registryRoot = resolve(repositoryRoot, "website/registry");
const verificationPath = join(registryRoot, "verification.json");
const reportPath = join(registryRoot, "report.md");

rmSync(scratch, { recursive: true, force: true });
mkdirSync(registryRoot, { recursive: true });

try {
  run(planrBin, [
    "--json",
    "agents",
    "preset",
    "evaluate",
    "--at-unix",
    String(atUnix),
    "--host",
    "codex",
    "--report-dir",
    reportDir,
  ]);
  copyFileSync(join(scratch, "verification.json"), verificationPath);
  copyFileSync(join(scratch, "report.md"), reportPath);
  writeFileSync(reportPath, readFileSync(reportPath, "utf8").replace(/  \n/g, "\n"));

  const manifest = `schema_version = 1
id = "planr-official"
version = "2026.07-native-v2"
generated_at_unix = ${atUnix}

[[entries]]
id = "balanced-codex-native-v2"
version = "2.0.0"
kind = "pack"
lifecycle = "published"
verification_status = "experimental"
verified_at_unix = ${atUnix}
review_at_unix = 1815523200
compatible_hosts = ["codex"]
min_planr_version = "1.4.0"
max_planr_version = "1.9.0"
verification_path = "website/registry/verification.json"

[entries.evaluation]
policy_id = "balanced"
policy_version = "1.0.0"
binding_id = "codex-openai"
binding_version = "2.0.0"
suite_id = "planr-preset-suite"
suite_version = "1.8.0"

${artifact("presets/policies/balanced.toml", "policy")}

${artifact("presets/bindings/codex-openai.toml", "host-binding")}

${artifact("website/registry/verification.json", "verification")}
`;
  writeFileSync(join(registryRoot, "manifest.toml"), manifest);

  run(process.execPath, [
    "website/build-catalog.mjs",
    "--planr-bin",
    planrBin,
    "--manifest",
    "website/registry/manifest.toml",
    "--content-root",
    ".",
    "--trust-store",
    "website/registry/trusted-maintainers.toml",
    "--entry",
    "balanced-codex-native-v2=codex",
    "--at-unix",
    String(atUnix),
    "--output",
    "website/data/catalog.json",
  ]);
  console.log("regenerated demoted native Codex evaluation, registry manifest, report, and website catalog");
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
