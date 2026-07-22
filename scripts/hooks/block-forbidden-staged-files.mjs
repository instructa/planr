#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const projectRoot = join(scriptDirectory, "..", "..");

function loadForbiddenPatterns() {
  const patternsFile = join(projectRoot, ".forbidden-paths.regex");
  if (!existsSync(patternsFile)) {
    console.log("No .forbidden-paths.regex found; skipping staged-file check.");
    return [];
  }

  return readFileSync(patternsFile, "utf8")
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"))
    .map((pattern) => new RegExp(pattern));
}

function getStagedFiles() {
  try {
    const output = execFileSync(
      "git",
      ["diff", "--cached", "--name-only", "--diff-filter=ACMR", "-z"],
      { cwd: projectRoot, encoding: "utf8" },
    );
    return output.split("\0").filter(Boolean);
  } catch (error) {
    console.error(`Unable to inspect staged files: ${error.message}`);
    process.exit(2);
  }
}

const patterns = loadForbiddenPatterns();
const forbidden = getStagedFiles().flatMap((file) => {
  const pattern = patterns.find((candidate) => candidate.test(file));
  return pattern ? [{ file, pattern: pattern.source }] : [];
});

if (forbidden.length > 0) {
  console.error("Forbidden files detected in the staging area:");
  for (const { file, pattern } of forbidden) {
    console.error(`- ${file} (${pattern})`);
  }
  console.error("Unstage these files and keep them local.");
  process.exit(1);
}

console.log("No forbidden files in the staging area.");
