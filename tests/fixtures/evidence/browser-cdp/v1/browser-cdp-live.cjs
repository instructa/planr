#!/usr/bin/env node
    const http = require("node:http");
    const fs = require("node:fs");
    const os = require("node:os");
    const path = require("node:path");
    const { spawn } = require("node:child_process");
    const crypto = require("node:crypto");

const targetUrl = process.argv[2];
const port = Number(process.argv[3]);
const schemaRef = "schema://planr.structured_observation_results.v1";
const html = `<!doctype html><html><head><title>Planr CDP Proof</title></head><body>
<h1 id="status">Ready</h1>
<button id="go">Go</button>
<div id="result"></div>
<div id="network"></div>
<script>
document.getElementById("result").textContent = localStorage.getItem("clicked") || "";
document.getElementById("go").addEventListener("click", async () => {
  localStorage.setItem("clicked", "done");
  document.getElementById("result").textContent = localStorage.getItem("clicked");
  history.pushState({}, "", "/next");
  const response = await fetch("/api/ping");
  document.getElementById("network").textContent = String(response.status);
});
</script></body></html>`;

function listen(port) {
  const server = http.createServer((req, res) => {
    if (req.url === "/api/ping") {
      res.writeHead(200, {"content-type": "application/json"});
      res.end(JSON.stringify({status: "ok"}));
      return;
    }
    if (req.url === "/health") {
      res.writeHead(200, {"content-type": "application/json"});
      res.end(JSON.stringify({status: "ok"}));
      return;
    }
    res.writeHead(200, {"content-type": "text/html"});
    res.end(html);
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve(server));
  });
}

async function waitForJson(url, deadlineMs = 10000) {
  const deadline = Date.now() + deadlineMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) return await response.json();
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`timed out waiting for ${url}`);
}

function waitForExit(child, deadlineMs) {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve(true);
  return new Promise((resolve) => {
    let settled = false;
    const finish = (exited) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.removeListener("exit", onExit);
      resolve(exited);
    };
    const onExit = () => finish(true);
    const timer = setTimeout(() => finish(false), deadlineMs);
    child.once("exit", onExit);
  });
}

async function terminateChild(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  if (await waitForExit(child, 2000)) return;
  child.kill("SIGKILL");
  if (!(await waitForExit(child, 2000))) {
    throw new Error("Chrome did not exit after SIGTERM and SIGKILL");
  }
}

function closeServer(server) {
  return new Promise((resolve, reject) => {
    server.close((error) => error ? reject(error) : resolve());
  });
}

