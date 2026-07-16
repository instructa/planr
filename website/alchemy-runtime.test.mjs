import assert from "node:assert/strict";
import test from "node:test";
import {
  assertAlchemyRuntime,
  MINIMUM_ALCHEMY_NODE_MAJOR,
} from "../scripts/check-alchemy-runtime.mjs";

test("accepts the minimum supported Alchemy deployment runtime", () => {
  assert.equal(assertAlchemyRuntime("22.0.0"), MINIMUM_ALCHEMY_NODE_MAJOR);
});

test("rejects Node runtimes supported by the Planr CLI but not by Alchemy", () => {
  assert.throws(
    () => assertAlchemyRuntime("18.16.1"),
    /Cloudflare deployment requires Node\.js 22 or newer; current runtime is 18\.16\.1/,
  );
  assert.throws(
    () => assertAlchemyRuntime("20.19.5"),
    /Cloudflare deployment requires Node\.js 22 or newer/,
  );
});
