import { readFileSync } from 'node:fs';

export function version() {
  const skill = readFileSync(new URL('../../SKILL.md', import.meta.url), 'utf8');
  const frontmatter = skill.split(/^---\r?$/m)[1] ?? '';
  const match = frontmatter.match(/^metadata:\r?\n  version: "([^"]+)"\r?$/m);
  if (!match) {
    throw Error('SKILL.md is missing metadata.version');
  }
  return match[1];
}
