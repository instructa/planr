#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

const [caseId = "case", counterPath = ".planr/eval-lifecycle-counts.json", mode = "baseline"] =
  process.argv.slice(2);

let effectiveCounterPath = counterPath;
let effectiveMode = mode;
try {
  const state = JSON.parse(fs.readFileSync(".planr/eval-subject-state.json", "utf8"));
  if (typeof state.counter_path === "string" && state.counter_path.length > 0) {
    effectiveCounterPath = state.counter_path;
  }
  if (typeof state.mode === "string" && state.mode.length > 0) {
    effectiveMode = state.mode;
  }
} catch {
  // The checked suite carries deterministic defaults for direct local execution.
}

const delays = {
  // Keep the measured work comfortably above process-startup jitter so a
  // replay of the same treatment does not cross the 10% regression gate.
  baseline: 300,
  better: 5,
  same: 300,
  worse: 1000,
};
const delayMs = delays[effectiveMode] ?? delays.baseline;
const started = Date.now();
while (Date.now() - started < delayMs) {
  // Busy wait intentionally: the eval runner measures bounded process duration.
}

fs.mkdirSync(path.dirname(effectiveCounterPath), { recursive: true });
let counts = {};
try {
  counts = JSON.parse(fs.readFileSync(effectiveCounterPath, "utf8"));
} catch {
  counts = {};
}
counts[caseId] = (counts[caseId] ?? 0) + 1;
fs.writeFileSync(effectiveCounterPath, JSON.stringify(counts, null, 2));

console.log(JSON.stringify({ event: "project_created", case_id: caseId, mode: effectiveMode }));
console.log(JSON.stringify({ event: "map_built", case_id: caseId, mode: effectiveMode }));
console.log(JSON.stringify({ event: "item_closed", case_id: caseId, mode: effectiveMode }));
