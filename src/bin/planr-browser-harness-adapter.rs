use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    process::{Command, ExitCode, Stdio},
};

const REQUEST_SCHEMA: &str = "planr.evidence.adapter-request.v1";
const RESULT_SCHEMA: &str = "planr.structured_observation_results.v2";
const OBSERVATION_TYPE: &str = "com.planr.web.dom_state";
const OBSERVATION_SCHEMA: &str = "schema://com.planr.web.dom_state.v2";
const RESULT_PREFIX: &str = "PLANR_EVIDENCE_RESULT_JSON=";

const BROWSER_SCRIPT: &str = r#"
import json
import time
from urllib.parse import urlsplit

request = json.loads(__PLANR_REQUEST_JSON_STRING__)
target = request["target"]
requirements = request["requirements"]
requirements_by_id = {requirement["id"]: requirement for requirement in requirements}
scenario = next(
    requirement["state_transitions"]
    for requirement in requirements
    if "scenario_id" in requirement["state_transitions"]
)
transitions = scenario["steps"]
total_deadline = time.monotonic() + 15.0
checkpoint_window_seconds = 0.75
captured = {}
storage_snapshots = {}
runtime_errors = []
external_requests = set()
target_url = urlsplit(target["uri"])
target_origin = f"{target_url.scheme}://{target_url.netloc}"

def ax_value(node, field):
    value = node.get(field) or {}
    return value.get("value")

def ensure_time():
    if time.monotonic() >= total_deadline:
        raise RuntimeError("Browser Evidence scenario exceeded its single total deadline")

def normalize(value):
    return " ".join(str(value or "").split())

def collect_events():
    for event in drain_events():
        method = event.get("method")
        params = event.get("params") or {}
        if method == "Runtime.exceptionThrown":
            runtime_errors.append(method)
        elif method == "Runtime.consoleAPICalled" and params.get("type") == "error":
            runtime_errors.append(method)
        elif method == "Log.entryAdded" and (params.get("entry") or {}).get("level") == "error":
            runtime_errors.append(method)
        elif method == "Network.requestWillBeSent":
            url = ((params.get("request") or {}).get("url") or "")
            parsed = urlsplit(url)
            if parsed.scheme in ("http", "https") and f"{parsed.scheme}://{parsed.netloc}" != target_origin:
                external_requests.add(url)

def settle():
    ensure_time()
    js("new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))))")
    collect_events()

def click_accessible(role, name, scope_text=None):
    if scope_text is not None:
        expression = (
            "(()=>{const norm=v=>(v||'').replace(/\\s+/g,' ').trim();"
            "const role=" + json.dumps(role) + ";const name=" + json.dumps(name) + ";"
            "const scope=" + json.dumps(scope_text) + ";"
            "const visible=e=>!e.disabled&&e.getClientRects().length>0&&"
            "getComputedStyle(e).visibility!=='hidden'&&getComputedStyle(e).display!=='none';"
            "const candidates=[...document.querySelectorAll('button,a,[role=button],[role=link]')].map(e=>{"
            "const actualRole=e.getAttribute('role')||(e.tagName==='A'?'link':'button');"
            "const actualName=norm(e.getAttribute('aria-label')||e.textContent);"
            "const roleMatches=role==='control'?['button','link'].includes(actualRole):actualRole===role;"
            "if(!roleMatches||!actualName.includes(name)||!visible(e))return null;"
            "let score=Number.MAX_SAFE_INTEGER;for(let node=e.parentElement;node&&node!==document.body;node=node.parentElement){"
            "const text=norm(node.textContent);if(text.includes(scope))score=Math.min(score,text.length);}"
            "return score===Number.MAX_SAFE_INTEGER?null:{e,score};}).filter(Boolean).sort((a,b)=>a.score-b.score);"
            "if(candidates.length===0||(candidates.length>1&&candidates[0].score===candidates[1].score))"
            "return {ok:false,count:candidates.length};candidates[0].e.click();return {ok:true,count:1};})()"
        )
        result = js(expression)
        if not result or not result.get("ok"):
            count = (result or {}).get("count", 0)
            raise RuntimeError(f"expected one scoped accessible {role!r} named {name!r}, found {count}")
        return
    nodes = cdp("Accessibility.getFullAXTree").get("nodes", [])
    matches = [
        node for node in nodes
        if not node.get("ignored")
        and (ax_value(node, "role") == role or (role == "control" and ax_value(node, "role") in ("button", "link")))
        and name in (ax_value(node, "name") or "")
        and node.get("backendDOMNodeId") is not None
    ]
    if len(matches) != 1:
        raise RuntimeError(f"expected one accessible {role!r} named {name!r}, found {len(matches)}")
    quad = cdp("DOM.getBoxModel", backendNodeId=matches[0]["backendDOMNodeId"])["model"]["content"]
    x = sum(quad[0::2]) / 4
    y = sum(quad[1::2]) / 4
    click_at_xy(x, y)

