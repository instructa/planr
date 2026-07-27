#!/usr/bin/env node
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (name) => readFileSync(path.join(root, "plugins/planr/skills", name, "SKILL.md"), "utf8");

const goal = read("planr-goal");
const loop = read("planr-loop");
const work = read("planr-work");
const review = read("planr-review");
const web = read("planr-verify-web");

assert.match(goal, /small coherent change is one implementation item plus one signal-bearing independent review/u);
assert.match(goal, /versioned verification policy and source-bound receipt runner/u);
assert.match(loop, /cheap, missing, failing, or explicitly high-risk evidence/u);
assert.match(loop, /maker never self-reviews when an independent checker is available/u);
assert.match(loop, /Keep one active write item/u);
assert.match(work, /npm run verification:run -- --receipt/u);
assert.match(work, /receipt path, digest, source revision, selected profile\/gates/u);
assert.match(review, /npm run verification:verify -- --receipt/u);
assert.match(review, /Receipt validation does not replace judgment/u);
assert.match(review, /Never export a second identity/u);
assert.match(web, /approved deployment decision before the deploy begins/u);
assert.match(web, /does not automatically trigger another full build or reviewer replay/u);

for (const [name, contents] of [["loop", loop], ["review", review], ["web", web]]) {
  assert.doesNotMatch(contents, /reviewer reruns (?:it|the logged verification evidence)/iu, `${name} must not require unconditional replay`);
}

process.stdout.write("planr risk-based guidance contract: ok (5 skills)\n");
