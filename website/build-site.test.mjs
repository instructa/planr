import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  buildSite,
  PUBLIC_SITE_FILES,
  verifyPublication,
} from "../scripts/build-site.mjs";

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "planr-site-build-"));
  const sourceRoot = join(root, "website");
  const outputRoot = join(root, "dist", "website");
  for (const relativePath of PUBLIC_SITE_FILES) {
    const path = join(sourceRoot, relativePath);
    await mkdir(join(path, ".."), { recursive: true });
    await writeFile(path, `public:${relativePath}\n`);
  }
  return { root, sourceRoot, outputRoot };
}

test("builds only the public runtime allowlist and preserves nested catalog bytes", async (t) => {
  const paths = await fixture();
  t.after(() => rm(paths.root, { recursive: true, force: true }));
  await writeFile(join(paths.sourceRoot, "build-catalog.mjs"), "must not be public\n");
  await mkdir(join(paths.sourceRoot, "registry"), { recursive: true });
  await writeFile(join(paths.sourceRoot, "registry", "trusted-maintainers.toml"), "not public\n");

  const files = await buildSite(paths);

  assert.deepEqual(files, [...PUBLIC_SITE_FILES].sort());
  assert.equal(
    await readFile(join(paths.outputRoot, "data", "catalog.json"), "utf8"),
    "public:data/catalog.json\n",
  );
});

test("fails before replacing output when a required source artifact is missing", async (t) => {
  const paths = await fixture();
  t.after(() => rm(paths.root, { recursive: true, force: true }));
  await mkdir(paths.outputRoot, { recursive: true });
  await writeFile(join(paths.outputRoot, "sentinel"), "keep on validation failure\n");
  await rm(join(paths.sourceRoot, "styles.css"));

  await assert.rejects(() => buildSite(paths), /missing public website artifact: styles\.css/);
  assert.equal(await readFile(join(paths.outputRoot, "sentinel"), "utf8"), "keep on validation failure\n");
});

test("publication verification rejects unexpected nested output", async (t) => {
  const paths = await fixture();
  t.after(() => rm(paths.root, { recursive: true, force: true }));
  await buildSite(paths);
  await mkdir(join(paths.outputRoot, "registry"), { recursive: true });
  await writeFile(join(paths.outputRoot, "registry", "manifest.toml"), "unexpected\n");

  await assert.rejects(() => verifyPublication(paths.outputRoot), /publish output mismatch/);
});

test("refuses an output directory inside the source tree", async (t) => {
  const paths = await fixture();
  t.after(() => rm(paths.root, { recursive: true, force: true }));

  await assert.rejects(
    () => buildSite({ sourceRoot: paths.sourceRoot, outputRoot: join(paths.sourceRoot, "dist") }),
    /must not be inside/,
  );
});