async function main() {
  const configuredChromePath = "__PLANR_CHROME_PATH__";
  const chromePath = configuredChromePath || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
  if (!fs.existsSync(chromePath)) throw new Error(`Chrome executable not found: ${chromePath}`);
  const debugPort = Number(process.argv[4]);
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "planr-cdp-profile-"));
  const server = await listen(port);
  const chrome = spawn(chromePath, [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--no-default-browser-check",
    `--remote-debugging-port=${debugPort}`,
    `--user-data-dir=${userDataDir}`,
    "about:blank"
  ], {stdio: ["ignore", "ignore", "ignore"]});
  try {
    const version = await waitForJson(`http://127.0.0.1:${debugPort}/json/version`);
    const targets = await waitForJson(`http://127.0.0.1:${debugPort}/json/list`);
    const page = targets.find((target) => target.type === "page" && target.webSocketDebuggerUrl);
    if (!page) throw new Error("Chrome CDP page target was not available");
    const ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, {once: true});
      ws.addEventListener("error", reject, {once: true});
    });
    let nextId = 1;
    const pending = new Map();
    const network = [];
    const consoleErrors = [];
    ws.addEventListener("message", (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id && pending.has(msg.id)) {
        const {resolve, reject} = pending.get(msg.id);
        pending.delete(msg.id);
        msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result || {});
      } else if (msg.method === "Network.responseReceived") {
        network.push({url: msg.params.response.url, status: msg.params.response.status});
      } else if (msg.method === "Runtime.exceptionThrown") {
        consoleErrors.push(msg.params.exceptionDetails.text || "exception");
      } else if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") {
        consoleErrors.push("console.error");
      } else if (msg.method === "Log.entryAdded" && msg.params.entry.level === "error") {
        consoleErrors.push(msg.params.entry.text || "log error");
      }
    });
    const send = (method, params = {}) => new Promise((resolve, reject) => {
      const id = nextId++;
      pending.set(id, {resolve, reject});
      ws.send(JSON.stringify({id, method, params}));
    });
    await send("Page.enable");
    await send("Runtime.enable");
    await send("Network.enable");
    await send("Log.enable");
    await send("Page.navigate", {url: targetUrl});
    await new Promise((resolve) => setTimeout(resolve, 500));
    const initialLocation = await send("Runtime.evaluate", {
      expression: `location.href`,
      returnByValue: true
    });
    const visible = await send("Runtime.evaluate", {
      expression: `({visible: document.querySelector("#status")?.textContent === "Ready", text: document.querySelector("#status")?.textContent || "", schema_ref: "${schemaRef}"})`,
      returnByValue: true
    });
    const rect = await send("Runtime.evaluate", {
      expression: `(() => { const r = document.querySelector("#go").getBoundingClientRect(); return {x: r.left + r.width / 2, y: r.top + r.height / 2}; })()`,
      returnByValue: true
    });
    const {x, y} = rect.result.value;
    await send("Input.dispatchMouseEvent", {type: "mouseMoved", x, y, button: "none"});
    await send("Input.dispatchMouseEvent", {type: "mousePressed", x, y, button: "left", clickCount: 1});
    await send("Input.dispatchMouseEvent", {type: "mouseReleased", x, y, button: "left", clickCount: 1});
    await new Promise((resolve) => setTimeout(resolve, 500));
    const afterClick = await send("Runtime.evaluate", {
      expression: `({clicked: localStorage.getItem("clicked") === "done", path: location.pathname, api_status: Number(document.querySelector("#network").textContent), schema_ref: "${schemaRef}"})`,
      returnByValue: true
    });
    const finalLocation = await send("Runtime.evaluate", {
      expression: `location.href`,
      returnByValue: true
    });
    await send("Page.reload", {ignoreCache: true});
    await new Promise((resolve) => setTimeout(resolve, 500));
    const afterReload = await send("Runtime.evaluate", {
      expression: `({persisted: localStorage.getItem("clicked") === "done" && document.querySelector("#result").textContent === "done", schema_ref: "${schemaRef}"})`,
      returnByValue: true
    });
    const api = network.find((entry) => entry.url.endsWith("/api/ping"));
    const runtimeIdentity = {
      kind: "chrome-cdp",
      product: version.Browser || null,
      protocol_version: version["Protocol-Version"] || null,
      user_agent: version["User-Agent"] || null,
      executable_path: chromePath,
      executable_digest: crypto.createHash("sha256").update(fs.readFileSync(chromePath)).digest("hex").replace(/^/, "sha256:"),
      debug_endpoint: `http://127.0.0.1:${debugPort}`
    };
    const observedTarget = {
      kind: "browser",
      initial_uri: initialLocation.result.value,
      final_uri: finalLocation.result.value
    };
    const helperDigest = crypto.createHash("sha256").update(fs.readFileSync(__filename)).digest("hex").replace(/^/, "sha256:");
    const sourceDigest = process.env.PLANR_TEST_STALE_FIXTURE_SOURCE_DIGEST === "1"
      ? "sha256:0000000000000000000000000000000000000000000000000000000000000000"
      : helperDigest;
    const fixtureSources = [{
      ref: `planr-test-fixture:browser-cdp-live-helper:${sourceDigest}`,
      path: ".planr/evidence/adapters/browser-cdp-live.cjs",
      digest: sourceDigest
    }];
    const fixtureDisclosure = {
      fixtures_used: true,
      mocks_used: false,
      fixture_refs: fixtureSources.map((source) => source.ref)
    };
    const result = {
      schema_version: "planr.structured_observation_results.v1",
      method: "raw_chrome_cdp",
      observed_target: observedTarget,
      target: JSON.parse(process.env.PLANR_EVIDENCE_TARGET_JSON || "null"),
      environment: JSON.parse(process.env.PLANR_EVIDENCE_ENVIRONMENT_JSON || "null"),
      execution_contract_digest: process.env.PLANR_EVIDENCE_EXECUTION_CONTRACT_DIGEST || null,
      runtime_identity: runtimeIdentity,
      fixture_sources: fixtureSources,
      fixture_disclosure: fixtureDisclosure,
      observations: [
        {requirement_id: "obs-pob-browser-cdp-visible", type: "com.example.browser.rendered_visibility", actual: {...visible.result.value, observed_target: observedTarget, runtime_identity: runtimeIdentity}},
        {requirement_id: "obs-pob-browser-cdp-interaction", type: "com.example.browser.user_interaction", actual: {schema_ref: schemaRef, clicked: afterClick.result.value.clicked, observed_target: observedTarget, runtime_identity: runtimeIdentity}},
        {requirement_id: "obs-pob-browser-cdp-navigation", type: "com.example.browser.navigation", actual: {schema_ref: schemaRef, path: afterClick.result.value.path, observed_target: observedTarget, runtime_identity: runtimeIdentity}},
        {requirement_id: "obs-pob-browser-cdp-network", type: "com.example.browser.network", actual: {schema_ref: schemaRef, api_status: api ? api.status : null, responses: network, observed_target: observedTarget, runtime_identity: runtimeIdentity}},
        {requirement_id: "obs-pob-browser-cdp-console", type: "com.example.browser.console", actual: {schema_ref: schemaRef, error_count: consoleErrors.length, errors: consoleErrors, observed_target: observedTarget, runtime_identity: runtimeIdentity}},
        {requirement_id: "obs-pob-browser-cdp-reload", type: "com.example.browser.reload_storage", actual: {...afterReload.result.value, observed_target: observedTarget, runtime_identity: runtimeIdentity}}
      ]
    };
    if (process.env.PLANR_TEST_OMIT_FIXTURE_DISCLOSURE === "1") {
      delete result.fixture_sources;
      delete result.fixture_disclosure;
    }
    console.log(JSON.stringify(result));
    ws.close();
  } finally {
    await terminateChild(chrome);
    await closeServer(server);
    fs.rmSync(userDataDir, {recursive: true, force: true});
  }
}

main().catch((error) => {
  console.error(error.stack || String(error));
  process.exit(1);
});
