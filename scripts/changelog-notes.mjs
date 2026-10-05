import { read, semver } from './versions.mjs';

try {
  if (process.argv.length !== 3) {
    throw Error('usage: node scripts/changelog-notes.mjs <version>');
  }
  const value = semver(process.argv[2]);
  const sections = read('CHANGELOG.md').split(/^## /m).slice(1);
  const matches = sections.filter(section => section.split('\n')[0].match(/^\[([^\]]+)\]/)?.[1] === value);
  if (matches.length !== 1) {
    throw Error(`CHANGELOG.md must contain exactly one section for ${value}`);
  }
  const notes = matches[0].slice(matches[0].indexOf('\n') + 1).trim();
  if (!notes) {
    throw Error(`CHANGELOG.md section for ${value} is empty`);
  }
  console.log(notes);
} catch (error) {
  console.error(`planr: ${error.message}`);
  process.exitCode = 1;
}
