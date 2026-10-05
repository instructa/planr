import { escape, inline } from './markdown.mjs';
import { icon, videoTag } from './layout.mjs';
import { mark } from './marks.mjs';
import { board, button, goalRow, group, picture, repos, row, runRow, tree, word } from './schematic.mjs';

// Landing copy allows `code` spans and nothing else.
const md = s => inline(s, href => ({ href }));

const copyButton = (text, label = 'Copy command') =>
  `<button class="copy" type="button" data-copy="${escape(text)}" aria-label="${escape(label)}">${icon.copy}${icon.check}</button>`;

const command = (text, prompt = '$') =>
  `<div class="line"><code><span class="ps" aria-hidden="true">${escape(prompt)} </span>${escape(text)}</code>${copyButton(text)}</div>`;

// A prompt is what people type to an agent: a person's sentence, so it is set in sans.
// A prompt that calls the skill, { skill, plain }, follows the agent picked on the page (site.js) and
// is rendered for the first one. A string is a follow-up in the same conversation and stays as it is.
const prompt = agent => p => {
  if (typeof p === 'string') {
    return `<div class="pq"><span class="p" aria-hidden="true">›</span><span class="pt">${md(p)}</span>${copyButton(p, 'Copy prompt')}</div>`;
  }
  const text = agent.invoke ? `${agent.invoke} ${p.skill}` : p.plain;
  const html = agent.invoke ? `<span class="inv">${escape(agent.invoke)}</span> ${escape(p.skill)}` : escape(p.plain);
  return `<div class="pq" data-skill="${escape(p.skill)}" data-plain="${escape(p.plain)}"><span class="p" aria-hidden="true">›</span>` +
    `<span class="pt">${html}</span>${copyButton(text, 'Copy prompt')}</div>`;
};

// The agent picker: radios that carry each agent's invocation and hint for site.js.
const agentPicker = agents => `<fieldset class="seg agents"><legend>${escape(agents.label)}</legend><div class="opts">${agents.list.map((a, k) =>
  `<label><input type="radio" name="agent" value="${a.id}"${a.invoke ? ` data-invoke="${escape(a.invoke)}"` : ''} data-hint="${escape(md(a.hint))}"${k === 0 ? ' checked' : ''}><span>${escape(a.name)}</span></label>`
).join('')}</div><p class="ahint" aria-live="polite">${md(agents.list[0].hint)}</p></fieldset>`;

const more = (link, out = false) =>
  `<a class="more" href="${link.href}"${out ? ' rel="noopener"' : ''}>${escape(link.label)}${out ? icon.out : '<span aria-hidden="true"> →</span>'}</a>`;

// Lines of real output in the board's two voices: the prefix steps back, task IDs step forward.
// --i orders the lines for the printing animation.
const line = (text, emphasis, i) => {
  const em = emphasis.some(e => text.includes(e));
  const html = escape(text)
    .replace(/^planr:/, '<span class="pfx">planr:</span>')
    .replace(/\b([A-Z]{2,}-\d+)\b/g, '<span class="tid">$1</span>');
  return `<span class="l${em ? ' em' : ''}" style="--i:${i}">${html || ' '}</span>`;
};

function terminal(steps, emphasis) {
  let i = 0;
  return `<pre class="session">${steps.map(step =>
    `<span class="l cmd" style="--i:${i++}"><span class="ps" aria-hidden="true">$ </span>${escape(step.command)}</span>` +
    step.output.map(text => line(text, emphasis, i++)).join('')
  ).join('')}</pre>`;
}

const panel = (label, note, body, cls = '') => `<div class="panel ${cls}">
  <div class="ph"><span class="lbl">${escape(label)}</span>${note ? `<span class="c">${escape(note)}</span>` : ''}</div>
  ${body}
</div>`;

