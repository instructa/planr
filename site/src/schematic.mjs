import { escape } from './markdown.mjs';
import { mark, GOAL } from './marks.mjs';

// Schematic pieces of the board, drawn from the board's own data for the sample (src/scene.mjs):
// the same rows, marks, tags and buttons, with more air and far fewer of them. Each piece is a picture
// for assistive technology (role="img") with a label built from the same data.

const COLOR = s => `var(--s-${s === 'in-progress' ? 'wip' : s === 'dropped' ? 'open' : s})`;
const PROGRESS = ['done', 'review', 'in-progress', 'open', 'decision', 'blocked', 'dropped'];
const WORDS = { 'in-progress': 'in progress' };
export const word = s => WORDS[s] ?? s;

// "2026-10-03T10:05:00+02:00" as the board shows it, "03.10. 10:05", read from the string itself
// so every build prints the same time.
export const when = iso => `${iso.slice(8, 10)}.${iso.slice(5, 7)}. ${iso.slice(11, 16)}`;

export const picture = (label, html, cls = '') =>
  `<div class="${cls}" role="img" aria-label="${escape(label)}">${html}</div>`;

// `extra` is trusted markup from this module (a button), placed after the meta.
export function row(task, { status = task.status, tag = '', meta = '', cls = '', extra = '' } = {}) {
  return `<div class="mrow${cls ? ` ${cls}` : ''}">${mark(status)}<span class="id">${escape(task.id)}</span>` +
    `<span class="tt">${escape(task.title)}</span>${tag ? `<span class="tag">${escape(tag)}</span>` : ''}` +
    `${meta ? `<span class="meta">${escape(meta)}</span>` : ''}${extra}</div>`;
}

export function goalRow(goal, { state = goal.state, tag = 'goal', meta = '', extra = '', cls = '' } = {}) {
  return `<div class="mrow${cls ? ` ${cls}` : ''}">${mark(GOAL[state])}<span class="id">${escape(goal.name)}</span>` +
    `<span class="tt">${escape(goal.title)}</span><span class="tag">${escape(tag)}</span>` +
    `${meta ? `<span class="meta">${escape(meta)}</span>` : ''}${extra}</div>`;
}

export const group = (label, count) =>
  `<div class="mgroup">${escape(label)}${count === undefined ? '' : `<span class="c">${count}</span>`}</div>`;

export const button = (label, primary = false) => `<span class="mbtn${primary ? ' primary' : ''}">${escape(label)}</span>`;

export function strip(tasks) {
  const counts = PROGRESS.map(s => [s, tasks.filter(t => t.status === s).length]).filter(([, n]) => n);
  return `<span class="mstrip">${counts.map(([s, n]) => `<i style="flex:${n};background:${COLOR(s)}"></i>`).join('')}</span>`;
}

export function runRow(run) {
  const done = run.changes.filter(c => c.from !== null && c.to === 'done').length;
  const added = run.changes.filter(c => c.from === null).length;
  const moved = run.changes.length - done - added;
  const summary = [done && `${done} done`, added && `${added} new`, moved && `${moved} moved`].filter(Boolean).join(' · ');
  const cells = run.changes.map(c => `<i style="background:${c.from === null ? 'var(--fg-4)' : COLOR(c.to)}"></i>`).join('');
  return `<div class="mrun"><span class="t">${when(run.date)}</span><span class="h">${escape(run.commit ?? '')}</span>` +
    `<span class="s">${escape(run.subject)}</span><span class="x"><span class="cells">${cells}</span>${summary}</span></div>`;
}

// The board, schematic: header, status strip with counts, Needs you and Up next.
export function board(data, l) {
  const tasks = data.tasks;
  const decide = tasks.filter(t => ['decision', 'blocked'].includes(t.status)).sort((a, b) => b.unlocks - a.unlocks);
  // Most recently moved first, as the board orders To check; the schematic shows two and counts the rest.
  const review = tasks.filter(t => t.status === 'review').sort((a, b) => (b.lastRun ?? -1) - (a.lastRun ?? -1));
  const check = review.slice(0, 2);
  const next = tasks.filter(t => t.ready).sort((a, b) => b.unlocks - a.unlocks || a.id.localeCompare(b.id));
  const counts = ['done', 'review', 'in-progress', 'open', 'decision', 'blocked']
    .map(s => [s, tasks.filter(t => t.status === s).length]).filter(([, n]) => n);
  const lever = t => (t.unlocks ? `unlocks ${t.unlocks}` : '');
  const html = `<div class="sb-top"><span class="proj">${escape(data.project)}</span>
      <span class="sb-tabs"><b>${escape(l.tabs[0])}</b>${l.tabs.slice(1).map(t => `<span>${escape(t)}</span>`).join('')}</span></div>
    <div class="sb-sum">${strip(tasks)}<div class="sb-counts">${counts.map(([s, n]) =>
      `<span><b>${n}</b>${mark(s)}${escape(word(s))}</span>`).join('')}</div></div>
    <div class="sb-grid">
      <div class="sb-panel"><div class="sb-h">${escape(l.needs)}</div>
        ${group(l.decide, decide.length)}${decide.map(t => row(t, { meta: lever(t) })).join('')}
        ${group(l.check, review.length)}${check.map(t => row(t)).join('')}
        ${review.length > check.length ? `<div class="mmore">${escape(l.more.replace('{n}', review.length - check.length))}</div>` : ''}</div>
      <div class="sb-panel"><div class="sb-h">${escape(l.next)}</div>
        ${next.map(t => row(t, { meta: t.goal[0] ?? '' })).join('')}</div>
    </div>`;
  const label = `${l.alt}: ${l.needs}, ${[...decide, ...review].map(t => `${t.id} ${word(t.status)}`).join(', ')}. ` +
    `${l.next}, ${next.map(t => t.id).join(', ')}.`;
  return picture(label, html, 'panel sboard');
}

