#!/usr/bin/env node
import { copyFile, mkdir, readdir, rm, stat } from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

export const PUBLIC_SITE_FILES = Object.freeze([
  "_headers",
  "app.mjs",
  "catalog-model.mjs",
  "data/catalog.json",
  "index.html",
  "styles.css",
]);

async function regularFile(path) {
  try {
    return (await stat(path)).isFile();
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

export async function publicationFiles(root) {
  const files = [];

  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) await visit(path);
      else if (entry.isFile()) files.push(relative(root, path).split(sep).join("/"));
      else throw new Error(`publish output contains unsupported entry: ${relative(root, path)}`);
    }
  }

  await visit(root);
  return files.sort();
}

export async function verifyPublication(root) {
  const actual = await publicationFiles(root);
  const expected = [...PUBLIC_SITE_FILES].sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `publish output mismatch\nexpected: ${expected.join(", ")}\nactual: ${actual.join(", ")}`,
    );
  }
  return actual;
}

export async function buildSite({ sourceRoot, outputRoot }) {
  const source = resolve(sourceRoot);
  const output = resolve(outputRoot);
  if (output === source || output.startsWith(`${source}${sep}`)) {
    throw new Error("publish output must not be inside the website source directory");
  }

  for (const relativePath of PUBLIC_SITE_FILES) {
    const input = join(source, relativePath);
    if (!(await regularFile(input))) {
      throw new Error(`missing public website artifact: ${relativePath}`);
    }
  }

  await rm(output, { recursive: true, force: true });
  for (const relativePath of PUBLIC_SITE_FILES) {
    const destination = join(output, relativePath);
    await mkdir(dirname(destination), { recursive: true });
    await copyFile(join(source, relativePath), destination);
  }

  return verifyPublication(output);
}

const modulePath = fileURLToPath(import.meta.url);
if (process.argv[1] && resolve(process.argv[1]) === modulePath) {
  const repositoryRoot = resolve(dirname(modulePath), "..");
  buildSite({
    sourceRoot: join(repositoryRoot, "website"),
    outputRoot: join(repositoryRoot, "dist", "website"),
  })
    .then((files) => console.log(`built ${files.length} public website artifacts in dist/website`))
    .catch((error) => {
      console.error(error instanceof Error ? error.message : error);
      process.exitCode = 1;
    });
}
