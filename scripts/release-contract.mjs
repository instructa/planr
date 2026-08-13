#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const NUMERIC_IDENTIFIER = "(?:0|[1-9][0-9]*)";
const RELEASE_VERSION = new RegExp(
  `^(${NUMERIC_IDENTIFIER})\\.(${NUMERIC_IDENTIFIER})\\.(${NUMERIC_IDENTIFIER})(?:-(alpha|beta|rc)\\.(${NUMERIC_IDENTIFIER}))?$`,
  "u",
);

export function parseReleaseVersion(version) {
  const match = RELEASE_VERSION.exec(version);
  if (!match) {
    throw new Error("release version must be canonical x.y.z[-alpha.N|-beta.N|-rc.N] SemVer");
  }
  return Object.freeze({
    version,
    major: match[1],
    minor: match[2],
    patch: match[3],
    channel: match[4] ?? "stable",
    prereleaseNumber: match[5] ?? null,
  });
}

export function changelogPredecessor(source, version) {
  parseReleaseVersion(version);
  const headings = [...source.matchAll(/^## \[([^\]]+)\](?:\s|$)/gmu)].map((match) => match[1]);
  const index = headings.indexOf(version);
  if (index < 0) throw new Error(`CHANGELOG.md has no '## [${version}]' release section`);
  const predecessor = headings[index + 1];
  if (!predecessor || predecessor === "Unreleased") {
    throw new Error(`CHANGELOG.md has no release section before ${version}`);
  }
  parseReleaseVersion(predecessor);
  return predecessor;
}

function runGit(args) {
  const result = spawnSync("git", args, { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`git ${args.join(" ")} failed: ${result.stderr.trim()}`);
  }
  return result.stdout.trim();
}

function verifyPredecessor(version) {
  const source = fs.readFileSync(path.resolve("CHANGELOG.md"), "utf8");
  const predecessor = changelogPredecessor(source, version);
  const ref = `refs/tags/v${predecessor}`;
  const localObject = runGit(["rev-parse", "--verify", ref]);
  if (!/^[0-9a-f]{40}$/u.test(localObject)) throw new Error(`local predecessor ${ref} is not an exact tag object`);

  const remoteLine = runGit(["ls-remote", "--exit-code", "--refs", "origin", ref]);
  const match = /^([0-9a-f]{40})\t(\S+)$/u.exec(remoteLine);
  if (!match || match[2] !== ref) throw new Error(`origin has no exact predecessor tag ${ref}`);
  if (match[1] !== localObject) throw new Error(`local predecessor ${ref} does not match origin`);
  process.stdout.write(`${JSON.stringify({ version, predecessor, ref, tag_object: localObject })}\n`);
}

if (path.resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  try {
    const command = process.argv[2];
    const version = process.argv[3];
    if (command === "validate-version") {
      parseReleaseVersion(version);
    } else if (command === "predecessor") {
      process.stdout.write(`${changelogPredecessor(fs.readFileSync(path.resolve("CHANGELOG.md"), "utf8"), version)}\n`);
    } else if (command === "verify-predecessor") {
      verifyPredecessor(version);
    } else {
      throw new Error("usage: release-contract.mjs <validate-version|predecessor|verify-predecessor> <version>");
    }
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
