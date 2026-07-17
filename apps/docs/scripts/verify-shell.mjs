import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import axe from 'axe-core';

const docsRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = dirname(dirname(docsRoot));
const outputDir = join(repositoryRoot, '.planr', 'artifacts', 'docs-shell');
const baseUrl = process.env.PLANR_DOCS_URL ?? 'http://localhost:3000';
const chromePath = process.env.CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const profileDir = await mkdtemp(join(tmpdir(), 'planr-docs-chrome-'));

await mkdir(outputDir, { recursive: true });

const chrome = spawn(chromePath, [
  '--headless=new',
  '--remote-debugging-port=0',
  `--user-data-dir=${profileDir}`,
  '--no-first-run',
  '--no-default-browser-check',
  '--disable-background-networking',
  '--disable-component-update',
  '--disable-default-apps',
  '--disable-extensions',
  '--disable-sync',
  '--hide-scrollbars',
  `${baseUrl}/`,
], { stdio: 'ignore' });

const results = {
  pages: [],
  interactions: [],
  accessibility: [],
  screenshots: [],
};

async function terminate(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise((resolve) => child.once('exit', resolve));
  child.kill('SIGTERM');
  const graceful = await Promise.race([
    exited.then(() => true),
    new Promise((resolve) => setTimeout(() => resolve(false), 5_000)),
  ]);
  if (graceful || child.exitCode !== null || child.signalCode !== null) return;
  child.kill('SIGKILL');
  await exited;
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function waitFor(getValue, description, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await getValue();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Timed out waiting for ${description}`);
}

class Cdp {
  constructor(url) {
    this.socket = new WebSocket(url);
    this.nextId = 0;
    this.pending = new Map();
    this.listeners = new Map();
  }

  async open() {
    await new Promise((resolve, reject) => {
      this.socket.addEventListener('open', resolve, { once: true });
      this.socket.addEventListener('error', reject, { once: true });
    });
    this.socket.addEventListener('message', (event) => {
      const message = JSON.parse(event.data);
      if (message.id) {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        if (message.error) pending.reject(new Error(message.error.message));
        else pending.resolve(message.result);
        return;
      }
      const listeners = this.listeners.get(message.method) ?? [];
      this.listeners.delete(message.method);
      for (const listener of listeners) listener(message.params);
    });
  }

  send(method, params = {}) {
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  event(method, timeoutMs = 15_000) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`Timed out waiting for ${method}`)), timeoutMs);
      const listeners = this.listeners.get(method) ?? [];
      listeners.push((params) => {
        clearTimeout(timer);
        resolve(params);
      });
      this.listeners.set(method, listeners);
    });
  }

  close() {
    this.socket.close();
  }
}

let cdp;

async function evaluate(expression) {
  const response = await cdp.send('Runtime.evaluate', {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  if (response.exceptionDetails) {
    throw new Error(response.exceptionDetails.exception?.description ?? response.exceptionDetails.text);
  }
  return response.result.value;
}

async function navigate(path) {
  const loaded = cdp.event('Page.loadEventFired');
  await cdp.send('Page.navigate', { url: `${baseUrl}${path}` });
  await loaded;
  await waitFor(
    () => evaluate(`document.readyState === 'complete' && document.body?.innerText.length > 0`),
    `${path} content`,
  );
  // Next.js hydration completes immediately after the load event; allow client handlers to attach.
  await new Promise((resolve) => setTimeout(resolve, 600));
  const page = await evaluate(`({ path: location.pathname, title: document.title, h1: document.querySelector('h1')?.textContent?.trim() })`);
  results.pages.push(page);
  return page;
}

async function click(selector) {
  const target = await evaluate(`(() => {
    const element = document.querySelector(${JSON.stringify(selector)});
    if (!element) return null;
    element.scrollIntoView({ block: 'center', inline: 'center' });
    const rect = element.getBoundingClientRect();
    const x = rect.x + rect.width / 2;
    const y = rect.y + rect.height / 2;
    const hit = document.elementFromPoint(x, y);
    return {
      x,
      y,
      visible: rect.width > 0 && rect.height > 0,
      targetIsHit: hit === element || element.contains(hit),
      hit: hit?.outerHTML?.slice(0, 180)
    };
  })()`);
  assert(target?.visible, `Visible click target not found: ${selector}`);
  assert(target.targetIsHit, `Click target is covered for ${selector}: ${target.hit}`);
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: target.x, y: target.y });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: target.x, y: target.y, button: 'left', buttons: 1, clickCount: 1 });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: target.x, y: target.y, button: 'left', buttons: 0, clickCount: 1 });
}

async function press(key, modifiers = 0) {
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key, code: key === 'k' ? 'KeyK' : key, modifiers });
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code: key === 'k' ? 'KeyK' : key, modifiers });
}

async function screenshot(name) {
  const path = join(outputDir, `${name}.png`);
  const capture = await cdp.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true });
  await writeFile(path, Buffer.from(capture.data, 'base64'));
  results.screenshots.push(path);
}

async function runAxe(path) {
  await navigate(path);
  await evaluate(axe.source);
  const audit = await evaluate(`axe.run(document, {
    runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] },
    resultTypes: ['violations']
  }).then(({ violations }) => violations.map(({ id, impact, help, nodes }) => ({
    id,
    impact,
    help,
    nodes: nodes.map(({ target, html, failureSummary }) => ({ target, html, failureSummary }))
  })))`);
  const serious = audit.filter((violation) => violation.impact === 'serious' || violation.impact === 'critical');
  results.accessibility.push({ path, violations: audit, seriousOrCritical: serious.length });
  assert(serious.length === 0, `${path} has serious/critical axe violations: ${JSON.stringify(serious)}`);
}

try {
  const activePortFile = join(profileDir, 'DevToolsActivePort');
  const [port, browserPath] = (await waitFor(
    async () => {
      try {
        return (await readFile(activePortFile, 'utf8')).trim().split('\n');
      } catch {
        return null;
      }
    },
    'Chrome DevTools endpoint',
  ));

  const targets = await waitFor(
    async () => {
      const response = await fetch(`http://127.0.0.1:${port}/json/list`);
      const entries = await response.json();
      return entries.find((entry) => entry.type === 'page') ? entries : null;
    },
    'Chrome page target',
  );
  const target = targets.find((entry) => entry.type === 'page');
  cdp = new Cdp(target.webSocketDebuggerUrl ?? `ws://127.0.0.1:${port}${browserPath}`);
  await cdp.open();
  await cdp.send('Page.enable');
  await cdp.send('Page.bringToFront');
  await cdp.send('Runtime.enable');
  await cdp.send('DOM.enable');
  await cdp.send('Browser.grantPermissions', {
    origin: baseUrl,
    permissions: ['clipboardReadWrite', 'clipboardSanitizedWrite'],
  });
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 1440,
    height: 980,
    deviceScaleFactor: 1,
    mobile: false,
  });

  const home = await navigate('/');
  assert(home.h1?.includes('Give every agent a plan.'), 'Homepage hero heading is missing');
  const homeContract = await evaluate(`({
    nav: [...document.querySelectorAll('header a')].map((link) => link.textContent.trim()),
    paths: [...document.querySelectorAll('.path-card')].length,
    lifecycle: [...document.querySelectorAll('.lifecycle-preview li')].length,
    copyButton: document.querySelector('[data-testid="copy-command"]')?.getAttribute('aria-label')
  })`);
  assert(homeContract.nav.some((label) => label.includes('Docs')), 'Global Docs navigation is missing');
  assert(homeContract.paths === 3, 'Homepage must expose three audience paths');
  assert(homeContract.lifecycle === 4, 'Homepage lifecycle preview is incomplete');
  assert(homeContract.copyButton === 'Copy command', 'Copy control has no accessible name');
  await screenshot('homepage-desktop');

  await press('Tab');
  const keyboardFocus = await evaluate(`(() => {
    const active = document.activeElement;
    return {
      tag: active?.tagName,
      label: active?.getAttribute('aria-label') ?? active?.textContent?.trim().slice(0, 80),
      focusVisible: Boolean(active?.matches(':focus-visible')),
    };
  })()`);
  assert(keyboardFocus.tag !== 'BODY' && keyboardFocus.focusVisible, 'Keyboard navigation did not expose a visible focus target');
  results.interactions.push({ name: 'keyboard-visible-focus', ...keyboardFocus, status: 'passed' });

  await click('[data-testid="copy-command"]');
  await waitFor(
    async () => (await evaluate(`document.querySelector('[data-testid="copy-command"]')?.getAttribute('aria-label')`)) === 'Command copied',
    'copy success state',
  );
  const clipboard = await waitFor(() => evaluate(`navigator.clipboard.readText()`), 'copied install command');
  assert(clipboard === 'brew install instructa/tap/planr', 'Copy control wrote the wrong command');
  const copyLabel = await evaluate(`document.querySelector('[data-testid="copy-command"]')?.getAttribute('aria-label')`);
  assert(copyLabel === 'Command copied', 'Copy success state was not announced');
  results.interactions.push({ name: 'copy-command', value: clipboard, status: 'passed' });

  const themeBefore = await evaluate(`document.documentElement.className`);
  await click('[data-theme-toggle]');
  const themeAfter = await waitFor(
    async () => {
      const value = await evaluate(`document.documentElement.className`);
      return value !== themeBefore ? value : null;
    },
    'theme change',
  );
  results.interactions.push({ name: 'theme-toggle', before: themeBefore, after: themeAfter, status: 'passed' });
  await screenshot('homepage-theme-toggled');

  await press('k', 2);
  const searchInput = await waitFor(
    () => evaluate(`(() => {
      const input = [...document.querySelectorAll('input')].find((element) => /search/i.test(element.placeholder ?? '') && element.offsetParent !== null);
      if (!input) return null;
      input.focus();
      return { placeholder: input.placeholder };
    })()`),
    'search dialog input',
  );
  assert(/search/i.test(searchInput.placeholder), 'Search dialog has no useful placeholder');
  await cdp.send('Input.insertText', { text: 'installation' });
  const searchState = await waitFor(
    () => evaluate(`(() => {
      const body = document.body.innerText;
      return /Installation/.test(body) ? { hasInstallation: true, dialog: Boolean(document.querySelector('[role="dialog"]')) } : null;
    })()`),
    'installation search result',
  );
  assert(searchState.dialog, 'Search results are not contained in a dialog');
  results.interactions.push({ name: 'keyboard-search', query: 'installation', status: 'passed' });
  await screenshot('search-results');
  await evaluate(`(() => {
    const input = [...document.querySelectorAll('input')].find((element) => /search/i.test(element.placeholder ?? '') && element.offsetParent !== null);
    input.value = '';
    input.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'deleteContentBackward' }));
    input.focus();
  })()`);
  await cdp.send('Input.insertText', { text: 'no-such-planr-document-zzzz' });
  const emptySearch = await waitFor(
    () => evaluate(`(() => {
      const dialog = document.querySelector('[role="dialog"]');
      return dialog && /no results/i.test(dialog.innerText) ? dialog.innerText : null;
    })()`),
    'search empty state',
  );
  assert(/no results/i.test(emptySearch), 'Search has no empty-result message');
  results.interactions.push({ name: 'search-empty-state', status: 'passed' });
  await screenshot('search-empty-state');
  await press('Escape');

  const installation = await navigate('/docs/getting-started/installation');
  assert(installation.h1 === 'Installation', 'Installation docs page did not render');
  const docsContract = await evaluate(`({
    sidebar: document.querySelector('#nd-sidebar')?.innerText,
    breadcrumb: document.querySelector('#nd-page > div')?.innerText,
    toc: document.querySelector('#nd-toc')?.innerText,
    copyButtons: document.querySelectorAll('[data-testid="copy-command"]').length
  })`);
  for (const section of ['Getting Started', 'Integrations', 'Concepts', 'Guides', 'Reference', 'Contributing', 'Operations']) {
    assert(docsContract.sidebar?.includes(section), `Sidebar hierarchy is missing ${section}`);
  }
  assert(docsContract.toc?.includes('Before you install'), 'Table of contents is missing document headings');
  assert(docsContract.breadcrumb === 'Getting Started', 'Breadcrumb did not render the parent collection');
  assert(docsContract.copyButtons === 3, 'Installation page command copy controls are missing');
  results.interactions.push({ name: 'docs-navigation-toc-breadcrumbs', status: 'passed', breadcrumb: docsContract.breadcrumb });
  await screenshot('docs-installation-desktop');

  const lifecycle = await navigate('/docs/getting-started/full-lifecycle');
  assert(lifecycle.h1 === 'Full Lifecycle Tutorial', 'Full lifecycle tutorial did not render');
  const lifecycleContract = await evaluate(`({
    body: document.querySelector('#nd-page')?.innerText,
    sidebar: document.querySelector('#nd-sidebar')?.innerText
  })`);
  for (const step of ['Create a local project', 'Turn the idea into a product plan', 'Review independently and close']) {
    assert(lifecycleContract.body?.includes(step), `Full lifecycle tutorial is missing ${step}`);
  }
  assert(lifecycleContract.sidebar?.includes('Choose Your Interface'), 'Onboarding navigation is missing the interface chooser');
  results.interactions.push({ name: 'full-lifecycle-discoverability', status: 'passed' });
  await screenshot('docs-full-lifecycle-desktop');

  const conceptRoutes = [
    ['/docs/concepts/local-first-model', 'Local-First Model', ['Canonical owner', 'SQLite map', 'Where to go next']],
    ['/docs/concepts/graph-and-readiness', 'Graph and Readiness', ['blocks', 'in_review', 'preview --close']],
    ['/docs/concepts/reviews-and-approvals', 'Reviews and Approvals', ['review_mode', 'not-complete', 'approval']],
    ['/docs/guides/handoff-and-resume', 'Handoff and Resume', ['Resume from zero chat context', 'Transfer to a different worker', 'Handoff checklist']],
    ['/docs/guides/recover-interrupted-work', 'Recover Interrupted Work', ['Preserve evidence', 'Choose the narrowest repair', 'Recovery does not repair source code']],
    ['/docs/troubleshooting', 'Troubleshooting', ['pick returns no item', 'Closure is rejected', 'Prepare a safe bug report']],
    ['/docs/faq', 'FAQ', ['source of truth', 'independent review', 'session crashes']],
  ];
  for (const [path, heading, phrases] of conceptRoutes) {
    const page = await navigate(path);
    assert(page.h1 === heading, `${heading} did not render`);
    const body = await evaluate(`document.querySelector('#nd-page')?.innerText`);
    for (const phrase of phrases) {
      assert(body?.toLowerCase().includes(phrase.toLowerCase()), `${heading} is missing ${phrase}`);
    }
  }
  results.interactions.push({ name: 'concepts-guides-failure-paths', routes: conceptRoutes.length, status: 'passed' });
  await navigate('/docs/guides/recover-interrupted-work');
  await screenshot('docs-recovery-guide-desktop');

  const referenceRoutes = [
    ['/docs/reference', 'Reference', ['Source-to-page coverage', 'Generated CLI help', 'Support matrix']],
    ['/docs/reference/cli-generated', 'Generated CLI Reference', ['planr routing bundle apply', 'planr review close', 'planr recover sweep']],
    ['/docs/reference/mcp', 'MCP Reference', ['planr_pick_item', 'planr://project/map', 'planr-summary']],
    ['/docs/reference/mcp-schemas-generated', 'Generated MCP Tool Schemas', ['additionalProperties', 'planr_pick_item', 'close_target']],
    ['/docs/reference/http-api', 'Local HTTP API', ['/health', '/v1/events/stream', '/v1/reviews/{id}/close', '127.0.0.1']],
    ['/docs/reference/configuration-and-storage', 'Configuration and Storage', ['PLANR_WORKER_ID', '.planr/planr.sqlite', 'PLANR_SKIP_CHECKSUM']],
    ['/docs/reference/data-and-status', 'Data and Status Contracts', ['closed_partial', 'hands_to', 'search_index']],
    ['/docs/reference/outputs-and-errors', 'Outputs and Errors', ['nothing_ready', 'invalid_transition', 'isError']],
    ['/docs/reference/support-matrix', 'Support Matrix', ['Linux arm64', 'Generic MCP', 'localhost-only']],
  ];
  for (const [path, heading, phrases] of referenceRoutes) {
    const page = await navigate(path);
    assert(page.h1 === heading, `${heading} did not render`);
    const body = await evaluate(`document.querySelector('#nd-page')?.innerText`);
    for (const phrase of phrases) {
      assert(body?.toLowerCase().includes(phrase.toLowerCase()), `${heading} is missing ${phrase}`);
    }
  }
  results.interactions.push({ name: 'exhaustive-reference-routes', routes: referenceRoutes.length, status: 'passed' });
  await navigate('/docs/reference/mcp');
  await screenshot('docs-mcp-reference-desktop');

  const maintenanceRoutes = [
    ['/docs/contributing/repository-setup', 'Repository Setup', ['Rust 1.85', 'frozen-lockfile', 'Worktree safety']],
    ['/docs/contributing/architecture', 'Architecture and Ownership', ['Canonical owner', 'Shared CLI, MCP, and HTTP mutations', 'generated reference']],
    ['/docs/contributing/docs-authoring', 'Documentation Authoring', ['Add a page', 'CommandBlock', 'Review checklist']],
    ['/docs/contributing/testing', 'Testing Changes', ['Documentation ladder', 'docs:verify-maintenance', 'skipped gate is not a pass']],
    ['/docs/contributing/security-and-privacy', 'Security and Privacy', ['local-first boundary', '127.0.0.1', 'Sensitive evidence']],
    ['/docs/operations/release', 'Release Planr', ['only supported release entry point', 'SHA256SUMS', 'Pre-releases']],
    ['/docs/operations/versioning-and-migrations', 'Versioning and Migrations', ['additive, idempotent', 'no documented guarantee', 'Migration change checklist']],
    ['/docs/operations/docs-deployment', 'Deploy the Documentation', ['Node.js 22', 'Alchemy', 'docs.planr.so', 'STAGE=prod', 'NEXT_PUBLIC_SITE_URL', 'Pre-traffic health check']],
    ['/docs/operations/health-and-diagnostics', 'Health and Diagnostics', ['no dedicated /health endpoint', 'Diagnostic ladder', 'Do not edit production content in place']],
    ['/docs/operations/rollback', 'Rollback', ['previously verified artifact', 'custom 404', 'binary downgrade']],
    ['/docs/operations/documentation-governance', 'Documentation Governance', ['Freshness triggers', 'Generated pages are never hand-edited', 'Page review contract']],
  ];
  for (const [path, heading, phrases] of maintenanceRoutes) {
    const page = await navigate(path);
    assert(page.h1 === heading, `${heading} did not render`);
    const body = await evaluate(`document.querySelector('#nd-page')?.innerText`);
    for (const phrase of phrases) {
      assert(body?.toLowerCase().includes(phrase.toLowerCase()), `${heading} is missing ${phrase}`);
    }
  }
  results.interactions.push({ name: 'contributor-operations-runbooks', routes: maintenanceRoutes.length, status: 'passed' });
  await navigate('/docs/contributing/docs-authoring');
  await screenshot('docs-authoring-desktop');
  await navigate('/docs/operations/docs-deployment');
  await screenshot('docs-deployment-desktop');

  const redirectCases = [
    ['/docs/concepts/mental-model', '/docs/concepts/local-first-model', 'Local-First Model'],
    ['/docs/guides/multi-agent-coordination', '/docs/guides/parallel-coordination', 'Parallel Coordination'],
    ['/docs/reference/cli/project-and-plans', '/docs/reference/cli-generated', 'Generated CLI Reference'],
    ['/docs/reference/mcp/tools', '/docs/reference/mcp-schemas-generated', 'Generated MCP Tool Schemas'],
    ['/docs/reference/storage-and-generated-files', '/docs/reference/configuration-and-storage', 'Configuration and Storage'],
  ];
  for (const [sourcePath, destinationPath, heading] of redirectCases) {
    const page = await navigate(sourcePath);
    assert(page.path === destinationPath, `${sourcePath} did not redirect to ${destinationPath}`);
    assert(page.h1 === heading, `${sourcePath} redirect rendered the wrong destination`);
  }
  results.interactions.push({ name: 'legacy-route-redirects', routes: redirectCases.length, status: 'passed' });

  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 720,
    height: 490,
    deviceScaleFactor: 2,
    mobile: false,
  });
  const zoomed = await navigate('/docs/contributing/docs-authoring');
  assert(zoomed.h1 === 'Documentation Authoring', 'Documentation Authoring did not render at the 200% test viewport');
  const zoomLayout = await evaluate(`({
    viewport: [window.innerWidth, window.innerHeight],
    horizontalOverflow: document.documentElement.scrollWidth > window.innerWidth + 1,
    titleVisible: Boolean(document.querySelector('h1')?.getBoundingClientRect().height)
  })`);
  assert(!zoomLayout.horizontalOverflow && zoomLayout.titleVisible, 'The docs page overflows horizontally at the 200% test viewport');
  results.interactions.push({ name: 'zoom-200-layout', ...zoomLayout, status: 'passed' });
  await screenshot('docs-authoring-zoom-200');

  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 390,
    height: 844,
    deviceScaleFactor: 2,
    mobile: true,
  });
  await navigate('/docs/getting-started/installation');
  const mobileButton = await evaluate(`(() => {
    const button = document.querySelector('button[aria-label="Open Sidebar"]');
    if (!button) return null;
    const rect = button.getBoundingClientRect();
    return { visible: rect.width > 0 && rect.height > 0 };
  })()`);
  assert(mobileButton?.visible, 'Mobile sidebar trigger is not visible');
  await click('button[aria-label="Open Sidebar"]');
  const mobileMenu = await waitFor(
    () => evaluate(`(() => {
      const menu = document.querySelector('#nd-sidebar-mobile');
      if (!menu) return null;
      const text = menu.innerText;
      return text.includes('Getting Started') && text.includes('Reference') ? { text } : null;
    })()`),
    'mobile documentation sidebar',
  );
  assert(mobileMenu.text.includes('Operations'), 'Mobile sidebar hierarchy is incomplete');
  results.interactions.push({ name: 'mobile-sidebar', viewport: '390x844@2x', status: 'passed' });
  await screenshot('docs-installation-mobile');

  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 1440,
    height: 980,
    deviceScaleFactor: 1,
    mobile: false,
  });
  const missing = await navigate('/this-route-does-not-exist');
  assert(missing.h1 === 'This page left the map.', 'Custom 404 state did not render');
  const missingActions = await evaluate(`document.querySelectorAll('.error-actions a').length`);
  assert(missingActions === 2, 'Custom 404 recovery actions are incomplete');
  results.interactions.push({ name: 'custom-404', status: 'passed' });
  await screenshot('not-found');

  for (const path of [
    '/',
    '/docs',
    '/docs/getting-started/installation',
    '/docs/getting-started/full-lifecycle',
    '/docs/concepts/local-first-model',
    '/docs/guides/recover-interrupted-work',
    '/docs/troubleshooting',
    '/docs/faq',
    '/docs/reference',
    '/docs/reference/cli-generated',
    '/docs/reference/mcp',
    '/docs/reference/mcp-schemas-generated',
    '/docs/reference/http-api',
    '/docs/reference/data-and-status',
    '/docs/reference/outputs-and-errors',
    '/docs/reference/support-matrix',
    '/docs/contributing/docs-authoring',
    '/docs/contributing/security-and-privacy',
    '/docs/operations/docs-deployment',
    '/docs/operations/documentation-governance',
    '/this-route-does-not-exist',
  ]) {
    await runAxe(path);
  }

  const consoleErrors = await cdp.send('Runtime.evaluate', {
    expression: 'true',
    returnByValue: true,
  });
  void consoleErrors;

  const reportPath = join(outputDir, 'report.json');
  await writeFile(reportPath, `${JSON.stringify(results, null, 2)}\n`);
  console.log(`browser_shell_verification=passed`);
  console.log(`pages=${results.pages.length} interactions=${results.interactions.length}`);
  console.log(`axe_pages=${results.accessibility.length} serious_or_critical=0`);
  console.log(`report=${reportPath}`);
  for (const path of results.screenshots) console.log(`screenshot=${path}`);
} finally {
  cdp?.close();
  await terminate(chrome);
  await rm(profileDir, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
