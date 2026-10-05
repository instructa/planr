// planr.so: theme, copy, install tabs, the agent prompts are written for, the printing terminal,
// the quickstart chooser & the docs table of contents.
// Without this file the page still reads: every prompt for the first agent, every quickstart path.

const root = document.documentElement;
const media = matchMedia('(prefers-color-scheme: dark)');
const motion = matchMedia('(prefers-reduced-motion: no-preference)');

// ---------- theme: the board's toggle, remembered ----------
const SUN = '<svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="3.2" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M8 1.5v1.6M8 12.9v1.6M1.5 8h1.6M12.9 8h1.6M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M3.4 12.6l1.1-1.1M11.5 4.5l1.1-1.1" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>';
const MOON = '<svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true"><path d="M13.2 10.2A5.6 5.6 0 0 1 5.8 2.8a5.6 5.6 0 1 0 7.4 7.4z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/></svg>';
const isDark = () => root.dataset.theme ? root.dataset.theme === 'dark' : media.matches;
const themeButton = document.querySelector('.theme');

function paintTheme() {
  themeButton.innerHTML = isDark() ? SUN : MOON;
  themeButton.setAttribute('aria-pressed', String(isDark()));
}

themeButton.addEventListener('click', () => {
  root.dataset.theme = isDark() ? 'light' : 'dark';
  try {
    localStorage.setItem('planr-theme', root.dataset.theme);
  } catch {}
  paintTheme();
});
media.addEventListener('change', paintTheme);
paintTheme();

// ---------- copy ----------
const live = Object.assign(document.createElement('div'), { className: 'sr', role: 'status' });
live.setAttribute('aria-live', 'polite');
live.style.cssText = 'position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%);white-space:nowrap';
document.body.append(live);

// Shell, prompt & snippet blocks in the docs get the same copy button as the landing's commands.
// Shell comments stay behind.
for (const pre of document.querySelectorAll('.prose pre.code:is([data-lang="sh"], [data-lang="text"], [data-lang="markdown"])')) {
  const sh = pre.dataset.lang === 'sh';
  const text = pre.textContent.split('\n').filter(l => !sh || !/^\s*#/.test(l)).join('\n').trim();
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'copy';
  button.dataset.copy = text;
  button.setAttribute('aria-label', sh ? 'Copy commands' : 'Copy');
  button.innerHTML = document.querySelector('.copy')?.innerHTML ??
    '<svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true"><rect x="5.25" y="5.25" width="8" height="8" rx="1.75" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M10.5 3.2v-.2a1.25 1.25 0 0 0-1.25-1.25h-5.5A1.75 1.75 0 0 0 2 3.5v5.5a1.25 1.25 0 0 0 1.25 1.25h.2" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg><svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.4l3 3 6-6.3" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>';
  pre.append(button);
}

document.addEventListener('click', async e => {
  const button = e.target.closest('[data-copy]');
  if (!button) {
    return;
  }
  try {
    await navigator.clipboard.writeText(button.dataset.copy);
  } catch {
    const area = Object.assign(document.createElement('textarea'), { value: button.dataset.copy });
    area.style.cssText = 'position:fixed;opacity:0';
    document.body.append(area);
    area.select();
    document.execCommand('copy');
    area.remove();
  }
  button.classList.add('done');
  live.textContent = 'Copied';
  clearTimeout(button.timer);
  button.timer = setTimeout(() => {
    button.classList.remove('done');
    live.textContent = '';
  }, 1600);
});

// ---------- the agent every prompt is written for, remembered ----------
// Picking an agent in the journey, an install tab or the quickstart rewrites every prompt that calls
// the skill: with the agent's invocation, or in plain words for agents without one.
const AGENT = 'planr-agent';
const stored = key => {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
};
const store = (key, value) => {
  try {
    localStorage.setItem(key, value);
  } catch {}
};
const tabSelect = new Map();

function setAgent(id, save = true) {
  const radio = document.querySelector(`input[name="agent"][value="${CSS.escape(id)}"]`);
  if (radio) {
    radio.checked = true;
    const invoke = radio.dataset.invoke;
    for (const pq of document.querySelectorAll('.pq[data-skill]')) {
      const pt = pq.querySelector('.pt');
      pt.textContent = invoke ? ` ${pq.dataset.skill}` : pq.dataset.plain;
      if (invoke) {
        pt.prepend(Object.assign(document.createElement('span'), { className: 'inv', textContent: invoke }));
      }
      pq.querySelector('.copy').dataset.copy = invoke ? `${invoke} ${pq.dataset.skill}` : pq.dataset.plain;
    }
    const hint = radio.closest('.agents')?.querySelector('.ahint');
    if (hint) {
      hint.innerHTML = radio.dataset.hint;
    }
  }
  const choice = document.querySelector(`input[name="qs-agent"][value="${CSS.escape(id)}"]`);
  if (choice) {
    choice.checked = true;
  }
  const tab = document.querySelector(`.itab[data-agent="${CSS.escape(id)}"]`);
  if (tab && tab.getAttribute('aria-selected') !== 'true') {
    tabSelect.get(tab.closest('[role="tablist"]'))(tab, false, false);
  }
  if (save) {
    store(AGENT, id);
  }
}

for (const radio of document.querySelectorAll('input[name="agent"]')) {
  radio.addEventListener('change', () => setAgent(radio.value));
}

// ---------- install tabs: the two modes & the tabs for people ----------
for (const list of document.querySelectorAll('.install [role="tablist"]')) {
  const tabs = [...list.querySelectorAll('[role="tab"]')];
  const select = (tab, focus, pick = true) => {
    for (const t of tabs) {
      const on = t === tab;
      t.setAttribute('aria-selected', String(on));
      t.tabIndex = on ? 0 : -1;
      document.getElementById(t.getAttribute('aria-controls')).hidden = !on;
    }
    if (focus) {
      tab.focus();
    }
    if (pick && tab.dataset.agent) {
      setAgent(tab.dataset.agent);
    }
  };
  tabSelect.set(list, select);
  list.addEventListener('click', e => {
    const tab = e.target.closest('[role="tab"]');
    if (tab) {
      select(tab);
    }
  });
  list.addEventListener('keydown', e => {
    const i = tabs.indexOf(document.activeElement);
    const next = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: tabs.length - 1 }[e.key];
    if (next !== undefined) {
      e.preventDefault();
      select(tabs[(next + tabs.length) % tabs.length], true);
    }
  });
}

