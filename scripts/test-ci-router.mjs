import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { assertSummary, routeSelection } from "./ci-router.mjs";
import { classifyChanges, POLICY_DIGEST, POLICY_VERSION } from "./verification-policy.mjs";

const docsSelection = classifyChanges([{ status: "M", path: "apps/docs/content/docs/agents/quickstart.mdx" }]);
assert.deepEqual(routeSelection(docsSelection), {
  profile: "focused-docs",
  policy_version: POLICY_VERSION,
  policy_digest: POLICY_DIGEST,
  changed_files_digest: docsSelection.changedFilesDigest,
  docs: "true",
  quality: "false",
  release: "false",
  linux_portability: "false",
});

const releaseRoute = routeSelection(classifyChanges([{ status: "M", path: ".github/workflows/ci.yml" }]));
assert.equal(releaseRoute.docs, "false");
assert.equal(releaseRoute.quality, "false");
assert.equal(releaseRoute.release, "true");
assert.equal(releaseRoute.linux_portability, "true");
const fullRoute = routeSelection(classifyChanges([{ status: "M", path: "scripts/ci-router.mjs" }]));
for (const job of ["docs", "quality", "release", "linux_portability"]) assert.equal(fullRoute[job], "true");
assert.throws(
  () => routeSelection({ ...docsSelection, selectedGates: [...docsSelection.selectedGates, "ownerless-gate"] }),
  /no CI owner/u,
);

const selected = { docs: true, quality: false, release: false, linux_portability: false };
const passingResults = { docs: "success", quality: "skipped", release: "skipped", linux_portability: "skipped" };
assert.deepEqual(assertSummary({ selected, results: passingResults }), { verdict: "pass", jobs: 4 });
for (const result of ["missing", "skipped", "cancelled", "failure"]) {
  assert.throws(
    () => assertSummary({ selected, results: { ...passingResults, docs: result } }),
    new RegExp(`selected CI job docs did not succeed: ${result}`, "u"),
  );
}
assert.throws(
  () => assertSummary({ selected, results: { ...passingResults, quality: "success" } }),
  /was not intentionally skipped/u,
);
assert.throws(() => assertSummary({ selected, results: passingResults, routerResult: "failure" }), /router did not succeed/u);

const temp = mkdtempSync(path.join(os.tmpdir(), "planr-ci-router-"));
try {
  const fixture = path.join(temp, "changes.json");
  const output = path.join(temp, "github-output.txt");
  const selection = path.join(temp, "selection.json");
  writeFileSync(fixture, JSON.stringify({ changes: [{ status: "M", path: "README.md" }] }));
  const result = spawnSync(process.execPath, [
    new URL("./ci-router.mjs", import.meta.url).pathname,
    "route", "--input", fixture, "--github-output", output, "--selection-output", selection,
  ], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(readFileSync(output, "utf8"), /^docs=true$/mu);
  assert.match(readFileSync(output, "utf8"), /^quality=false$/mu);
  assert.equal(JSON.parse(readFileSync(selection, "utf8")).profile, "focused-docs");
} finally {
  rmSync(temp, { recursive: true, force: true });
}

console.log("ci_router=passed docs_only_skips=quality,release,linux-portability summary_fail_closed=missing,skipped,cancelled,failure");
