import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { version } from '../skills/planr/scripts/lib/version.mjs';

export const root = fileURLToPath(new URL('../', import.meta.url));
export const read = file => readFileSync(new URL(`../${file}`, import.meta.url), 'utf8');
export const manifest = file => JSON.parse(read(file));
export const pluginManifests = ['.claude-plugin/plugin.json', 'plugin.json'];
export const marketplaces = ['.claude-plugin/marketplace.json', '.agents/plugins/marketplace.json'];

export function npmSource(file) {
  const marketplace = manifest(file);
  const plugins = marketplace.plugins.filter(plugin => plugin.name === 'planr');
  if (marketplace.name !== 'planr' || plugins.length !== 1) {
    throw Error(`${file}: expected one planr plugin in the planr marketplace`);
  }
  const source = plugins[0].source;
  if (source?.source !== 'npm' || source.package !== 'planr') {
    throw Error(`${file}: planr must use the npm package planr`);
  }
  return source;
}

export function semver(value) {
  const core = '(0|[1-9][0-9]*)';
  const pre = '(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)';
  const build = '[0-9A-Za-z-]+';
  const pattern = `^${core}\\.${core}\\.${core}(?:-${pre}(?:\\.${pre})*)?(?:\\+${build}(?:\\.${build})*)?$`;
  if (typeof value !== 'string' || value !== value.trim() || !new RegExp(pattern).test(value)) {
    throw Error(`invalid semver: ${value}`);
  }
  return value;
}

export function versions() {
  const lock = manifest('bb-plugin/package-lock.json');
  return {
    'SKILL.md metadata.version (runtime)': version(),
    'package.json': manifest('package.json').version,
    'bb-plugin/package.json': manifest('bb-plugin/package.json').version,
    'bb-plugin/package-lock.json': lock.version,
    'bb-plugin/package-lock.json packages[""]': lock.packages[''].version,
    ...Object.fromEntries(pluginManifests.map(file => [file, manifest(file).version])),
    ...Object.fromEntries(marketplaces.map(file => [`${file} npm pin`, npmSource(file).version])),
  };
}

export function checkVersions(tag) {
  const entries = Object.entries(versions());
  const expected = semver(entries[0][1]);
  for (const [file, value] of entries) {
    if (semver(value) !== expected) {
      throw Error(`${file}: ${value} differs from runtime version ${expected}`);
    }
  }
  if (tag !== undefined && tag !== `v${expected}`) {
    throw Error(`tag ${tag} differs from v${expected}`);
  }
  return expected;
}
