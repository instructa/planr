import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import copy from '../content/landing.mjs';
import llms from '../content/llms.mjs';
import quickstart from '../content/quickstart.mjs';
import { renderDocs } from './docs.mjs';
import { page, docPage, SITE, themeBoot } from './layout.mjs';
import { landing, walkthrough } from './landing.mjs';
import { session, shareFonts, FONTS } from './sample.mjs';
import { scene } from './scene.mjs';

// The board page shares the site's fonts and reads the site's stored theme before it paints,
// so the embedded board never flashes the other theme.
const forSite = (root, html) => {
  const charset = '<meta charset="utf-8">';
  if (!html.includes(charset)) {
    throw Error('board: no charset meta to add the theme boot after');
  }
  return shareFonts(root, html).replace(charset, () => `${charset}\n<script>${themeBoot}</script>`);
};

const site = fileURLToPath(new URL('../', import.meta.url));
const root = fileURLToPath(new URL('../../', import.meta.url));
const dist = join(site, 'dist');
const fonts = join(root, 'skills/planr/assets/board/fonts');

const write = (route, html) => {
  const file = join(dist, route.endsWith('/') ? `${route}index.html` : route);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, html);
};

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });

// Fonts and their licenses come straight from the board's assets.
cpSync(fonts, join(dist, 'fonts'), { recursive: true, filter: f => !f.endsWith('.DS_Store') });
// Notes such as brand/MOTION.md and dotfiles stay out of the output.
const shipped = f => !/(^|[\\/])\.[^\\/]+$/.test(f) && !f.endsWith('.md') && !f.endsWith('favicon.svg');
cpSync(join(site, 'assets'), join(dist, 'assets'), { recursive: true, filter: shipped });
cpSync(join(site, 'assets/favicon.svg'), join(dist, 'favicon.svg'));

const hash = createHash('sha256')
  .update(readFileSync(join(site, 'assets/site.css')))
  .update(readFileSync(join(site, 'assets/site.js')))
  .update(readFileSync(join(site, 'assets/header.js')))
  .update(readFileSync(join(site, 'assets/graph.js')))
  .digest('hex').slice(0, 10);
const build = { hash };

// Videos appear once their files are in site/assets/media; until then their slot does not render.
// The landing shows the short teaser, the docs start page the tutorial.
const media = join(site, 'assets/media');
const clip = name => existsSync(join(media, `${name}.mp4`)) && existsSync(join(media, `${name}.jpg`)) ? {
  src: `/assets/media/${name}.mp4`,
  poster: `/assets/media/${name}.jpg`,
  captions: existsSync(join(media, `${name}.vtt`)) ? `/assets/media/${name}.vtt` : null,
} : null;
const teaser = clip('planr-teaser');
const tutorial = clip('planr-intro');

// Docs, rendered from the repository. Every section the quickstart shows or hides has to name a
// question and an answer of its chooser, and every answer has to show something.
const docs = renderDocs(root);
for (const doc of docs.filter(d => d.chooser)) {
  const asked = new Map(quickstart.questions.map(q => [q.key, new Set(q.options.map(([value]) => value))]));
  const used = new Set();
  for (const condition of doc.shows) {
    for (const pair of condition.split(/\s+/)) {
      const [key, value] = pair.split('=');
      if (!asked.get(key)?.has(value)) {
        throw Error(`${doc.file}: show ${pair} is not a choice in content/quickstart.mjs`);
      }
      used.add(pair);
    }
  }
  const unused = [...asked].flatMap(([key, values]) => [...values].map(v => `${key}=${v}`)).filter(p => !used.has(p));
  if (unused.length) {
    throw Error(`${doc.file}: no section for ${unused.join(', ')}`);
  }
}
for (const doc of docs) {
  const extra = doc.route === '/docs/' ? { tutorial, copy: copy.tutorial } : doc.chooser ? { choose: quickstart } : {};
  write(doc.route, docPage(doc, docs, build, extra));
}

// For agents: every docs page as Markdown with absolute links, and /llms.txt, which lists them by
// the sidebar's groups. deploy/alchemy.run.ts serves them as text/markdown & text/plain.
for (const doc of docs) {
  write(doc.md.route, doc.md.text);
}
const missing = docs.filter(d => !llms.notes[d.route]).map(d => d.route);
if (missing.length) {
  throw Error(`content/llms.mjs: no note for ${missing.join(', ')}`);
}
const groups = [...new Set(docs.map(d => d.group))];
write('/llms.txt', `# planr

> ${llms.summary}

${llms.details}
${groups.map(g => `
## ${g}

${docs.filter(d => d.group === g).map(d => `- [${d.nav}](${SITE}${d.md.route}): ${llms.notes[d.route]}`).join('\n')}
`).join('')}`);

// The landing's sample: one replayed planning repo. The setup writes the board page, then the build
// reads the task file and runs the "How it works" commands, in that order.
const { session: s, how } = copy;
const commands = [...s.setup, `cat ${how.file.path}`, ...how.ready.commands];
const run = session(root, commands, { date: s.date, bodies: s.bodies, name: s.repo });
if (!run.board) {
  throw Error('landing: session.setup must write the board with planr board --out .planr/board.html');
}
const steps = {
  file: run.steps[s.setup.length].output,
  ready: run.steps.slice(s.setup.length + 1),
};
write('/sample/board.html', forSite(root, run.board));

const sample = await scene(root, { bodies: s.bodies, name: s.repo });
write('/', page({
  route: '/',
  title: copy.title,
  description: copy.description.replace(/&/g, '&amp;'),
  body: landing(copy, steps, sample, { video: teaser }),
  scripts: [`/assets/header.js?v=${build.hash}`, `/assets/graph.js?v=${build.hash}`],
  reveal: true,
  main: 'is-landing',
  build,
  docsCount: docs.length,
}));

write('/how-it-works/', page({
  route: '/how-it-works/',
  title: copy.walkthrough.title,
  description: copy.walkthrough.description.replace(/&/g, '&amp;'),
  body: walkthrough(copy, sample),
  main: 'is-landing',
  build,
  docsCount: docs.length,
}));

write('/404.html', page({
  route: '/404.html',
  title: 'Not found',
  description: 'This page does not exist.',
  body: `<main id="main" class="missing" tabindex="-1">
  <div class="panel"><div class="ph"><h1>Not found</h1><span class="c">404</span></div>
  <p>There is no page at this address.</p>
  <p><a class="btn primary" href="/">Overview</a> <a class="btn" href="/docs/">Docs</a></p></div>
</main>`,
  build,
  docsCount: docs.length,
}));

write('/robots.txt', `User-agent: *\nAllow: /\nSitemap: ${SITE}/sitemap.xml\n`);
write('/sitemap.xml', `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${['/', '/how-it-works/', ...docs.map(d => d.route)].map(r => `  <url><loc>${SITE}${r}</loc></url>`).join('\n')}
</urlset>
`);

console.log(`build: ${docs.length} docs, landing, board (${FONTS.length} shared fonts) in ${dist}`);