def set_control(locator, value, action):
    expression = (
        "(()=>{const norm=v=>(v||'').replace(/\\s+/g,' ').trim();"
        "const locator=" + json.dumps(locator) + ";let matches=[];"
        "if(locator.subject){const e=document.querySelector(locator.subject);if(e)matches=[e];}"
        "else{matches=[...document.querySelectorAll('label')].filter(l=>norm(l.textContent).toLowerCase().startsWith(locator.label.toLowerCase()))"
        ".map(l=>l.control||l.querySelector('input,textarea,select')).filter(Boolean);}"
        "if(matches.length!==1)return {ok:false,reason:'control-count',count:matches.length};"
        "const e=matches[0];"
        "if(e.disabled)return {ok:false,reason:'disabled'};"
        "const action=" + json.dumps(action) + ";"
        "if(action==='fill'&&!['INPUT','TEXTAREA'].includes(e.tagName))"
        "return {ok:false,reason:'not-fillable'};"
        "if(action==='select'&&e.tagName!=='SELECT')"
        "return {ok:false,reason:'not-select'};"
        "let value=" + json.dumps(value) + ";"
        "if(action==='select'){const options=[...e.options].filter(o=>!o.disabled&&(o.value===value||norm(o.textContent)===value));"
        "if(options.length!==1)return {ok:false,reason:'option-unavailable',count:options.length};value=options[0].value;}"
        "const prototype=e.tagName==='SELECT'?HTMLSelectElement.prototype:"
        "e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;"
        "const setter=Object.getOwnPropertyDescriptor(prototype,'value').set;"
        "setter.call(e,value);"
        "e.dispatchEvent(new Event('input',{bubbles:true}));"
        "e.dispatchEvent(new Event('change',{bubbles:true}));"
        "return {ok:e.value===value,value:e.value};})()"
    )
    result = js(expression)
    if not result or not result.get("ok"):
        reason = (result or {}).get("reason", "unknown")
        raise RuntimeError(f"{action} failed for {locator!r}: {reason}")

def select_values(labels):
    expression = (
        "(()=>{const norm=v=>(v||'').replace(/\\s+/g,' ').trim();const labels=" + json.dumps(labels) + ";"
        "const result={};for(const name of labels){const matches=[...document.querySelectorAll('label')]"
        ".filter(l=>norm(l.textContent).toLowerCase().startsWith(name.toLowerCase())).map(l=>l.control||l.querySelector('select')).filter(e=>e&&e.tagName==='SELECT');"
        "if(matches.length===1){const option=matches[0].selectedOptions[0];result[name]=norm(option?.textContent||matches[0].value);}}"
        "return result;})()"
    )
    return js(expression) or {}

def accessibility_state():
    expression = (
        "(()=>{const visible=e=>!e.disabled&&e.getClientRects().length>0&&"
        "getComputedStyle(e).visibility!=='hidden'&&getComputedStyle(e).display!=='none';"
        "const controls=[...document.querySelectorAll('button,a,input,textarea,select')].filter(visible);"
        "const name=e=>(e.getAttribute('aria-label')||e.getAttribute('title')||"
        "(e.labels&&[...e.labels].map(l=>l.textContent).join(' '))||e.textContent||'').trim();"
        "return {unnamed_controls:controls.filter(e=>!name(e)).length,"
        "unlabelled_fields:controls.filter(e=>['INPUT','TEXTAREA','SELECT'].includes(e.tagName)&&!(e.labels&&e.labels.length)&&!e.getAttribute('aria-label')).length};})()"
    )
    return js(expression)

def storage_state(expected, compare_snapshot):
    if not isinstance(expected, dict):
        return None
    key = expected.get("key")
    expression = (
        "(()=>{const key=" + json.dumps(key) + ";const raw=localStorage.getItem(key);let parsed=null;"
        "try{parsed=raw===null?null:JSON.parse(raw)}catch{}"
        "return {raw,present:raw!==null,transaction_count:Array.isArray(parsed?.transactions)?parsed.transactions.length:null};})()"
    )
    observed = js(expression) or {"raw": None, "present": False, "transaction_count": None}
    raw = observed.pop("raw", None)
    observed["key"] = key
    observed["matches_snapshot"] = (
        compare_snapshot is not None
        and compare_snapshot in storage_snapshots
        and raw == storage_snapshots[compare_snapshot]
    )
    return observed

