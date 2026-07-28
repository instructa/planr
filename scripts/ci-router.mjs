#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { classifyChanges, parseGitNameStatus } from "./verification-policy.mjs";

export const CI_JOB_GATES = Object.freeze({
  docs: Object.freeze(["docs-content", "docs-typecheck", "docs-lint", "docs-build", "docs-artifact"]),
  quality: Object.freeze(["rust-fmt", "rust-clippy", "rust-test", "generated-reference"]),
  release: Object.freeze(["github-actions", "release-contract", "release-evaluation"]),
  linux_portability: Object.freeze(["linux-portability"]),
});

export function routeSelection(selection) {
  if (!selection || !Array.isArray(selection.selectedGates)) throw new Error("verification selection is incomplete");
  const knownGates = new Set(Object.values(CI_JOB_GATES).flat());
  const unknown = selection.selectedGates.filter((gate) => !knownGates.has(gate));
  if (unknown.length > 0) throw new Error(`verification gates have no CI owner: ${unknown.join(", ")}`);

  const outputs = {
    profile: requiredOutput(selection.profile, "profile"),
    policy_version: requiredOutput(selection.policyVersion, "policy version"),
    policy_digest: requiredOutput(selection.policyDigest, "policy digest"),
    changed_files_digest: requiredOutput(selection.changedFilesDigest, "changed-files digest"),
    live_browser: String(selection.liveVerification?.browser === true),
  };
  for (const [job, gates] of Object.entries(CI_JOB_GATES)) {
    outputs[job] = String(gates.some((gate) => selection.selectedGates.includes(gate)));
  }
  return outputs;
}

export function assertSummary({ selected, results, routerResult = "success" }) {
  if (routerResult !== "success") throw new Error(`router did not succeed: ${routerResult || "missing"}`);
  for (const job of Object.keys(CI_JOB_GATES)) {
    const expected = selected[job];
    const result = results[job] || "missing";
    if (expected === true && result !== "success") throw new Error(`selected CI job ${job} did not succeed: ${result}`);
    if (expected === false && result !== "skipped") throw new Error(`unselected CI job ${job} was not intentionally skipped: ${result}`);
    if (typeof expected !== "boolean") throw new Error(`CI selection is missing for job ${job}`);
  }
  return { verdict: "pass", jobs: Object.keys(CI_JOB_GATES).length };
}

function requiredOutput(value, label) {
  if (typeof value !== "string" || value.length === 0 || /[\r\n]/u.test(value)) throw new Error(`${label} is missing or unsafe`);
  return value;
}

function parsePairs(values, label, transform = (value) => value) {
  const parsed = {};
  for (const entry of values) {
    const separator = entry.indexOf("=");
    if (separator < 1) throw new Error(`invalid ${label}: ${entry}`);
    const key = entry.slice(0, separator);
    if (!(key in CI_JOB_GATES)) throw new Error(`unknown CI job in ${label}: ${key}`);
    parsed[key] = transform(entry.slice(separator + 1));
  }
  return parsed;
}

function takeValues(args, name) {
  const values = [];
  for (let index = 0; index < args.length;) {
    if (args[index] !== name) {
      index += 1;
      continue;
    }
    if (index === args.length - 1) throw new Error(`${name} requires a value`);
    values.push(args[index + 1]);
    args.splice(index, 2);
  }
  return values;
}

function takeValue(args, name) {
  const values = takeValues(args, name);
  if (values.length > 1) throw new Error(`${name} may be specified only once`);
  return values[0];
}

function selectionFromGit(base, head) {
  try {
    const output = execFileSync("git", ["diff", "--name-status", "-z", "--find-renames", base, head], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return classifyChanges(parseGitNameStatus(output), { baseRevision: base, headRevision: head });
  } catch {
    return classifyChanges(undefined, { baseRevision: base, headRevision: head });
  }
}

function writeOutputs(outputPath, outputs) {
  const lines = Object.entries(outputs).map(([key, value]) => `${key}=${value}`).join("\n");
  if (outputPath) appendFileSync(outputPath, `${lines}\n`, { encoding: "utf8" });
  process.stdout.write(`${lines}\n`);
}

function main() {
  const args = process.argv.slice(2);
  const command = args.shift();
  if (command === "route") {
    const base = takeValue(args, "--base");
    const head = takeValue(args, "--head") ?? "HEAD";
    const input = takeValue(args, "--input");
    const output = takeValue(args, "--github-output");
    const selectionPath = takeValue(args, "--selection-output");
    if (args.length > 0 || (!base && !input) || (base && input)) throw new Error("route requires exactly one of --base or --input");
    const selection = input
      ? classifyChanges(JSON.parse(readFileSync(input, "utf8")).changes)
      : selectionFromGit(base, head);
    if (selectionPath) writeFileSync(selectionPath, `${JSON.stringify(selection, null, 2)}\n`, { mode: 0o600 });
    writeOutputs(output, routeSelection(selection));
    return;
  }
  if (command === "summary") {
    const selected = parsePairs(takeValues(args, "--selected"), "selection", (value) => {
      if (value !== "true" && value !== "false") throw new Error(`invalid selection boolean: ${value || "missing"}`);
      return value === "true";
    });
    const results = parsePairs(takeValues(args, "--result"), "result");
    const routerResult = takeValue(args, "--router-result");
    if (args.length > 0) throw new Error(`unknown arguments: ${args.join(" ")}`);
    process.stdout.write(`${JSON.stringify(assertSummary({ selected, results, routerResult }))}\n`);
    return;
  }
  throw new Error("usage: ci-router.mjs <route|summary> ...");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}
