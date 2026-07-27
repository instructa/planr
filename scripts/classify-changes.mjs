#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { classifyChanges, parseGitNameStatus } from "./verification-policy.mjs";

const args = process.argv.slice(2);
const json = takeFlag("--json");
const inputPath = takeValue("--input");
const base = takeValue("--base");
const head = takeValue("--head") ?? "HEAD";

let changes;
let selectionContext = {};
try {
  if (inputPath) {
    const input = JSON.parse(readFileSync(inputPath, "utf8"));
    changes = Array.isArray(input) ? input : input.changes;
    if (!Array.isArray(input)) {
      selectionContext = { baseRevision: input.baseRevision, headRevision: input.headRevision };
    }
  } else if (base) {
    const output = execFileSync("git", ["diff", "--name-status", "-z", "--find-renames", base, head], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    changes = parseGitNameStatus(output);
    selectionContext = { baseRevision: base, headRevision: head };
  } else {
    const input = JSON.parse(readFileSync(0, "utf8"));
    changes = Array.isArray(input) ? input : input.changes;
    if (!Array.isArray(input)) {
      selectionContext = { baseRevision: input.baseRevision, headRevision: input.headRevision };
    }
  }
} catch {
  changes = undefined;
}

if (args.length > 0) {
  process.stderr.write(`Unknown arguments: ${args.join(" ")}\n`);
  process.exit(2);
}

const selection = classifyChanges(changes, selectionContext);
if (json) {
  process.stdout.write(`${JSON.stringify(selection, null, 2)}\n`);
} else {
  process.stdout.write(`profile=${selection.profile} policy=${selection.policyVersion} changed=${selection.changes.length}\n`);
  for (const reason of selection.escalationReasons) {
    process.stdout.write(`escalation=${reason.code}${reason.path ? ` path=${reason.path}` : ""} ${reason.detail}\n`);
  }
  for (const gate of selection.selectedGates) process.stdout.write(`gate=${gate}\n`);
}

function takeFlag(name) {
  const index = args.indexOf(name);
  if (index === -1) return false;
  args.splice(index, 1);
  return true;
}

function takeValue(name) {
  const index = args.indexOf(name);
  if (index === -1) return undefined;
  if (index === args.length - 1) {
    process.stderr.write(`${name} requires a value\n`);
    process.exit(2);
  }
  const [, value] = args.splice(index, 2);
  return value;
}
