#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { projectComposition, safeIdentifier } from "./catalog-model.mjs";

function usage(message) {
  if (message) console.error(message);
  console.error(
    "usage: node website/build-catalog.mjs --manifest <registry.toml> --content-root <dir> --trust-store <trusted.toml> --entry <id=host> [--entry <id=host> ...] --at-unix <unix> --output <catalog.json> [--planr-bin <planr>]",
  );
  process.exit(2);
}

function parseArgs(argv) {
  const values = { entries: [], planrBin: "planr" };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!value) usage(`missing value for ${flag}`);
    index += 1;
    if (flag === "--entry") values.entries.push(value);
    else if (flag === "--manifest") values.manifest = value;
    else if (flag === "--content-root") values.contentRoot = value;
    else if (flag === "--trust-store") values.trustStore = value;
    else if (flag === "--at-unix") values.atUnix = Number(value);
    else if (flag === "--output") values.output = value;
    else if (flag === "--planr-bin") values.planrBin = value;
    else usage(`unknown argument ${flag}`);
  }
  if (
    !values.manifest ||
    !values.contentRoot ||
    !values.trustStore ||
    !values.output ||
    !Number.isSafeInteger(values.atUnix) ||
    values.entries.length === 0
  ) {
    usage("all required arguments must be provided");
  }
  return values;
}

function runJson(binary, args, cwd) {
  const result = spawnSync(binary, args, { cwd, encoding: "utf8" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(result.stderr.trim() || result.stdout.trim() || `${binary} exited ${result.status}`);
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`${binary} returned invalid JSON: ${error.message}`);
  }
}

function entrySpec(raw) {
  const separator = raw.lastIndexOf("=");
  if (separator < 1 || separator === raw.length - 1) usage(`invalid --entry ${raw}; expected id=host`);
  return { id: raw.slice(0, separator), host: raw.slice(separator + 1) };
}

function artifactPath(verified, kind, contentRoot) {
  const matches = verified.entry.artifacts.filter((artifact) => artifact.kind === kind);
  if (matches.length !== 1) throw new Error(`entry ${verified.entry.id} must contain one ${kind}`);
  return resolve(contentRoot, matches[0].path);
}

const options = parseArgs(process.argv.slice(2));
const invocationRoot = process.cwd();
const planrBin =
  isAbsolute(options.planrBin) || !options.planrBin.includes("/")
    ? options.planrBin
    : resolve(invocationRoot, options.planrBin);
const manifest = resolve(invocationRoot, options.manifest);
const contentRoot = resolve(invocationRoot, options.contentRoot);
const trustStore = resolve(invocationRoot, options.trustStore);
const scratch = mkdtempSync(join(tmpdir(), "planr-preset-site-"));

try {
  const compositions = options.entries.map((raw) => {
    const { id, host } = entrySpec(raw);
    const verified = runJson(
      planrBin,
      [
        "--json",
        "agents",
        "preset",
        "registry",
        "verify",
        manifest,
        "--entry",
        id,
        "--content-root",
        contentRoot,
        "--trust-store",
        trustStore,
        "--at-unix",
        String(options.atUnix),
        "--host",
        host,
      ],
      scratch,
    );
    const verificationPath = artifactPath(verified, "verification", contentRoot);
    const policyId = safeIdentifier(verified.entry.evaluation?.policy_id, "policy id");
    const bindingId = safeIdentifier(verified.entry.evaluation?.binding_id, "binding id");
    const preview = verified.catalog_preview;
    if (!preview) throw new Error(`entry ${id} verification did not return a catalog preview`);
    const verificationEnvelope = JSON.parse(readFileSync(verificationPath, "utf8"));
    return projectComposition({ verified, preview, verificationEnvelope });
  });
  const catalog = {
    schemaVersion: 1,
    generatedAtUnix: options.atUnix,
    source: {
      state: "verified_registry_projection",
      manifest: options.manifest,
      entryCount: compositions.length,
      trust: "planr_registry_v1",
    },
    compositions,
  };
  const output = resolve(invocationRoot, options.output);
  await mkdir(dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify(catalog, null, 2)}\n`, { mode: 0o644 });
  console.log(`wrote ${compositions.length} verified composition(s) to ${output}`);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