// Two ways in: one prompt a person pastes into their agent, or the install tabs for people.
// Both modes and all tabs share one grid cell, so switching never changes the box's height.
function heroInstall(install, platforms) {
  const [agentsLabel, humansLabel] = install.modes;
  const a = install.agents;
  const tab = (cls, id, controls, label, on, extra = '') =>
    `<button class="${cls}" type="button" role="tab" id="${id}" aria-controls="${controls}" aria-selected="${on}" tabindex="${on ? 0 : -1}"${extra}>${escape(label)}</button>`;
  const tabs = platforms.map((p, k) =>
    tab('itab', `itab-${p.id}`, `ipanel-${p.id}`, p.name, k === 0, p.agent ? ` data-agent="${p.agent}"` : '')
  ).join('');
  const panels = platforms.map((p, k) => {
    const [first, ...rest] = p.commands ?? [];
    const body = !p.commands
      ? `<p class="itext">${md(p.body)}</p>`
      : rest.length > 1
        ? `${command(first, p.prompt)}<p class="inote">Or ${rest.map(c => `<code>${escape(c)}</code>`).join(' or ')}. ${md(p.note ?? '')}</p>`
        : p.commands.map(c => command(c, p.prompt)).join('') + (p.note ? `<p class="inote">${md(p.note)}</p>` : '');
    const foot = `<p class="iwhere">${p.where ? `<span>${escape(p.where)}</span>` : ''}${more(p.link)}</p>`;
    return `<div class="ipanel" role="tabpanel" id="ipanel-${p.id}" aria-labelledby="itab-${p.id}"${k === 0 ? '' : ' hidden'}>${body}${foot}</div>`;
  }).join('');
  return `<div class="panel install" id="install">
    <div class="ph"><h2 class="lbl" id="install-label">${escape(install.label)}</h2>
      <div class="modes" role="tablist" aria-labelledby="install-label">${tab('mode', 'imode-agents', 'imodep-agents', agentsLabel, true)}${tab('mode', 'imode-humans', 'imodep-humans', humansLabel, false)}</div></div>
    <div class="imodes">
      <div class="imode" role="tabpanel" id="imodep-agents" aria-labelledby="imode-agents">
        <div class="say"><span class="p" aria-hidden="true">›</span><span class="pt">${escape(a.prompt)}</span>${copyButton(a.prompt, 'Copy prompt')}</div>
        <p class="itext">${md(a.body)}</p>
        <p class="iwhere"><span>${escape(a.where)}</span>${more(a.link)}</p>
      </div>
      <div class="imode" role="tabpanel" id="imodep-humans" aria-labelledby="imode-humans" hidden>
        <div class="itabs" role="tablist" aria-label="${escape(humansLabel)}">${tabs}</div>
        <div class="ipanels">${panels}</div>
      </div>
    </div>
  </div>`;
}

// JSON for a WebGL scene, read by its script (assets/graph.js).
const sceneData = (id, value) =>
  `<script type="application/json" id="${id}">${JSON.stringify(value).replace(/</g, '\\u003c')}</script>`;

