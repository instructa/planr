#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const output = process.argv[2];
if (!output) throw new Error("usage: write-ci-promotion-receipt.mjs <output.json>");

let event;
try {
  event = JSON.parse(fs.readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
} catch {
  throw new Error("GITHUB_EVENT_PATH is missing or invalid");
}

const required = (name, pattern = /^\S+$/u) => {
  const value = process.env[name];
  if (!value || !pattern.test(value)) throw new Error(`${name} is missing or invalid`);
  return value;
};

const result = (name) => {
  const value = required(name, /^(?:success|skipped)$/u);
  return value;
};

const receipt = {
  schema_version: "planr.ci-promotion-receipt.v1",
  repository: required("GITHUB_REPOSITORY", /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u),
  workflow: required("GITHUB_WORKFLOW"),
  run_id: required("GITHUB_RUN_ID", /^[1-9][0-9]*$/u),
  run_attempt: required("GITHUB_RUN_ATTEMPT", /^[1-9][0-9]*$/u),
  event: required("GITHUB_EVENT_NAME"),
  source_ref: required("GITHUB_REF"),
  source_base_sha: requiredBaseSha(event),
  source_sha: required("GITHUB_SHA", /^[0-9a-f]{40}$/u),
  conclusion: "success",
  policy: {
    profile: required("PLANR_PROFILE"),
    version: required("PLANR_POLICY_VERSION"),
    digest: required("PLANR_POLICY_DIGEST", /^sha256:[0-9a-f]{64}$/u),
    changed_files_digest: required("PLANR_CHANGED_FILES_DIGEST", /^sha256:[0-9a-f]{64}$/u),
  },
  jobs: {
    docs: result("PLANR_DOCS_RESULT"),
    quality: result("PLANR_QUALITY_RESULT"),
    release: result("PLANR_RELEASE_RESULT"),
    linux_portability: result("PLANR_LINUX_RESULT"),
  },
};

function requiredBaseSha(payload) {
  const value = process.env.GITHUB_EVENT_NAME === "pull_request"
    ? payload?.pull_request?.base?.sha
    : payload?.before;
  if (typeof value !== "string" || !/^[0-9a-f]{40}$/u.test(value)) {
    throw new Error("GitHub event is missing a valid CI base SHA");
  }
  return value;
}

fs.mkdirSync(path.dirname(path.resolve(output)), { recursive: true });
fs.writeFileSync(output, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
console.log(JSON.stringify({ verdict: "pass", source_sha: receipt.source_sha, run_id: receipt.run_id }));