def observe(requirement, compare_snapshot=None):
    selector = requirement["subject"]
    expression = (
        "(()=>{const e=document.querySelector(" + json.dumps(selector) + ");"
        "if(!e)return null;"
        "const visible=typeof e.checkVisibility==='function'"
        "?e.checkVisibility({checkOpacity:true,checkVisibilityCSS:true})"
        ":!!(e.getClientRects().length&&getComputedStyle(e).visibility!=='hidden');"
        "return {text:(e.innerText||e.textContent||'').replace(/\\s+/g,' ').trim(),visible,"
        "horizontal_overflow:document.documentElement.scrollWidth>document.documentElement.clientWidth};})()"
    )
    element = js(expression)
    expected = requirement["expected"]
    text = (element or {}).get("text")
    contains = expected.get("contains_text") or []
    excludes = expected.get("excludes_text") or []
    collect_events()
    return {
        "schema_ref": requirement["payload_schema"]["schema_ref"],
        "selector": selector,
        "text": text,
        "visible": (element or {}).get("visible", False),
        "contains_text": [value for value in contains if value in (text or "")],
        "excludes_text": [value for value in excludes if value not in (text or "")],
        "storage": storage_state(expected.get("storage"), compare_snapshot),
        "selected": select_values(list((expected.get("selected") or {}).keys())),
        "accessibility": accessibility_state(),
        "layout": {"horizontal_overflow": (element or {}).get("horizontal_overflow", False)},
        "runtime": {
            "error_count": len(runtime_errors),
            "external_request_count": len(external_requests),
        },
    }

def expected_matches(actual, expected):
    if isinstance(expected, dict):
        return isinstance(actual, dict) and all(
            key in actual and expected_matches(actual[key], value)
            for key, value in expected.items()
        )
    return actual == expected

def capture_checkpoint(transition):
    requirement_ids = transition["requirements"]
    compare_snapshot = transition.get("compare_storage_snapshot")
    checkpoint_deadline = min(total_deadline, time.monotonic() + checkpoint_window_seconds)
    latest = {}
    while True:
        latest = {
            requirement_id: observe(requirements_by_id[requirement_id], compare_snapshot)
            for requirement_id in requirement_ids
        }
        if all(expected_matches(latest[requirement_id], requirements_by_id[requirement_id]["expected"])
               for requirement_id in requirement_ids):
            break
        if time.monotonic() >= checkpoint_deadline:
            break
        time.sleep(0.05)
    captured.update(latest)

tab = new_tab("about:blank")
try:
    cdp("Page.enable")
    cdp("Runtime.enable")
    cdp("Network.enable")
    cdp("Log.enable")
    drain_events()
    cdp("Page.navigate", url=target["uri"])
    if not wait_for_load(timeout=max(0.1, min(5.0, total_deadline - time.monotonic()))):
        raise RuntimeError("target did not finish loading")
    initial = page_info()["url"]
    activate_tab(tab)
    for transition in transitions:
        ensure_time()
        action = transition["action"]
        if action == "click":
            click_accessible(transition["role"], transition["name"], transition.get("scope_text"))
            settle()
        elif action in ("fill", "select"):
            locator = {key: transition[key] for key in ("subject", "label") if key in transition}
            set_control(locator, transition["value"], action)
            settle()
        elif action == "reset":
            js("localStorage.removeItem(" + json.dumps(transition["storage_key"]) + ");sessionStorage.clear();true")
            cdp("Page.navigate", url=target["uri"])
            if not wait_for_load(timeout=max(0.1, min(5.0, total_deadline - time.monotonic()))):
                raise RuntimeError("target did not finish loading after reset")
            settle()
        elif action == "snapshot_storage":
            storage_snapshots[transition["snapshot"]] = js(
                "localStorage.getItem(" + json.dumps(transition["storage_key"]) + ")"
            )
        elif action == "reload":
            cdp("Page.reload", ignoreCache=True)
            if not wait_for_load(timeout=max(0.1, min(5.0, total_deadline - time.monotonic()))):
                raise RuntimeError("target did not finish loading after reload")
            settle()
        elif action == "viewport":
            cdp("Emulation.setDeviceMetricsOverride", width=transition["width"], height=transition["height"], deviceScaleFactor=1, mobile=False)
            settle()
        elif action == "checkpoint":
            capture_checkpoint(transition)

    if set(captured) != set(requirements_by_id):
        raise RuntimeError("Browser Evidence scenario did not capture the exact sealed requirement set")
    collect_events()
    observations = [
        {
            "requirement_id": requirement["id"],
            "type": requirement["type"],
            "actual": captured[requirement["id"]],
        }
        for requirement in requirements
    ]
    statuses = [
        expected_matches(captured[requirement["id"]], requirement["expected"])
        for requirement in requirements
    ]

    result = {
        "schema_version": "planr.structured_observation_results.v2",
        "request_id": request["request_id"],
        "request_digest": request["request_digest"],
        "method": "browser-harness",
        "target": target,
        "observed_target": {
            "kind": target["kind"],
            "initial_uri": initial,
            "final_uri": page_info()["url"],
        },
        "environment": request["environment"],
        "execution_contract_digest": request["execution_contract_digest"],
        "fixture_disclosure": request["fixture_disclosure"],
        "observations": observations,
    }
    methods = [requirement.get("execution_method") for requirement in requirements]
    if all(method is not None for method in methods):
        method = methods[0]
        result["agent_skill"] = {
            "schema_version": method["result_schema"]["schema_ref"],
            "skill": method["skill"],
            "invoked": True,
            "invocation_id": "inv-" + request["request_id"],
            "observations": [
                {
                    "requirement_id": requirement["id"],
                    "status": "passed" if passed else "failed",
                }
                for requirement, passed in zip(requirements, statuses)
            ],
        }
    print("PLANR_EVIDENCE_RESULT_JSON=" + json.dumps(result, separators=(",", ":")))