// An orchestrator thread, its stream threads and the worker sessions under each, as in a bb sidebar.
export function tree(data, spec) {
  const byId = new Map(data.tasks.map(t => [t.id, t]));
  const goals = new Map(data.goals.map(g => [g.name, g]));
  const task = id => {
    const t = byId.get(id);
    if (!t) {
      throw Error(`schematic: ${id} is not in the sample`);
    }
    return t;
  };
  const lines = [`<div class="ot-root">${mark('in-progress')}<span class="nm">${escape(spec.root)}</span>` +
    `<span class="tag">${escape(spec.rootTag)}</span></div>`];
  const said = [`${spec.root}`];
  for (const s of spec.streams) {
    const g = s.goal ? goals.get(s.goal) : null;
    if (s.goal && !g) {
      throw Error(`schematic: goal ${s.goal} is not in the sample`);
    }
    // A stream is named after its goal unless the spec names it; its mark follows the goal's state.
    const name = s.name ?? `${g.stream} ${g.title.replace(/\s*\([^)]*\)$/, '')}`;
    lines.push(`<div class="ot-stream">${mark(s.mark ?? (g ? GOAL[g.state] : 'decision'))}` +
      `<span class="nm">${escape(name)}</span>` +
      `<span class="tag">${escape(s.tag)}</span></div>`);
    said.push(`${name} (${s.tag})`);
    for (const [who, id] of s.workers ?? []) {
      const t = task(id);
      lines.push(`<div class="ot-worker"><span class="dot"></span><span class="who">${escape(who)}</span>` +
        `${mark(t.status)}<span class="id">${escape(t.id)}</span><span class="tt">${escape(t.title)}</span></div>`);
      said.push(`${who} on ${t.id}`);
    }
    for (const id of s.tasks ?? []) {
      const t = task(id);
      lines.push(`<div class="ot-worker">${mark(t.status)}<span class="id">${escape(t.id)}</span>` +
        `<span class="tt">${escape(t.title)}</span></div>`);
      said.push(`${t.id} ${word(t.status)}`);
    }
  }
  return picture(`${spec.alt}: ${said.join(', ')}.`, lines.join(''), 'otree');
}

// The planning repo beside the project repo it plans. The plan's side is the sample's own files with
// their marks; the code's side is named in the copy. Agents work between them, the person at the board.
export function repos(data, spec, name) {
  const byId = new Map(data.tasks.map(t => [t.id, t]));
  const goal = data.goals.find(g => g.name === spec.plan.goal);
  if (!goal) {
    throw Error(`schematic: goal ${spec.plan.goal} is not in the sample`);
  }
  const shown = spec.plan.tasks.map(id => {
    if (!byId.has(id)) {
      throw Error(`schematic: ${id} is not in the sample`);
    }
    return byId.get(id);
  });
  const dir = (label, n) => `<li class="d"><span class="fn">${escape(label)}</span>${n === undefined ? '' : `<span class="n">${n}</span>`}</li>`;
  const file = (label, m = '', meta = '', cls = 'f in') =>
    `<li class="${cls}">${m}<span class="fn">${escape(label)}</span>${meta ? `<span class="m">${escape(meta)}</span>` : ''}</li>`;
  const plan = [
    dir('tasks/', data.tasks.length),
    ...shown.map(t => file(`${t.id}.md`, mark(t.status), word(t.status))),
    `<li class="f in more">${escape(spec.plan.more.replace('{n}', data.tasks.length - shown.length))}</li>`,
    dir('plans/', data.goals.length),
    file(goal.file.replace(/^.*\//, ''), mark(GOAL[goal.state]), goal.state),
    file('planr.config.json', '', '', 'f'),
  ].join('');
  const code = spec.code.entries.map(([label, note]) =>
    label.endsWith('/') ? `<li class="d"><span class="fn">${escape(label)}</span>${note ? `<span class="m">${escape(note)}</span>` : ''}</li>`
      : file(label, '', note, 'f')
  ).join('');
  const side = (cls, label, repo, list) => `<div class="panel repo ${cls}"><div class="ph"><span class="lbl">${escape(label)}</span>` +
    `<span class="c">${escape(repo)}</span></div><ul class="ft">${list}</ul></div>`;
  const html = side('plan', spec.plan.label, `${name}/`, plan) +
    `<div class="bridge"><span class="who">${escape(spec.agents.label)}</span>` +
    `<span class="arr to-plan"><i></i>${escape(spec.agents.read)}</span>` +
    `<span class="arr to-code">${escape(spec.agents.build)}<i></i></span></div>` +
    side('code', spec.code.label, spec.code.name, code) +
    `<div class="you"><i></i><b>${escape(spec.you.label)}</b> ${escape(spec.you.body)}</div>`;
  const label = `${spec.alt}: ${name}/ holds ${data.tasks.length} tasks & ${data.goals.length} goals, ` +
    `${spec.code.name} holds ${spec.code.entries.map(([l]) => l).join(', ')}. ` +
    `${spec.agents.label} ${spec.agents.read} the plan & ${spec.agents.build} in the code. ` +
    `${spec.you.label} ${spec.you.body}.`;
  return picture(label, html, 'repos');
}
