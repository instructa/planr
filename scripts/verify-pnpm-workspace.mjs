import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  unlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const expectedWorkspaces = [".", "apps/docs"];

function runPnpm(cwd, args) {
  const result = spawnSync("pnpm", args, {
    cwd,
    encoding: "utf8",
    env: { ...process.env, CI: "true" },
  });

  if (result.status !== 0) {
    throw new Error(
      [
        `pnpm ${args.join(" ")} failed in ${cwd}`,
        result.stdout.trim(),
        result.stderr.trim(),
      ]
        .filter(Boolean)
        .join("\n"),
    );
  }

  return result.stdout.trim();
}

async function createFixture(parent, name, { dogfood }) {
  const fixture = path.join(parent, name);
  await mkdir(path.join(fixture, "apps", "docs"), { recursive: true });

  await Promise.all([
    copyFile(path.join(repoRoot, "package.json"), path.join(fixture, "package.json")),
    copyFile(
      path.join(repoRoot, "pnpm-workspace.yaml"),
      path.join(fixture, "pnpm-workspace.yaml"),
    ),
    copyFile(path.join(repoRoot, "pnpm-lock.yaml"), path.join(fixture, "pnpm-lock.yaml")),
    copyFile(
      path.join(repoRoot, "apps", "docs", "package.json"),
      path.join(fixture, "apps", "docs", "package.json"),
    ),
  ]);

  if (dogfood) {
    await mkdir(path.join(fixture, "apps", "web"), { recursive: true });
    await writeFile(
      path.join(fixture, "apps", "web", "package.json"),
      `${JSON.stringify(
        {
          name: "ignored-planr-dogfood",
          private: true,
          version: "0.0.0",
          dependencies: { "dogfood-only-dependency": "99.99.99" },
        },
        null,
        2,
      )}\n`,
    );
  }

  return fixture;
}

function workspaceInventory(fixture) {
  const listed = JSON.parse(
    runPnpm(fixture, ["--recursive", "list", "--depth", "-1", "--json"]),
  );

  return listed
    .map((workspace) => path.relative(fixture, workspace.path) || ".")
    .sort();
}

const rootPackage = JSON.parse(await readFile(path.join(repoRoot, "package.json"), "utf8"));
const pinnedMatch = /^pnpm@([^+]+)/u.exec(rootPackage.packageManager ?? "");
assert.ok(pinnedMatch, "package.json must pin pnpm in packageManager");

const actualPnpmVersion = runPnpm(repoRoot, ["--version"]);
assert.equal(
  actualPnpmVersion,
  pinnedMatch[1],
  `expected pinned pnpm ${pinnedMatch[1]}, received ${actualPnpmVersion}`,
);

const fixtureRoot = await mkdtemp(path.join(os.tmpdir(), "planr-pnpm-workspace-"));

try {
  const clean = await createFixture(fixtureRoot, "clean", { dogfood: false });
  const dogfood = await createFixture(fixtureRoot, "dogfood", { dogfood: true });

  assert.deepEqual(workspaceInventory(clean), expectedWorkspaces);
  assert.deepEqual(workspaceInventory(dogfood), expectedWorkspaces);

  for (const fixture of [clean, dogfood]) {
    const before = await readFile(path.join(fixture, "pnpm-lock.yaml"));
    runPnpm(fixture, ["install", "--lockfile-only", "--frozen-lockfile", "--ignore-scripts"]);
    const after = await readFile(path.join(fixture, "pnpm-lock.yaml"));
    assert.deepEqual(after, before, `frozen install changed ${fixture}/pnpm-lock.yaml`);
  }

  await Promise.all([
    unlink(path.join(clean, "pnpm-lock.yaml")),
    unlink(path.join(dogfood, "pnpm-lock.yaml")),
  ]);

  runPnpm(clean, ["install", "--lockfile-only", "--ignore-scripts"]);
  runPnpm(dogfood, ["install", "--lockfile-only", "--ignore-scripts"]);

  const cleanLockfile = await readFile(path.join(clean, "pnpm-lock.yaml"));
  const dogfoodLockfile = await readFile(path.join(dogfood, "pnpm-lock.yaml"));
  assert.deepEqual(
    dogfoodLockfile,
    cleanLockfile,
    "ignored apps/web changed regenerated pnpm-lock.yaml",
  );

  console.log(
    `pnpm_workspace_check=passed version=${actualPnpmVersion} workspaces=${expectedWorkspaces.join(",")} frozen=passed regeneration=identical`,
  );
} finally {
  await rm(fixtureRoot, { recursive: true, force: true });
}
