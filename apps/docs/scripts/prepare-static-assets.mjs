import { cp, mkdir, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { legacyRedirects } from '../redirects.mjs';

const appRoot = path.resolve(import.meta.dirname, '..');
const outputRoot = path.join(appRoot, 'out');
const markdownRoot = path.join(outputRoot, 'api', 'markdown');
const docsRoot = path.join(outputRoot, 'docs');

async function copyMarkdown(directory = markdownRoot) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const source = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      await copyMarkdown(source);
      continue;
    }
    if (!entry.name.endsWith('.md')) continue;
    const relative = path.relative(markdownRoot, source);
    const destination = path.join(docsRoot, relative);
    await mkdir(path.dirname(destination), { recursive: true });
    await cp(source, destination);
  }
}

await copyMarkdown();

const redirects = legacyRedirects
  .map(({ source, destination }) => `${source} ${destination} 308`)
  .join('\n');

await writeFile(path.join(outputRoot, '_redirects'), `${redirects}\n`);
await writeFile(
  path.join(outputRoot, '_headers'),
  `/api/search\n  Content-Type: application/json; charset=utf-8\n/docs/*.md\n  Content-Type: text/markdown; charset=utf-8\n/llms.txt\n  Content-Type: text/markdown; charset=utf-8\n/llms-full.txt\n  Content-Type: text/markdown; charset=utf-8\n`,
);

console.log('Prepared direct Cloudflare assets for docs HTML, Markdown, search, redirects, and 404s.');
