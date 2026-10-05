// A small Markdown renderer for the planr docs: ATX headings, paragraphs, fenced code,
// lists, block quotes, GFM tables, code spans, links, autolinks, bold and italics.
// It covers what docs/*.md use; anything else renders as escaped text.
// HTML comments are dropped, as on GitHub. One kind is read: `<!-- show: key=value … -->` on the
// line before a heading wraps that heading's section in a <div data-show> for the docs chooser.

export const escape = s => String(s).replace(/[&<>"]/g, c =>
  ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]
);

// GitHub's heading anchors: lowercase, punctuation removed, spaces to hyphens.
export function slugger() {
  const seen = new Map();
  return text => {
    const base = text.toLowerCase().trim()
      .replace(/[^\p{L}\p{N}\s_-]/gu, '')
      .replace(/\s/g, '-');
    const n = seen.get(base) ?? 0;
    seen.set(base, n + 1);
    return n ? `${base}-${n}` : base;
  };
}

// Plain text of inline Markdown, for anchors and the table of contents.
export const plain = s => s
  .replace(/`([^`]*)`/g, '$1')
  .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
  .replace(/<(https?:[^>]+)>/g, '$1')
  .replace(/(\*\*|__)(.+?)\1/g, '$2')
  .replace(/(^|\W)[*_](.+?)[*_](?=\W|$)/g, '$1$2');

export function inline(src, link = href => ({ href })) {
  // Code spans and links become placeholders, so emphasis can span them and never reaches inside.
  const held = [];
  const hold = html => `\u0000${held.push(html) - 1}\u0000`;
  let out = '';
  let i = 0;
  while (i < src.length) {
    const ch = src[i];
    if (ch === '\\' && /[\\`*_[\]()<>#|!-]/.test(src[i + 1] ?? '')) {
      out += hold(escape(src[i + 1]));
      i += 2;
      continue;
    }
    if (ch === '`') {
      const run = src.slice(i).match(/^`+/)[0];
      const end = src.indexOf(run, i + run.length);
      if (end > 0) {
        const code = src.slice(i + run.length, end).replace(/^ (.+) $/, '$1');
        out += hold(`<code>${escape(code)}</code>`);
        i = end + run.length;
        continue;
      }
    }
    if (ch === '<') {
      const m = src.slice(i).match(/^<(https?:\/\/[^\s>]+)>/);
      if (m) {
        out += hold(anchor(link(m[1]), escape(m[1])));
        i += m[0].length;
        continue;
      }
    }
    if (ch === '[') {
      const m = linkAt(src, i);
      if (m) {
        out += hold(anchor(link(m.href), inline(m.text, link)));
        i = m.end;
        continue;
      }
    }
    out += escape(ch);
    i++;
  }
  return emphasis(out).replace(/\u0000(\d+)\u0000/g, (_, n) => held[n]);
}

function anchor({ href, external }, html) {
  const rel = external ? ' rel="noopener"' : '';
  return `<a href="${escape(href)}"${rel}>${html}</a>`;
}

function linkAt(src, start) {
  let depth = 0;
  for (let i = start; i < src.length; i++) {
    if (src[i] === '`') {
      const run = src.slice(i).match(/^`+/)[0];
      const end = src.indexOf(run, i + run.length);
      i = end > 0 ? end + run.length - 1 : i;
    } else if (src[i] === '[') {
      depth++;
    } else if (src[i] === ']' && --depth === 0) {
      const m = src.slice(i + 1).match(/^\(\s*<?([^\s)>]+)>?(?:\s+"[^"]*")?\s*\)/);
      return m ? { text: src.slice(start + 1, i), href: m[1], end: i + 1 + m[0].length } : null;
    }
  }
  return null;
}

// Runs on escaped text, so the markers it adds are the only tags in it.
const emphasis = html => html
  .replace(/\*\*(?=\S)(.+?)(?<=\S)\*\*/g, '<strong>$1</strong>')
  .replace(/(^|[^\w*])\*(?=\S)(.+?)(?<=\S)\*(?![\w*])/g, '$1<em>$2</em>')
  .replace(/(^|[^\w])_(?=\S)(.+?)(?<=\S)_(?!\w)/g, '$1<em>$2</em>');

const FENCE = /^ {0,3}(`{3,}|~{3,})\s*([\w+-]*)/;
const HEADING = /^ {0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const ITEM = /^( {0,3})([-*+]|\d{1,9}[.)])(\s+|$)(.*)$/;
const RULE = /^ {0,3}([-*_])(\s*\1){2,}\s*$/;
const DIVIDER = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;
const QUOTE = /^ {0,3}>/;
const COMMENT = /^ {0,3}<!--/;
const SHOW = /^\s*<!--\s*show:\s*(.+?)\s*-->\s*$/;

const startsBlock = (line, next = '') =>
  COMMENT.test(line) || FENCE.test(line) || HEADING.test(line) || ITEM.test(line) || QUOTE.test(line) ||
  RULE.test(line) || (line.includes('|') && DIVIDER.test(next));

const cellsOf = row => row.trim().replace(/^\|/, '').replace(/\|$/, '')
  .split(/(?<!\\)\|/).map(c => c.trim());

// Returns { html, headings } where headings lists { level, id, text } in order.
export function render(markdown, { link, highlight = (code) => escape(code), slug = slugger() } = {}) {
  const headings = [];
  const span = s => inline(s, link);
  let show = null;

  function blocks(lines, tight = false) {
    const html = [];
    let i = 0;
    while (i < lines.length) {
      const line = lines[i];
      if (!line.trim()) {
        i++;
        continue;
      }
      let m;
      if (COMMENT.test(line)) {
        const body = [];
        while (i < lines.length) {
          body.push(lines[i]);
          i++;
          if (body.at(-1).includes('-->')) {
            break;
          }
        }
        show = body.join(' ').match(SHOW)?.[1] ?? null;
        continue;
      }
      if ((m = line.match(FENCE))) {
        const [fence, , lang] = m;
        const marker = fence.trim().match(/^(`{3,}|~{3,})/)[1];
        const body = [];
        i++;
        while (i < lines.length && !lines[i].trim().startsWith(marker)) {
          body.push(lines[i]);
          i++;
        }
        i++;
        html.push({ type: 'code', html: highlight(body.join('\n'), lang), lang });
        continue;
      }
      if ((m = line.match(HEADING))) {
        const level = m[1].length;
        const id = slug(plain(m[2]));
        headings.push({ level, id, text: plain(m[2]), html: span(m[2]), show });
        html.push({ type: 'heading', level, id, html: span(m[2]), show });
        show = null;
        i++;
        continue;
      }
      if (RULE.test(line)) {
        html.push({ type: 'raw', html: '<hr>' });
        i++;
        continue;
      }
      if (QUOTE.test(line)) {
        const body = [];
        while (i < lines.length && lines[i].trim() && (QUOTE.test(lines[i]) || !startsBlock(lines[i]))) {
          body.push(lines[i].replace(/^ {0,3}> ?/, ''));
          i++;
        }
        html.push({ type: 'raw', html: `<blockquote>${join(blocks(body))}</blockquote>` });
        continue;
      }
      if (line.includes('|') && DIVIDER.test(lines[i + 1] ?? '')) {
        const head = cellsOf(line);
        const align = cellsOf(lines[i + 1]).map(c =>
          c.startsWith(':') && c.endsWith(':') ? 'center' : c.endsWith(':') ? 'right' : ''
        );
        i += 2;
        const rows = [];
        while (i < lines.length && lines[i].includes('|') && lines[i].trim()) {
          rows.push(cellsOf(lines[i]));
          i++;
        }
        const cell = (tag, c, k) =>
          `<${tag}${align[k] ? ` style="text-align:${align[k]}"` : ''}>${span(c ?? '')}</${tag}>`;
        html.push({
          type: 'table',
          html: `<thead><tr>${head.map((c, k) => cell('th', c, k)).join('')}</tr></thead>` +
            `<tbody>${rows.map(r => `<tr>${head.map((_, k) => cell('td', r[k], k)).join('')}</tr>`).join('')}</tbody>`,
        });
        continue;
      }
      if ((m = line.match(ITEM))) {
        const ordered = /\d/.test(m[2]);
        const start = ordered ? parseInt(m[2], 10) : 1;
        const items = [];
        let loose = false;
        while (i < lines.length) {
          const im = lines[i].match(ITEM);
          if (!im || /\d/.test(im[2]) !== ordered) {
            break;
          }
          const indent = im[1].length + im[2].length + Math.max(1, Math.min(4, im[3].length || 1));
          const body = [im[4]];
          i++;
          while (i < lines.length) {
            const l = lines[i];
            if (!l.trim()) {
              const following = lines[i + 1] ?? '';
              if (following.trim() && /^\s+/.test(following) && following.search(/\S/) >= indent) {
                body.push('');
                loose = true;
                i++;
                continue;
              }
              break;
            }
            if (l.search(/\S/) >= indent) {
              body.push(l.slice(indent));
            } else if (ITEM.test(l) || startsBlock(l, lines[i + 1])) {
              break;
            } else {
              body.push(l.trim());
            }
            i++;
          }
          items.push(body);
          if (!lines[i]?.trim() && ITEM.test(lines[i + 1] ?? '') &&
            /\d/.test(lines[i + 1].match(ITEM)[2]) === ordered) {
            loose = true;
            i++;
          }
        }
        const tag = ordered ? 'ol' : 'ul';
        const attr = ordered && start !== 1 ? ` start="${start}"` : '';
        html.push({
          type: 'raw',
          html: `<${tag}${attr}>${items.map(b => `<li>${join(blocks(b, !loose))}</li>`).join('')}</${tag}>`,
        });
        continue;
      }
      const para = [line.trim()];
      i++;
      while (i < lines.length && lines[i].trim() && !startsBlock(lines[i], lines[i + 1])) {
        para.push(lines[i].trim());
        i++;
      }
      html.push({ type: tight ? 'text' : 'p', html: span(para.join('\n')) });
    }
    return html;
  }

  const join = list => list.map(b => {
    switch (b.type) {
      case 'code':
        return `<pre class="code"${b.lang ? ` data-lang="${escape(b.lang)}"` : ''}><code>${b.html}</code></pre>`;
      case 'heading':
        return `<h${b.level} id="${b.id}">${b.html}</h${b.level}>`;
      case 'table':
        return `<div class="table"><table>${b.html}</table></div>`;
      case 'p':
        return `<p>${b.html}</p>`;
      default:
        return b.html;
    }
  }).join('\n');

  // A heading with a show condition opens a section that runs to the next heading of its level or
  // above; sections nest, so a step inherits the conditions of the part it belongs to.
  const sections = list => {
    const out = [];
    const open = [];
    for (const b of list) {
      if (b.type === 'heading') {
        while (open.length && open.at(-1) >= b.level) {
          open.pop();
          out.push({ type: 'raw', html: '</div>' });
        }
        if (b.show) {
          open.push(b.level);
          out.push({ type: 'raw', html: `<div class="part" data-show="${escape(b.show)}">` });
        }
      }
      out.push(b);
    }
    return out.concat(open.map(() => ({ type: 'raw', html: '</div>' })));
  };

  const html = join(sections(blocks(markdown.replace(/\r\n?/g, '\n').split('\n'))));
  return { html, headings };
}
