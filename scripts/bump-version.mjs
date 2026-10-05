import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { root, read, semver, versions, pluginManifests, marketplaces } from './versions.mjs';

try {
  if (process.argv.length !== 3) {
    throw Error('usage: node scripts/bump-version.mjs <version>');
  }
  const value = semver(process.argv[2]);
  versions();
  const changes = [];
  const manifests = [
    'package.json', 'bb-plugin/package.json', 'bb-plugin/package-lock.json', ...pluginManifests,
  ];
  for (const file of manifests) {
    let content = read(file);
    if (!/^  "version": "[^"]+"/m.test(content)) {
      throw Error(`${file}: missing root version`);
    }
    content = content.replace(/(^  "version": ")[^"]+(".*$)/m, `$1${value}$2`);
    if (file.endsWith('package-lock.json')) {
      if (!/^    "": \{\r?\n      "name": "[^"]+",\r?\n      "version": "[^"]+"/m.test(content)) {
        throw Error(`${file}: missing packages[""] version`);
      }
      content = content.replace(/(^      "version": ")[^"]+(".*$)/m, `$1${value}$2`);
    }
    changes.push([file, content]);
  }
  for (const file of marketplaces) {
    const content = read(file).replace(/("version": ")[^"]+(".*$)/m,
      (_, start, end) => `${start}${value}${end}`
    );
    changes.push([file, content]);
  }
  const skill = read('skills/planr/SKILL.md').replace(/(^  version: ")[^"]+("\r?$)/m, `$1${value}$2`);
  changes.push(['skills/planr/SKILL.md', skill]);
  for (const [file, content] of changes) {
    writeFileSync(join(root, file), content);
  }
  console.log(`planr: versions set to ${value}`);
} catch (error) {
  console.error(`planr: ${error.message}`);
  process.exitCode = 1;
}