finally:
    close_tab(tab)
"#;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--identity") => {
            println!(
                "{}",
                json!({
                    "schema_version": "planr.evidence.adapter-identity.v1",
                    "adapter": "planr-browser-harness-adapter",
                    "adapter_version": env!("CARGO_PKG_VERSION"),
                    "protocol": REQUEST_SCHEMA,
                })
            );
            return Ok(());
        }
        Some("--version") | Some("-V") => {
            println!(
                "planr-browser-harness-adapter {}",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        }
        Some(argument) => bail!("unsupported argument: {argument}"),
        None => {}
    }

    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("reading Planr Evidence adapter request")?;
    if input.trim().is_empty() {
        return probe_browser_harness();
    }
    let request: Value = serde_json::from_str(&input)
        .context("Planr Evidence adapter request must be one JSON object")?;
    validate_request(&request)?;
    let script = browser_script(&request)?;
    let output = run_browser_harness(&script)?;
    let result = parse_browser_result(&output.stdout)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn probe_browser_harness() -> Result<()> {
    let output = Command::new("browser-harness")
        .arg("--version")
        .env("BH_RECORD", "0")
        .output()
        .context("starting browser-harness availability probe")?;
    if !output.status.success() {
        bail!(
            "browser-harness availability probe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    println!(
        "{}",
        json!({
            "status": "available",
            "tool": "browser-harness",
            "version": String::from_utf8_lossy(&output.stdout).trim(),
        })
    );
    Ok(())
}

fn validate_request(request: &Value) -> Result<()> {
    if request.get("schema_version").and_then(Value::as_str) != Some(REQUEST_SCHEMA) {
        bail!("unsupported Planr Evidence adapter request schema");
    }
    for field in ["request_id", "request_digest", "execution_contract_digest"] {
        if request.get(field).and_then(Value::as_str).is_none() {
            bail!("Planr Evidence adapter request is missing {field}");
        }
    }
    let target = request
        .get("target")
        .and_then(Value::as_object)
        .context("Planr Evidence adapter request is missing target")?;
    if target.get("kind").and_then(Value::as_str) != Some("browser") {
        bail!("Browser Harness adapter requires target.kind=browser");
    }
    let uri = target
        .get("uri")
        .and_then(Value::as_str)
        .context("Browser Harness adapter requires target.uri")?;
    if !(uri.starts_with("http://") || uri.starts_with("https://")) {
        bail!("Browser Harness adapter target.uri must use http or https");
    }
    let requirements = request
        .get("requirements")
        .and_then(Value::as_array)
        .filter(|requirements| !requirements.is_empty())
        .context("Browser Harness adapter requires at least one requirement")?;
    let mut requirement_ids = BTreeSet::new();
    for requirement in requirements {
        let requirement_id = requirement
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .context("Browser Harness requirement needs a non-empty id")?;
        if !requirement_ids.insert(requirement_id.to_string()) {
            bail!("Browser Harness requirement IDs must be unique");
        }
    }
    let transitions = resolve_transitions(requirements)?;
    validate_transitions(transitions, &requirement_ids)?;
    let expected_method = requirements[0].get("execution_method");
    for requirement in requirements {
        if requirement.get("type").and_then(Value::as_str) != Some(OBSERVATION_TYPE) {
            bail!("Browser Harness adapter received an unsupported observation type");
        }
        if requirement
            .get("subject")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            bail!("Browser Harness DOM-state requirement needs a non-empty CSS selector subject");
        }
        let expected = requirement
            .get("expected")
            .and_then(Value::as_object)
            .context("Browser Harness DOM-state requirement needs an expected object")?;
        validate_expected(expected)?;
        if requirement.get("target") != request.get("target") {
            bail!("Browser Harness requirement target must equal the sealed batch target");
        }
        if requirement
            .get("payload_schema")
            .and_then(|schema| schema.get("schema_ref"))
            .and_then(Value::as_str)
            != Some(OBSERVATION_SCHEMA)
        {
            bail!("Browser Harness DOM-state requirement has the wrong payload schema");
        }
        if requirement.get("execution_method") != expected_method {
            bail!("Browser Harness adapter requires one shared execution method per batch");
        }
        if let Some(method) = requirement.get("execution_method") {
            if method.get("kind").and_then(Value::as_str) != Some("agent_skill")
                || method.get("skill").and_then(Value::as_str) != Some("browser-harness")
                || method
                    .get("result_schema")
                    .and_then(|schema| schema.get("schema_ref"))
                    .and_then(Value::as_str)
                    != Some("planr.evidence.agent-skill-result.v1")
            {
                bail!("Browser Harness adapter execution method does not match browser-harness");
            }
        }
    }
    if request
        .get("result_contract")
        .and_then(|contract| contract.get("schema_ref"))
        .and_then(Value::as_str)
        != Some("schema://planr.structured_observation_results.v2")
    {
        bail!("Browser Harness adapter requires the structured result v2 contract");
    }
    Ok(())
}

