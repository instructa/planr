import { existsSync, readFileSync, statSync } from 'node:fs';
import { posix } from 'node:path';
import { render, escape } from './markdown.mjs';
import { highlight } from './highlight.mjs';
import { markTables } from './marks.mjs';

export const REPO = 'https://github.com/instructa/planr';
export const SITE = 'https://planr.so';

// The docs are rendered from the repository at build time; nothing is copied into site/.
// Order is the reading order of the sidebar and the previous/next links.
// `chooser` turns the page's `<!-- show: … -->` sections into the interactive chooser (layout.mjs).
export const DOCS = [
  { group: 'Start', file: 'README.md', route: '/docs/', nav: 'Overview', heading: 'Overview' },
  { group: 'Start', file: 'docs/quickstart.md', route: '/docs/quickstart/', nav: 'Quickstart', chooser: true },
  { group: 'Start', file: 'docs/setup.md', route: '/docs/setup/', nav: 'Agent setup' },
  { group: 'Plugins', file: 'docs/plugins/claude-code.md', route: '/docs/plugins/claude-code/', nav: 'Claude Code' },
  { group: 'Plugins', file: 'docs/plugins/codex.md', route: '/docs/plugins/codex/', nav: 'Codex' },
  { group: 'Plugins', file: 'docs/plugins/bb.md', route: '/docs/plugins/bb/', nav: 'bb' },
  { group: 'Use', file: 'docs/concepts.md', route: '/docs/concepts/', nav: 'Concepts' },
  { group: 'Use', file: 'docs/cli.md', route: '/docs/cli/', nav: 'CLI' },
  { group: 'Use', file: 'docs/board.md', route: '/docs/board/', nav: 'Board' },
  { group: 'Reference', file: 'skills/planr/references/format.md', route: '/docs/format/', nav: 'File format' },
  { group: 'Reference', file: 'docs/contract.md', route: '/docs/contract/', nav: 'Contract' },
];

// Every page is also served as Markdown for agents, at its route with `.md`:
// /docs/ is /docs/index.md, /docs/plugins/codex/ is /docs/plugins/codex.md.
export const mdRoute = route => (route === '/docs/' ? '/docs/index.md' : route.replace(/\/$/, '.md'));

// Links between rendered docs go to site routes, or with `md` to the absolute URL of the page's
// Markdown, so an agent reading it can follow them. Links to any other repository path go to GitHub.
function linker(root, file, md = false) {
  return href => {
    if (/^[a-z][a-z\d+.-]*:/i.test(href)) {
      return { href, external: !href.startsWith('mailto:') };
    }
    if (href.startsWith('#')) {
      return { href };
    }
    const [path, hash = ''] = href.split('#');
    const target = posix.normalize(posix.join(posix.dirname(file), path));
    if (target.startsWith('..')) {
      throw Error(`${file}: link ${href} leaves the repository`);
    }
    const doc = DOCS.find(d => d.file === target);
    if (doc) {
      return { href: (md ? SITE + mdRoute(doc.route) : doc.route) + (hash ? `#${hash}` : '') };
    }
    const abs = `${root}/${target}`;
    if (!existsSync(abs)) {
      throw Error(`${file}: link ${href} points to a missing file ${target}`);
    }
    const kind = statSync(abs).isDirectory() ? 'tree' : 'blob';
    return { href: `${REPO}/${kind}/main/${target.replace(/\/$/, '')}${hash ? `#${hash}` : ''}`, external: true };
  };
}

// The Markdown source with every link made absolute through `link`. Code blocks & code spans stay
// as written, and so do HTML comments such as the quickstart's show marks.
function absolute(source, link) {
  let fence = null;
  return source.split('\n').map(line => {
    const open = line.match(/^ {0,3}(`{3,}|~{3,})/);
    if (fence || open) {
      fence = !fence ? open[1] : open && line.trim().startsWith(fence) ? null : fence;
      return line;
    }
    return line.split(/(`+[^`]*`+)/).map((part, k) => k % 2 ? part : part.replace(
      /\]\(\s*<?([^\s)>]+)>?(\s+"[^"]*")?\s*\)/g, (_, href, title = '') => `](${link(href).href}${title})`
    )).join('');
  }).join('\n');
}

export function renderDocs(root) {
  return DOCS.map((doc, i) => {
    const source = readFileSync(`${root}/${doc.file}`, 'utf8');
    const { html, headings } = render(source, { link: linker(root, doc.file), highlight });
    const h1 = headings.find(h => h.level === 1);
    return {
      ...doc,
      title: doc.heading ?? h1?.text ?? doc.nav,
      heading: doc.heading ? escape(doc.heading) : h1?.html ?? escape(doc.nav),
      // The first paragraph doubles as the page description.
      description: (html.match(/<p>([\s\S]*?)<\/p>/)?.[1] ?? '').replace(/<[^>]+>/g, '').replace(/\s+/g, ' ').trim(),
      html: markTables(html.replace(/<h1 id="[^"]*">[\s\S]*?<\/h1>\n?/, '')),
      toc: headings.filter(h => h.level === 2),
      shows: headings.filter(h => h.show).map(h => h.show),
      source: `${REPO}/blob/main/${doc.file}`,
      md: { route: mdRoute(doc.route), text: absolute(source, linker(root, doc.file, true)) },
      prev: DOCS[i - 1] ?? null,
      next: DOCS[i + 1] ?? null,
    };
  });
}