// ---------- docs: the quickstart shows only the chosen path ----------
// Sections carry their conditions (data-show="agent=codex"); the chooser's answers hide the others.
const chooser = document.querySelector('.chooser');
if (chooser) {
  let saved = {};
  try {
    saved = JSON.parse(stored('planr-quickstart') ?? '{}');
  } catch {}
  for (const [key, value] of Object.entries({ ...saved, agent: stored(AGENT) })) {
    const input = value && chooser.querySelector(`input[name="qs-${key}"][value="${CSS.escape(value)}"]`);
    if (input) {
      input.checked = true;
    }
  }
  const show = () => {
    const chosen = Object.fromEntries(new FormData(chooser));
    for (const part of document.querySelectorAll('.part[data-show]')) {
      part.hidden = !part.dataset.show.split(/\s+/).every(pair => {
        const [key, value] = pair.split('=');
        return chosen[`qs-${key}`] === value;
      });
    }
    for (const a of document.querySelectorAll('.toc a')) {
      a.parentElement.hidden = !!document.getElementById(decodeURIComponent(a.hash.slice(1)))?.closest('.part[hidden]');
    }
  };
  chooser.addEventListener('change', e => {
    const key = e.target.name.replace(/^qs-/, '');
    if (key === 'agent') {
      store(AGENT, e.target.value);
    } else {
      store('planr-quickstart', JSON.stringify({ ...saved, [key]: e.target.value }));
      saved[key] = e.target.value;
    }
    show();
  });
  chooser.hidden = false;
  show();
}

// The remembered agent applies once everything that follows it is in place.
const remembered = stored(AGENT);
if (remembered && document.querySelector(`input[name="agent"][value="${CSS.escape(remembered)}"]`)) {
  setAgent(remembered, false);
}

// ---------- the session prints once as each panel comes into view ----------
if (motion.matches && 'IntersectionObserver' in window) {
  const io = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (entry.isIntersecting) {
        entry.target.classList.replace('pending', 'play');
        io.unobserve(entry.target);
      }
    }
  }, { rootMargin: '0px 0px -32px 0px' });
  for (const session of document.querySelectorAll('.how .term .session')) {
    // Only panels still below the fold wait; anything already visible stays as it is.
    if (session.getBoundingClientRect().top > innerHeight) {
      session.classList.add('pending');
      io.observe(session);
    }
  }
}

// ---------- docs: the table of contents follows the reading position ----------
const toc = [...document.querySelectorAll('.toc a')];
if (toc.length && 'IntersectionObserver' in window) {
  const headings = toc.map(a => document.getElementById(decodeURIComponent(a.hash.slice(1)))).filter(Boolean);
  const mark = id => toc.forEach(a => a.setAttribute('aria-current', String(a.hash === `#${id}`)));
  // Headings in sections the quickstart chooser hides have no box and never count.
  const shown = () => headings.filter(h => h.offsetParent);
  const spy = new IntersectionObserver(() => {
    const above = shown().filter(h => h.getBoundingClientRect().top < innerHeight * 0.3);
    mark((above.at(-1) ?? shown()[0] ?? headings[0]).id);
  }, { rootMargin: '0px 0px -70% 0px', threshold: [0, 1] });
  headings.forEach(h => spy.observe(h));
  addEventListener('scroll', () => {
    if (innerHeight + scrollY >= document.documentElement.scrollHeight - 4) {
      mark((shown().at(-1) ?? headings.at(-1)).id);
    }
  }, { passive: true });
}