fn resolve_transitions<'a>(requirements: &'a [Value]) -> Result<&'a Value> {
    let mut declaration: Option<(&str, &Value)> = None;
    let mut references = Vec::new();
    for requirement in requirements {
        let state = requirement
            .get("state_transitions")
            .and_then(Value::as_object)
            .context("Browser Harness requirement needs a closed state_transitions object")?;
        match (
            state.get("scenario_id"),
            state.get("scenario_ref"),
            state.get("steps"),
        ) {
            (Some(id), None, Some(steps)) if state.len() == 2 => {
                let id = id
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 128)
                    .context("Browser Harness scenario_id must be a bounded non-empty string")?;
                if declaration.replace((id, steps)).is_some() {
                    bail!("Browser Harness batch must declare exactly one scenario");
                }
            }
            (None, Some(reference), None) if state.len() == 1 => {
                references.push(
                    reference
                        .as_str()
                        .filter(|value| !value.is_empty() && value.len() <= 128)
                        .context(
                            "Browser Harness scenario_ref must be a bounded non-empty string",
                        )?,
                );
            }
            _ => bail!(
                "Browser Harness state_transitions must be one scenario declaration or reference"
            ),
        }
    }
    let (scenario_id, steps) =
        declaration.context("Browser Harness batch has no scenario declaration")?;
    if references.iter().any(|reference| *reference != scenario_id) {
        bail!("Browser Harness scenario references must resolve to the one batch declaration");
    }
    Ok(steps)
}