// The journey's pictures: schematic board pieces for the sample, one per step.
function visual(v, data, ui) {
  const byId = new Map(data.tasks.map(t => [t.id, t]));
  const goals = new Map(data.goals.map(g => [g.name, g]));
  const task = id => {
    if (!byId.has(id)) {
      throw Error(`landing: ${id} is not in the sample`);
    }
    return byId.get(id);
  };
  const goal = name => {
    if (!goals.has(name)) {
      throw Error(`landing: goal ${name} is not in the sample`);
    }
    return goals.get(name);
  };
  const head = text => `<div class="sb-h">${escape(text)}</div>`;
  const commit = message => `<div class="mcommit"><span class="h">${escape(ui.commit)}</span><span class="s">${escape(message)}</span></div>`;
  const html = {
    plan: () => v.tasks.map(([id, status]) => {
      const t = task(id);
      return row(t, { status, meta: t.depends.length ? `depends ${t.depends.join(', ')}` : '' });
    }).join('') + '<div class="msep"></div>' + goalRow(goal(v.goal), { state: 'draft', meta: 'draft' }),
    // Approving & checking happen in the conversation; the board's button is the other way, so it
    // stays secondary. The commit line is the agent's record that a person asked.
    approve: () => head(ui.needs) + group(ui.goals, 1) +
      goalRow(goal(v.goal), { state: 'draft', extra: button(ui.approve) }) +
      goalRow(goal(v.goal), { state: 'approved', tag: 'approved', cls: 'after' }) + commit(v.commit),
    work: () => `<div class="mflow">${v.statuses.map(s => `<span class="mchip">${mark(s)}${escape(word(s))}</span>`)
      .join('<span class="arr" aria-hidden="true">→</span>')}</div>` + row(task(v.task), { status: v.statuses[1] }),
    decide: () => head(ui.needs) + group(ui.decide, 1) +
      row(task(v.from), { status: 'decision', meta: 'unlocks 1' }) +
      '<div class="medge"><span></span>depends</div>' +
      row(task(v.to), { status: 'open', tag: ui.ready, cls: 'wash' }),
    check: () => head(ui.needs) + group(ui.check, 1) +
      row(task(v.task), { status: 'review', extra: button(ui.checked) }) +
      row(task(v.task), { status: 'done', cls: 'after' }) + commit(v.commit),
    // New work: tasks that join an existing goal, then a next goal with its tasks, as a draft.
    grow: () => {
      const fresh = ([id, title, depends = []]) => {
        if (byId.has(id)) {
          throw Error(`landing: ${id} is already in the sample`);
        }
        depends.forEach(task);
        return row({ id, title }, { status: 'open', tag: ui.new, cls: 'in', meta: depends.length ? `depends ${depends.join(', ')}` : '' });
      };
      if (goals.has(v.next.name)) {
        throw Error(`landing: goal ${v.next.name} is already in the sample`);
      }
      return goalRow(goal(v.goal)) + v.tasks.map(fresh).join('') + '<div class="msep"></div>' +
        goalRow({ name: v.next.name, title: v.next.title }, { state: 'draft', meta: 'draft' }) + v.next.tasks.map(fresh).join('');
    },
    runs: () => head(ui.runs) + [...data.runs].reverse().filter(r => !r.initial).slice(0, v.count).map(runRow).join(''),
  }[v.kind];
  if (!html) {
    throw Error(`landing: unknown visual ${v.kind}`);
  }
  return picture(v.alt, html(), `panel mini m-${v.kind}`);
}

// A block on the landing: text beside (or above) a schematic. The status-mark field of the hero
// continues behind them in the page margins (assets/header.js), with a soft halo around each picture.
const block = ({ id, cls = '', text, picture }) => `<section class="block${cls ? ` ${cls}` : ''}" id="${id}" aria-labelledby="${id}-h">
    ${text}
    <div class="visual">${picture}</div>
  </section>`;

