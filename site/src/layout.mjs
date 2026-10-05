import { readFileSync } from 'node:fs';
import { escape } from './markdown.mjs';
import { REPO, SITE } from './docs.mjs';

export { SITE };

// Drawn icons, one stroke weight (1.5) on a 16 grid, like the board's theme button.
export const icon = {
  sun: '<svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="3.2" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M8 1.5v1.6M8 12.9v1.6M1.5 8h1.6M12.9 8h1.6M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M3.4 12.6l1.1-1.1M11.5 4.5l1.1-1.1" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>',
  moon: '<svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true"><path d="M13.2 10.2A5.6 5.6 0 0 1 5.8 2.8a5.6 5.6 0 1 0 7.4 7.4z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/></svg>',
  copy: '<svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true"><rect x="5.25" y="5.25" width="8" height="8" rx="1.75" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M10.5 3.2v-.2a1.25 1.25 0 0 0-1.25-1.25h-5.5A1.75 1.75 0 0 0 2 3.5v5.5a1.25 1.25 0 0 0 1.25 1.25h.2" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>',
  check: '<svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.4l3 3 6-6.3" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  out: '<svg width="11" height="11" viewBox="0 0 12 12" aria-hidden="true"><path d="M4.5 2.5h5v5M9.25 2.75L3 9" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  menu: '<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h11" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>',
  // GitHub mark: Octicons mark-github-16, MIT
  github: '<svg width="17" height="17" viewBox="0 0 16 16" aria-hidden="true">' +
    '<path fill="currentColor" d="' +
    'M6.766 11.328c-2.063-.25-3.516-1.734-3.516-3.656 ' +
    '0-.781.281-1.625.75-2.188-.203-.515-.172-1.609.063-2.062.625-.078 1.468.25 ' +
    '1.968.703.594-.187 1.219-.281 1.985-.281.765 0 1.39.094 1.953.265.484-.437 ' +
    '1.344-.765 1.969-.687.218.422.25 1.515.046 2.047.5.593.766 1.39.766 2.203 0 ' +
    '1.922-1.453 3.375-3.547 3.64.531.344.89 1.094.89 1.954v1.625c0 ' +
    '.468.391.734.86.547C13.781 14.359 16 11.53 16 8.03 16 3.61 12.406 0 7.984 0 3.563 0 ' +
    '0 3.61 0 8.031a7.88 7.88 0 0 0 5.172 ' +
    '7.422c.422.156.828-.125.828-.547v-1.25c-.219.094-.5.156-.75.156-1.031 ' +
    '0-1.64-.562-2.078-1.609-.172-.422-.36-.672-.719-.719-.187-.015-.25-.093-.25-.187 ' +
    '0-.188.313-.328.625-.328.453 0 .844.281 1.25.86.313.452.64.655 1.031.655s.641-.14 ' +
    '1-.5c.266-.265.47-.5.657-.656' +
    '"/></svg>',
};

// The planr logo, inlined from the brand files (site/assets/brand) so it takes the text color.
// The home link carries the name; the drawing itself is hidden from assistive technology.
const brand = name => readFileSync(new URL(`../assets/brand/${name}`, import.meta.url), 'utf8')
  .replace(/<!--[\s\S]*?-->\s*/g, '')
  .replace(/\s+role="img" aria-label="[^"]*"/, '')
  .replace('<svg ', '<svg class="logo" aria-hidden="true" focusable="false" ')
  .replace(/\n\s*/g, '\n')
  .trim();
export const logo = brand('planr-logo.svg');
// The reveal plays once on load and ends on the static logo; under reduced motion it shows that frame.
export const logoAnimated = brand('planr-logo-animated.svg');

// No autoplay or download until play; keeps poster, controls and inline playback on phones.
// Videos play with sound unless the caller marks a silent clip muted.
export const videoTag = ({ src, poster, captions, muted = false }, title) =>
  `<video controls${muted ? ' muted' : ''} playsinline preload="none" poster="${poster}"
        width="1920" height="1080" title="${escape(title)}">
        <source src="${src}" type="video/mp4">
        ${captions ? `<track kind="captions" src="${captions}" srclang="en" label="English" default>` : ''}
      </video>`;

// Applied before first paint, so a stored theme never flashes the other one.
export const themeBoot = `try{const t=localStorage.getItem('planr-theme');if(t==='light'||t==='dark')document.documentElement.dataset.theme=t}catch{}`;

export function page({ route, title, description, body, main = '', build, docsCount, scripts = [], reveal = false, markdown = '' }) {
  const tab = (href, label, current, extra = '') =>
    `<a class="tab" href="${href}"${current ? ' aria-current="page"' : ''}>${label}${extra}</a>`;
  const isDocs = route.startsWith('/docs/');
  const full = route === '/' ? title : `${title} · planr`;
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(full)}</title>
<meta name="description" content="${description}">
<link rel="canonical" href="${SITE}${route}">${markdown ? `\n<link rel="alternate" type="text/markdown" href="${markdown}">` : ''}
<meta property="og:type" content="website">
<meta property="og:site_name" content="planr">
<meta property="og:title" content="${escape(full)}">
<meta property="og:description" content="${description}">
<meta property="og:url" content="${SITE}${route}">
<meta name="color-scheme" content="light dark">
<meta name="theme-color" media="(prefers-color-scheme: light)" content="#f4f5f7">
<meta name="theme-color" media="(prefers-color-scheme: dark)" content="#0a0c0f">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="preload" href="/fonts/MonaSans-Variable.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="/fonts/MonaspaceNeon-400.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="/assets/site.css?v=${build.hash}">
<script>${themeBoot}</script>
<script src="/assets/site.js?v=${build.hash}" defer></script>
${scripts.map(src => `<script type="module" src="${src}"></script>`).join('\n')}
</head>
<body class="${main}">
<a class="skip" href="#main">Skip to content</a>
<header class="top">
  <a class="brand" href="/" aria-label="planr home">${reveal ? logoAnimated : logo}</a>
  <nav class="tabs" aria-label="Site">
    ${tab('/', 'Overview', route === '/')}
    ${tab('/how-it-works/', 'How it works', route === '/how-it-works/')}
    ${tab('/docs/', 'Docs', isDocs, `<span class="n" aria-hidden="true">${docsCount}</span>`)}
  </nav>
  <div class="links">
    <a class="gh" href="${REPO}" rel="noopener" aria-label="planr on GitHub" title="GitHub">${icon.github}</a>
    <button class="theme" type="button" aria-label="Toggle light or dark theme" title="Theme">${icon.moon}</button>
  </div>