fn validate_expected(expected: &serde_json::Map<String, Value>) -> Result<()> {
    if expected.is_empty()
        || expected.keys().any(|key| {
            !matches!(
                key.as_str(),
                "text"
                    | "visible"
                    | "contains_text"
                    | "excludes_text"
                    | "storage"
                    | "selected"
                    | "accessibility"
                    | "layout"
                    | "runtime"
            )
        })
    {
        bail!("Browser Harness DOM-state expected contains unsupported fields");
    }
    if expected.get("text").is_some_and(|value| !value.is_string())
        || expected
            .get("visible")
            .is_some_and(|value| !value.is_boolean())
    {
        bail!("Browser Harness DOM-state text/visible expectation has the wrong type");
    }
    for field in ["contains_text", "excludes_text"] {
        if let Some(values) = expected.get(field) {
            let values = values
                .as_array()
                .filter(|values| !values.is_empty() && values.len() <= 64)
                .with_context(|| {
                    format!("Browser Harness {field} must be a non-empty bounded array")
                })?;
            if values
                .iter()
                .any(|value| !bounded_nonempty_string(Some(value), 1024))
            {
                bail!("Browser Harness {field} entries must be bounded non-empty strings");
            }
        }
    }
    if let Some(storage) = expected.get("storage") {
        let storage = storage
            .as_object()
            .context("Browser Harness storage expectation must be an object")?;
        if storage.is_empty()
            || storage.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "key" | "present" | "transaction_count" | "matches_snapshot"
                )
            })
            || !bounded_nonempty_string(storage.get("key"), 512)
            || storage
                .get("present")
                .is_some_and(|value| !value.is_boolean())
            || storage
                .get("matches_snapshot")
                .is_some_and(|value| !value.is_boolean())
            || storage
                .get("transaction_count")
                .is_some_and(|value| value.as_u64().is_none())
        {
            bail!("Browser Harness storage expectation is invalid");
        }
    }
    if let Some(selected) = expected.get("selected") {
        let selected = selected
            .as_object()
            .filter(|selected| !selected.is_empty() && selected.len() <= 32)
            .context("Browser Harness selected expectation must be a non-empty bounded object")?;
        if selected.iter().any(|(label, value)| {
            label.is_empty() || label.len() > 512 || !bounded_nonempty_string(Some(value), 512)
        }) {
            bail!("Browser Harness selected expectation labels and values must be bounded strings");
        }
    }
    validate_bounded_count_object(
        expected.get("accessibility"),
        &["unnamed_controls", "unlabelled_fields"],
        "accessibility",
    )?;
    if let Some(layout) = expected.get("layout") {
        let layout = layout
            .as_object()
            .context("Browser Harness layout expectation must be an object")?;
        if layout.is_empty()
            || layout.keys().any(|key| key != "horizontal_overflow")
            || layout
                .get("horizontal_overflow")
                .is_none_or(|value| !value.is_boolean())
        {
            bail!("Browser Harness layout expectation is invalid");
        }
    }
    validate_bounded_count_object(
        expected.get("runtime"),
        &["error_count", "external_request_count"],
        "runtime",
    )?;
    Ok(())
}

fn validate_bounded_count_object(
    value: Option<&Value>,
    allowed: &[&str],
    name: &str,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let object = value
        .as_object()
        .filter(|object| !object.is_empty())
        .with_context(|| {
            format!("Browser Harness {name} expectation must be a non-empty object")
        })?;
    if object
        .iter()
        .any(|(key, value)| !allowed.contains(&key.as_str()) || value.as_u64().is_none())
    {
        bail!("Browser Harness {name} expectation is invalid");
    }
    Ok(())
}

fn validate_transitions(transitions: &Value, requirement_ids: &BTreeSet<String>) -> Result<()> {
    let transitions = transitions
        .as_array()
        .context("Browser Harness state_transitions must be an array")?;
    if transitions.is_empty() || transitions.len() > 256 {
        bail!("Browser Harness requires 1 to 256 state transitions per batch");
    }
    let mut captured_requirement_ids = BTreeSet::new();
    let mut snapshot_ids = BTreeSet::new();
    for transition in transitions {
        let object = transition
            .as_object()
            .context("Browser Harness state transition must be an object")?;
        match object.get("action").and_then(Value::as_str) {
            Some("click") => {
                if !(object.len() == 3 || object.len() == 4)
                    || !bounded_nonempty_string(object.get("role"), 128)
                    || !bounded_nonempty_string(object.get("name"), 512)
                    || object
                        .get("scope_text")
                        .is_some_and(|value| !bounded_nonempty_string(Some(value), 1024))
                    || object.keys().any(|key| {
                        !matches!(key.as_str(), "action" | "role" | "name" | "scope_text")
                    })
                {
                    bail!("Browser Harness click transition is invalid");
                }
            }
            Some("fill" | "select") => {
                if object.len() != 3
                    || (bounded_nonempty_string(object.get("subject"), 1024)
                        == bounded_nonempty_string(object.get("label"), 512))
                    || !bounded_string(object.get("value"), 4096)
                    || object.keys().any(|key| {
                        !matches!(key.as_str(), "action" | "subject" | "label" | "value")
                    })
                {
                    bail!("Browser Harness fill/select transition is invalid");
                }
            }
            Some("reset") => {
                if object.len() != 2 || !bounded_nonempty_string(object.get("storage_key"), 512) {
                    bail!("Browser Harness reset transition is invalid");
                }
            }
            Some("snapshot_storage") => {
                if object.len() != 3
                    || !bounded_nonempty_string(object.get("storage_key"), 512)
                    || !bounded_nonempty_string(object.get("snapshot"), 128)
                {
                    bail!("Browser Harness snapshot_storage transition is invalid");
                }
                let snapshot = object["snapshot"].as_str().unwrap();
                if !snapshot_ids.insert(snapshot.to_string()) {
                    bail!("Browser Harness storage snapshot IDs must be unique");
                }
            }
            Some("reload") => {
                if object.len() != 1 {
                    bail!("Browser Harness reload transition accepts no additional fields");
                }
            }
            Some("viewport") => {
                let width = object.get("width").and_then(Value::as_u64);
                let height = object.get("height").and_then(Value::as_u64);
                if object.len() != 3
                    || width.is_none_or(|width| !(320..=2560).contains(&width))
                    || height.is_none_or(|height| !(320..=2000).contains(&height))
                {
                    bail!("Browser Harness viewport transition is invalid");
                }
            }
            Some("checkpoint") => {
                if !(object.len() == 2 || object.len() == 3)
                    || object.keys().any(|key| {
                        !matches!(
                            key.as_str(),
                            "action" | "requirements" | "compare_storage_snapshot"
                        )
                    })
                {
                    bail!("Browser Harness checkpoint transition is invalid");
                }
                if let Some(snapshot) = object.get("compare_storage_snapshot") {
                    let snapshot = snapshot
                        .as_str()
                        .filter(|snapshot| snapshot_ids.contains(*snapshot))
                        .context("Browser Harness checkpoint references an unknown prior storage snapshot")?;
                    if snapshot.len() > 128 {
                        bail!("Browser Harness checkpoint snapshot ID is too long");
                    }
                }
                let ids = object
                    .get("requirements")
                    .and_then(Value::as_array)
                    .filter(|ids| !ids.is_empty() && ids.len() <= 64)
                    .context(
                        "Browser Harness checkpoint requires a non-empty bounded requirement list",
                    )?;
                for id in ids {
                    let id = id
                        .as_str()
                        .filter(|id| requirement_ids.contains(*id))
                        .context("Browser Harness checkpoint references an unknown requirement")?;
                    if !captured_requirement_ids.insert(id.to_string()) {
                        bail!(
                            "Browser Harness checkpoint requirements must be captured exactly once"
                        );
                    }
                }
            }
            _ => bail!("Browser Harness transition action is unsupported"),
        }
    }
    if &captured_requirement_ids != requirement_ids {
        bail!("Browser Harness checkpoints must capture the exact sealed requirement set");
    }
    Ok(())
}