export function landing(copy, run, sample, media) {
  const { hero, install, platforms, video, ui, session, how, orchestrate: o, walk, close } = copy;
  const where = `~/${session.repo} · ${session.note}`;
  const file = `<pre class="session file">${run.file.map((text, i) => line(text, how.file.emphasis, i)).join('')}</pre>`;
  const txt = (id, part, extra = '') => `<div class="txt">
        <h2 id="${id}-h">${md(part.heading)}</h2>
        ${part.body.map(p => `<p>${md(p)}</p>`).join('')}
        ${extra}${part.link ? more(part.link) : ''}
      </div>`;

  return `<main id="main" class="landing" tabindex="-1">
  <section class="hero" aria-labelledby="hero-h">
    <h1 id="hero-h">${md(hero.heading)}</h1>
    <p class="lead">${md(hero.lead)}</p>
    ${heroInstall(install, platforms)}
    <p class="hlinks">${hero.links.map(l => more(l)).join('')}</p>
  </section>
${media.video ? `
  <section class="video" aria-labelledby="video-h">
    <h2 id="video-h" class="vh">${md(video.heading)}</h2>
    <div class="panel vframe">
      ${videoTag(media.video, video.title)}
    </div>
  </section>` : ''}
  <div class="blocks">
  ${block({
    id: 'ready', cls: 'wide',
    text: txt('ready', how.ready, `<p class="open"><a class="more" href="/sample/board.html">${escape(how.ready.open)}${icon.out}</a></p>`),
    picture: `<figure class="schem">
        ${board(sample.data, ui)}
        ${panel('Terminal', where, terminal(run.ready, how.ready.emphasis), 'term accent')}
      </figure>`,
  })}
  ${block({ id: 'repos', cls: 'flip', text: txt('repos', how.repos), picture: repos(sample.data, how.repos, session.repo) })}
  ${block({ id: 'files', text: txt('files', how.file), picture: panel('File', how.file.path, file, 'filep') })}
  ${block({
    id: 'orchestrate', cls: 'flip',
    text: txt('orchestrate', o, `<dl class="roles">${o.roles.map(([name, text]) => `<div><dt>${escape(name)}</dt><dd>${md(text)}</dd></div>`).join('')}</dl>
      <h3 class="rec">${escape(o.recommend)}</h3>
      <ul class="companions">${o.companions.map(c => `<li>${more({ label: c.name, href: c.href }, true)}<span>${md(c.body)}</span></li>`).join('')}</ul>`),
    picture: `<div class="panel threads">
      <div class="ph"><span class="lbl">${escape(o.panel)}</span><span class="c">${escape(o.panelNote)}</span></div>
      ${tree(sample.data, o.tree)}
    </div>`,
  })}
  <section class="walk" aria-labelledby="walk-h">
    <div><h2 id="walk-h">${md(walk.heading)}</h2><p>${md(walk.body)}</p></div>
    <a class="btn" href="${walk.link.href}">${escape(walk.link.label)}<span aria-hidden="true">→</span></a>
  </section>
  </div>

  <section class="closing" aria-labelledby="close-h">
    <div class="center">
      <h2 id="close-h">${md(close.heading)}</h2>
      <p>${md(close.body)}</p>
      <div class="actions">${close.links.map(l => `<a class="btn${l.primary ? ' primary' : ''}" href="${l.href}">${escape(l.label)}</a>`).join('')}</div>
    </div>
    ${sceneData('graph-data', sample.graph)}
  </section>
</main>`;
}

// How it works: one goal from plan to done, prompt by prompt, with the agent picker the prompts follow.
export function walkthrough(copy, sample) {
  const { agents, journey, ui, walkthrough: w } = copy;
  const ask = prompt(agents.list[0]);
  return `<main id="main" class="landing walkthrough" tabindex="-1">
  <section class="journey" aria-labelledby="journey-h">
    <h1 id="journey-h">${md(journey.heading)}</h1>
    <p class="lead">${md(journey.lead)}</p>
    ${agentPicker(agents)}
    <ol class="jsteps">
      ${journey.steps.map((s, i) => `<li class="jstep${s.visual ? '' : ' plain'}">
        <span class="jn" aria-hidden="true">${i + 1}</span>
        <div class="jtxt">
          <h2>${md(s.title)}</h2>
          <p>${md(s.body)}</p>
          ${[s.prompt ?? []].flat().map(ask).join('')}
          ${s.note ? `<p class="jnote">${md(s.note)}</p>` : ''}
          ${s.link ? more(s.link) : ''}
        </div>
        ${s.visual ? `<div class="jvis">${visual(s.visual, sample.data, ui)}</div>` : ''}
      </li>`).join('')}
    </ol>
  </section>

  <section class="next" aria-labelledby="next-h">
    <h2 id="next-h">${md(w.next.heading)}</h2>
    <p>${md(w.next.body)}</p>
    <div class="actions">${w.next.links.map(l => `<a class="btn${l.primary ? ' primary' : ''}" href="${l.href}">${escape(l.label)}</a>`).join('')}</div>
  </section>
</main>`;
}
