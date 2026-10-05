// The board's status marks (DESIGN.md, Shapes), drawn the same way on a 12×12 grid.
// Status is never color alone: every mark sits next to its status name.
const COLOR = s => `var(--s-${s === 'in-progress' ? 'wip' : s})`;
const BODY = {
  'open': c => `<circle cx="6" cy="6" r="4.6" fill="none" stroke="${c}" stroke-width="1.5"/>`,
  'in-progress': c => `<circle cx="6" cy="6" r="4.6" fill="none" stroke="${c}" stroke-width="1.5"/><path d="M6 2.6a3.4 3.4 0 0 1 0 6.8z" fill="${c}"/>`,
  'review': c => `<circle cx="6" cy="6" r="4.6" fill="none" stroke="${c}" stroke-width="1.5"/><circle cx="6" cy="6" r="2" fill="${c}"/>`,
  'done': c => `<circle cx="6" cy="6" r="5.4" fill="${c}"/><path d="M3.6 6.1l1.6 1.6 3.2-3.3" fill="none" stroke="var(--mark-ink)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>`,
  'blocked': c => `<circle cx="6" cy="6" r="5.4" fill="${c}"/><rect x="3.3" y="5.25" width="5.4" height="1.5" rx=".75" fill="var(--mark-ink)"/>`,
  'decision': c => `<rect x="2.1" y="2.1" width="7.8" height="7.8" rx="1.6" transform="rotate(45 6 6)" fill="${c}"/><rect x="5.3" y="3.3" width="1.4" height="3.3" rx=".7" fill="var(--mark-ink)"/><circle cx="6" cy="8.2" r=".8" fill="var(--mark-ink)"/>`,
  'dropped': c => `<circle cx="6" cy="6" r="4.6" fill="none" stroke="${c}" stroke-width="1.5"/><path d="M3.2 8.8l5.6-5.6" stroke="${c}" stroke-width="1.5" stroke-linecap="round"/>`,
};
// Goal states reuse the task marks with the same meaning, as on the board.
export const GOAL = { draft: 'decision', approved: 'open', running: 'in-progress', review: 'review', done: 'done' };

export const mark = s => `<svg class="mark" width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">${BODY[s](COLOR(s === 'dropped' ? 'open' : s))}</svg>`;

// Tables whose first column lists task statuses or goal states get the mark before each name.
export function markTables(html) {
  return html.replace(/<table><thead><tr><th>(Status|Goal state)<\/th>[\s\S]*?<\/table>/g, (table, kind) =>
    table.replace(/<tr><td><code>([a-z-]+)<\/code><\/td>/g, (row, name) => {
      const s = kind === 'Status' ? name : GOAL[name];
      return BODY[s] ? `<tr><td><span class="st">${mark(s)}<code>${name}</code></span></td>` : row;
    })
  );
}