fn bounded_nonempty_string(value: Option<&Value>, max_len: usize) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty() && value.len() <= max_len)
}

fn bounded_string(value: Option<&Value>, max_len: usize) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| value.len() <= max_len)
}

fn browser_script(request: &Value) -> Result<String> {
    let request_json = serde_json::to_string(request)?;
    let request_literal = serde_json::to_string(&request_json)?;
    Ok(BROWSER_SCRIPT.replace("__PLANR_REQUEST_JSON_STRING__", &request_literal))
}

fn run_browser_harness(script: &str) -> Result<std::process::Output> {
    let mut child = Command::new("browser-harness")
        .env("BH_RECORD", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting browser-harness")?;
    child
        .stdin
        .take()
        .context("opening browser-harness stdin")?
        .write_all(script.as_bytes())
        .context("writing browser-harness program")?;
    let output = child
        .wait_with_output()
        .context("waiting for browser-harness")?;
    if !output.status.success() {
        bail!(
            "browser-harness failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}

fn parse_browser_result(stdout: &[u8]) -> Result<Value> {
    let stdout = std::str::from_utf8(stdout).context("browser-harness stdout must be UTF-8")?;
    let result = stdout
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(RESULT_PREFIX))
        .context("browser-harness did not return a structured Evidence result")?;
    let parsed = serde_json::from_str::<Value>(result)
        .context("browser-harness Evidence result must be JSON")?;
    if parsed.get("schema_version").and_then(Value::as_str) != Some(RESULT_SCHEMA) {
        bail!("browser-harness returned the wrong Evidence result schema");
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Value {
        json!({
            "schema_version": REQUEST_SCHEMA,
            "request_id": "ereq-test",
            "request_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "execution_contract_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "target": {"kind": "browser", "uri": "http://127.0.0.1:3000/workflow"},
            "environment": {"kind": "local", "id": "browser-local", "digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"},
            "fixture_disclosure": {"fixtures_used": false, "mocks_used": false},
            "result_contract": {"schema_ref": "schema://planr.structured_observation_results.v2"},
            "requirements": [{
                "id": "obs-state",
                "type": OBSERVATION_TYPE,
                "subject": "#status",
                "expected": {"text": "B", "visible": true},
                "target": {"kind": "browser", "uri": "http://127.0.0.1:3000/workflow"},
                "payload_schema": {"schema_ref": OBSERVATION_SCHEMA},
                "execution_method": {
                    "kind": "agent_skill",
                    "skill": "browser-harness",
                    "result_schema": {"schema_ref": "planr.evidence.agent-skill-result.v1"}
                },
                "state_transitions": {
                    "scenario_id": "fixture-form-v2",
                    "steps": [
                        {"action": "fill", "label": "Name", "value": "Pikachu"},
                        {"action": "select", "label": "Type", "value": "Electric"},
                        {"action": "click", "role": "button", "name": "Advance"},
                        {"action": "checkpoint", "requirements": ["obs-state"]}
                    ]
                }
            }]
        })
    }

    #[test]
    fn accepts_the_closed_dom_state_batch() {
        let request = request();
        validate_request(&request).unwrap();
        let script = browser_script(&request).unwrap();
        assert!(script.contains("click_accessible"));
        assert!(script.contains("set_control"));
        assert!(script.contains("capture_checkpoint"));
        assert!(script.contains("total_deadline = time.monotonic() + 15.0"));
        assert!(script.contains("ereq-test"));
        assert!(!script.contains("time.monotonic() + 5.0"));
        assert!(!script.contains("capture_screenshot"));
        assert!(!script.contains("start_recording"));
        assert!(!script.contains("stop_recording"));
    }

    #[test]
    fn rejects_arbitrary_actions_and_observation_types() {
        let mut wrong_action = request();
        wrong_action["requirements"][0]["state_transitions"]["steps"][0]["action"] = json!("eval");
        assert!(validate_request(&wrong_action).is_err());

        let mut extra_fill_key = request();
        extra_fill_key["requirements"][0]["state_transitions"]["steps"][0]["arbitrary"] =
            json!("code");
        assert!(validate_request(&extra_fill_key).is_err());

        let mut unbounded_value = request();
        unbounded_value["requirements"][0]["state_transitions"]["steps"][0]["value"] =
            json!("x".repeat(4097));
        assert!(validate_request(&unbounded_value).is_err());

        let mut wrong_type = request();
        wrong_type["requirements"][0]["type"] = json!("com.planr.web.visual_state");
        assert!(validate_request(&wrong_type).is_err());

        let mut missing_checkpoint = request();
        missing_checkpoint["requirements"][0]["state_transitions"]["steps"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(validate_request(&missing_checkpoint).is_err());

        let mut wrong_target = request();
        wrong_target["requirements"][0]["target"]["uri"] = json!("http://127.0.0.1:3000/other");
        assert!(validate_request(&wrong_target).is_err());
    }

    #[test]
    fn accepts_one_scenario_declaration_with_exact_requirement_references() {
        let mut request = request();
        let mut second = request["requirements"][0].clone();
        second["id"] = json!("obs-second");
        second["state_transitions"] = json!({"scenario_ref": "fixture-form-v2"});
        request["requirements"].as_array_mut().unwrap().push(second);
        request["requirements"][0]["state_transitions"]["steps"][3]["requirements"] =
            json!(["obs-state", "obs-second"]);
        validate_request(&request).unwrap();

        let mut unknown_reference = request.clone();
        unknown_reference["requirements"][1]["state_transitions"]["scenario_ref"] =
            json!("other-scenario");
        assert!(validate_request(&unknown_reference).is_err());

        let mut second_declaration = request;
        second_declaration["requirements"][1]["state_transitions"] =
            second_declaration["requirements"][0]["state_transitions"].clone();
        assert!(validate_request(&second_declaration).is_err());
    }

    #[test]
    fn extracts_only_the_marked_structured_result() {
        let stdout = format!(
            "update banner\n{RESULT_PREFIX}{}\n",
            json!({"schema_version": RESULT_SCHEMA, "request_id": "ereq-test"})
        );
        let parsed = parse_browser_result(stdout.as_bytes()).unwrap();
        assert_eq!(parsed["request_id"], "ereq-test");
    }

    #[test]
    fn documented_dom_state_schema_accepts_the_adapter_payload() {
        let schema: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/docs/contracts/schemas/com.planr.web.dom_state.v2.schema.json"
        )))
        .unwrap();
        let validator = jsonschema::draft202012::options().build(&schema).unwrap();
        assert!(validator.is_valid(&json!({
            "schema_ref": OBSERVATION_SCHEMA,
            "selector": "#status",
            "text": "B",
            "visible": true,
            "contains_text": ["B"],
            "excludes_text": ["Error"],
            "storage": null,
            "selected": {},
            "accessibility": {"unnamed_controls": 0, "unlabelled_fields": 0},
            "layout": {"horizontal_overflow": false},
            "runtime": {"error_count": 0, "external_request_count": 0}
        })));
    }
}
