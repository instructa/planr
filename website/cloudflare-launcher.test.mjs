import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { cloudflareTestSteps } from "../scripts/cloudflare-test.mjs";

test("launcher keeps deploy and destroy pinned to the test stage", () => {
  assert.deepEqual(cloudflareTestSteps("deploy"), [
    ["pnpm", ["site:check"]],
    ["pnpm", ["exec", "alchemy", "deploy", "--stage", "test"]],
  ]);
  assert.deepEqual(cloudflareTestSteps("destroy"), [
    ["pnpm", ["exec", "alchemy", "destroy", "--stage", "test"]],
  ]);
  assert.throws(() => cloudflareTestSteps("prod"), /usage:/);
});

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const launcher = join(repositoryRoot, "scripts", "cloudflare-test.mjs");
for (const version of ["v18.16.1", "v20.19.5"]) {
  const node = join(homedir(), ".nvm", "versions", "node", version, "bin", "node");
  test(
    `direct launcher rejects ${version} before invoking pnpm`,
    { skip: !existsSync(node) && `local ${version} runtime is unavailable` },
    () => {
      const result = spawnSync(node, [launcher, "deploy"], {
        cwd: repositoryRoot,
        encoding: "utf8",
      });
      assert.equal(result.status, 1);
      assert.match(result.stderr, /Cloudflare deployment requires Node\.js 22 or newer/);
      assert.doesNotMatch(result.stderr, /Corepack|ERR_VM|addAbortListener|experimental-strip-types/);
    },
  );
}
