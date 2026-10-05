import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, posix, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

// Checks the built site offline: every internal link and asset resolves, every #fragment exists,
// every GitHub link into this repository names a path that exists here, and every planr.so URL, in
// the pages and in the files agents read (each page's .md, llms.txt), names a page or file of this
// build. Those agent files may only link absolutely.
const dist = fileURLToPath(new URL('../dist/', import.meta.url));
const root = fileURLToPath(new URL('../../', import.meta.url));
const REPO = 'https://github.com/instructa/planr/';
const SITE = 'https://planr.so';

const pages = [];
const raw = [];
const walk = dir => {
  for (const f of readdirSync(dir)) {
    const p = join(dir, f);
    if (statSync(p).isDirectory()) {
      walk(p);
    } else if (f.endsWith('.html')) {
      pages.push(p);
    } else if (f.endsWith('.md') || f === 'llms.txt') {
      raw.push(p);
    }
  }
};
walk(dist);

const ids = new Map();
const idsOf = file => {
  if (!ids.has(file)) {
    const html = readFileSync(file, 'utf8');
    ids.set(file, new Set([...html.matchAll(/\sid="([^"]+)"/g)].map(m => m[1])));
  }
  return ids.get(file);
};
const target = path => {
  const p = join(dist, decodeURI(path));
  if (existsSync(p) && statSync(p).isDirectory()) {
    return existsSync(join(p, 'index.html')) ? join(p, 'index.html') : null;
  }
  return existsSync(p) ? p : null;
};

const errors = [];
let checked = 0;
for (const page of pages) {
  const html = readFileSync(page, 'utf8');
  const route = '/' + relative(dist, page).split('\\').join('/').replace(/index\.html$/, '');
  for (const [, attr, raw] of html.matchAll(/\s(href|src)="([^"]*)"/g)) {
    const url = raw.replace(/&amp;/g, '&');
    checked++;
    if (url.startsWith(REPO)) {
      const m = url.slice(REPO.length).match(/^(blob|tree)\/main\/([^#?]+)/);
      if (m && !existsSync(join(root, m[2]))) {
        errors.push(`${route}: ${url} names a missing repository path`);
      }
      continue;
    }
    if (url.startsWith(SITE + '/') && !url.startsWith(SITE + '/sitemap.xml')) {
      const [ref] = url.slice(SITE.length).split('#');
      if (!target(ref)) {
        errors.push(`${route}: ${url} is not a page of this build`);
      }
      continue;
    }
    if (/^(https?:|mailto:|data:)/.test(url)) {
      continue;
    }
    const [ref, hash] = url.split('#');
    const path = ref.split('?')[0];
    const resolved = path ? posix.resolve(posix.dirname(route + 'x'), path) : route;
    const file = target(resolved);
    if (!file) {
      errors.push(`${route}: ${attr} ${url} does not resolve`);
      continue;
    }
    if (hash && file.endsWith('.html') && !idsOf(file).has(hash)) {
      errors.push(`${route}: ${url} has no #${hash}`);
    }
  }
}
// Any planr.so or repository URL in a file, also inside prompts & code: it has to exist.
const urls = (route, text) => {
  for (const [match] of text.matchAll(/https:\/\/(?:planr\.so|github\.com\/instructa\/planr)\/[^\s)>`"'<]*/g)) {
    const url = match.replace(/[.,;:]+$/, '');
    checked++;
    if (url.startsWith(REPO)) {
      const m = url.slice(REPO.length).match(/^(blob|tree)\/main\/([^#?]+)/);
      if (m && !existsSync(join(root, m[2]))) {
        errors.push(`${route}: ${url} names a missing repository path`);
      }
    } else if (!target(url.slice(SITE.length).split('#')[0])) {
      errors.push(`${route}: ${url} is not a page of this build`);
    }
  }
};
for (const page of pages) {
  urls('/' + relative(dist, page), readFileSync(page, 'utf8'));
}
for (const file of raw) {
  const route = '/' + relative(dist, file);
  const text = readFileSync(file, 'utf8');
  urls(route, text);
  for (const [, href] of text.replace(/```[\s\S]*?```/g, '').matchAll(/\]\(\s*<?([^\s)>]+)/g)) {
    checked++;
    if (!/^(https?:|mailto:|#)/.test(href)) {
      errors.push(`${route}: ${href} is relative; agents need absolute links`);
    }
  }
}
if (errors.length) {
  console.error(errors.join('\n'));
  console.error(`check: ${errors.length} broken of ${checked} links in ${pages.length} pages & ${raw.length} agent files`);
  process.exit(1);
}
console.log(`check: ${checked} links in ${pages.length} pages & ${raw.length} agent files resolve`);
