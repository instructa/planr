use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    process::{Command, ExitCode, Stdio},
};

const REQUEST_SCHEMA: &str = "planr.evidence.adapter-request.v1";
const RESULT_SCHEMA: &str = "planr.structured_observation_results.v2";
const OBSERVATION_TYPE: &str = "com.planr.web.dom_state";
const OBSERVATION_SCHEMA: &str = "schema://com.planr.web.dom_state.v1";
const RESULT_PREFIX: &str = "PLANR_EVIDENCE_RESULT_JSON=";

const BROWSER_SCRIPT: &str = r#"
import json
import time

request = json.loads(__PLANR_REQUEST_JSON_STRING__)
target = request["target"]
requirements = request["requirements"]

def ax_value(node, field):
    value = node.get(field) or {}
    return value.get("value")

def click_accessible(role, name):
    nodes = cdp("Accessibility.getFullAXTree").get("nodes", [])
    matches = [
        node for node in nodes
        if not node.get("ignored")
        and ax_value(node, "role") == role
        and ax_value(node, "name") == name
        and node.get("backendDOMNodeId") is not None
    ]
    if len(matches) != 1:
        raise RuntimeError(f"expected one accessible {role!r} named {name!r}, found {len(matches)}")
    quad = cdp("DOM.getBoxModel", backendNodeId=matches[0]["backendDOMNodeId"])["model"]["content"]
    x = sum(quad[0::2]) / 4
    y = sum(quad[1::2]) / 4
    click_at_xy(x, y)

