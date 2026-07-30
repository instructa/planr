import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const vectors = [
  {
    path: "docs/contracts/fixtures/evidence/v1/examples/evidence-receipt.json",
    digestField: "receipt_digest",
  },
  {
    path: "docs/contracts/fixtures/evidence/v1/examples/evidence-policy.json",
    digestField: "policy_digest",
  },
];

for (const vector of vectors) {
  const value = JSON.parse(readFileSync(vector.path, "utf8"));
  const actual = value[vector.digestField];
  assert.match(actual, /^sha256:[a-f0-9]{64}$/u, `${vector.path} has no ${vector.digestField}`);
  const preimage = { ...value };
  delete preimage[vector.digestField];
  const expected = sha256(canonicalJson(preimage));
  assert.equal(actual, expected, `${vector.path} ${vector.digestField} mismatch`);
}

console.log(`verified ${vectors.length} evidence contract digest vectors`);

function sha256(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}