</header>
${body}
<footer class="foot">
  <span>planr uses the <a href="${REPO}/blob/main/LICENSE" rel="noopener">MIT license</a>. Mona Sans &amp; Monaspace use <a href="${REPO}/tree/main/skills/planr/assets/board/fonts" rel="noopener">SIL OFL 1.1</a>.</span>
</footer>
</body>
</html>
`;
}

// The quickstart's chooser: one radio group per question. It stays hidden until site.js runs, so
// without the script the page reads like the Markdown on GitHub, with every path.
function chooser(spec) {
  const group = q => `<fieldset class="seg" data-key="${q.key}"><legend>${escape(q.label)}</legend><div class="opts">${q.options.map(([value, label, note], k) =>
    `<label><input type="radio" name="qs-${q.key}" value="${value}"${k === 0 ? ' checked' : ''}><span>${escape(label)}${note ? ` <em class="note">${escape(note)}</em>` : ''}</span></label>`
  ).join('')}</div></fieldset>`;
  return `<form class="chooser" aria-labelledby="chooser-h" hidden>
      <div class="ph"><h2 class="lbl" id="chooser-h">${escape(spec.label)}</h2><span class="c">${escape(spec.note)}</span></div>
      ${spec.questions.map(group).join('')}
    </form>
`;
}

export function docPage(doc, docs, build, { tutorial = null, copy = {}, choose = null } = {}) {
  const groups = [...new Set(docs.map(d => d.group))];
  const nav = groups.map(g => `<div class="ng"><h2>${escape(g)}</h2><ul>${docs.filter(d => d.group === g).map(d =>
    `<li><a href="${d.route}"${d.route === doc.route ? ' aria-current="page"' : ''}>${escape(d.nav)}</a></li>`
  ).join('')}</ul></div>`).join('');
  // The docs start page carries the tutorial video after its introduction, before the first section.
  // The tutorial is silent, so it stays muted.
  const watch = tutorial ? `<section class="tutorial" aria-labelledby="watch-the-tutorial">
      <h2 id="watch-the-tutorial" class="th">${escape(copy.heading)}</h2>
      <div class="panel vframe">${videoTag({ ...tutorial, muted: true }, copy.title)}</div>
    </section>\n` : '';
  // The chooser goes after the introduction, before the first section it filters.
  const intro = choose ? chooser(choose) : watch;
  const at = doc.html.search(/<div class="part"|<h2/);
  const article = intro && at >= 0 ? doc.html.slice(0, at) + intro + doc.html.slice(at) : doc.html + intro;
  const sections = watch ? [{ id: 'watch-the-tutorial', html: escape(copy.heading) }, ...doc.toc] : doc.toc;
  const toc = sections.length > 1 ? `<nav class="toc" aria-label="On this page"><h2>On this page</h2><ul>${sections.map(h =>
    `<li><a href="#${h.id}">${h.html}</a></li>`
  ).join('')}</ul></nav>` : '';
  const pager = `<nav class="pager" aria-label="Previous &amp; next page">${doc.prev
    ? `<a class="prev" href="${doc.prev.route}"><span>Previous</span>${escape(doc.prev.nav)}</a>` : '<span></span>'}${doc.next
    ? `<a class="next" href="${doc.next.route}"><span>Next</span>${escape(doc.next.nav)}</a>` : '<span></span>'}</nav>`;
  const body = `<div class="docs">
  <details class="dnav-m"><summary>${icon.menu}<span>${escape(doc.nav)}</span></summary><nav aria-label="Docs">${nav}</nav></details>
  <nav class="dnav" aria-label="Docs">${nav}</nav>
  <main id="main" class="doc" tabindex="-1">
    <article class="prose">
      <h1>${doc.heading}</h1>
      ${anchors(article)}
    </article>
    <p class="src"><a href="${doc.source}" rel="noopener">View ${escape(doc.file)} on GitHub${icon.out}</a><a href="${doc.md.route}">Markdown</a></p>
    ${pager}
  </main>
  ${toc}
</div>`;
  return page({
    route: doc.route,
    title: doc.route === '/docs/' ? 'Docs' : doc.title,
    description: doc.description,
    body,
    main: 'is-docs',
    markdown: doc.md.route,
    build,
    docsCount: docs.length,
  });
}

// Each h2 and h3 gets a quiet # link for sharing a section.
const anchors = html => html.replace(/<h([23]) id="([^"]+)">([\s\S]*?)<\/h\1>/g, (_, n, id, inner) =>
  `<h${n} id="${id}">${inner}<a class="hash" href="#${id}" aria-label="Link to this section">#</a></h${n}>`
);