def set_control(selector, value, action):
    expression = (
        "(()=>{const e=document.querySelector(" + json.dumps(selector) + ");"
        "if(!e)return {ok:false,reason:'missing'};"
        "if(e.disabled)return {ok:false,reason:'disabled'};"
        "const action=" + json.dumps(action) + ";"
        "if(action==='fill'&&!['INPUT','TEXTAREA'].includes(e.tagName))"
        "return {ok:false,reason:'not-fillable'};"
        "if(action==='select'&&e.tagName!=='SELECT')"
        "return {ok:false,reason:'not-select'};"
        "const value=" + json.dumps(value) + ";"
        "if(action==='select'&&![...e.options].some(o=>o.value===value&&!o.disabled))"
        "return {ok:false,reason:'option-unavailable'};"
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
        raise RuntimeError(f"{action} failed for {selector!r}: {reason}")

def observe(selector):
    expression = (
        "(()=>{const e=document.querySelector(" + json.dumps(selector) + ");"
        "if(!e)return null;"
        "const visible=typeof e.checkVisibility==='function'"
        "?e.checkVisibility({checkOpacity:true,checkVisibilityCSS:true})"
        ":!!(e.getClientRects().length&&getComputedStyle(e).visibility!=='hidden');"
        "return {text:(e.textContent||'').trim(),visible};})()"
    )
    return js(expression)

def expected_matches(actual, expected):
    return actual is not None and all(actual.get(key) == value for key, value in expected.items())

tab = new_tab(target["uri"])
try:
    if not wait_for_load():
        raise RuntimeError("target did not finish loading")
    initial = page_info()["url"]
    transitions = requirements[0].get("state_transitions") or []
    if transitions:
        activate_tab(tab)
    for transition in transitions:
        action = transition["action"]
        if action == "click":
            click_accessible(transition["role"], transition["name"])
        else:
            set_control(transition["subject"], transition["value"], action)

    observations = []
    statuses = []
    for requirement in requirements:
        deadline = time.monotonic() + 5.0
        actual = None
        while time.monotonic() < deadline:
            actual = observe(requirement["subject"])
            if expected_matches(actual, requirement["expected"]):
                break
            time.sleep(0.05)
        passed = expected_matches(actual, requirement["expected"])
        statuses.append(passed)
        actual = actual or {"text": None, "visible": False}
        observations.append({
            "requirement_id": requirement["id"],
            "type": requirement["type"],
            "actual": {
                "schema_ref": requirement["payload_schema"]["schema_ref"],
                "selector": requirement["subject"],
                "text": actual.get("text"),
                "visible": actual.get("visible", False),
            },
        })

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
    let expected_transitions = requirements[0]
        .get("state_transitions")
        .cloned()
        .unwrap_or_else(|| json!([]));
    validate_transitions(&expected_transitions)?;
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
        if expected.is_empty()
            || expected
                .keys()
                .any(|key| !matches!(key.as_str(), "text" | "visible"))
            || expected.get("text").is_some_and(|value| !value.is_string())
            || expected
                .get("visible")
                .is_some_and(|value| !value.is_boolean())
        {
            bail!(
                "Browser Harness DOM-state expected supports only string text and boolean visible"
            );
        }
        if requirement
            .get("payload_schema")
            .and_then(|schema| schema.get("schema_ref"))
            .and_then(Value::as_str)
            != Some(OBSERVATION_SCHEMA)
        {
            bail!("Browser Harness DOM-state requirement has the wrong payload schema");
        }
        if requirement
            .get("state_transitions")
            .cloned()
            .unwrap_or_else(|| json!([]))
            != expected_transitions
        {
            bail!("Browser Harness adapter requires one shared transition sequence per batch");
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

fn validate_transitions(transitions: &Value) -> Result<()> {
    let transitions = transitions
        .as_array()
        .context("Browser Harness state_transitions must be an array")?;
    if transitions.len() > 64 {
        bail!("Browser Harness supports at most 64 state transitions per batch");
    }
    for transition in transitions {
        let object = transition
            .as_object()
            .context("Browser Harness state transition must be an object")?;
        match object.get("action").and_then(Value::as_str) {
            Some("click") => {
                if object.len() != 3
                    || !bounded_nonempty_string(object.get("role"), 128)
                    || !bounded_nonempty_string(object.get("name"), 512)
                {
                    bail!("Browser Harness click transitions require only action, role, and name");
                }
            }
            Some("fill" | "select") => {
                if object.len() != 3
                    || !bounded_nonempty_string(object.get("subject"), 1024)
                    || !bounded_string(object.get("value"), 4096)
                {
                    bail!(
                        "Browser Harness fill/select transitions require only action, subject, and value"
                    );
                }
            }
            _ => bail!("Browser Harness supports only click, fill, and select transitions"),
        }
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
                "state_transitions": [
                    {"action": "fill", "subject": "#name", "value": "Pikachu"},
                    {"action": "select", "subject": "#type", "value": "electric"},
                    {"action": "click", "role": "button", "name": "Advance"}
                ]
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
        assert!(script.contains("if transitions:\n        activate_tab(tab)"));
        assert!(script.contains("ereq-test"));
        assert!(!script.contains("capture_screenshot"));
        assert!(!script.contains("start_recording"));
        assert!(!script.contains("stop_recording"));
    }

    #[test]
    fn rejects_arbitrary_actions_and_observation_types() {
        let mut wrong_action = request();
        wrong_action["requirements"][0]["state_transitions"][0]["action"] = json!("eval");
        assert!(validate_request(&wrong_action).is_err());

        let mut extra_fill_key = request();
        extra_fill_key["requirements"][0]["state_transitions"][0]["arbitrary"] = json!("code");
        assert!(validate_request(&extra_fill_key).is_err());

        let mut unbounded_value = request();
        unbounded_value["requirements"][0]["state_transitions"][0]["value"] =
            json!("x".repeat(4097));
        assert!(validate_request(&unbounded_value).is_err());

        let mut wrong_type = request();
        wrong_type["requirements"][0]["type"] = json!("com.planr.web.visual_state");
        assert!(validate_request(&wrong_type).is_err());
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
            "/docs/contracts/schemas/com.planr.web.dom_state.v1.schema.json"
        )))
        .unwrap();
        let validator = jsonschema::draft202012::options().build(&schema).unwrap();
        assert!(validator.is_valid(&json!({
            "schema_ref": OBSERVATION_SCHEMA,
            "selector": "#status",
            "text": "B",
            "visible": true
        })));
    }
}
