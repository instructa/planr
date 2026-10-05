import { escape } from './markdown.mjs';

// Ink-only highlighting, in the board's two voices: comments step back, keys step forward.
// No hue: color belongs to status marks.

const RULES = {
  sh: line => line.replace(/^(\s*)(#.*)$/, (_, a, c) => `${a}\u0001${c}\u0002`),
  markdown: line => line
    .replace(/^([\w-]+)(:)/, '\u0003$1\u0004$2')
    .replace(/(\s)(#\s.*)$/, '$1\u0001$2\u0002')
    .replace(/^(#{1,6} .*)$/, '\u0003$1\u0004'),
  json: line => line.replace(/("[\w-]+")(\s*:)/g, '\u0003$1\u0004$2'),
  ts: line => line.replace(/(\/\/.*)$/, '\u0001$1\u0002'),
};
RULES.yaml = RULES.markdown;
RULES.md = RULES.markdown;

export function highlight(code, lang = '') {
  const rule = RULES[lang];
  return code.split('\n').map(line => {
    const marked = rule ? rule(line) : line;
    return escape(marked)
      .replace(/\u0001([^\u0002]*)\u0002/g, '<span class="c">$1</span>')
      .replace(/\u0003([^\u0004]*)\u0004/g, '<span class="k">$1</span>');
  }).join('\n');
}
