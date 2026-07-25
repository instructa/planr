use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command as StdCommand;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

fn planr() -> Command {
    let mut cmd = Command::cargo_bin("planr").expect("planr binary");
    // Tests may run inside a real host session (Cursor terminal, Codex
    // sandbox); scrub the host-identifying vars observed_client() reads
    // so detection is opt-in per test and results are deterministic.
    for var in [
        "CODEX_SANDBOX",
        "CODEX_SESSION_ID",
        "CLAUDECODE",
        "CURSOR_AGENT",
        "CURSOR_INVOKED_AS",
        "PLANR_PROFILE",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

fn single_json_document(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("stdout must be exactly one JSON document")
}

fn assert_eval_envelope(value: &Value, command: &str, ok: bool) {
    assert_eq!(value["command"], command, "{command} command");
    assert_eq!(value["ok"], ok, "{command} ok: {value}");
    assert!(value.get("object").is_some(), "{command} missing object");
    assert!(value["warnings"].is_array(), "{command} warnings");
    assert!(value["reasons"].is_array(), "{command} reasons");
    assert!(value.get("error").is_some(), "{command} missing error");
    if ok {
        assert!(value["error"].is_null(), "{command} successful error");
    } else if value["error"].is_object() {
        assert!(
            value["error"]["reasons"].is_array(),
            "{command} error.reasons"
        );
        assert!(
            value["error"].get("field").is_some(),
            "{command} error.field"
        );
    }
}

fn verified_eval_route_observation(
    client: &str,
    model: &str,
    effort: &str,
    profile: &str,
) -> Value {
    let stage = |enforcement: &str, evidence: &str| {
        json!({
            "profile": profile,
            "client": client,
            "provider": {"value": "openai", "enforcement": enforcement, "evidence": evidence},
            "runtime": {"value": "codex-cli", "enforcement": enforcement, "evidence": evidence},
            "profile_config_digest": {"value": "sha256:2222222222222222222222222222222222222222222222222222222222222222", "enforcement": enforcement, "evidence": evidence},
            "runner_harness_version": {"value": "supplied-evidence-v1", "enforcement": enforcement, "evidence": evidence},
            "agent_type": {"value": "codex-worker", "enforcement": enforcement, "evidence": evidence},
            "model": {"value": model, "enforcement": enforcement, "evidence": evidence},
            "effort": {"value": effort, "enforcement": enforcement, "evidence": evidence},
            "context_fork": {"value": {"mode": "none"}, "enforcement": enforcement, "evidence": evidence}
        })
    };
    json!({
        "requested": stage("requested_only", "policy"),
        "resolved": stage("verified", "binding"),
        "effective": stage("verified", "host_report"),
        "transition": {"kind": "initial", "reason": "host reported effective route", "evidence": ["host_report"]},
        "policy": {"id": "eval-route-policy", "version": "v1"},
        "binding": {"id": "eval-route-binding", "version": "v1"},
        "metering": {
            "wall_time_seconds": {"value": 1, "confidence": "trusted"},
            "tool_calls": {"confidence": "unavailable"},
            "tokens": {"confidence": "unavailable"},
            "credits_micros": {"confidence": "unavailable"}
        }
    })
}

fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json_value).collect()),
        Value::Object(object) => {
            let mut entries = object
                .iter()
                .map(|(key, value)| (key.clone(), canonical_json_value(value)))
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut sorted = serde_json::Map::new();
            for (key, value) in entries {
                sorted.insert(key, value);
            }
            Value::Object(sorted)
        }
        scalar => scalar.clone(),
    }
}

fn replace_json_string(value: &Value, from: &str, to: &str) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| replace_json_string(value, from, to))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), replace_json_string(value, from, to)))
                .collect(),
        ),
        Value::String(text) => Value::String(text.replace(from, to)),
        scalar => scalar.clone(),
    }
}

fn with_canonical_digest(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.remove("digest");
    }
    let bytes = serde_json::to_vec(&canonical_json_value(&value)).unwrap();
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    value["digest"] = json!(digest);
    value
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn mcp_text_value(response: &Value) -> Value {
    serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("MCP text content"),
    )
    .expect("MCP text content must be JSON")
}

#[test]
fn project_plan_map_pick_log_close_flow() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Example"])
        .assert()
        .success()
        .stdout(predicate::str::contains("initialized"));
    for file in [
        "product.md",
        "ownership.md",
        "flows.md",
        "state-ssot.md",
        "constraints.md",
        "quality-gates.md",
    ] {
        assert!(
            dir.path().join(".planr/project").join(file).exists(),
            "missing project pack file {file}"
        );
    }

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "new",
            "Example app",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    let plan_id = value["plan"]["id"].as_str().unwrap();
    let plan_path = value["plan"]["path"].as_str().unwrap();
    for file in [
        "PLANR_MANIFEST.json",
        "README.md",
        "PRODUCT_SPEC.md",
        "UX_FLOWS.md",
        "DESIGN_SYSTEM_SPEC.md",
        "TECH_ARCHITECTURE.md",
        "ADRS.md",
        "AI_SPEC.md",
        "SAFETY_PRIVACY_SECURITY.md",
        "API_AND_DATA_MODEL.md",
        "CLIENT_IMPLEMENTATION_SPEC.md",
        "BACKEND_IMPLEMENTATION_SPEC.md",
        "ANALYTICS_OBSERVABILITY_SPEC.md",
        "QA_ACCEPTANCE_TESTS.md",
        "RELEASE_READINESS.md",
        "TASKS.md",
        "REFERENCES.md",
    ] {
        assert!(
            std::path::Path::new(plan_path).join(file).exists(),
            "missing product plan file {file}"
        );
    }

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "map",
            "build",
            "--from",
            plan_id,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("created"));

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    let item_id = value["item"]["id"].as_str().unwrap();

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            item_id,
            "--summary",
            "Implemented in test",
            "--cmd",
            "cargo test",
        ])
        .assert()
        .success();
    let conn = Connection::open(&db).unwrap();
    let run_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(run_count, 1);

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "search", "Implemented"])
        .assert()
        .success()
        .stdout(predicate::str::contains("result"));

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "close",
            item_id,
            "--summary",
            "Done",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("item closed"));
}

#[test]
fn doctor_install_and_mcp_smoke() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Example"])
        .assert()
        .success();

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "doctor", "--client", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("doctor complete"));

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "install",
            "codex",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("mcp"));

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "install", "claude"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude integration written"));
    assert!(dir.path().join(".mcp.json").exists());

    let mut cmd = planr();
    cmd.current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("planr_map_show"))
        .stdout(predicate::str::contains("planr_plan_create"))
        .stdout(predicate::str::contains("planr_log_read"));
}

#[test]
fn mcp_contract_install_fixtures_and_cli_docs_do_not_drift() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Contract"])
        .assert()
        .success();

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    planr()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("planr {}\n", env!("CARGO_PKG_VERSION")));
    let fixture: Value =
        serde_json::from_slice(&fs::read(root.join("docs/fixtures/mcp-contract.json")).unwrap())
            .unwrap();

    // Distribution version drift guard: the npm wrapper and both plugin
    // manifests must carry the crate version, or plugin installs report a
    // stale version even though the content is current.
    for manifest in [
        "package.json",
        "plugins/planr/.codex-plugin/plugin.json",
        "plugins/planr/.claude-plugin/plugin.json",
        ".cursor-plugin/plugin.json",
    ] {
        let value: Value = serde_json::from_slice(&fs::read(root.join(manifest)).unwrap()).unwrap();
        assert_eq!(
            value["version"].as_str(),
            Some(env!("CARGO_PKG_VERSION")),
            "{manifest} version must match Cargo.toml"
        );
    }

    let mcp_input = [
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string(),
        json!({"jsonrpc":"2.0","id":2,"method":"resources/list"}).to_string(),
        json!({"jsonrpc":"2.0","id":3,"method":"prompts/list"}).to_string(),
        json!({
            "jsonrpc":"2.0",
            "id":4,
            "method":"tools/call",
            "params":{"name":"planr_missing_tool","arguments":{}}
        })
        .to_string(),
    ]
    .join("\n")
        + "\n";
    let mcp_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(mcp_input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let responses = String::from_utf8(mcp_output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();

    let mut actual_tools = responses[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let mut expected_tools = fixture["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    actual_tools.sort();
    expected_tools.sort();
    assert_eq!(actual_tools, expected_tools);
    for tool in responses[0]["result"]["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object", "{name} schema must be an object");
        assert!(
            schema["properties"].is_object(),
            "{name} must declare real properties"
        );
        let properties = schema["properties"].as_object().unwrap();
        // Every required field must be a declared property.
        for required in schema["required"].as_array().unwrap() {
            assert!(
                properties.contains_key(required.as_str().unwrap()),
                "{name} requires undeclared field {required}"
            );
        }
        // Hook ingestion is the only tool allowed to accept arbitrary keys.
        if name == "planr_review_ingest" {
            assert_eq!(schema["additionalProperties"], true, "{name}");
        } else {
            assert_eq!(schema["additionalProperties"], false, "{name}");
        }
    }
    // Spot-check schema/handler agreement for a mutating tool.
    let item_create = responses[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "planr_item_create")
        .unwrap();
    let required = item_create["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(required, vec!["title", "description"]);

    let mut actual_resources = responses[1]["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| resource["uri"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let mut expected_resources = fixture["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| resource.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    actual_resources.sort();
    expected_resources.sort();
    assert_eq!(actual_resources, expected_resources);

    let mut actual_prompts = responses[2]["result"]["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|prompt| prompt["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let mut expected_prompts = fixture["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|prompt| prompt.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    actual_prompts.sort();
    expected_prompts.sort();
    assert_eq!(actual_prompts, expected_prompts);

    let unknown_tool_text = responses[3]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(unknown_tool_text.contains(fixture["unknown_tool_error"].as_str().unwrap()));

    for client in ["codex", "claude", "cursor"] {
        let output = planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "install", client, "--dry-run"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let output = String::from_utf8(output).unwrap();
        for fragment in fixture["install_fragments"][client].as_array().unwrap() {
            assert!(
                output.contains(fragment.as_str().unwrap()),
                "missing {client} install fragment {fragment}"
            );
        }
        planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "install", client])
            .assert()
            .success();
    }
    assert!(
        dir.path()
            .join(".planr/integrations/codex-mcp.toml")
            .exists()
    );
    assert!(dir.path().join(".mcp.json").exists());
    assert!(dir.path().join(".cursor/mcp.json").exists());
    // `planr install cursor` is the one-command Cursor setup: MCP config plus
    // subagent roles plus the full skill set, matching the plugin experience.
    for provisioned in [
        ".cursor/agents/planr-worker.md",
        ".cursor/agents/planr-reviewer.md",
        ".cursor/skills/planr/SKILL.md",
        ".cursor/skills/planr-loop/SKILL.md",
        ".cursor/skills/planr-work/SKILL.md",
        ".cursor/skills/planr-review/SKILL.md",
    ] {
        assert!(
            dir.path().join(provisioned).exists(),
            "install cursor should write {provisioned}"
        );
    }

    let prompt_cli = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "prompt",
            "cli",
            "--client",
            "codex",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let prompt_cli: Value = serde_json::from_slice(&prompt_cli).unwrap();
    assert_eq!(prompt_cli["global_config_edited"], false);
    assert!(
        prompt_cli["prompt"]
            .as_str()
            .unwrap()
            .contains("planr map preview")
    );

    let prompt_mcp = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "prompt", "mcp"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let prompt_mcp: Value = serde_json::from_slice(&prompt_mcp).unwrap();
    assert!(
        prompt_mcp["prompt"]
            .as_str()
            .unwrap()
            .contains("\"mcpServers\"")
    );

    let prompt_http = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "prompt", "http"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let prompt_http: Value = serde_json::from_slice(&prompt_http).unwrap();
    assert!(prompt_http["prompt"].as_str().unwrap().contains("/review"));

    let cli_reference =
        fs::read_to_string(root.join("apps/docs/content/docs/reference/cli.mdx")).unwrap();
    for command in fixture["cli_reference_commands"].as_array().unwrap() {
        assert!(
            cli_reference.contains(command.as_str().unwrap()),
            "CLI reference missing {command}"
        );
    }
    let review_help = planr()
        .current_dir(dir.path())
        .args(["review", "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let review_help = String::from_utf8(review_help).unwrap();
    for subcommand in ["annotate", "ingest", "artifact", "evidence", "close"] {
        assert!(review_help.contains(subcommand));
    }
}

#[test]
fn eval_cli_and_mcp_share_one_surface() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Eval app"])
        .assert()
        .success();
    let evidence_item = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Eval evidence owner",
            "--description",
            "owns eval evidence refs",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let evidence_item: Value = serde_json::from_slice(&evidence_item).unwrap();
    let evidence_item_id = evidence_item["item"]["id"].as_str().unwrap();
    let evidence_log = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "log",
            "add",
            "--item",
            evidence_item_id,
            "--summary",
            "Eval comparison evidence is attached here",
            "--kind",
            "verification",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let evidence_log: Value = serde_json::from_slice(&evidence_log).unwrap();
    let evidence_log_id = evidence_log["log"]["id"].as_str().unwrap();

    let suite = with_canonical_digest(json!({
        "suite_id": "eval-suite",
        "suite_version": "v1",
        "schema_version": "eval.suite.v1",
        "fixtures": [{"id": "fixture", "path": "fixture.json", "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111"}],
        "scorers": [{"id": "score", "version": "v1", "kind": "test"}],
        "cases": [{
            "case_id": "case-a",
            "fixture_id": "fixture",
            "fixture_ids": ["fixture"],
            "scorer_id": "score",
            "scorer_ids": ["score@v1"],
            "measures": ["duration_ms", "cost_micros"],
            "sampling": {"repetitions": 3, "warmups": 0, "seed": 1, "min_successful_samples": 3}
        }],
        "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false},
        "comparison_policy_digest": "default"
    }));
    let suite_digest = suite["digest"].as_str().unwrap().to_string();
    let suite_fixture_digest = suite["fixtures"][0]["digest"].as_str().unwrap().to_string();
    let suite_path = dir.path().join("suite.json");
    fs::write(&suite_path, serde_json::to_vec_pretty(&suite).unwrap()).unwrap();
    let suite_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "suite-check",
            "--input",
            suite_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let suite_value = single_json_document(&suite_output);
    assert_eval_envelope(&suite_value, "eval.suite.check", true);
    assert_eq!(suite_value["object"]["verdict"], "valid");
    assert_eq!(suite_value["object"]["suite"]["digest"], suite_digest);

    let mut bad_suite = suite.clone();
    bad_suite["digest"] = json!("suite-a");
    let bad_suite_path = dir.path().join("bad-suite.json");
    fs::write(
        &bad_suite_path,
        serde_json::to_vec_pretty(&bad_suite).unwrap(),
    )
    .unwrap();
    let bad_suite_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "suite-check",
            "--input",
            bad_suite_path.to_str().unwrap(),
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let bad_suite_value = single_json_document(&bad_suite_output);
    assert_eq!(bad_suite_value["error"]["code"], "invalid_input");
    assert!(
        bad_suite_value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("expected sha256")
    );

    let sample_suite_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/eval/planr-lifecycle-smoke.suite.json");
    let sample_suite_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "suite-check",
            "--input",
            sample_suite_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let sample_suite_value = single_json_document(&sample_suite_output);
    assert_eval_envelope(&sample_suite_value, "eval.suite.check", true);
    assert_eq!(
        sample_suite_value["object"]["suite"]["normalized_manifest"]["schema_version"],
        "eval.suite.v1"
    );
    assert_eq!(
        sample_suite_value["object"]["suite"]["digest"],
        "sha256:65c096c54bf89c49a151321a84b28d6419aa08930c8a78ac92f0df3641a6b8ed"
    );

    let runner_fixture = dir.path().join("fixture.json");
    let runner_fixture_bytes = br#"{"fixture":true}"#;
    fs::write(&runner_fixture, runner_fixture_bytes).unwrap();
    let runner_fixture_digest = sha256_prefixed(runner_fixture_bytes);
    let runner_manifest = with_canonical_digest(json!({
        "schema_version": "eval.suite.v1",
        "suite_id": "runner-suite",
        "suite_version": "v1",
        "fixtures": [{"id": "fixture", "path": "fixture.json", "digest": runner_fixture_digest}],
        "scorers": [{"id": "score", "version": "v1", "kind": "test"}],
        "cases": [{
            "case_id": "case-a",
            "fixture_id": "fixture",
            "fixture_ids": ["fixture"],
            "scorer_id": "score",
            "scorer_ids": ["score@v1"],
            "subject": {"kind": "binary", "argv": ["/bin/echo", "ok"]},
            "assertions": [{"kind": "exit_code", "expected": 0}],
            "measures": ["duration_ms"],
            "sampling": {"repetitions": 1, "warmups": 0, "seed": 1, "min_successful_samples": 1},
            "timeout_ms": 1000,
            "output_limit_bytes": 4096
        }],
        "comparison_policy": {"required_case_coverage": 1.0},
        "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
    }));
    let runner_suite_digest = runner_manifest["digest"].as_str().unwrap().to_string();
    let runner_suite_path = dir.path().join("runner-suite.json");
    fs::write(
        &runner_suite_path,
        serde_json::to_vec_pretty(&runner_manifest).unwrap(),
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "suite-check",
            "--input",
            runner_suite_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    let mut altered_runner_manifest = runner_manifest.clone();
    altered_runner_manifest["cases"][0]["subject"]["argv"] = json!(["/bin/echo", "mutated"]);
    let altered_runner_input = json!({
        "id": "altered-runner",
        "suite_digest": runner_suite_digest.clone(),
        "subject": {"kind": "binary", "revision": "altered", "argv": ["/bin/echo", "ok"]},
        "repo_root": ".",
        "runner_manifest": altered_runner_manifest
    });
    let altered_runner_path = dir.path().join("altered-runner.json");
    fs::write(
        &altered_runner_path,
        serde_json::to_vec_pretty(&altered_runner_input).unwrap(),
    )
    .unwrap();
    let altered_runner_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "run",
            "--input",
            altered_runner_path.to_str().unwrap(),
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let altered_runner_value = single_json_document(&altered_runner_output);
    assert_eq!(altered_runner_value["error"]["code"], "invalid_input");
    assert!(
        altered_runner_value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not match frozen suite")
    );
    let escaped_runner_input = json!({
        "id": "escaped-runner",
        "suite_digest": runner_suite_digest.clone(),
        "subject": {"kind": "binary", "revision": "escaped", "argv": ["/bin/echo", "ok"]},
        "repo_root": dir.path().to_str().unwrap(),
        "runner_manifest": runner_manifest.clone()
    });
    let escaped_runner_path = dir.path().join("escaped-runner.json");
    fs::write(
        &escaped_runner_path,
        serde_json::to_vec_pretty(&escaped_runner_input).unwrap(),
    )
    .unwrap();
    let escaped_runner_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "run",
            "--input",
            escaped_runner_path.to_str().unwrap(),
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let escaped_runner_value = single_json_document(&escaped_runner_output);
    assert_eq!(escaped_runner_value["error"]["code"], "invalid_input");
    assert!(
        escaped_runner_value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("repo_root must be relative")
    );

    fs::write(&runner_fixture, br#"{"fixture":false}"#).unwrap();
    let mutated_fixture_input = json!({
        "id": "mutated-fixture-runner",
        "suite_digest": runner_suite_digest.clone(),
        "subject": {"kind": "binary", "revision": "mutated-fixture", "argv": ["/bin/echo", "ok"]},
        "repo_root": ".",
        "runner_manifest": runner_manifest.clone()
    });
    let mutated_fixture_path = dir.path().join("mutated-fixture-runner.json");
    fs::write(
        &mutated_fixture_path,
        serde_json::to_vec_pretty(&mutated_fixture_input).unwrap(),
    )
    .unwrap();
    let mutated_fixture_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "run",
            "--input",
            mutated_fixture_path.to_str().unwrap(),
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let mutated_fixture_value = single_json_document(&mutated_fixture_output);
    assert_eq!(mutated_fixture_value["error"]["code"], "invalid_input");
    assert!(
        mutated_fixture_value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("digest mismatch")
    );
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "show",
            "run",
            "mutated-fixture-runner",
        ])
        .assert()
        .failure();
    let no_mutated_run_rows: i64 = Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM eval_runs WHERE id = 'mutated-fixture-runner'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(no_mutated_run_rows, 0);

    let run_payload = |id: &str, value: f64, samples: usize| {
        let baseline_attempt = |index: usize,
                                terminal_status: &str,
                                parent: Option<(&str, &str)>| {
            let attempt_id = format!("attempt-baseline-{index}");
            let mut attempt = json!({
                "id": attempt_id,
                "attempt_index": index,
                "terminal_status": terminal_status,
                "countable": true,
                "effective_client": "codex",
                "effective_provider": "openai",
                "effective_runtime": "codex-cli",
                "effective_model": "gpt-5.6-terra",
                "effective_effort": "high",
                "effective_profile_id": "eval-terra-high",
                "profile_config_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                "runner_harness_version": "supplied-evidence-v1",
                "route_observation": verified_eval_route_observation(
                    "codex",
                    "gpt-5.6-terra",
                    "high",
                    "eval-terra-high"
                ),
                "outcome": {"status": terminal_status}
            });
            if let Some((field, parent_id)) = parent {
                attempt[field] = json!(parent_id);
            }
            attempt
        };
        let sample = |index: usize, measure: &str, value: Value, unit: &str, attempt: Value| {
            let metering_basis = if value.is_null() {
                "unavailable"
            } else {
                "actual_trusted"
            };
            json!({
                "repetition_index": index,
                "seed": index,
                "measure": measure,
                "value": value,
                "unit": unit,
                "source": if measure == "duration_ms" { "process" } else { "metering" },
                "metering_basis": metering_basis,
                "basis_source": if measure == "duration_ms" { "process" } else { "metering" },
                "basis_confidence": if metering_basis == "actual_trusted" { "verified" } else { "unavailable" },
                "attempt": attempt
            })
        };
        let baseline_samples = vec![
            sample(
                0,
                "duration_ms",
                json!(value),
                "ms",
                baseline_attempt(0, "fail", None),
            ),
            sample(
                1,
                "duration_ms",
                json!(value),
                "ms",
                baseline_attempt(
                    1,
                    "pass",
                    Some(("retry_of_attempt_id", "attempt-baseline-0")),
                ),
            ),
            sample(
                2,
                "duration_ms",
                json!(value),
                "ms",
                baseline_attempt(
                    2,
                    "pass",
                    Some(("fallback_of_attempt_id", "attempt-baseline-1")),
                ),
            ),
            sample(
                2,
                "cost_micros",
                Value::Null,
                "micros",
                baseline_attempt(
                    2,
                    "pass",
                    Some(("fallback_of_attempt_id", "attempt-baseline-1")),
                ),
            ),
        ];
        let samples_value = if id == "baseline-run" {
            baseline_samples
        } else {
            (0..samples)
                .map(|index| {
                    json!({
                        "repetition_index": index,
                        "seed": index,
                        "measure": "duration_ms",
                        "value": value,
                        "unit": "ms",
                        "source": "process",
                        "metering_basis": "actual_trusted",
                        "basis_source": "process",
                        "basis_confidence": "verified"
                    })
                })
                .collect::<Vec<_>>()
        };
        let declared_samples = samples_value.len();
        json!({
            "id": id,
            "suite_digest": suite_digest,
            "subject": {"kind": "binary", "revision": id, "argv": ["planr", "eval"]},
            "runner_version": "eval-runner-v1",
            "testbed_fingerprint": {"os": "test"},
            "source_state": {"commit": id},
            "status": "success",
            "cases": [{
                "case": {
                    "case_id": "case-a",
                    "scorer_id": "score",
                    "scorer_version": "v1",
                    "fixture_digest": suite_fixture_digest,
                    "status": "pass",
                    "repetition_count": declared_samples,
                    "assertions": [
                        {"kind": "safety_pass", "status": "pass"},
                        {"kind": "quality_pass", "status": "pass"}
                    ]
                },
                "samples": samples_value
            }]
        })
    };
    let invalid_supplied_payload = json!({
        "id": "supplied-extra-case-run",
        "suite_digest": suite_digest,
        "subject": {"kind": "binary", "revision": "supplied-extra-case-run", "argv": ["planr", "eval"]},
        "runner_version": "eval-runner-v1",
        "testbed_fingerprint": {"os": "test"},
        "source_state": {"commit": "supplied-extra-case-run"},
        "status": "success",
        "cases": [{
            "case": {
                "case_id": "case-extra",
                "scorer_id": "score",
                "scorer_version": "v1",
                "fixture_digest": suite_fixture_digest,
                "status": "pass",
                "repetition_count": 1
            },
            "samples": [{
                "repetition_index": 0,
                "seed": 0,
                "measure": "duration_ms",
                "value": 10.0,
                "unit": "ms",
                "source": "process"
            }]
        }]
    });
    let invalid_supplied_path = dir.path().join("supplied-extra-case-run.json");
    fs::write(
        &invalid_supplied_path,
        serde_json::to_vec_pretty(&invalid_supplied_payload).unwrap(),
    )
    .unwrap();
    let invalid_supplied_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "run",
            "--input",
            invalid_supplied_path.to_str().unwrap(),
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let invalid_supplied_value = single_json_document(&invalid_supplied_output);
    assert_eq!(invalid_supplied_value["error"]["code"], "invalid_input");
    assert!(
        invalid_supplied_value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("is not in frozen suite")
    );
    let no_invalid_supplied_rows: i64 = Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM eval_runs WHERE id = 'supplied-extra-case-run'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(no_invalid_supplied_rows, 0);
    let mut cli_mcp_run = None;
    for (name, payload) in [
        ("baseline.json", run_payload("baseline-run", 100.0, 3)),
        ("better.json", run_payload("better-run", 80.0, 3)),
        ("worse.json", run_payload("worse-run", 130.0, 3)),
        ("thin.json", run_payload("thin-run", 100.0, 1)),
        ("cli-mcp-run.json", run_payload("cli-mcp-run", 90.0, 3)),
    ] {
        let path = dir.path().join(name);
        fs::write(&path, serde_json::to_vec_pretty(&payload).unwrap()).unwrap();
        let run_output = planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "eval",
                "run",
                "--input",
                path.to_str().unwrap(),
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let run_value = single_json_document(&run_output);
        assert_eval_envelope(&run_value, "eval.run", true);
        assert_eq!(run_value["object"]["run"]["status"], "success");
        if name == "cli-mcp-run.json" {
            cli_mcp_run = Some(run_value.clone());
        }
    }

    let show_run_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "show",
            "run",
            "baseline-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show_run = single_json_document(&show_run_output);
    assert_eval_envelope(&show_run, "eval.show", true);
    assert_eq!(show_run["object"]["run"]["id"], "baseline-run");
    let cli_run_semantics = |run: &Value| {
        let attempt_semantics = |attempt: &Value| {
            json!({
                "id": attempt["id"],
                "attempt_index": attempt["attempt_index"],
                "terminal_status": attempt["terminal_status"],
                "countable": attempt["countable"],
                "retry_of_attempt_id": attempt["retry_of_attempt_id"],
                "fallback_of_attempt_id": attempt["fallback_of_attempt_id"],
                "escalation_of_attempt_id": attempt["escalation_of_attempt_id"],
                "resume_of_attempt_id": attempt["resume_of_attempt_id"],
                "effective_client": attempt["effective_client"],
                "effective_provider": attempt["effective_provider"],
                "effective_runtime": attempt["effective_runtime"],
                "effective_model": attempt["effective_model"],
                "effective_effort": attempt["effective_effort"],
                "effective_profile_id": attempt["effective_profile_id"],
                "profile_config_digest": attempt["profile_config_digest"],
                "runner_harness_version": attempt["runner_harness_version"],
                "route_observation": attempt["route_observation"],
                "outcome": attempt["outcome"]
            })
        };
        let sample_semantics = |sample: &Value| {
            json!({
                "repetition_index": sample["repetition_index"],
                "seed": sample["seed"],
                "warmup": sample["warmup"],
                "measure": sample["measure"],
                "value": sample["value"],
                "unit": sample["unit"],
                "source": sample["source"],
                "attempt_id": sample["attempt_id"],
                "attempt_index": sample["attempt_index"],
                "metering_basis": sample["metering_basis"],
                "basis_source": sample["basis_source"],
                "basis_confidence": sample["basis_confidence"],
                "estimate_provenance": sample["estimate_provenance"]
            })
        };
        json!({
            "attempt_lineage": run["object"]["run"]["attempt_lineage"],
            "sample_metering": run["object"]["run"]["sample_metering"],
            "efficiency_summary": run["object"]["run"]["efficiency_summary"],
            "case_attempts": run["object"]["run"]["cases"]
                .as_array()
                .unwrap()
                .iter()
                .map(|case| {
                    json!({
                        "case_id": case["case_id"],
                        "attempts": case["attempts"].as_array().unwrap().iter().map(attempt_semantics).collect::<Vec<_>>(),
                        "samples": case["samples"].as_array().unwrap().iter().map(sample_semantics).collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>()
        })
    };
    let cli_mcp_run = cli_mcp_run.expect("cli-mcp-run response");
    let cli_mcp_run_semantics =
        replace_json_string(&cli_run_semantics(&cli_mcp_run), "cli-mcp-run", "mcp-run");

    let improved_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "compare",
            "baseline-run",
            "better-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let improved = single_json_document(&improved_output);
    assert_eval_envelope(&improved, "eval.compare", true);
    assert_eq!(improved["object"]["verdict"], "improved");
    let improved_id = improved["object"]["comparison"]["id"].as_str().unwrap();
    let gate_improved_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "gate",
            improved_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let gate_improved = single_json_document(&gate_improved_output);
    assert_eval_envelope(&gate_improved, "eval.gate", true);

    let regressed_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "compare",
            "baseline-run",
            "worse-run",
        ])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let regressed = single_json_document(&regressed_output);
    assert_eval_envelope(&regressed, "eval.compare", false);
    assert_eq!(regressed["object"]["verdict"], "regressed");
    let regressed_id = regressed["object"]["comparison"]["id"].as_str().unwrap();
    let cli_compare_semantics = |value: &Value| {
        json!({
            "verdict": value["object"]["verdict"],
            "reasons": value["object"]["comparison"]["reasons"],
            "baseline_run_id": value["object"]["comparison"]["baseline_run_id"],
            "candidate_run_id": value["object"]["comparison"]["candidate_run_id"],
            "baseline_efficiency_summary": value["object"]["baseline_efficiency_summary"],
            "candidate_efficiency_summary": value["object"]["candidate_efficiency_summary"],
            "efficiency_summary": value["object"]["efficiency_summary"],
            "effort_recommendation": value["object"]["effort_recommendation"]
        })
    };
    let improved_semantics = cli_compare_semantics(&improved);
    let regressed_semantics = cli_compare_semantics(&regressed);
    let show_regressed_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "show",
            "comparison",
            regressed_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show_regressed = single_json_document(&show_regressed_output);
    let show_regressed_semantics = cli_compare_semantics(&show_regressed);
    let gate_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "gate",
            regressed_id,
        ])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let gate = single_json_document(&gate_output);
    assert_eval_envelope(&gate, "eval.gate", false);
    assert_eq!(gate["ok"], false);
    assert_eq!(gate["object"]["verdict"], "regressed");
    let before_ref = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "show",
            evidence_item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let before_ref: Value = serde_json::from_slice(&before_ref).unwrap();
    let evidence_ref_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "evidence-ref",
            "comparison",
            regressed_id,
            "log",
            evidence_log_id,
            "--item",
            evidence_item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let evidence_ref = single_json_document(&evidence_ref_output);
    assert_eval_envelope(&evidence_ref, "eval.evidence.ref", true);
    assert_eq!(
        evidence_ref["object"]["evidence_ref"]["target_id"],
        regressed_id
    );
    assert_eq!(
        evidence_ref["object"]["evidence_ref"]["closure_authority"],
        false
    );
    let after_ref = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "show",
            evidence_item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let after_ref: Value = serde_json::from_slice(&after_ref).unwrap();
    assert_eq!(after_ref["item"]["status"], before_ref["item"]["status"]);

    let non_review_item = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Not a review",
            "--description",
            "ordinary item",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let non_review_item: Value = serde_json::from_slice(&non_review_item).unwrap();
    let non_review_item_id = non_review_item["item"]["id"].as_str().unwrap();
    let non_review_attachment = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "evidence-ref",
            "run",
            "baseline-run",
            "review",
            non_review_item_id,
            "--item",
            evidence_item_id,
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let non_review_attachment = single_json_document(&non_review_attachment);
    assert_eq!(non_review_attachment["error"]["code"], "invalid_input");
    assert!(
        non_review_attachment["error"]["message"]
            .as_str()
            .unwrap()
            .contains("must be a review item")
    );

    let unrelated_review_target = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Unrelated review target",
            "--description",
            "reviewed elsewhere",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let unrelated_review_target: Value = serde_json::from_slice(&unrelated_review_target).unwrap();
    let unrelated_review_target_id = unrelated_review_target["item"]["id"].as_str().unwrap();
    let unrelated_review = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "request",
            unrelated_review_target_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let unrelated_review: Value = serde_json::from_slice(&unrelated_review).unwrap();
    let unrelated_review_id = unrelated_review["review"]["id"].as_str().unwrap();
    let wrong_review_target = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "evidence-ref",
            "run",
            "baseline-run",
            "review",
            unrelated_review_id,
            "--item",
            evidence_item_id,
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let wrong_review_target = single_json_document(&wrong_review_target);
    assert_eq!(wrong_review_target["error"]["code"], "invalid_input");
    assert!(
        wrong_review_target["error"]["message"]
            .as_str()
            .unwrap()
            .contains("must review item")
    );

    let owned_review = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "request",
            evidence_item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let owned_review: Value = serde_json::from_slice(&owned_review).unwrap();
    let owned_review_id = owned_review["review"]["id"].as_str().unwrap();
    let review_ref_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "evidence-ref",
            "run",
            "baseline-run",
            "review",
            owned_review_id,
            "--item",
            evidence_item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let review_ref = single_json_document(&review_ref_output);
    assert_eval_envelope(&review_ref, "eval.evidence.ref", true);

    let insufficient_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "compare",
            "baseline-run",
            "thin-run",
        ])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let insufficient = single_json_document(&insufficient_output);
    assert_eval_envelope(&insufficient, "eval.compare", false);
    assert_eq!(insufficient["object"]["verdict"], "insufficient_evidence");
    let insufficient_id = insufficient["object"]["comparison"]["id"].as_str().unwrap();

    let invalid_path = dir.path().join("invalid-run.json");
    fs::write(
        &invalid_path,
        format!(r#"{{"suite_digest":"{suite_digest}"}}"#),
    )
    .unwrap();
    let invalid_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "run",
            "--input",
            invalid_path.to_str().unwrap(),
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let invalid = single_json_document(&invalid_output);
    assert_eval_envelope(&invalid, "eval.run", false);
    assert_eq!(invalid["error"]["code"], "invalid_input");
    assert_eq!(
        invalid["error"]["reasons"],
        json!(["missing_required_field"])
    );
    assert_eq!(invalid["error"]["field"], "subject");
    assert!(
        invalid["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing required eval field: subject")
    );

    let missing_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "compare",
            "baseline-run",
            "missing-run",
        ])
        .assert()
        .code(4)
        .get_output()
        .stdout
        .clone();
    let missing = single_json_document(&missing_output);
    assert_eval_envelope(&missing, "eval.compare", false);
    assert_eq!(missing["error"]["code"], "infrastructure_error");
    assert_eq!(missing["error"]["reasons"], json!(["not_found"]));
    assert_eq!(missing["error"]["field"], Value::Null);

    let invalidation_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "invalidate",
            "comparison",
            regressed_id,
            "--reason",
            "reviewed stale comparison",
            "--reason-code",
            "stale",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let invalidation = single_json_document(&invalidation_output);
    assert_eval_envelope(&invalidation, "eval.invalidate", true);
    let invalidation_id = invalidation["object"]["invalidation"]["id"]
        .as_str()
        .unwrap();
    assert_eq!(invalidation["object"]["verdict"], "invalidated");

    let show_invalidation_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "show",
            "invalidation",
            invalidation_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show_invalidation = single_json_document(&show_invalidation_output);
    assert_eval_envelope(&show_invalidation, "eval.show", true);

    let rescore_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "rescore",
            "baseline-run",
            "--id",
            "rescore-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rescore = single_json_document(&rescore_output);
    assert_eval_envelope(&rescore, "eval.rescore", true);
    assert_eq!(rescore["object"]["run"]["rescore_of"], "baseline-run");
    let package = dir.path().join("eval-package.json");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "export",
            "--include-logs",
            "--out",
            package.to_str().unwrap(),
        ])
        .assert()
        .success();
    let package_json: Value = serde_json::from_slice(&fs::read(&package).unwrap()).unwrap();
    let refs = package_json["eval_evidence_refs"].as_array().unwrap();
    assert_eq!(refs.len(), 2);
    assert!(
        refs.iter()
            .any(|reference| reference["target_id"] == regressed_id)
    );
    assert!(
        refs.iter()
            .any(|reference| reference["target_id"] == "baseline-run")
    );
    assert!(
        refs.iter()
            .all(|reference| reference["closure_authority"] == false)
    );
    assert!(
        package_json["eval_runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["id"] == "baseline-run")
    );
    let packaged_baseline = package_json["eval_runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["id"] == "baseline-run")
        .unwrap();
    assert_eq!(packaged_baseline["cases"].as_array().unwrap().len(), 1);
    let packaged_baseline_attempts = packaged_baseline["cases"][0]["attempts"]
        .as_array()
        .expect("eval package exports attempts");
    assert!(
        !packaged_baseline_attempts.is_empty(),
        "eval package must export first-class attempts"
    );
    assert_eq!(
        packaged_baseline["cases"][0]["samples"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    let packaged_sample_attempt_id = packaged_baseline["cases"][0]["samples"][0]["attempt_id"]
        .as_str()
        .unwrap();
    assert!(
        packaged_baseline_attempts
            .iter()
            .any(|attempt| attempt["id"] == packaged_sample_attempt_id),
        "sample attempt_id must resolve to packaged attempt evidence"
    );
    assert!(
        package_json["eval_comparisons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|comparison| comparison["id"] == regressed_id)
    );
    let packaged_suite = package_json["eval_suite_snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|suite| suite["digest"] == suite_digest)
        .unwrap();
    let packaged_regressed_comparison = package_json["eval_comparisons"]
        .as_array()
        .unwrap()
        .iter()
        .find(|comparison| comparison["id"] == regressed_id)
        .unwrap();
    let packaged_invalidation = package_json["eval_invalidations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|invalidation| invalidation["id"] == invalidation_id)
        .unwrap();
    let packaged_regressed_ref = refs
        .iter()
        .find(|reference| reference["target_id"] == regressed_id)
        .unwrap();
    assert!(package_json["plan_files"].is_null());
    let assert_rejected_empty_import =
        |name: &str, package_value: Value, expected_message: &str| {
            let rejected_package = dir.path().join(name);
            fs::write(
                &rejected_package,
                serde_json::to_vec_pretty(&package_value).unwrap(),
            )
            .unwrap();
            let rejected_target = tempdir().unwrap();
            let rejected_db = rejected_target.path().join(".planr/planr.sqlite");
            planr()
                .current_dir(rejected_target.path())
                .args([
                    "--db",
                    rejected_db.to_str().unwrap(),
                    "project",
                    "init",
                    "Rejected eval import target",
                ])
                .assert()
                .success();
            planr()
                .current_dir(rejected_target.path())
                .args([
                    "--db",
                    rejected_db.to_str().unwrap(),
                    "import",
                    rejected_package.to_str().unwrap(),
                    "--confirm",
                ])
                .assert()
                .failure()
                .stderr(predicate::str::contains(expected_message));
            planr()
                .current_dir(rejected_target.path())
                .args([
                    "--db",
                    rejected_db.to_str().unwrap(),
                    "--json",
                    "item",
                    "show",
                    evidence_item_id,
                ])
                .assert()
                .failure();
            planr()
                .current_dir(rejected_target.path())
                .args([
                    "--db",
                    rejected_db.to_str().unwrap(),
                    "--json",
                    "eval",
                    "show",
                    "suite",
                    &suite_digest,
                ])
                .assert()
                .failure();
            planr()
                .current_dir(rejected_target.path())
                .args([
                    "--db",
                    rejected_db.to_str().unwrap(),
                    "--json",
                    "eval",
                    "show",
                    "run",
                    "baseline-run",
                ])
                .assert()
                .failure();
        };
    let assert_rejected_preview = |name: &str, package_value: Value, expected_message: &str| {
        let rejected_package = dir.path().join(name);
        fs::write(
            &rejected_package,
            serde_json::to_vec_pretty(&package_value).unwrap(),
        )
        .unwrap();
        let rejected_target = tempdir().unwrap();
        let rejected_db = rejected_target.path().join(".planr/planr.sqlite");
        planr()
            .current_dir(rejected_target.path())
            .args([
                "--db",
                rejected_db.to_str().unwrap(),
                "project",
                "init",
                "Rejected eval preview target",
            ])
            .assert()
            .success();
        planr()
            .current_dir(rejected_target.path())
            .args([
                "--db",
                rejected_db.to_str().unwrap(),
                "import",
                rejected_package.to_str().unwrap(),
                "--preview",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(expected_message));
    };
    let mut duplicate_run_package = package_json.clone();
    let mut conflicting_run = duplicate_run_package["eval_runs"][0].clone();
    conflicting_run["subject_revision"] = json!("conflicting-duplicate");
    duplicate_run_package["eval_runs"]
        .as_array_mut()
        .unwrap()
        .push(conflicting_run);
    assert_rejected_empty_import(
        "eval-package-duplicate-run.json",
        duplicate_run_package,
        "conflicting eval run id",
    );
    let mut missing_suite_package = package_json.clone();
    missing_suite_package["eval_suite_snapshots"] = json!([]);
    assert_rejected_empty_import(
        "eval-package-missing-suite.json",
        missing_suite_package,
        &format!("suite_digest {suite_digest} is unresolved"),
    );
    let mut missing_comparison_run_package = package_json.clone();
    missing_comparison_run_package["eval_runs"] = json!([]);
    missing_comparison_run_package["eval_invalidations"] = json!([]);
    missing_comparison_run_package["eval_evidence_refs"] = json!([]);
    assert_rejected_empty_import(
        "eval-package-missing-comparison-run.json",
        missing_comparison_run_package,
        "run dependency baseline-run is unresolved",
    );
    let mut missing_invalidation_target_package = package_json.clone();
    let invalidation_index = missing_invalidation_target_package["eval_invalidations"]
        .as_array()
        .unwrap()
        .iter()
        .position(|invalidation| invalidation["id"] == invalidation_id)
        .unwrap();
    missing_invalidation_target_package["eval_invalidations"][invalidation_index]["target_id"] =
        json!("missing-comparison-target");
    missing_invalidation_target_package["eval_evidence_refs"] = json!([]);
    assert_rejected_empty_import(
        "eval-package-missing-invalidation-target.json",
        missing_invalidation_target_package,
        "comparison target missing-comparison-target is unresolved",
    );
    let mut closure_authority_package = package_json.clone();
    closure_authority_package["eval_evidence_refs"][0]["closure_authority"] = json!(true);
    assert_rejected_empty_import(
        "eval-package-closure-authority.json",
        closure_authority_package,
        "closure_authority must be false",
    );
    let mut mismatched_attempt_package = package_json.clone();
    mismatched_attempt_package["eval_runs"][0]["cases"][0]["samples"][0]["repetition_index"] =
        json!(99);
    assert_rejected_empty_import(
        "eval-package-sample-attempt-mismatch.json",
        mismatched_attempt_package,
        "identity does not match referenced attempt",
    );
    let mut invalid_confidence_package = package_json.clone();
    invalid_confidence_package["eval_runs"][0]["cases"][0]["samples"][0]["basis_confidence"] =
        json!("actual_trusted");
    assert_rejected_preview(
        "eval-package-invalid-basis-confidence-preview.json",
        invalid_confidence_package.clone(),
        "invalid eval basis confidence: actual_trusted",
    );
    assert_rejected_empty_import(
        "eval-package-invalid-basis-confidence.json",
        invalid_confidence_package,
        "invalid eval basis confidence: actual_trusted",
    );
    let mut unavailable_value_package = package_json.clone();
    unavailable_value_package["eval_runs"][0]["cases"][0]["samples"][0]["metering_basis"] =
        json!("unavailable");
    unavailable_value_package["eval_runs"][0]["cases"][0]["samples"][0]["basis_confidence"] =
        json!("unavailable");
    unavailable_value_package["eval_runs"][0]["cases"][0]["samples"][0]["value"] = json!(42);
    assert_rejected_preview(
        "eval-package-unavailable-numeric-preview.json",
        unavailable_value_package.clone(),
        "unavailable eval sample values must be null",
    );
    assert_rejected_empty_import(
        "eval-package-unavailable-numeric.json",
        unavailable_value_package,
        "unavailable eval sample values must be null",
    );
    let mut missing_estimate_package = package_json.clone();
    missing_estimate_package["eval_runs"][0]["cases"][0]["samples"][0]["metering_basis"] =
        json!("estimated");
    missing_estimate_package["eval_runs"][0]["cases"][0]["samples"][0]["basis_confidence"] =
        json!("estimated");
    missing_estimate_package["eval_runs"][0]["cases"][0]["samples"][0]
        .as_object_mut()
        .unwrap()
        .remove("estimate_provenance");
    assert_rejected_preview(
        "eval-package-missing-estimate-provenance-preview.json",
        missing_estimate_package.clone(),
        "estimated eval samples require estimate_provenance",
    );
    assert_rejected_empty_import(
        "eval-package-missing-estimate-provenance.json",
        missing_estimate_package,
        "estimated eval samples require estimate_provenance",
    );
    let mut malformed_estimate_package = package_json.clone();
    malformed_estimate_package["eval_runs"][0]["cases"][0]["samples"][0]["metering_basis"] =
        json!("estimated");
    malformed_estimate_package["eval_runs"][0]["cases"][0]["samples"][0]["basis_confidence"] =
        json!("estimated");
    malformed_estimate_package["eval_runs"][0]["cases"][0]["samples"][0]["estimate_provenance"] = json!({
        "method": "fixture-rate-card",
        "version": "live-oracle-v1",
        "rate_micros_per_attempt": 42
    });
    assert_rejected_preview(
        "eval-package-malformed-estimate-provenance-preview.json",
        malformed_estimate_package.clone(),
        "estimated eval samples require estimate_provenance.pricing_reference_id",
    );
    assert_rejected_empty_import(
        "eval-package-malformed-estimate-provenance.json",
        malformed_estimate_package,
        "estimated eval samples require estimate_provenance.pricing_reference_id",
    );
    let mut invalid_run_status_package = package_json.clone();
    invalid_run_status_package["eval_runs"][0]["status"] = json!("not-a-status");
    assert_rejected_empty_import(
        "eval-package-invalid-run-status.json",
        invalid_run_status_package,
        "FOREIGN KEY constraint failed",
    );

    let import_target = tempdir().unwrap();
    let import_db = import_target.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "project",
            "init",
            "Eval import target",
        ])
        .assert()
        .success();
    let import_preview = planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "--json",
            "import",
            package.to_str().unwrap(),
            "--preview",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let import_preview: Value = serde_json::from_slice(&import_preview).unwrap();
    assert_eq!(
        import_preview["report"]["would_create"]["eval_evidence_refs"],
        2
    );
    assert!(
        import_preview["report"]["would_create"]["eval_case_results"]
            .as_u64()
            .unwrap()
            >= 4
    );
    assert!(
        import_preview["report"]["would_create"]["eval_samples"]
            .as_u64()
            .unwrap()
            >= 10
    );
    assert!(
        import_preview["report"]["would_create"]["eval_runs"]
            .as_u64()
            .unwrap()
            >= 4
    );
    assert!(
        import_preview["report"]["would_create"]["eval_comparisons"]
            .as_u64()
            .unwrap()
            >= 3
    );
    let import_apply = planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "--json",
            "import",
            package.to_str().unwrap(),
            "--confirm",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let import_apply: Value = serde_json::from_slice(&import_apply).unwrap();
    assert_eq!(import_apply["imported"]["eval_evidence_refs"], 2);
    let imported_comparison_output = planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "--json",
            "eval",
            "show",
            "comparison",
            regressed_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let imported_comparison = single_json_document(&imported_comparison_output);
    assert_eq!(imported_comparison["object"]["verdict"], "regressed");
    assert_eq!(
        imported_comparison["object"]["comparison"]["created_at"],
        packaged_regressed_comparison["created_at"]
    );
    let imported_suite_output = planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "--json",
            "eval",
            "show",
            "suite",
            &suite_digest,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let imported_suite = single_json_document(&imported_suite_output);
    assert_eq!(
        imported_suite["object"]["suite"]["created_at"],
        packaged_suite["created_at"]
    );
    let imported_run_output = planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "--json",
            "eval",
            "show",
            "run",
            "baseline-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let imported_run = single_json_document(&imported_run_output);
    assert_eq!(
        imported_run["object"]["run"]["created_at"],
        packaged_baseline["created_at"]
    );
    assert_eq!(
        imported_run["object"]["run"]["cases"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        imported_run["object"]["run"]["cases"][0]["samples"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        imported_run["object"]["run"]["cases"][0]["attempts"],
        packaged_baseline["cases"][0]["attempts"]
    );
    assert_eq!(
        imported_run["object"]["run"]["cases"][0]["samples"],
        packaged_baseline["cases"][0]["samples"]
    );
    assert_eq!(
        imported_run["object"]["run"]["cases"][0]["created_at"],
        packaged_baseline["cases"][0]["created_at"]
    );
    assert_eq!(
        imported_run["object"]["run"]["cases"][0]["updated_at"],
        packaged_baseline["cases"][0]["updated_at"]
    );
    assert_eq!(
        imported_run["object"]["run"]["cases"][0]["samples"][0]["captured_at"],
        packaged_baseline["cases"][0]["samples"][0]["captured_at"]
    );
    let imported_attempts = imported_run["object"]["run"]["cases"][0]["attempts"]
        .as_array()
        .unwrap();
    let retry_attempt = imported_attempts
        .iter()
        .find(|attempt| attempt["id"] == "attempt-baseline-1")
        .unwrap();
    let fallback_attempt = imported_attempts
        .iter()
        .find(|attempt| attempt["id"] == "attempt-baseline-2")
        .unwrap();
    assert_eq!(
        retry_attempt["retry_of_attempt_id"],
        json!("attempt-baseline-0")
    );
    assert_eq!(
        fallback_attempt["fallback_of_attempt_id"],
        json!("attempt-baseline-1")
    );
    assert_eq!(fallback_attempt["effective_model"], json!("gpt-5.6-terra"));
    assert_eq!(
        fallback_attempt["route_observation"]["effective"]["effort"]["value"],
        json!("high")
    );
    let imported_samples = imported_run["object"]["run"]["cases"][0]["samples"]
        .as_array()
        .unwrap();
    let unavailable_cost = imported_samples
        .iter()
        .find(|sample| sample["measure"] == "cost_micros")
        .unwrap();
    assert_eq!(unavailable_cost["value"], Value::Null);
    assert_eq!(unavailable_cost["metering_basis"], json!("unavailable"));
    assert_eq!(unavailable_cost["basis_confidence"], json!("unavailable"));
    let reproduced_comparison_output = planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "--json",
            "eval",
            "compare",
            "baseline-run",
            "worse-run",
        ])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let reproduced_comparison = single_json_document(&reproduced_comparison_output);
    assert_eq!(reproduced_comparison["object"]["verdict"], "regressed");
    let imported_invalidation_output = planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "--json",
            "eval",
            "show",
            "invalidation",
            invalidation_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let imported_invalidation = single_json_document(&imported_invalidation_output);
    assert_eq!(
        imported_invalidation["object"]["invalidation"]["created_at"],
        packaged_invalidation["created_at"]
    );
    let imported_package = import_target.path().join("eval-package-imported.json");
    planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "export",
            "--include-logs",
            "--out",
            imported_package.to_str().unwrap(),
        ])
        .assert()
        .success();
    let imported_package_json: Value =
        serde_json::from_slice(&fs::read(&imported_package).unwrap()).unwrap();
    let imported_refs = imported_package_json["eval_evidence_refs"]
        .as_array()
        .unwrap();
    let imported_regressed_ref = imported_refs
        .iter()
        .find(|reference| reference["target_id"] == regressed_id)
        .unwrap();
    assert_eq!(
        imported_regressed_ref["created_at"],
        packaged_regressed_ref["created_at"]
    );
    let run_collision_package = dir.path().join("eval-package-run-collision.json");
    let mut run_collision_json = package_json.clone();
    run_collision_json["eval_runs"][0]["subject_revision"] = json!("conflicting-revision");
    fs::write(
        &run_collision_package,
        serde_json::to_vec_pretty(&run_collision_json).unwrap(),
    )
    .unwrap();
    planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "import",
            run_collision_package.to_str().unwrap(),
            "--confirm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("conflicting eval run id"));
    let comparison_collision_package = dir.path().join("eval-package-comparison-collision.json");
    let mut comparison_collision_json = package_json.clone();
    let comparison_index = comparison_collision_json["eval_comparisons"]
        .as_array()
        .unwrap()
        .iter()
        .position(|comparison| comparison["id"] == regressed_id)
        .unwrap();
    comparison_collision_json["eval_comparisons"][comparison_index]["verdict"] = json!("improved");
    fs::write(
        &comparison_collision_package,
        serde_json::to_vec_pretty(&comparison_collision_json).unwrap(),
    )
    .unwrap();
    planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "import",
            comparison_collision_package.to_str().unwrap(),
            "--confirm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("conflicting eval comparison id"));
    let sample_collision_package = dir.path().join("eval-package-sample-collision.json");
    let mut sample_collision_json = package_json.clone();
    let run_index = sample_collision_json["eval_runs"]
        .as_array()
        .unwrap()
        .iter()
        .position(|run| run["id"] == "baseline-run")
        .unwrap();
    sample_collision_json["eval_runs"][run_index]["id"] = json!("new-run-with-colliding-sample");
    sample_collision_json["eval_runs"][run_index]["subject_revision"] =
        json!("new-run-with-colliding-sample");
    sample_collision_json["eval_runs"][run_index]["cases"][0]["id"] =
        json!("evcase-new-run-with-colliding-sample-case-a-score");
    sample_collision_json["eval_comparisons"] = json!([]);
    sample_collision_json["eval_invalidations"] = json!([]);
    sample_collision_json["eval_evidence_refs"] = json!([]);
    fs::write(
        &sample_collision_package,
        serde_json::to_vec_pretty(&sample_collision_json).unwrap(),
    )
    .unwrap();
    planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "import",
            sample_collision_package.to_str().unwrap(),
            "--confirm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("conflicting eval attempt id"));
    planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "--json",
            "eval",
            "show",
            "run",
            "new-run-with-colliding-sample",
        ])
        .assert()
        .failure();
    let imported_review_output = planr()
        .current_dir(import_target.path())
        .args([
            "--db",
            import_db.to_str().unwrap(),
            "--json",
            "item",
            "show",
            owned_review_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let imported_review: Value = serde_json::from_slice(&imported_review_output).unwrap();
    assert_eq!(imported_review["item"]["work_type"], "review");

    let dangling_package = dir.path().join("eval-package-dangling.json");
    let mut dangling_json = package_json.clone();
    dangling_json["eval_comparisons"] = json!([]);
    dangling_json["eval_invalidations"] = json!([]);
    fs::write(
        &dangling_package,
        serde_json::to_vec_pretty(&dangling_json).unwrap(),
    )
    .unwrap();
    let dangling_target = tempdir().unwrap();
    let dangling_db = dangling_target.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dangling_target.path())
        .args([
            "--db",
            dangling_db.to_str().unwrap(),
            "project",
            "init",
            "Dangling eval import target",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dangling_target.path())
        .args([
            "--db",
            dangling_db.to_str().unwrap(),
            "import",
            dangling_package.to_str().unwrap(),
            "--confirm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "eval evidence ref comparison target",
        ));
    planr()
        .current_dir(dangling_target.path())
        .args([
            "--db",
            dangling_db.to_str().unwrap(),
            "--json",
            "item",
            "show",
            evidence_item_id,
        ])
        .assert()
        .failure();
    planr()
        .current_dir(dangling_target.path())
        .args([
            "--db",
            dangling_db.to_str().unwrap(),
            "--json",
            "eval",
            "show",
            "run",
            "baseline-run",
        ])
        .assert()
        .failure();

    let foreign_artifact_item = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Foreign artifact owner",
            "--description",
            "owns a different artifact",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let foreign_artifact_item: Value = serde_json::from_slice(&foreign_artifact_item).unwrap();
    let foreign_artifact_item_id = foreign_artifact_item["item"]["id"].as_str().unwrap();
    let foreign_artifact = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "artifact",
            "add",
            "foreign eval artifact",
            "--item",
            foreign_artifact_item_id,
            "--content",
            "foreign proof",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let foreign_artifact: Value = serde_json::from_slice(&foreign_artifact).unwrap();
    let foreign_artifact_id = foreign_artifact["artifact"]["id"].as_str().unwrap();
    let foreign_artifact_ref = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "evidence-ref",
            "run",
            "better-run",
            "artifact",
            foreign_artifact_id,
            "--item",
            evidence_item_id,
        ])
        .assert()
        .code(3)
        .get_output()
        .stdout
        .clone();
    let foreign_artifact_ref = single_json_document(&foreign_artifact_ref);
    assert_eq!(foreign_artifact_ref["error"]["code"], "invalid_input");
    assert!(
        foreign_artifact_ref["error"]["message"]
            .as_str()
            .unwrap()
            .contains("artifact attachment must belong")
    );
    let owned_artifact = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "artifact",
            "add",
            "owned eval artifact",
            "--item",
            evidence_item_id,
            "--content",
            "owned proof",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let owned_artifact: Value = serde_json::from_slice(&owned_artifact).unwrap();
    let owned_artifact_id = owned_artifact["artifact"]["id"].as_str().unwrap();
    let owned_artifact_ref = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "evidence-ref",
            "run",
            "better-run",
            "artifact",
            owned_artifact_id,
            "--item",
            evidence_item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let owned_artifact_ref = single_json_document(&owned_artifact_ref);
    assert_eval_envelope(&owned_artifact_ref, "eval.evidence.ref", true);

    let mut mcp_mutated_fixture_input = mutated_fixture_input.clone();
    mcp_mutated_fixture_input["id"] = json!("mcp-mutated-fixture-runner");
    mcp_mutated_fixture_input["subject"]["revision"] = json!("mcp-mutated-fixture");
    let mut mcp_invalid_supplied_payload = invalid_supplied_payload.clone();
    mcp_invalid_supplied_payload["id"] = json!("mcp-supplied-extra-case-run");
    mcp_invalid_supplied_payload["subject"]["revision"] = json!("mcp-supplied-extra-case-run");
    mcp_invalid_supplied_payload["source_state"]["commit"] = json!("mcp-supplied-extra-case-run");

    let mcp_input = [
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_eval_suite_check","arguments":{"input": suite, "source_path": "mcp"}}}).to_string(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"planr_eval_run","arguments":{"input": run_payload("mcp-run", 90.0, 3)}}}).to_string(),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"planr_eval_show","arguments":{"kind":"comparison","id":regressed_id}}}).to_string(),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"planr_eval_compare","arguments":{"baseline_run_id":"baseline-run","candidate_run_id":"better-run"}}}).to_string(),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"planr_eval_gate","arguments":{"comparison_id":improved_id}}}).to_string(),
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"planr_eval_compare","arguments":{"baseline_run_id":"baseline-run","candidate_run_id":"worse-run"}}}).to_string(),
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"planr_eval_gate","arguments":{"comparison_id":regressed_id}}}).to_string(),
        json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"planr_eval_compare","arguments":{"baseline_run_id":"baseline-run","candidate_run_id":"thin-run"}}}).to_string(),
        json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"planr_eval_gate","arguments":{"comparison_id":insufficient_id}}}).to_string(),
        json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"planr_eval_evidence_ref","arguments":{"target_kind":"run","target_id":"mcp-run","attachment_kind":"log","attachment_id":evidence_log_id,"item_id":evidence_item_id}}}).to_string(),
        json!({"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"planr_eval_invalidate","arguments":{"target_kind":"run","target_id":"mcp-run","reason":"mcp parity invalidation","reason_codes":["parity"]}}}).to_string(),
        json!({"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"planr_eval_rescore","arguments":{"run_id":"baseline-run","id":"mcp-rescore-run"}}}).to_string(),
        json!({"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"planr_eval_show","arguments":{"kind":"bogus","id":regressed_id}}}).to_string(),
        json!({"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"planr_eval_compare","arguments":{"baseline_run_id":"baseline-run","candidate_run_id":"missing-run"}}}).to_string(),
        json!({"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"planr_eval_run","arguments":{"input":altered_runner_input}}}).to_string(),
        json!({"jsonrpc":"2.0","id":16,"method":"tools/call","params":{"name":"planr_eval_run","arguments":{"input":escaped_runner_input}}}).to_string(),
        json!({"jsonrpc":"2.0","id":17,"method":"tools/call","params":{"name":"planr_eval_run","arguments":{"input":mcp_mutated_fixture_input}}}).to_string(),
        json!({"jsonrpc":"2.0","id":18,"method":"tools/call","params":{"name":"planr_eval_run","arguments":{"input":mcp_invalid_supplied_payload}}}).to_string(),
    ]
    .join("\n")
        + "\n";
    let mcp_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(mcp_input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let responses = String::from_utf8(mcp_output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    for (index, command) in [
        "eval.suite.check",
        "eval.run",
        "eval.show",
        "eval.compare",
        "eval.gate",
    ]
    .iter()
    .enumerate()
    {
        let envelope = mcp_text_value(&responses[index]);
        assert_eval_envelope(&envelope, command, true);
    }
    let mcp_show = mcp_text_value(&responses[2]);
    assert_eq!(
        cli_compare_semantics(&mcp_show),
        show_regressed_semantics,
        "MCP comparison show must preserve CLI semantic fields"
    );
    let mcp_run = mcp_text_value(&responses[1]);
    assert_eq!(
        cli_run_semantics(&mcp_run),
        cli_mcp_run_semantics,
        "Direct CLI and MCP eval.run responses must project equivalent lineage, sample metering, and efficiency semantics"
    );
    let mcp_gate = mcp_text_value(&responses[4]);
    assert_eq!(mcp_show["object"]["verdict"], "regressed");
    assert_eq!(mcp_gate["object"]["ok"], true);
    let mcp_regressed_compare = mcp_text_value(&responses[5]);
    assert_eval_envelope(&mcp_regressed_compare, "eval.compare", false);
    assert_eq!(mcp_regressed_compare["object"]["verdict"], "regressed");
    assert_eq!(
        cli_compare_semantics(&mcp_regressed_compare),
        regressed_semantics
    );
    let mcp_improved_compare = mcp_text_value(&responses[3]);
    assert_eq!(
        cli_compare_semantics(&mcp_improved_compare),
        improved_semantics
    );
    let mcp_regressed_gate = mcp_text_value(&responses[6]);
    assert_eval_envelope(&mcp_regressed_gate, "eval.gate", false);
    assert_eq!(mcp_regressed_gate["object"]["verdict"], "regressed");
    let mcp_insufficient_compare = mcp_text_value(&responses[7]);
    assert_eval_envelope(&mcp_insufficient_compare, "eval.compare", false);
    assert_eq!(
        mcp_insufficient_compare["object"]["verdict"],
        "insufficient_evidence"
    );
    let mcp_insufficient_gate = mcp_text_value(&responses[8]);
    assert_eval_envelope(&mcp_insufficient_gate, "eval.gate", false);
    assert_eq!(
        mcp_insufficient_gate["object"]["verdict"],
        "insufficient_evidence"
    );
    assert_eval_envelope(&mcp_text_value(&responses[9]), "eval.evidence.ref", true);
    assert_eval_envelope(&mcp_text_value(&responses[10]), "eval.invalidate", true);
    assert_eval_envelope(&mcp_text_value(&responses[11]), "eval.rescore", true);
    assert_eq!(responses[12]["result"]["isError"], true);
    let mcp_invalid = mcp_text_value(&responses[12]);
    assert_eval_envelope(&mcp_invalid, "eval.show", false);
    assert_eq!(mcp_invalid["error"]["code"], "invalid_input");
    assert_eq!(mcp_invalid["error"]["reasons"], json!(["invalid_value"]));
    assert_eq!(responses[13]["result"]["isError"], true);
    let mcp_missing = mcp_text_value(&responses[13]);
    assert_eval_envelope(&mcp_missing, "eval.compare", false);
    assert_eq!(mcp_missing["error"]["code"], "infrastructure_error");
    assert_eq!(responses[14]["result"]["isError"], true);
    let mcp_altered_runner = mcp_text_value(&responses[14]);
    assert_eval_envelope(&mcp_altered_runner, "eval.run", false);
    assert_eq!(mcp_altered_runner["error"]["code"], "invalid_input");
    assert!(
        mcp_altered_runner["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not match frozen suite")
    );
    assert_eq!(responses[15]["result"]["isError"], true);
    let mcp_escaped_runner = mcp_text_value(&responses[15]);
    assert_eval_envelope(&mcp_escaped_runner, "eval.run", false);
    assert_eq!(mcp_escaped_runner["error"]["code"], "invalid_input");
    assert!(
        mcp_escaped_runner["error"]["message"]
            .as_str()
            .unwrap()
            .contains("repo_root must be relative")
    );
    assert_eq!(responses[16]["result"]["isError"], true);
    let mcp_mutated_fixture = mcp_text_value(&responses[16]);
    assert_eval_envelope(&mcp_mutated_fixture, "eval.run", false);
    assert_eq!(mcp_mutated_fixture["error"]["code"], "invalid_input");
    assert!(
        mcp_mutated_fixture["error"]["message"]
            .as_str()
            .unwrap()
            .contains("digest mismatch")
    );
    assert_eq!(responses[17]["result"]["isError"], true);
    let mcp_invalid_supplied = mcp_text_value(&responses[17]);
    assert_eval_envelope(&mcp_invalid_supplied, "eval.run", false);
    assert_eq!(mcp_invalid_supplied["error"]["code"], "invalid_input");
    assert!(
        mcp_invalid_supplied["error"]["message"]
            .as_str()
            .unwrap()
            .contains("is not in frozen suite")
    );
    let no_mcp_invalid_rows: i64 = Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM eval_runs WHERE id IN ('mcp-mutated-fixture-runner', 'mcp-supplied-extra-case-run')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(no_mcp_invalid_rows, 0);
}

#[test]
fn pick_returns_ranked_privacy_safe_recall_context() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Recall"])
        .assert()
        .success();

    let item_id = create_test_item(
        dir.path(),
        &db,
        "Implement login session cache",
        "Build token refresh using the session cache",
    );
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "context",
            "add",
            "Use the session cache for login token refresh decisions.",
            "--tag",
            "decision",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "context",
            "add",
            "The marketing palette uses magenta accents.",
            "--tag",
            "decision",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "context",
            "add",
            "Login token secret sk-test-should-not-appear.",
            "--tag",
            "decision",
        ])
        .assert()
        .success();

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let picked: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(picked["item"]["id"], item_id);
    let relevant = picked["relevant_contexts"].as_array().unwrap();
    assert_eq!(relevant.len(), 1);
    assert!(
        relevant[0]["content"]
            .as_str()
            .unwrap()
            .contains("session cache")
    );
    assert_eq!(picked["privacy"]["source_file_content_included"], false);
    assert!(
        !serde_json::to_string(&picked)
            .unwrap()
            .contains("sk-test-should-not-appear")
    );
}

#[test]
fn agent_registry_routes_picks_and_degrades_without_blocking() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Routing"])
        .assert()
        .success();

    // No registry: agents list reports missing, exit zero.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "agents", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let listed: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(listed["reason"], "missing");
    assert!(listed["registry"].is_null());

    let registry_path = dir.path().join(".planr/agents.toml");
    fs::write(
        &registry_path,
        r#"
[profiles.implementer]
client = "cursor"
model = "model-standard"
effort = "medium"
cost_tier = "standard"

[profiles.driver]
client = "cursor"
model = "model-premium"
effort = "high"
cost_tier = "premium"

[[routes]]
match = { work_type = "code" }
profile = "implementer"
fallbacks = ["driver"]
"#,
    )
    .unwrap();

    // Valid registry: check passes with no warnings.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "agents", "check"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let checked: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(checked["ok"], true);
    assert_eq!(checked["warnings"].as_array().unwrap().len(), 0);

    // agents list shows resolved profiles and routes.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "agents", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let listed: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        listed["registry"]["profiles"]["implementer"]["model"],
        "model-standard"
    );
    assert_eq!(listed["registry"]["routes"][0]["profile"], "implementer");

    // A code item picks up the routing recommendation in its pick packet.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Implement routing feature",
            "--description",
            "Wire routing into pick",
            "--work-type",
            "code",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&output).unwrap();
    let item_id = created["item"]["id"].as_str().unwrap().to_string();

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let picked: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(picked["item"]["id"], item_id);
    assert_eq!(picked["routing"]["profile"], "implementer");
    assert_eq!(picked["routing"]["client"], "cursor");
    assert_eq!(picked["routing"]["model"], "model-standard");
    assert_eq!(picked["routing"]["effort"], "medium");
    assert_eq!(picked["routing"]["cost_tier"], "standard");
    assert_eq!(picked["routing"]["fallbacks"], json!(["driver"]));
    assert_eq!(picked["routing"]["matched_selector"], "work_type=code");

    // Malformed registry: check exits non-zero, but picking degrades to
    // no routing block instead of failing.
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "pick", "release", &item_id])
        .assert()
        .success();
    fs::write(&registry_path, "profiles = [broken").unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "agents", "check"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("parse failed"));
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let picked: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(picked["item"]["id"], item_id);
    assert!(picked.get("routing").is_none());

    // Deleting the registry restores pre-feature behavior entirely.
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "pick", "release", &item_id])
        .assert()
        .success();
    fs::remove_file(&registry_path).unwrap();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let picked: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(picked["item"]["id"], item_id);
    assert!(picked.get("routing").is_none());
}

#[test]
fn route_overrides_pin_items_and_survive_registry_drift() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Overrides"])
        .assert()
        .success();
    let registry_path = dir.path().join(".planr/agents.toml");
    fs::write(
        &registry_path,
        r#"
[profiles.implementer]
client = "cursor"
model = "model-standard"

[profiles.driver]
client = "cursor"
model = "model-premium"
effort = "high"

[[routes]]
match = { work_type = "code" }
profile = "implementer"
"#,
    )
    .unwrap();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Gnarly refactor",
            "--description",
            "Needs the premium tier",
            "--work-type",
            "code",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&output).unwrap();
    let item_id = created["item"]["id"].as_str().unwrap().to_string();

    // Without an override the policy route wins.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "route",
            &item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let shown: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(shown["source"], "policy");
    assert_eq!(shown["routing"]["profile"], "implementer");
    assert!(shown["override"].is_null());

    // Pinning an unknown profile fails and names the known ones.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "route",
            &item_id,
            "--set",
            "ghost",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "known profiles: driver, implementer",
        ));

    // A valid pin beats the policy route, in item route and in the pick packet.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "route",
            &item_id,
            "--set",
            "driver",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pinned: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(pinned["override"], "driver");
    assert!(pinned["warning"].is_null());
    assert_eq!(pinned["routing"]["matched_selector"], "override");
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let picked: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(picked["item"]["id"], item_id);
    assert_eq!(picked["routing"]["profile"], "driver");
    assert_eq!(picked["routing"]["matched_selector"], "override");

    // A pin whose profile later leaves the registry is never an error:
    // policy routing takes over and the show output carries a repair hint.
    fs::write(
        &registry_path,
        r#"
[profiles.implementer]
client = "cursor"
model = "model-standard"

[[routes]]
match = { work_type = "code" }
profile = "implementer"
"#,
    )
    .unwrap();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "route",
            &item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let dangling: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(dangling["override"], "driver");
    assert_eq!(dangling["source"], "policy");
    assert_eq!(dangling["routing"]["profile"], "implementer");
    assert!(
        dangling["hint"]
            .as_str()
            .unwrap()
            .contains("not in .planr/agents.toml")
    );

    // Clearing restores policy routing; both mutations left graph events.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "route",
            &item_id,
            "--clear",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let cleared: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(cleared["cleared"], true);
    assert_eq!(cleared["previous"], "driver");
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "event",
            "list",
            "--item",
            &item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let events = serde_json::from_slice::<Value>(&output).unwrap();
    let event_types = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["event_type"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"route_overridden".to_string()));
    assert!(event_types.contains(&"route_override_cleared".to_string()));

    // Missing registry: --set warns but stores, so offline edits work.
    fs::remove_file(&registry_path).unwrap();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "route",
            &item_id,
            "--set",
            "driver",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let offline: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(offline["override"], "driver");
    assert!(offline["warning"].as_str().unwrap().contains("no registry"));
}

#[test]
fn run_profile_recording_emits_advisory_mismatch_events_only() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Audit"])
        .assert()
        .success();
    fs::write(
        dir.path().join(".planr/agents.toml"),
        r#"
[profiles.coder]
client = "cursor"
model = "model-standard"

[profiles.driver]
client = "cursor"
model = "model-premium"

[[routes]]
match = { work_type = "code" }
profile = "coder"
"#,
    )
    .unwrap();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Audited work",
            "--description",
            "Runs record their profile",
            "--work-type",
            "code",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let item_id = serde_json::from_slice::<Value>(&output).unwrap()["item"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mismatch_events = |item: &str| -> Vec<Value> {
        let output = planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "event",
                "list",
                "--item",
                item,
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice::<Value>(&output).unwrap()["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["event_type"] == "route_mismatch_observed")
            .cloned()
            .collect()
    };

    // Matching profile: run recorded, no event.
    planr()
        .current_dir(dir.path())
        .env_remove("PLANR_PROFILE")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "on the declared profile",
            "--cmd",
            "cargo test",
            "--profile",
            "coder",
        ])
        .assert()
        .success();
    assert!(mismatch_events(&item_id).is_empty());

    // No profile: no comparison, no event.
    planr()
        .current_dir(dir.path())
        .env_remove("PLANR_PROFILE")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "anonymous run",
            "--cmd",
            "cargo test",
        ])
        .assert()
        .success();
    assert!(mismatch_events(&item_id).is_empty());

    // Profile without commands/tests records no run: nothing to compare.
    planr()
        .current_dir(dir.path())
        .env_remove("PLANR_PROFILE")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "notes only",
            "--profile",
            "driver",
        ])
        .assert()
        .success();
    assert!(mismatch_events(&item_id).is_empty());

    // Wrong profile: exactly one advisory event naming both sides + run.
    planr()
        .current_dir(dir.path())
        .env_remove("PLANR_PROFILE")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "ran on the wrong tier",
            "--cmd",
            "cargo test",
            "--profile",
            "driver",
        ])
        .assert()
        .success();
    let events = mismatch_events(&item_id);
    assert_eq!(events.len(), 1);
    let payload: Value = serde_json::from_str(events[0]["payload"].as_str().unwrap_or_default())
        .unwrap_or_else(|_| events[0]["payload"].clone());
    assert_eq!(payload["declared_profile"], "coder");
    assert_eq!(payload["actual_profile"], "driver");
    assert!(payload["run_id"].as_str().unwrap().starts_with("run-"));

    // PLANR_PROFILE env is the fallback when no flag is passed.
    planr()
        .current_dir(dir.path())
        .env("PLANR_PROFILE", "driver")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "env-attributed run",
            "--cmd",
            "cargo test",
        ])
        .assert()
        .success();
    assert_eq!(mismatch_events(&item_id).len(), 2);

    // Mismatches are advisory: the item still closes normally.
    planr()
        .current_dir(dir.path())
        .env_remove("PLANR_PROFILE")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &item_id,
            "--summary",
            "closing despite mismatches",
            "--profile",
            "driver",
            "--cmd",
            "cargo test",
        ])
        .assert()
        .success();
    assert_eq!(mismatch_events(&item_id).len(), 3);
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "show",
            &item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<Value>(&output).unwrap()["item"]["status"],
        "closed"
    );

    // No registry: a recorded profile has nothing to compare against.
    let fresh = tempdir().unwrap();
    let fresh_db = fresh.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(fresh.path())
        .args([
            "--db",
            fresh_db.to_str().unwrap(),
            "project",
            "init",
            "Bare",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(fresh.path())
        .args([
            "--db",
            fresh_db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Unrouted",
            "--description",
            "No registry here",
            "--work-type",
            "code",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let bare_item = serde_json::from_slice::<Value>(&output).unwrap()["item"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    planr()
        .current_dir(fresh.path())
        .env_remove("PLANR_PROFILE")
        .args([
            "--db",
            fresh_db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &bare_item,
            "--summary",
            "profiled run without registry",
            "--cmd",
            "cargo test",
            "--profile",
            "coder",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(fresh.path())
        .args([
            "--db",
            fresh_db.to_str().unwrap(),
            "--json",
            "event",
            "list",
            "--item",
            &bare_item,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(
        serde_json::from_slice::<Value>(&output).unwrap()["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["event_type"] != "route_mismatch_observed")
    );
}

#[test]
fn route_audit_survives_logs_events_and_cli_mcp_trace_without_inference() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Route audit",
        ])
        .assert()
        .success();
    fs::write(
        dir.path().join(".planr/agents.toml"),
        r#"
[profiles.coder]
client = "cursor"
model = "model-standard"
effort = "high"

[[routes]]
match = { work_type = "code" }
profile = "coder"
"#,
    )
    .unwrap();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Observed route",
            "--description",
            "three stage route evidence",
            "--work-type",
            "code",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let item_id = serde_json::from_slice::<Value>(&output).unwrap()["item"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let observation = json!({
        "requested": {
            "role": "worker",
            "profile": "coder",
            "client": "codex",
            "agent_type": {"value": "planr-terra-high", "enforcement": "requested_only", "evidence": "binding"},
            "model": {"value": "model-primary", "enforcement": "requested_only", "evidence": "policy"},
            "effort": {"value": "high", "enforcement": "requested_only", "evidence": "policy"},
            "context_fork": {"value": {"mode": "none"}, "enforcement": "requested_only", "evidence": "policy"}
        },
        "resolved": {
            "role": "worker",
            "profile": "coder",
            "client": "codex",
            "agent_type": {"value": "planr-terra-high", "enforcement": "verified", "evidence": "binding"},
            "model": {"value": "model-primary", "enforcement": "verified", "evidence": "binding"},
            "effort": {"value": "high", "enforcement": "verified", "evidence": "binding"},
            "context_fork": {"value": {"mode": "none"}, "enforcement": "verified", "evidence": "binding"}
        },
        "effective": {
            "role": "planr-terra-high",
            "profile": "coder",
            "client": "codex",
            "agent_type": {"value": "planr-terra-high", "enforcement": "verified", "evidence": "host_report"},
            "model": {"value": null, "enforcement": "unavailable"},
            "effort": {"value": null, "enforcement": "unavailable"},
            "context_fork": {"value": {"mode": "none"}, "enforcement": "verified", "evidence": "host_report"},
            "thread_id": "thread-terra-high",
            "status": "completed"
        },
        "transition": {
            "kind": "availability_fallback",
            "reason": "primary profile unavailable; same-class fallback selected",
            "evidence": ["host_report"]
        },
        "policy": {"id": "policy-a", "version": "1.0.0"},
        "binding": {"id": "binding-a", "version": "2.0.0"},
        "metering": {
            "wall_time_seconds": {"value": 12, "confidence": "trusted"},
            "tool_calls": {"value": 4, "confidence": "trusted"},
            "tokens": {"value": null, "confidence": "unavailable"},
            "credits_micros": {"value": null, "confidence": "unavailable"}
        }
    });
    fs::write(
        dir.path().join("route-audit.json"),
        serde_json::to_vec_pretty(&observation).unwrap(),
    )
    .unwrap();

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "native host omitted effective model evidence",
            "--cmd",
            "cargo test",
            "--profile",
            "coder",
            "--route-audit",
            "route-audit.json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let logged: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(logged["log"]["route_observation"], observation);

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "trace",
            "item",
            &item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let cli_trace: Value = serde_json::from_slice(&output).unwrap();
    let route = &cli_trace["routing"]["runs"][0]["route_observation"];
    assert_eq!(
        route["requested"]["agent_type"]["value"],
        "planr-terra-high"
    );
    assert_eq!(route["requested"]["model"]["value"], "model-primary");
    assert_eq!(route["resolved"]["effort"]["value"], "high");
    assert!(route["effective"]["model"]["value"].is_null());
    assert_eq!(route["effective"]["model"]["enforcement"], "unavailable");
    assert_eq!(route["effective"]["context_fork"]["value"]["mode"], "none");
    assert_eq!(route["effective"]["role"], "planr-terra-high");
    assert_eq!(route["effective"]["thread_id"], "thread-terra-high");
    assert_eq!(route["effective"]["status"], "completed");
    assert_eq!(route["policy"]["id"], "policy-a");
    assert_eq!(route["binding"]["version"], "2.0.0");
    assert_eq!(route["metering"]["tool_calls"]["value"], 4);
    assert_eq!(route["metering"]["tokens"]["confidence"], "unavailable");

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "trace", "item", &item_id])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "effective: model unknown (unavailable)",
        ))
        .stdout(predicate::str::contains("transition availability_fallback"));

    let mcp_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_trace_item","arguments":{"item_id":&item_id}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_output: Value = serde_json::from_slice(&mcp_output).unwrap();
    let mcp_trace: Value =
        serde_json::from_str(mcp_output["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(mcp_trace, cli_trace);

    let mcp_log = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({
                "jsonrpc":"2.0",
                "id":1,
                "method":"tools/call",
                "params":{
                    "name":"planr_log_add",
                    "arguments":{
                        "item":&item_id,
                        "summary":"MCP preserves the same route observation",
                        "commands":["cargo test"],
                        "profile":"coder",
                        "route_observation":&observation
                    }
                }
            })
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_log: Value = serde_json::from_slice(&mcp_log).unwrap();
    let mcp_log: Value =
        serde_json::from_str(mcp_log["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(mcp_log["log"]["route_observation"], observation);

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "event",
            "list",
            "--item",
            &item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let events: Value = serde_json::from_slice(&output).unwrap();
    for event_type in [
        "route_requested",
        "route_resolved",
        "route_effective_observed",
        "route_availability_fallback",
    ] {
        assert!(
            events["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["event_type"] == event_type),
            "missing {event_type}: {events}"
        );
    }

    let mut invalid = observation.clone();
    invalid["effective"]["model"] = json!({
        "value": "model-observed",
        "enforcement": "requested_only",
        "evidence": "user_reported"
    });
    fs::write(
        dir.path().join("invalid-route-audit.json"),
        serde_json::to_vec_pretty(&invalid).unwrap(),
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "must fail before persistence",
            "--cmd",
            "cargo test",
            "--route-audit",
            "invalid-route-audit.json",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot use requested_only as effective execution proof",
        ));

    let mut disguised_verified = observation.clone();
    disguised_verified["effective"]["model"] = json!({
        "value": "model-observed",
        "enforcement": "verified",
        "evidence": "policy"
    });
    fs::write(
        dir.path().join("disguised-verified-route-audit.json"),
        serde_json::to_vec_pretty(&disguised_verified).unwrap(),
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "configuration evidence is not execution proof",
            "--cmd",
            "cargo test",
            "--route-audit",
            "disguised-verified-route-audit.json",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "lacks host_report, telemetry_receipt, process_exit, or local_observation evidence",
        ));

    let mut unproven_estimate = observation.clone();
    unproven_estimate["effective"]["model"] = json!({
        "value": "model-observed",
        "enforcement": "estimated"
    });
    let mcp_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({
                "jsonrpc":"2.0",
                "id":1,
                "method":"tools/call",
                "params":{
                    "name":"planr_log_add",
                    "arguments":{
                        "item":&item_id,
                        "summary":"unproven effective estimate must not persist",
                        "commands":["cargo test"],
                        "route_observation":unproven_estimate
                    }
                }
            })
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_output: Value = serde_json::from_slice(&mcp_output).unwrap();
    assert_eq!(mcp_output["result"]["isError"], true);
    assert!(
        mcp_output["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("is estimated but has no evidence source")
    );

    let conn = Connection::open(&db).unwrap();
    let runs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE item_id = ?1",
            [&item_id],
            |row| row.get(0),
        )
        .unwrap();
    let logs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM logs WHERE item_id = ?1",
            [&item_id],
            |row| row.get(0),
        )
        .unwrap();
    let effective_events: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE item_id = ?1 AND event_type = 'route_effective_observed'",
            [&item_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(runs, 2, "invalid route audits must not persist runs");
    assert_eq!(logs, 2, "invalid route audits must not persist logs");
    assert_eq!(
        effective_events, 2,
        "invalid route audits must not emit effective-route events"
    );
}

#[test]
fn trace_routing_section_and_doctor_registry_diagnostics() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Doctor"])
        .assert()
        .success();

    // Absent registry: informational, never a failure.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "doctor"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doctor: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(doctor["registry"]["status"], "absent");

    // A trace without declared routes or profiled runs has no routing key.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Traced work",
            "--description",
            "Declared vs actual",
            "--work-type",
            "code",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let item_id = serde_json::from_slice::<Value>(&output).unwrap()["item"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    planr()
        .current_dir(dir.path())
        .env_remove("PLANR_PROFILE")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "anonymous run",
            "--cmd",
            "cargo test",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "trace",
            "item",
            &item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(
        serde_json::from_slice::<Value>(&output)
            .unwrap()
            .get("routing")
            .is_none(),
        "no registry and no profiled runs must keep the trace shape unchanged"
    );

    // Degraded registry: doctor warns with the parser's context.
    let registry_path = dir.path().join(".planr/agents.toml");
    fs::write(&registry_path, "[profiles.broken\nclient=").unwrap();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "doctor"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doctor: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(doctor["registry"]["status"], "degraded");
    assert!(
        doctor["registry"]["error"]
            .as_str()
            .unwrap()
            .contains("line")
    );

    fs::write(
        &registry_path,
        r#"
[profiles.coder]
client = "cursor"
model = "model-standard"
effort = "xhigh"

[profiles.ghost-ref]
client = "cursor"
model = "model-standard"

[[routes]]
match = { work_type = "code" }
profile = "coder"

[[routes]]
match = { work_type = "docs" }
profile = "nonexistent"
"#,
    )
    .unwrap();

    // Loaded registry: counts and validation warnings.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "doctor"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doctor: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(doctor["registry"]["status"], "ok");
    assert_eq!(doctor["registry"]["profiles"], 2);
    assert_eq!(doctor["registry"]["routes"], 2);
    assert!(
        doctor["registry"]["warnings"][0]
            .as_str()
            .unwrap()
            .contains("nonexistent")
    );

    // Core does not render or inspect host-specific model artifacts.
    fs::write(
        &registry_path,
        r#"
[profiles.coder]
client = "cursor"
model = "model-next"
effort = "high"

[[routes]]
match = { work_type = "code" }
profile = "coder"
"#,
    )
    .unwrap();
    // Trace now shows declared vs actual with an advisory marker.
    planr()
        .current_dir(dir.path())
        .env_remove("PLANR_PROFILE")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "wrong tier",
            "--cmd",
            "cargo test",
            "--profile",
            "budget-helper",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "trace",
            "item",
            &item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let trace: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(trace["routing"]["declared"]["profile"], "coder");
    assert_eq!(trace["routing"]["mismatches"], 1);
    let runs = trace["routing"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert!(runs[0]["profile"].is_null());
    assert!(runs[0]["mismatch"].is_null());
    assert_eq!(runs[1]["profile"], "budget-helper");
    assert_eq!(runs[1]["mismatch"], true);
    let human = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "trace", "item", &item_id])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let human = String::from_utf8(human).unwrap();
    assert!(human.contains("routing declared: coder (work_type=code)"));
    assert!(human.contains("advisory"));
    assert!(!human.to_lowercase().contains("error"));
}

#[test]
fn package_round_trips_agent_registry_without_silent_overwrite() {
    let source = tempdir().unwrap();
    let source_db = source.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(source.path())
        .args([
            "--db",
            source_db.to_str().unwrap(),
            "project",
            "init",
            "Source",
        ])
        .assert()
        .success();
    let registry_toml = r#"
[profiles.coder]
client = "cursor"
model = "model-standard"

[[routes]]
match = { work_type = "code" }
profile = "coder"
"#;
    fs::write(source.path().join(".planr/agents.toml"), registry_toml).unwrap();
    let package_path = source.path().join("package.json");
    planr()
        .current_dir(source.path())
        .args([
            "--db",
            source_db.to_str().unwrap(),
            "export",
            "--out",
            package_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    let package: Value = serde_json::from_slice(&fs::read(&package_path).unwrap()).unwrap();
    assert_eq!(package["agent_registry"]["path"], ".planr/agents.toml");
    assert_eq!(package["agent_registry"]["content"], registry_toml);

    // Fresh destination: preview names the registry, confirm writes it,
    // and `agents check` passes there.
    let dest = tempdir().unwrap();
    let dest_db = dest.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dest.path())
        .args(["--db", dest_db.to_str().unwrap(), "project", "init", "Dest"])
        .assert()
        .success();
    let output = planr()
        .current_dir(dest.path())
        .args([
            "--db",
            dest_db.to_str().unwrap(),
            "--json",
            "import",
            package_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(preview["mode"], "preview");
    assert_eq!(preview["report"]["agent_registry"]["action"], "create");
    assert!(
        !dest.path().join(".planr/agents.toml").exists(),
        "preview must not write anything"
    );
    let output = planr()
        .current_dir(dest.path())
        .args([
            "--db",
            dest_db.to_str().unwrap(),
            "--json",
            "import",
            package_path.to_str().unwrap(),
            "--confirm",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let applied: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(applied["imported"]["agent_registry"]["action"], "create");
    assert_eq!(
        fs::read_to_string(dest.path().join(".planr/agents.toml")).unwrap(),
        registry_toml
    );
    planr()
        .current_dir(dest.path())
        .args(["--db", dest_db.to_str().unwrap(), "agents", "check"])
        .assert()
        .success();

    // Re-import over the identical registry: reported, nothing to do.
    let output = planr()
        .current_dir(dest.path())
        .args([
            "--db",
            dest_db.to_str().unwrap(),
            "--json",
            "import",
            package_path.to_str().unwrap(),
            "--confirm",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let applied: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(applied["imported"]["agent_registry"]["action"], "identical");

    // A differing local registry is never silently overwritten.
    let local_registry = "[profiles.local]\nclient = \"cursor\"\nmodel = \"model-premium\"\n";
    fs::write(dest.path().join(".planr/agents.toml"), local_registry).unwrap();
    let output = planr()
        .current_dir(dest.path())
        .args([
            "--db",
            dest_db.to_str().unwrap(),
            "--json",
            "import",
            package_path.to_str().unwrap(),
            "--confirm",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let applied: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(applied["imported"]["agent_registry"]["action"], "conflict");
    assert!(
        applied["imported"]["agent_registry"]["hint"]
            .as_str()
            .unwrap()
            .contains("never overwritten")
    );
    assert_eq!(
        fs::read_to_string(dest.path().join(".planr/agents.toml")).unwrap(),
        local_registry
    );

    // Packages without a registry (pre-registry exports) import unchanged.
    let mut stripped: Value = serde_json::from_slice(&fs::read(&package_path).unwrap()).unwrap();
    stripped.as_object_mut().unwrap().remove("agent_registry");
    let stripped_path = source.path().join("stripped.json");
    fs::write(
        &stripped_path,
        serde_json::to_vec_pretty(&stripped).unwrap(),
    )
    .unwrap();
    let output = planr()
        .current_dir(dest.path())
        .args([
            "--db",
            dest_db.to_str().unwrap(),
            "--json",
            "import",
            stripped_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&output).unwrap();
    assert!(preview["report"]["agent_registry"].is_null());
}

#[test]
fn skill_pairing_travels_through_pick_route_list_and_prompt() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Pool"])
        .assert()
        .success();
    fs::create_dir_all(dir.path().join(".planr")).unwrap();
    fs::write(
        dir.path().join(".planr/agents.toml"),
        r#"[profiles.designer]
client = "claude-code"
model = "model-review"
effort = "high"
cost_tier = "premium"
skill = "frontend-design"

[profiles.backender]
client = "cursor"
model = "model-standard"

[[routes]]
match = { work_type = "frontend" }
profile = "designer"
fallbacks = ["backender"]

[[routes]]
match = { work_type = "backend" }
profile = "backender"
"#,
    )
    .unwrap();

    // Use-case work types are free-form: the frontend item routes to the
    // designer profile and the pick packet carries the paired skill.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "create",
            "Polish hero section",
            "--description",
            "design pass",
            "--work-type",
            "frontend",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pick: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(pick["routing"]["profile"], "designer");
    assert_eq!(pick["routing"]["skill"], "frontend-design");
    assert_eq!(pick["routing"]["matched_selector"], "work_type=frontend");
    let item_id = pick["item"]["id"].as_str().unwrap().to_string();

    // item route and agents list surface the pairing too.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "route",
            &item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let route: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(route["routing"]["skill"], "frontend-design");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "agents", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("skill=frontend-design"));

    // prompt routing names the paired skill and the dispatch rule.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "prompt", "routing"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let prompt = String::from_utf8(output).unwrap();
    assert!(prompt.contains("| work_type=frontend | designer | claude-code | - | model-review | high | premium | frontend-design | backender |"));
    assert!(prompt.contains("dispatch the worker with that skill"));

    // Retagging via item update re-resolves routing on the next pick:
    // planning agents tag map-build output against the declared routes
    // without user involvement.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "update",
            &item_id,
            "--work-type",
            "backend",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "route",
            &item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let route: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(route["routing"]["profile"], "backender");
    assert_eq!(route["routing"]["matched_selector"], "work_type=backend");

    // The retag is auditable: item update records an item_updated event
    // (a routing-relevant mutation was previously invisible in the log).
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "event", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let events: Value = serde_json::from_slice(&output).unwrap();
    let update_event = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["event_type"] == "item_updated")
        .expect("item update must record an event");
    assert_eq!(update_event["payload"]["changed"]["work_type"], "backend");

    // Profiles without a skill omit the key entirely: no-skill registries
    // keep byte-identical routing blocks.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "API endpoint",
            "--description",
            "backend pass",
            "--work-type",
            "backend",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&output).unwrap();
    let backend_id = created["item"]["id"].as_str().unwrap().to_string();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "route",
            &backend_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let route: Value = serde_json::from_slice(&output).unwrap();
    assert!(
        !route["routing"].as_object().unwrap().contains_key("skill"),
        "no-skill profile must omit the key, got {route}"
    );
}

#[test]
fn item_create_with_bad_after_is_atomic_and_link_writes_fail_loudly() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Atomic"])
        .assert()
        .success();
    // A bad --after fails before the item persists: no half-applied
    // create, so a retry cannot duplicate.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "item",
            "create",
            "Throwaway",
            "--description",
            "d",
            "--after",
            "i-truncated",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("item not found: i-truncated"));
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let map: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        map["items"].as_array().unwrap().len(),
        0,
        "no item may persist"
    );

    // link add with an unknown endpoint errors instead of writing nothing.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "item",
            "create",
            "Real",
            "--description",
            "d",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "link",
            "add",
            "i-nope",
            "i-also-nope",
            "--type",
            "blocks",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown item `i-nope`"));

    // cancel without a flag names the repair path.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let map: Value = serde_json::from_slice(&output).unwrap();
    let id = map["items"][0]["id"].as_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "item", "cancel", &id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--preview"));
}

#[test]
fn agents_init_flag_specs_generate_a_pool_and_fail_closed() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Pool"])
        .assert()
        .success();

    // QA-2: validation is fail-closed — nothing is written on any error.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "agents",
            "init",
            "--route",
            "frontend=nosuch",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--profile"));
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "agents",
            "init",
            "--profile",
            "broken-no-slash",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--profile <id>=<client>/<model>[@<effort>][#<tier>]",
        ));
    assert!(!dir.path().join(".planr/agents.toml").exists());

    // QA-1: a full pool spec generates a zero-warning registry that routes.
    // The pinned skill exists in the project so the (separate) skill-
    // existence advisory stays out of this structural zero-warning check.
    fs::create_dir_all(dir.path().join(".cursor/skills/frontend-design")).unwrap();
    fs::write(
        dir.path().join(".cursor/skills/frontend-design/SKILL.md"),
        "---\nname: frontend-design\n---\n",
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "agents",
            "init",
            "--profile",
            "driver=cursor/model-premium@high#premium",
            "--profile",
            "designer=claude-code/model-review@high#premium",
            "--skill",
            "designer=frontend-design",
            "--route",
            "frontend=designer,driver",
            "--route",
            "review=driver",
            "--default-route",
            "designer,driver",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .args(["--db", &db_arg, "--json", "agents", "check"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let check: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(check["warnings"].as_array().unwrap().len(), 0);
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "item",
            "create",
            "Hero polish",
            "--description",
            "design pass",
            "--work-type",
            "frontend",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pick: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(pick["routing"]["profile"], "designer");
    assert_eq!(pick["routing"]["skill"], "frontend-design");
    assert_eq!(pick["routing"]["fallbacks"][0], "driver");

    // A hand-written Cursor registry with render-unsafe values keeps its
    // independent renderer on the static role instead of writing broken artifacts.
    fs::write(
        dir.path().join(".planr/agents.toml"),
        "[profiles.\"evil\nid\"]\nclient = \"cursor\"\nmodel = \"model-standard\"\n\n[[routes]]\nmatch = { work_type = \"code\" }\nprofile = \"evil\nid\"\n",
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "install", "cursor", "--no-mcp", "--force"])
        .assert()
        .success();
    let worker = fs::read_to_string(dir.path().join(".cursor/agents/planr-worker.md")).unwrap();
    assert!(
        !worker.contains("generated from"),
        "render-unsafe profile must fall back to the static role: {worker}"
    );
    assert!(
        worker.starts_with("---\n"),
        "role file must stay parseable markdown"
    );

    // QA-3: spec flags never overwrite without --force either.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "agents",
            "init",
            "--profile",
            "solo=cursor/model-standard",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));

    // QA-5: --interactive without a TTY errors cleanly, naming the grammar.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "agents",
            "init",
            "--interactive",
            "--force",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("needs a terminal"));

    // QA-6: --interactive conflicts with spec flags at parse time.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "agents",
            "init",
            "--interactive",
            "--profile",
            "a=cursor/m",
            "--force",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn observed_client_lands_on_runs_and_flags_declared_client_deviation() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Hosts"])
        .assert()
        .success();
    fs::create_dir_all(dir.path().join(".planr")).unwrap();
    fs::write(
        dir.path().join(".planr/agents.toml"),
        "[profiles.coder]\nclient = \"cursor\"\nmodel = \"model-standard\"\n\n[route_default]\nprofile = \"coder\"\n",
    )
    .unwrap();
    let item = create_test_item(dir.path(), &db, "Routed work", "host audit");

    // A run logged from a Claude session against a cursor-declared route:
    // observed_client is recorded and one advisory event flags the
    // deviation — without any profile being reported.
    planr()
        .current_dir(dir.path())
        .env("CLAUDECODE", "1")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item,
            "--summary",
            "built elsewhere",
            "--cmd",
            "cargo test",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "trace",
            "item",
            &item,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let trace: Value = serde_json::from_slice(&output).unwrap();
    let run = &trace["routing"]["runs"][0];
    assert_eq!(run["observed_client"], "claude-code");
    assert_eq!(run["client_mismatch"], true, "trace must flag it: {trace}");
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "event", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let events: Value = serde_json::from_slice(&output).unwrap();
    let mismatches: Vec<&Value> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["event_type"] == "client_mismatch_observed")
        .collect();
    assert_eq!(mismatches.len(), 1, "exactly one advisory event: {events}");
    assert_eq!(mismatches[0]["payload"]["declared_client"], "cursor");
    assert_eq!(mismatches[0]["payload"]["observed_client"], "claude-code");
    // The payload names the log kind so audit consumers can discount
    // legitimate cases (driver verification logs on routed items).
    assert_eq!(mismatches[0]["payload"]["log_kind"], "completion");

    // Human event output is greppable: one event per line with type and
    // compact payload, not a bare count.
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "event", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("client_mismatch_observed"))
        .stdout(predicate::str::contains(
            "\"observed_client\":\"claude-code\"",
        ));

    // Human trace drops the legacy identity-derived client bracket when
    // the run carries an observed host.
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "trace", "item", &item])
        .assert()
        .success()
        .stdout(predicate::str::contains("on claude-code"))
        .stdout(predicate::str::contains("[human]").not());

    // Matching host: no new event. (CURSOR_AGENT is scrubbed by the
    // helper, so set it explicitly.)
    planr()
        .current_dir(dir.path())
        .env("CURSOR_AGENT", "1")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item,
            "--summary",
            "built at home",
            "--cmd",
            "cargo test",
        ])
        .assert()
        .success();
    // No host env at all: no observed_client key, no event.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item,
            "--summary",
            "built anonymously",
            "--cmd",
            "cargo test",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "trace",
            "item",
            &item,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let trace: Value = serde_json::from_slice(&output).unwrap();
    let runs = trace["routing"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[1]["observed_client"], "cursor");
    assert_eq!(runs[1]["client_mismatch"], false);
    assert!(
        !runs[2].as_object().unwrap().contains_key("observed_client"),
        "unknown host must store nothing: {trace}"
    );
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "event", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let events: Value = serde_json::from_slice(&output).unwrap();
    let mismatch_count = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["event_type"] == "client_mismatch_observed")
        .count();
    assert_eq!(mismatch_count, 1, "matching/unknown hosts add no events");
}

/// Regression harness for the first-pick hang observed in a live loop
/// run (2 of 6 subagent sessions: the first `planr pick` hung until
/// killed, retry instant). Root-cause candidate fixed in storage:
/// busy_timeout was set after journal_mode, so concurrent first
/// connections raced the WAL conversion with a zero timeout. This test
/// storms one database with parallel first-pick processes under a hard
/// watchdog: a hang fails the suite instead of burning a worker.
#[test]
fn parallel_first_picks_finish_within_the_watchdog() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Storm"])
        .assert()
        .success();
    let bin = assert_cmd::cargo::cargo_bin("planr");

    for round in 0..4 {
        // Fresh, independent items so every concurrent pick can lease one.
        for worker in 0..8 {
            planr()
                .current_dir(dir.path())
                .args([
                    "--db",
                    &db_arg,
                    "item",
                    "create",
                    &format!("Storm r{round} w{worker}"),
                    "--description",
                    "parallel first-pick storm",
                ])
                .assert()
                .success();
        }
        let mut children = Vec::new();
        for worker in 0..8 {
            let mut command = std::process::Command::new(&bin);
            command
                .current_dir(dir.path())
                .args(["--db", &db_arg, "--json", "pick"])
                .env("PLANR_WORKER_ID", format!("storm-r{round}-w{worker}"))
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            for var in [
                "CODEX_SANDBOX",
                "CODEX_SESSION_ID",
                "CLAUDECODE",
                "CURSOR_AGENT",
                "CURSOR_INVOKED_AS",
                "PLANR_PROFILE",
            ] {
                command.env_remove(var);
            }
            children.push(command.spawn().unwrap());
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        for mut child in children {
            let status = loop {
                match child.try_wait().unwrap() {
                    Some(status) => break status,
                    None if std::time::Instant::now() > deadline => {
                        let _ = child.kill();
                        panic!("round {round}: a first pick hung past the 30s watchdog");
                    }
                    None => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            };
            assert!(
                status.success(),
                "round {round}: parallel pick failed (lock contention must wait, not error)"
            );
        }
    }
}

#[test]
fn cancel_reasons_routing_alias_and_skill_existence_warning() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Polish"])
        .assert()
        .success();

    // Cancel with --reason records an item_cancelled event carrying it.
    let item = create_test_item(dir.path(), &db, "Doomed", "will be cancelled");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "item",
            "cancel",
            &item,
            "--confirm",
            "--reason",
            "superseded by a better slice",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "event", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let events: Value = serde_json::from_slice(&output).unwrap();
    let cancelled = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["event_type"] == "item_cancelled")
        .expect("cancel must record an event");
    assert_eq!(
        cancelled["payload"]["reason"],
        "superseded by a better slice"
    );

    // A registry pinning a nonexistent skill draws exactly one advisory
    // warning; check still exits 0. HOME is pinned to the tempdir so a
    // developer's real skill collection cannot leak in.
    fs::write(
        dir.path().join(".planr/agents.toml"),
        "[profiles.designer]\nclient = \"cursor\"\nmodel = \"model-review\"\nskill = \"definitely-missing-skill\"\n\n[route_default]\nprofile = \"designer\"\n",
    )
    .unwrap();
    let output = planr()
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .args(["--db", &db_arg, "--json", "agents", "check"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let check: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(check["ok"], true);
    let warnings = check["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "one skill warning expected: {check}");
    assert!(
        warnings[0]
            .as_str()
            .unwrap()
            .contains("definitely-missing-skill")
    );

    // Creating the skill in a project skill directory clears the warning.
    fs::create_dir_all(dir.path().join(".cursor/skills/definitely-missing-skill")).unwrap();
    fs::write(
        dir.path()
            .join(".cursor/skills/definitely-missing-skill/SKILL.md"),
        "---\nname: definitely-missing-skill\n---\n",
    )
    .unwrap();
    let output = planr()
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .args(["--db", &db_arg, "--json", "agents", "check"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let check: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(check["warnings"].as_array().unwrap().len(), 0);

    // `agents routing` resolves as an alias for `prompt routing`.
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "agents", "routing"])
        .assert()
        .success()
        .stdout(predicate::str::contains("## Model routing"));
}

#[test]
fn plan_work_type_annotations_seed_routed_items_without_retags() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Annotated"])
        .assert()
        .success();
    fs::write(
        dir.path().join(".planr/agents.toml"),
        "[profiles.frontender]\nclient = \"claude-code\"\nmodel = \"model-review\"\n\n[profiles.backender]\nclient = \"cursor\"\nmodel = \"model-standard\"\n\n[[routes]]\nmatch = { work_type = \"frontend\" }\nprofile = \"frontender\"\n\n[[routes]]\nmatch = { work_type = \"backend\" }\nprofile = \"backender\"\n",
    )
    .unwrap();
    // A build plan whose task list carries annotations: heading style,
    // checklist style, prose parenthetical (must NOT annotate), and an
    // unannotated line (stays code).
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "plan", "new", "Annotated"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&output).unwrap();
    let product_id = created["plan"]["id"].as_str().unwrap().to_string();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "--json",
            "plan",
            "split",
            &product_id,
            "--slice",
            "v1",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let split: Value = serde_json::from_slice(&output).unwrap();
    let plan_id = split["plan"]["id"].as_str().unwrap().to_string();
    let plan_path = split["plan"]["path"].as_str().unwrap().to_string();
    let content = fs::read_to_string(&plan_path).unwrap();
    let tasks = "### TASK-001 (backend): REST API\n\nbuild the api\n\n### TASK-002: Wire everything\n\n- [ ] (frontend) Build the form\n- [ ] Ship docs (see appendix A) tomorrow\n\n";
    let start = content.find("## Phase 1").unwrap();
    let end = content.find("## Out Of Scope").unwrap();
    fs::write(
        &plan_path,
        format!("{}{}{}", &content[..start], tasks, &content[end..]),
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "map", "build", "--from", &plan_id])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let map: Value = serde_json::from_slice(&output).unwrap();
    let work_type = |title: &str| {
        map["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["title"] == title)
            .unwrap_or_else(|| panic!("item `{title}` missing: {map}"))["work_type"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(work_type("REST API"), "backend");
    assert_eq!(work_type("Wire everything"), "code");
    assert_eq!(work_type("Build the form"), "frontend");
    assert_eq!(
        work_type("Ship docs (see appendix A) tomorrow"),
        "code",
        "prose parentheticals must not annotate"
    );

    // Routing binds with zero retags: the annotated backend item resolves.
    let api_id = map["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "REST API")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "item", "route", &api_id])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let route: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(route["routing"]["profile"], "backender");
    assert_eq!(route["routing"]["matched_selector"], "work_type=backend");
}

#[test]
fn install_writes_host_hooks_by_default_with_additive_merge() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Hooked"])
        .assert()
        .success();

    // Pre-existing foreign hooks must survive the merge.
    fs::create_dir_all(dir.path().join(".cursor")).unwrap();
    fs::write(
        dir.path().join(".cursor/hooks.json"),
        "{\"version\":1,\"hooks\":{\"sessionStart\":[{\"command\":\"echo mine\"}]}}",
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "install", "cursor", "--no-mcp"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hooks written"));
    let cursor_hooks: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".cursor/hooks.json")).unwrap())
            .unwrap();
    let session_start = cursor_hooks["hooks"]["sessionStart"].as_array().unwrap();
    assert_eq!(
        session_start[0]["command"], "echo mine",
        "foreign hook kept"
    );
    assert!(
        session_start[1]["command"]
            .as_str()
            .unwrap()
            .contains("planr prime"),
    );
    assert!(
        session_start[1]["command"]
            .as_str()
            .unwrap()
            .ends_with("|| true"),
        "prime hooks must fail open"
    );
    assert!(
        session_start[1]["command"]
            .as_str()
            .unwrap()
            .contains("--cursor-json"),
        "cursor needs its context-injection envelope"
    );
    // preCompact cannot restore model context in Cursor: not wired.
    assert!(cursor_hooks["hooks"].get("preCompact").is_none());
    assert!(
        cursor_hooks["hooks"]["subagentStop"][0]["command"]
            .as_str()
            .unwrap()
            .contains("planr-evidence-guard")
    );
    let guard = dir.path().join(".cursor/hooks/planr-evidence-guard.sh");
    assert!(guard.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert!(
            fs::metadata(&guard).unwrap().permissions().mode() & 0o111 != 0,
            "guard must be executable"
        );
    }

    // Idempotent: a second install adds nothing.
    let before = fs::read_to_string(dir.path().join(".cursor/hooks.json")).unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "install", "cursor", "--no-mcp"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hooks written").not());
    assert_eq!(
        before,
        fs::read_to_string(dir.path().join(".cursor/hooks.json")).unwrap()
    );

    // Upgrade path: stale planr-owned entries from an older version are
    // reconciled in place — the outdated sessionStart command is
    // replaced, the retired preCompact entry removed, an old guard
    // script refreshed — while foreign entries survive untouched.
    fs::write(
        dir.path().join(".cursor/hooks.json"),
        "{\"version\":1,\"hooks\":{\"sessionStart\":[{\"command\":\"echo mine\"},{\"command\":\"planr prime 2>/dev/null || true\",\"timeout\":10}],\"preCompact\":[{\"command\":\"planr prime 2>/dev/null || true\",\"timeout\":10},{\"command\":\"echo foreign-compact\"}]}}",
    )
    .unwrap();
    fs::write(
        dir.path().join(".cursor/hooks/planr-evidence-guard.sh"),
        "#!/bin/bash\n# old unscoped guard\nexit 0\n",
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "install", "cursor", "--no-mcp"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hooks written"));
    let upgraded: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".cursor/hooks.json")).unwrap())
            .unwrap();
    let session_start = upgraded["hooks"]["sessionStart"].as_array().unwrap();
    assert_eq!(session_start[0]["command"], "echo mine");
    assert!(
        session_start[1]["command"]
            .as_str()
            .unwrap()
            .contains("--cursor-json"),
        "stale planr entry must be upgraded in place: {upgraded}"
    );
    assert_eq!(session_start.len(), 2, "no duplicate planr entries");
    let pre_compact = upgraded["hooks"]["preCompact"].as_array().unwrap();
    assert_eq!(
        pre_compact.len(),
        1,
        "retired planr entry removed, foreign kept: {upgraded}"
    );
    assert_eq!(pre_compact[0]["command"], "echo foreign-compact");
    assert!(
        fs::read_to_string(dir.path().join(".cursor/hooks/planr-evidence-guard.sh"))
            .unwrap()
            .contains("PLANR_WORKER_ID"),
        "old guard script must be refreshed"
    );

    // Claude: merge into an existing settings.json without losing keys.
    fs::create_dir_all(dir.path().join(".claude")).unwrap();
    fs::write(
        dir.path().join(".claude/settings.json"),
        "{\"permissions\":{\"allow\":[\"Bash\"]}}",
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "install", "claude", "--no-mcp"])
        .assert()
        .success();
    let claude: Value = serde_json::from_str(
        &fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        claude["permissions"]["allow"][0], "Bash",
        "foreign keys kept"
    );
    assert_eq!(
        claude["hooks"]["SessionStart"][0]["matcher"],
        "startup|resume|compact"
    );
    assert!(
        claude["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("planr prime --hook-json")
    );
    // SessionStart's `compact` source covers post-compaction refresh;
    // PreCompact injects nothing and its envelope differs: not wired.
    assert!(claude["hooks"].get("PreCompact").is_none());

    // Codex: hooks file + trust note; --no-hooks skips entirely.
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "install", "codex", "--no-mcp"])
        .assert()
        .success()
        .stdout(predicate::str::contains("run /hooks"));
    let codex: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".codex/hooks.json")).unwrap())
            .unwrap();
    assert!(
        codex["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("planr prime")
    );
    // PostCompact ignores stdout for context injection: not wired.
    assert!(codex.get("PostCompact").is_none());

    let skipped = tempdir().unwrap();
    let skipped_db = skipped.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(skipped.path())
        .args([
            "--db",
            skipped_db.to_str().unwrap(),
            "project",
            "init",
            "NoHooks",
        ])
        .assert()
        .success();
    planr()
        .current_dir(skipped.path())
        .args([
            "--db",
            skipped_db.to_str().unwrap(),
            "install",
            "cursor",
            "--no-mcp",
            "--no-hooks",
        ])
        .assert()
        .success();
    assert!(!skipped.path().join(".cursor/hooks.json").exists());

    // An unparseable existing file is never touched, only warned about.
    let broken = tempdir().unwrap();
    let broken_db = broken.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(broken.path())
        .args([
            "--db",
            broken_db.to_str().unwrap(),
            "project",
            "init",
            "Broken",
        ])
        .assert()
        .success();
    fs::create_dir_all(broken.path().join(".claude")).unwrap();
    fs::write(broken.path().join(".claude/settings.json"), "not json {").unwrap();
    planr()
        .current_dir(broken.path())
        .args([
            "--db",
            broken_db.to_str().unwrap(),
            "install",
            "claude",
            "--no-mcp",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("hooks skipped"));
    assert_eq!(
        fs::read_to_string(broken.path().join(".claude/settings.json")).unwrap(),
        "not json {"
    );
}

#[test]
fn evidence_guard_reminds_about_unlogged_picks_and_stays_silent_otherwise() {
    if std::process::Command::new("jq")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("jq not installed; skipping guard execution test");
        return;
    }
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Guarded"])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "install", "cursor", "--no-mcp"])
        .assert()
        .success();
    let guard = dir.path().join(".cursor/hooks/planr-evidence-guard.sh");
    // The guard calls bare `planr`; point PATH at the freshly built binary
    // and PLANR_DB at this project's database.
    let bin_dir = assert_cmd::cargo::cargo_bin("planr")
        .parent()
        .unwrap()
        .to_path_buf();
    let path_env = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let run_guard = |dir: &std::path::Path, worker: Option<&str>| {
        let mut command = std::process::Command::new("bash");
        command
            .arg(&guard)
            .current_dir(dir)
            .env("PATH", &path_env)
            .env("PLANR_DB", &db_arg)
            .env_remove("PLANR_WORKER_ID")
            .env_remove("PLANR_SESSION_ID");
        if let Some(worker) = worker {
            command.env("PLANR_WORKER_ID", worker);
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "guard must always exit 0");
        String::from_utf8(output.stdout).unwrap()
    };

    // No picked items: silent.
    assert!(run_guard(dir.path(), Some("guard-w1")).trim().is_empty());

    // A picked item without a completion log: one advisory follow-up —
    // but only for the worker that owns it. Anonymous shells and other
    // workers stay silent so nobody gets steered toward foreign items.
    let item = create_test_item(dir.path(), &db, "Unlogged work", "guard target");
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "guard-w1")
        .args(["--db", &db_arg, "pick"])
        .assert()
        .success();
    assert!(run_guard(dir.path(), None).trim().is_empty());
    assert!(
        run_guard(dir.path(), Some("someone-else"))
            .trim()
            .is_empty()
    );
    let reminder = run_guard(dir.path(), Some("guard-w1"));
    let message: Value = serde_json::from_str(&reminder).expect("guard emits valid JSON");
    assert!(
        message["followup_message"]
            .as_str()
            .unwrap()
            .contains(&item),
    );

    // After logging evidence, the guard goes silent again.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "log",
            "add",
            "--item",
            &item,
            "--summary",
            "done",
        ])
        .assert()
        .success();
    assert!(run_guard(dir.path(), Some("guard-w1")).trim().is_empty());
}

#[test]
fn prime_emits_compact_state_and_stays_silent_without_a_db() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();

    // No database: exit 0, no output, and crucially no db created —
    // prime runs from hooks in every repo, planr project or not.
    let output = planr()
        .current_dir(dir.path())
        .args(["prime"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(output.is_empty(), "no-db prime must stay silent");
    assert!(
        !dir.path().join(".planr/planr.sqlite").exists(),
        "prime must not create a database"
    );

    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Primed"])
        .assert()
        .success();
    let item = create_test_item(dir.path(), &db, "Held work", "prime target");
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "primer-1")
        .args(["--db", &db_arg, "pick"])
        .assert()
        .success();
    // Non-ASCII near the truncation boundary: prime must never panic
    // (chars, not bytes).
    let long_contract = format!(
        "GOAL CONTRACT pln-x: DONE when everything is closed with evidence. {}",
        "ü".repeat(400)
    );
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "context",
            "add",
            &long_contract,
            "--tag",
            "goal-contract",
        ])
        .assert()
        .success();

    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "primer-1")
        .args(["--db", &db_arg, "prime"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("project: Primed"));
    assert!(text.contains(&format!("you hold: {item}")));
    assert!(text.contains("(no completion log yet)"));
    assert!(text.contains("goal contract: GOAL CONTRACT pln-x"));
    assert!(text.contains("next: continue"));

    // Claude SessionStart envelope: valid JSON with the injected context.
    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "primer-1")
        .args(["--db", &db_arg, "prime", "--hook-json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let envelope: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        envelope["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    assert!(
        envelope["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("## planr state")
    );

    // Cursor envelope: additional_context for command-hook injection.
    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "primer-1")
        .args(["--db", &db_arg, "prime", "--cursor-json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let envelope: Value = serde_json::from_slice(&output).unwrap();
    assert!(
        envelope["additional_context"]
            .as_str()
            .unwrap()
            .contains("## planr state")
    );
}

#[test]
fn pick_peek_reads_the_packet_without_leasing() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Peek"])
        .assert()
        .success();
    fs::write(
        dir.path().join(".planr/agents.toml"),
        "[profiles.coder]\nclient = \"cursor\"\nmodel = \"model-standard\"\n\n[route_default]\nprofile = \"coder\"\n",
    )
    .unwrap();
    let item = create_test_item(dir.path(), &db, "Dispatch me", "peek target");

    // Peek returns the full packet including routing, marked as peek.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "pick", "--peek"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let peek: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(peek["peek"], true);
    assert_eq!(peek["item"]["id"], item.as_str());
    assert_eq!(peek["item"]["status"], "ready", "peek must not lease");
    assert_eq!(peek["routing"]["profile"], "coder");

    // No lease, no pick events were written.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "event", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let events: Value = serde_json::from_slice(&output).unwrap();
    assert!(
        events["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["event_type"] != "item_picked"),
        "peek must record no pick event: {events}"
    );

    // The worker's real pick leases the same item normally afterwards.
    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "worker-1")
        .args(["--db", &db_arg, "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pick: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(pick["item"]["id"], item.as_str());
    assert_eq!(pick["item"]["status"], "picked");
    assert_eq!(pick["item"]["worker_id"], "worker-1");
    assert!(pick.get("peek").is_none());
}

#[test]
fn mcp_route_tools_reuse_cli_json_shapes() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "McpRoute"])
        .assert()
        .success();
    fs::write(
        dir.path().join(".planr/agents.toml"),
        r#"
[profiles.driver]
client = "cursor"
model = "model-premium"

[[routes]]
match = { work_type = "code" }
profile = "driver"
"#,
    )
    .unwrap();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Routed work",
            "--description",
            "mcp",
            "--work-type",
            "code",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&output).unwrap();
    let item_id = created["item"]["id"].as_str().unwrap().to_string();

    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_agents_list","arguments":{}}}).to_string(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"planr_item_route","arguments":{"item_id":item_id}}}).to_string(),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"planr_item_route_set","arguments":{"item_id":item_id,"profile":"driver"}}}).to_string(),
        // Unknown profile: a tool-result error, not a dead server.
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"planr_item_route_set","arguments":{"item_id":item_id,"profile":"ghost"}}}).to_string(),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"planr_item_route_clear","arguments":{"item_id":item_id}}}).to_string(),
    ]
    .join("\n")
        + "\n";
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let responses = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 5, "unexpected responses: {responses:?}");
    let tool_payload = |index: usize| -> Value {
        serde_json::from_str(
            responses[index]["result"]["content"][0]["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap()
    };

    let listed = tool_payload(0);
    assert_eq!(
        listed["registry"]["profiles"]["driver"]["model"],
        "model-premium"
    );
    assert_eq!(listed["warnings"].as_array().unwrap().len(), 0);

    let shown = tool_payload(1);
    assert_eq!(shown["source"], "policy");
    assert_eq!(shown["routing"]["profile"], "driver");

    let pinned = tool_payload(2);
    assert_eq!(pinned["override"], "driver");
    assert_eq!(pinned["routing"]["matched_selector"], "override");

    assert_eq!(responses[3]["result"]["isError"], true);
    assert!(
        responses[3]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("ghost")
    );

    let cleared = tool_payload(4);
    assert_eq!(cleared["cleared"], true);
    assert_eq!(cleared["previous"], "driver");
}

#[test]
fn usage_policy_cli_and_mcp_reuse_shapes_and_fail_closed() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Policy"])
        .assert()
        .success();

    let missing = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "policy", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let missing: Value = serde_json::from_slice(&missing).unwrap();
    assert!(missing["policy"].is_null());
    assert_eq!(missing["reason"], "missing");
    assert_eq!(missing["enforcement"], "unavailable");

    fs::write(
        dir.path().join(".planr/policy.toml"),
        r#"
schema_version = 1
id = "policy-a"
version = "1.0.0"

[usage]
max_active_agents = 3
max_parallel_readers = 2
max_parallel_writers = 1
max_depth = 1
max_attempts = 4
max_wall_time_seconds = 600
max_tool_calls = 100
max_tokens = 10000
max_credits_micros = 1000000
review_reserve_percent = 20
budget_exhaustion = "downgrade_noncritical"
metering = "trusted"

[transitions.retry]
max_same_route_retries = 1

[transitions.availability_fallback]
max_fallbacks = 1
require_same_capability_class = true

[transitions.quality_escalation]
max_escalations = 1
require_verification_evidence = true

[transitions.quota_downgrade]
enabled = true
max_downgrades = 1
noncritical_only = true

[transitions.safety_stop]
enabled = true

[materiality]
changed_files_threshold = 10
changed_lines_threshold = 500

[execution]
max_read_scope_entries = 4
max_write_scope_entries = 2

[execution.roles.worker]
tools = ["cargo"]
commands = [{ program = "cargo", args = ["test"] }]

[execution.roles.worker.filesystem]
read_roots = ["src", "tests"]
write_roots = ["src", "tests"]
allow_overwrite = true
"#,
    )
    .unwrap();

    let shown = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "policy", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let shown: Value = serde_json::from_slice(&shown).unwrap();
    assert_eq!(shown["policy"]["id"], "policy-a");
    assert_eq!(shown["policy"]["usage"]["max_depth"], 1);
    assert_eq!(
        shown["policy"]["execution"]["roles"]["worker"]["tools"][0],
        "cargo"
    );
    assert_eq!(shown["enforcement"], "available");

    let checked = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "policy", "check"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let checked: Value = serde_json::from_slice(&checked).unwrap();
    assert_eq!(checked["ok"], true);
    assert_eq!(checked["policy_id"], "policy-a");

    let first_item = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "create",
            "Policy reader one",
            "--description",
            "authoritative admission",
        ])
        .assert()
        .success();
    drop(first_item);
    let first_pick = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-reader-1")
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let first_pick: Value = serde_json::from_slice(&first_pick).unwrap();
    let first_item_id = first_pick["item"]["id"].as_str().unwrap().to_string();
    let first_pick_token = first_pick["runtime"]["pick_token"]
        .as_str()
        .unwrap()
        .to_string();

    let mut admission_request = json!({
        "item_id": &first_item_id,
        "pick_token": first_pick_token,
        "role": "worker",
        "contract": {
            "objective": "verify admission parity",
            "inputs": ["policy fixture"],
            "outputs": ["admission decision"],
            "read_scope": ["src", "tests"],
            "write_scope": [],
            "acceptance_criteria": ["all surfaces agree"],
            "verification": ["cargo test"],
            "evidence_requirements": ["decision record"],
            "max_attempts": 1,
            "stop_conditions": ["decision returned"],
            "risk": "low",
            "materiality_triggers": [],
            "context": ["bounded"],
            "max_context_bytes": 100
        },
        "permissions": {
            "tools": ["cargo"],
            "commands": [{"program": "cargo", "args": ["test"]}],
            "overwrite_existing": true
        },
        "isolation": {"kind": "shared"},
        "approvals": []
    });
    fs::write(
        dir.path().join("admission.json"),
        serde_json::to_vec_pretty(&admission_request).unwrap(),
    )
    .unwrap();

    let bin = assert_cmd::cargo::cargo_bin("planr");
    let start_policy_server = |worker: &str| {
        let port = free_port();
        let server = StdCommand::new(&bin)
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", worker)
            .args([
                "--db",
                db.to_str().unwrap(),
                "serve",
                "--port",
                &port.to_string(),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        thread::sleep(Duration::from_millis(150));
        (server, port)
    };

    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-intruder")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "policy",
            "admit",
            "admission.json",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("current lease owner"));
    let foreign_mcp = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-intruder")
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_policy_admit","arguments":{"request":&admission_request}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(
        String::from_utf8(foreign_mcp)
            .unwrap()
            .contains("current lease owner")
    );
    let (mut foreign_server, foreign_port) = start_policy_server("policy-intruder");
    let foreign_http = http_request(
        foreign_port,
        "POST",
        "/v1/policy/admit",
        &admission_request.to_string(),
    );
    assert!(foreign_http.starts_with("HTTP/1.1 400"));
    assert_eq!(
        http_json(&foreign_http)["error"]["code"],
        "invalid_transition"
    );
    foreign_server.kill().unwrap();
    foreign_server.wait().unwrap();

    let admitted = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-reader-1")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "policy",
            "admit",
            "admission.json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let admitted: Value = serde_json::from_slice(&admitted).unwrap();
    assert_eq!(admitted["admission"]["status"], "allowed");

    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_policy_show","arguments":{}}}).to_string(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"planr_policy_check","arguments":{}}}).to_string(),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"planr_policy_admit","arguments":{"request":admission_request}}}).to_string(),
    ]
    .join("\n")
        + "\n";
    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-reader-1")
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let responses = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let payload = |index: usize| -> Value {
        serde_json::from_str(
            responses[index]["result"]["content"][0]["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(payload(0), shown);
    assert_eq!(payload(1), checked);
    assert_eq!(payload(2), admitted);

    let (mut server, port) = start_policy_server("policy-reader-1");
    let http_admitted = http_json(&http_request(
        port,
        "POST",
        "/v1/policy/admit",
        &admission_request.to_string(),
    ));
    assert_eq!(http_admitted, admitted);
    server.kill().unwrap();
    server.wait().unwrap();

    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-reader-1")
        .args([
            "--db",
            db.to_str().unwrap(),
            "pick",
            "release",
            &first_item_id,
        ])
        .assert()
        .success();
    let repicked = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-reader-1")
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let repicked: Value = serde_json::from_slice(&repicked).unwrap();
    let current_pick_token = repicked["runtime"]["pick_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(
        admission_request["pick_token"].as_str().unwrap(),
        current_pick_token
    );

    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-reader-1")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "policy",
            "admit",
            "admission.json",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("pick token does not match"));
    let stale_mcp = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-reader-1")
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_policy_admit","arguments":{"request":&admission_request}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(
        String::from_utf8(stale_mcp)
            .unwrap()
            .contains("pick token does not match")
    );
    let (mut stale_server, stale_port) = start_policy_server("policy-reader-1");
    let stale_http = http_request(
        stale_port,
        "POST",
        "/v1/policy/admit",
        &admission_request.to_string(),
    );
    assert!(stale_http.starts_with("HTTP/1.1 400"));
    assert_eq!(
        http_json(&stale_http)["error"]["code"],
        "invalid_transition"
    );
    stale_server.kill().unwrap();
    stale_server.wait().unwrap();

    admission_request["pick_token"] = json!(current_pick_token);
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "create",
            "Stale lease probe",
            "--description",
            "historical admission must not survive repick",
        ])
        .assert()
        .success();
    let probe_pick = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-stale-probe")
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let probe_pick: Value = serde_json::from_slice(&probe_pick).unwrap();
    let probe_id = probe_pick["item"]["id"].as_str().unwrap().to_string();
    let mut probe_request = admission_request.clone();
    probe_request["item_id"] = json!(&probe_id);
    probe_request["pick_token"] = probe_pick["runtime"]["pick_token"].clone();
    fs::write(
        dir.path().join("probe-admission.json"),
        serde_json::to_vec_pretty(&probe_request).unwrap(),
    )
    .unwrap();
    let probe_cli = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-stale-probe")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "policy",
            "admit",
            "probe-admission.json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let probe_cli: Value = serde_json::from_slice(&probe_cli).unwrap();
    assert_eq!(
        probe_cli["admission"]["reason"],
        "authoritative_state_unavailable"
    );
    let probe_mcp = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-stale-probe")
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_policy_admit","arguments":{"request":&probe_request}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let probe_mcp: Value = serde_json::from_slice(&probe_mcp).unwrap();
    let probe_mcp: Value =
        serde_json::from_str(probe_mcp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(probe_mcp, probe_cli);
    let (mut probe_server, probe_port) = start_policy_server("policy-stale-probe");
    assert_eq!(
        http_json(&http_request(
            probe_port,
            "POST",
            "/v1/policy/admit",
            &probe_request.to_string(),
        )),
        probe_cli
    );
    probe_server.kill().unwrap();
    probe_server.wait().unwrap();
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-stale-probe")
        .args(["--db", db.to_str().unwrap(), "pick", "release", &probe_id])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "cancel",
            &probe_id,
            "--confirm",
        ])
        .assert()
        .success();

    fs::write(
        dir.path().join("admission.json"),
        serde_json::to_vec_pretty(&admission_request).unwrap(),
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-reader-1")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "policy",
            "admit",
            "admission.json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"status\": \"allowed\""));

    let mut readers = Vec::new();
    for index in 2..=3 {
        planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "item",
                "create",
                &format!("Policy reader {index}"),
                "--description",
                "authoritative admission",
            ])
            .assert()
            .success();
        let picked = planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", format!("policy-reader-{index}"))
            .args(["--db", db.to_str().unwrap(), "--json", "pick"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let picked: Value = serde_json::from_slice(&picked).unwrap();
        readers.push((
            picked["item"]["id"].as_str().unwrap().to_string(),
            picked["runtime"]["pick_token"]
                .as_str()
                .unwrap()
                .to_string(),
        ));
        if index == 2 {
            let mut second_reader = admission_request.clone();
            second_reader["item_id"] = json!(&readers[0].0);
            second_reader["pick_token"] = json!(&readers[0].1);
            fs::write(
                dir.path().join("second-reader.json"),
                serde_json::to_vec_pretty(&second_reader).unwrap(),
            )
            .unwrap();
            planr()
                .current_dir(dir.path())
                .env("PLANR_WORKER_ID", "policy-reader-2")
                .args([
                    "--db",
                    db.to_str().unwrap(),
                    "--json",
                    "policy",
                    "admit",
                    "second-reader.json",
                ])
                .assert()
                .success()
                .stdout(predicate::str::contains("\"status\": \"allowed\""));
        }
    }

    let mut third_reader = admission_request.clone();
    third_reader["item_id"] = json!(&readers[1].0);
    third_reader["pick_token"] = json!(&readers[1].1);
    fs::write(
        dir.path().join("third-reader.json"),
        serde_json::to_vec_pretty(&third_reader).unwrap(),
    )
    .unwrap();
    let cli_limit = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-reader-3")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "policy",
            "admit",
            "third-reader.json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let cli_limit: Value = serde_json::from_slice(&cli_limit).unwrap();
    assert_eq!(cli_limit["admission"]["reason"], "reader_limit_exceeded");
    assert_eq!(cli_limit["concurrency"]["parallel_readers"], 2);

    let mcp_limit = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-reader-3")
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_policy_admit","arguments":{"request":&third_reader}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_limit: Value = serde_json::from_slice(&mcp_limit).unwrap();
    let mcp_limit: Value =
        serde_json::from_str(mcp_limit["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(mcp_limit, cli_limit);
    let (mut server, port) = start_policy_server("policy-reader-3");
    let http_limit = http_json(&http_request(
        port,
        "POST",
        "/v1/policy/admit",
        &third_reader.to_string(),
    ));
    assert_eq!(http_limit, cli_limit);
    server.kill().unwrap();
    server.wait().unwrap();

    let mut unsafe_request = admission_request.clone();
    unsafe_request["permissions"]["destructive_operations"] = json!(["delete_path"]);
    fs::write(
        dir.path().join("unsafe-admission.json"),
        serde_json::to_vec_pretty(&unsafe_request).unwrap(),
    )
    .unwrap();
    let rejected = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-reader-1")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "policy",
            "admit",
            "unsafe-admission.json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rejected: Value = serde_json::from_slice(&rejected).unwrap();
    assert_eq!(rejected["admission"]["status"], "rejected");
    assert_eq!(rejected["admission"]["reason"], "unsafe_operation");
    assert_eq!(rejected["admission"]["safety_stop"], true);
    let events = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "event", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let events: Value = serde_json::from_slice(&events).unwrap();
    assert!(events["events"].as_array().unwrap().iter().any(|event| {
        event["event_type"] == "execution_rejected"
            && event["payload"]["admission"]["reason"] == "unsafe_operation"
    }));

    fs::write(
        dir.path().join(".planr/policy.toml"),
        "schema_version = 1\nmodel = \"provider/model\"\n",
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "policy", "check"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("parse_error"));
    let degraded = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "policy", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let degraded: Value = serde_json::from_slice(&degraded).unwrap();
    assert_eq!(degraded["reason"], "degraded");
    assert_eq!(degraded["enforcement"], "unavailable");
    assert!(!degraded["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn concurrent_picks_do_not_duplicate_one_item() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Race"])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "create",
            "Only Item",
            "--description",
            "race",
        ])
        .assert()
        .success();

    let bin = assert_cmd::cargo::cargo_bin("planr");
    let mut a = StdCommand::new(&bin)
        .current_dir(dir.path())
        .env("PLANR_SESSION_ID", "session-a")
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut b = StdCommand::new(&bin)
        .current_dir(dir.path())
        .env("PLANR_SESSION_ID", "session-b")
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    assert!(a.wait().unwrap().success());
    assert!(b.wait().unwrap().success());

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["counts"]["picked"], 1);
}

#[test]
fn runtime_control_and_approval_gates_are_enforced() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Runtime"])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Runtime Item",
            "--description",
            "runtime and approval test",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&output).unwrap();
    let item_id = created["item"]["id"].as_str().unwrap();

    let unpicked_progress = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "progress",
            item_id,
            "--percent",
            "1",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let unpicked_progress: Value = serde_json::from_slice(&unpicked_progress).unwrap();
    assert_eq!(unpicked_progress["error"]["code"], "invalid_transition");

    let mut mcp = planr();
    let mcp_output = mcp
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "planr_pick_progress",
                    "arguments": {"item_id": item_id, "percent": 1}
                }
            })
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_response: Value = serde_json::from_str(
        String::from_utf8(mcp_output)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(mcp_response["result"]["isError"], true);
    assert!(
        mcp_response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("invalid_transition")
    );

    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_SESSION_ID", "runtime-a")
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let picked: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(picked["item"]["id"], item_id);

    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_SESSION_ID", "runtime-a")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "heartbeat",
            item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let heartbeat: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(heartbeat["item"]["status"], "running");
    assert!(heartbeat["runtime"]["last_heartbeat_at"].is_string());

    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_SESSION_ID", "runtime-a")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "progress",
            item_id,
            "--percent",
            "42",
            "--note",
            "halfway",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let progress: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(progress["runtime"]["progress_percent"], 42);
    assert_eq!(progress["runtime"]["progress_note"], "halfway");

    let paused = planr()
        .current_dir(dir.path())
        .env("PLANR_SESSION_ID", "runtime-a")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "pause",
            item_id,
            "--note",
            "waiting",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let paused: Value = serde_json::from_slice(&paused).unwrap();
    assert_eq!(paused["item"]["status"], "picked");
    assert!(paused["runtime"]["paused_at"].is_string());

    let resumed = planr()
        .current_dir(dir.path())
        .env("PLANR_SESSION_ID", "runtime-a")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "resume",
            item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let resumed: Value = serde_json::from_slice(&resumed).unwrap();
    assert_eq!(resumed["item"]["status"], "running");
    assert!(resumed["runtime"]["paused_at"].is_null());

    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE items SET last_heartbeat_at = datetime('now', '-1 hour') WHERE id = ?1",
        [item_id],
    )
    .unwrap();
    let stale = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "stale",
            "--older-than-seconds",
            "60",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stale: Value = serde_json::from_slice(&stale).unwrap();
    assert_eq!(stale["stale"][0]["item"]["id"], item_id);

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "stale",
            "--older-than-seconds",
            "60",
            "--release",
        ])
        .assert()
        .success();
    let status_and_worker: (String, Option<String>) = conn
        .query_row(
            "SELECT status, worker_id FROM items WHERE id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status_and_worker, ("ready".to_string(), None));

    planr()
        .current_dir(dir.path())
        .env("PLANR_SESSION_ID", "runtime-a")
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success();
    let requested = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "approval",
            "request",
            item_id,
            "--reason",
            "needs human signoff",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let requested: Value = serde_json::from_slice(&requested).unwrap();
    assert_eq!(requested["approval"]["status"], "requested");

    let preview = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "map",
            "preview",
            "--close",
            item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&preview).unwrap();
    assert_eq!(preview["can_close"], false);
    assert_eq!(preview["approval_blocks_close"], true);

    let blocked_close = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "close",
            item_id,
            "--summary",
            "premature",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let blocked_close: Value = serde_json::from_slice(&blocked_close).unwrap();
    assert_eq!(blocked_close["error"]["code"], "invalid_transition");

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "approval",
            "deny",
            item_id,
            "--by",
            "qa",
            "--comment",
            "missing proof",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "close",
            item_id,
            "--summary",
            "still blocked",
        ])
        .assert()
        .failure();

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "approval",
            "approve",
            item_id,
            "--by",
            "qa",
            "--comment",
            "proof accepted",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "close",
            item_id,
            "--summary",
            "approved",
        ])
        .assert()
        .success();

    let mut mcp = planr();
    mcp.current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n\
             {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"planr_approval_list\",\"arguments\":{}}}\n",
        )
        .assert()
        .success()
        .stdout(predicate::str::contains("planr_pick_heartbeat"))
        .stdout(predicate::str::contains("planr_approval_request"))
        .stdout(predicate::str::contains("approvals"));
}

#[test]
fn open_review_blocks_target_close_until_review_closes() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Review"])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Reviewed Item",
            "--description",
            "review gate",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    let item_id = value["item"]["id"].as_str().unwrap();

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "request",
            item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    let review_id = value["review"]["id"].as_str().unwrap();

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "close",
            item_id,
            "--summary",
            "premature",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["error"]["code"], "invalid_transition");

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "review",
            "close",
            review_id,
            "--verdict",
            "complete",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "close",
            item_id,
            "--summary",
            "reviewed",
        ])
        .assert()
        .success();
}

#[test]
fn human_review_feedback_contract_writes_annotations_artifacts_and_followups() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Human Gates",
        ])
        .assert()
        .success();
    let created = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Human reviewed item",
            "--description",
            "needs a review contract",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&created).unwrap();
    let item_id = created["item"]["id"].as_str().unwrap();
    let review = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "request",
            item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let review: Value = serde_json::from_slice(&review).unwrap();
    let review_id = review["review"]["id"].as_str().unwrap();

    let annotation = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "annotate",
            item_id,
            "--message",
            "Add missing acceptance coverage",
            "--severity",
            "blocking",
            "--file",
            "tests/e2e.rs",
            "--line",
            "42",
            "--author",
            "qa",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let annotation: Value = serde_json::from_slice(&annotation).unwrap();
    assert_eq!(annotation["annotation"]["severity"], "blocking");

    let feedback_path = dir.path().join("review-feedback.json");
    fs::write(
        &feedback_path,
        serde_json::to_vec(&json!({
            "reviewer": "hook-reviewer",
            "verdict": "not-complete",
            "findings": ["Hook finding requires a failing-path test"],
            "annotations": [
                {
                    "message": "Hook annotation is persisted",
                    "severity": "warning",
                    "file": "src/app.rs",
                    "line": 7
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let feedback = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "ingest",
            item_id,
            "--from",
            feedback_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let feedback: Value = serde_json::from_slice(&feedback).unwrap();
    assert_eq!(feedback["auto_closed"], false);
    assert_eq!(feedback["auto_approved"], false);
    assert_eq!(feedback["annotations"].as_array().unwrap().len(), 1);

    let mut mcp = planr();
    mcp.current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}}\n{}\n",
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "planr_review_annotate",
                    "arguments": {
                        "item_id": item_id,
                        "message": "MCP annotation",
                        "severity": "info"
                    }
                }
            })
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains("planr_review_ingest"))
        .stdout(predicate::str::contains("MCP annotation"));

    let close = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "close",
            review_id,
            "--verdict",
            "not-complete",
            "--findings",
            "Add missing test",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let close: Value = serde_json::from_slice(&close).unwrap();
    assert_eq!(close["verdict"], "not-complete");
    assert_eq!(close["created"].as_array().unwrap().len(), 2);
    let artifact_path = close["artifact"]["path"].as_str().unwrap();
    let artifact_body = fs::read_to_string(artifact_path).unwrap();
    assert!(artifact_body.contains("Add missing test"));
    assert!(artifact_body.contains("Hook annotation is persisted"));
    assert!(artifact_body.contains("Source file content included: false"));

    let explicit_artifact = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "artifact",
            review_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let explicit_artifact: Value = serde_json::from_slice(&explicit_artifact).unwrap();
    assert_eq!(explicit_artifact["artifact"]["kind"], "review");

    let events = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "event", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let events: Value = serde_json::from_slice(&events).unwrap();
    let event_types = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["event_type"].as_str())
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"review_annotation_added"));
    assert!(event_types.contains(&"review_feedback_ingested"));
    assert!(event_types.contains(&"review_artifact_written"));

    let http_item = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "HTTP human gate item",
            "--description",
            "http review routes",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let http_item: Value = serde_json::from_slice(&http_item).unwrap();
    let http_item_id = http_item["item"]["id"].as_str().unwrap();
    let port = free_port();
    let bin = assert_cmd::cargo::cargo_bin("planr");
    let mut server = StdCommand::new(&bin)
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(150));
    let http_review = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{http_item_id}/reviews"),
        "{}",
    ));
    let http_review_id = http_review["review"]["id"].as_str().unwrap();
    let http_annotation = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{http_item_id}/review-annotations"),
        "{\"message\":\"HTTP annotation\",\"severity\":\"blocking\"}",
    ));
    assert_eq!(http_annotation["annotation"]["severity"], "blocking");
    let http_feedback = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{http_item_id}/review-feedback"),
        "{\"reviewer\":\"http-hook\",\"findings\":[\"HTTP finding\"]}",
    ));
    assert_eq!(http_feedback["auto_closed"], false);
    let http_close = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/reviews/{http_review_id}/close"),
        "{\"verdict\":\"complete\",\"findings\":[]}",
    ));
    assert_eq!(http_close["verdict"], "complete");
    let http_artifact = http_json(&http_request(
        port,
        "GET",
        &format!("/v1/reviews/{http_review_id}/artifact"),
        "",
    ));
    assert_eq!(http_artifact["artifact"]["kind"], "review");
    let http_artifact_again = http_json(&http_request(
        port,
        "GET",
        &format!("/v1/reviews/{http_review_id}/artifact"),
        "",
    ));
    assert_eq!(
        http_artifact_again["artifact"]["id"],
        http_artifact["artifact"]["id"]
    );
    server.kill().unwrap();
    server.wait().unwrap();
}

#[test]
fn map_graph_intelligence_uses_dependency_paths_and_transitive_pressure() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Graph IQ"])
        .assert()
        .success();

    let root = create_test_item(dir.path(), &db, "Root", "root");
    let short = create_test_item_after(dir.path(), &db, "Short branch", "short", &root);
    let long_a = create_test_item_after(dir.path(), &db, "Long branch A", "a", &root);
    let long_b = create_test_item_after(dir.path(), &db, "Long branch B", "b", &long_a);
    let long_c = create_test_item_after(dir.path(), &db, "Long branch C", "c", &long_b);

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "map",
            "lane",
            "--critical",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let critical: Value = serde_json::from_slice(&output).unwrap();
    let critical_ids = critical["critical"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(critical_ids, vec![root.clone(), long_a, long_b, long_c]);
    assert!(!critical_ids.contains(&short));

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "map", "pressure"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pressure: Value = serde_json::from_slice(&output).unwrap();
    let top = &pressure["pressure"].as_array().unwrap()[0];
    assert_eq!(top["id"], root);
    assert_eq!(top["direct_blocks"], 2);
    assert_eq!(top["transitive_blocks"], 4);
    assert_eq!(top["blocks"], 4);

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "map", "status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let status: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(status["analysis"]["critical"][0]["id"], root);
    assert_eq!(status["analysis"]["pressure"][0]["transitive_blocks"], 4);
    assert!(status["analysis"]["cycles"].as_array().unwrap().is_empty());
}

#[test]
fn http_server_survives_aborted_and_garbage_connections() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Resilience",
        ])
        .assert()
        .success();
    let port = free_port();
    let bin = assert_cmd::cargo::cargo_bin("planr");
    let mut server = StdCommand::new(&bin)
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(150));

    // Connection dropped mid-request: header promises a body that never comes.
    {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .write_all(b"POST /v1/projects HTTP/1.1\r\nContent-Length: 50\r\n\r\n")
            .unwrap();
        drop(stream);
    }
    // Pure garbage bytes.
    {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .write_all(b"\x00\x01\x02 not http at all\r\n")
            .unwrap();
        drop(stream);
    }
    // Oversized declared body is rejected without allocation.
    {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .write_all(b"POST /v1/projects HTTP/1.1\r\nContent-Length: 99999999999\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        assert!(
            response.contains("413") || response.contains("payload_too_large"),
            "expected payload rejection, got: {response}"
        );
    }

    // The server must still answer normal requests afterwards.
    let health = http_request(port, "GET", "/health", "");
    assert!(health.contains("\"ok\":true"), "server died: {health}");

    server.kill().unwrap();
    server.wait().unwrap();
}

#[test]
fn http_protocol_correctness_status_codes_cors_and_live_sse() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Protocol"])
        .assert()
        .success();
    let port = free_port();
    let bin = assert_cmd::cargo::cargo_bin("planr");
    let mut server = StdCommand::new(&bin)
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(150));

    // Unknown route is a 404, not a 200 with an error body.
    let missing = http_request(port, "GET", "/v1/definitely-not-a-route", "");
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");
    assert!(missing.contains("not_found"), "{missing}");

    // Missing entity is a 404.
    let missing_item = http_request(port, "GET", "/v1/items/itm_nope", "");
    assert!(missing_item.starts_with("HTTP/1.1 404"), "{missing_item}");

    // Garbage JSON on a mutating route is a 400, and creates nothing.
    let garbage = http_request(port, "POST", "/v1/projects", "{definitely not json");
    assert!(garbage.starts_with("HTTP/1.1 400"), "{garbage}");
    let projects = http_json(&http_request(port, "GET", "/v1/projects", ""));
    assert_eq!(projects["projects"].as_array().unwrap().len(), 1);

    // Every response carries CORS headers; OPTIONS preflight is answered.
    let health = http_request(port, "GET", "/health", "");
    assert!(
        health
            .to_lowercase()
            .contains("access-control-allow-origin: *"),
        "{health}"
    );
    let preflight = http_request(port, "OPTIONS", "/v1/projects", "");
    assert!(preflight.starts_with("HTTP/1.1 204"), "{preflight}");

    // SSE is a live stream: an event recorded after the stream opens arrives.
    let stream_port = port;
    let listener_handle = thread::spawn(move || {
        http_sse_read_until(stream_port, "/v1/events/stream", "event: item_created")
    });
    thread::sleep(Duration::from_millis(400));
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "create",
            "Streamed item",
            "--description",
            "born during an open SSE stream",
        ])
        .assert()
        .success();
    let streamed = listener_handle.join().unwrap();
    assert!(streamed.contains("event: item_created"), "{streamed}");
    assert!(
        streamed.contains("Streamed item") || streamed.contains("data: "),
        "{streamed}"
    );

    server.kill().unwrap();
    server.wait().unwrap();
}

#[test]
fn mcp_server_survives_failing_tool_calls_and_answers_errors() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "McpErr"])
        .assert()
        .success();

    let input = [
        // Tool call that previously killed the server (missing item).
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_map_preview","arguments":{"close":"itm_does_not_exist"}}}).to_string(),
        // Tool call missing a required argument.
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"planr_map_preview","arguments":{}}}).to_string(),
        // Unknown JSON-RPC method must be answered with -32601, not ok:true.
        json!({"jsonrpc":"2.0","id":3,"method":"definitely/not-a-method"}).to_string(),
        // Notification (no id) must get no response line.
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
        // Unparseable line must produce a -32700 parse error response.
        "{not json".to_string(),
        // Server must still be alive and serving.
        json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}).to_string(),
    ]
    .join("\n")
        + "\n";

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let responses = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();

    // 5 responses for 6 inputs: the notification is silent.
    assert_eq!(responses.len(), 5, "unexpected responses: {responses:?}");

    assert_eq!(responses[0]["id"], 1);
    assert_eq!(responses[0]["result"]["isError"], true);
    assert!(
        responses[0]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not_found")
    );

    assert_eq!(responses[1]["id"], 2);
    assert_eq!(responses[1]["result"]["isError"], true);

    assert_eq!(responses[2]["id"], 3);
    assert_eq!(responses[2]["error"]["code"], -32601);

    assert_eq!(responses[3]["id"], Value::Null);
    assert_eq!(responses[3]["error"]["code"], -32700);

    assert_eq!(responses[4]["id"], 4);
    assert!(responses[4]["result"]["tools"].as_array().unwrap().len() > 10);
}

#[test]
fn map_show_renders_visual_dag_tree_and_state_line() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Graph View",
        ])
        .assert()
        .success();

    let root = create_test_item(dir.path(), &db, "Root work", "root");
    let mid = create_test_item_after(dir.path(), &db, "Middle work", "mid", &root);
    let leaf = create_test_item_after(dir.path(), &db, "Leaf work", "leaf", &mid);

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let explicit_tree = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "map",
            "show",
            "--view",
            "tree",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        output, explicit_tree,
        "the explicit tree view must be byte-identical to the default"
    );
    let human = String::from_utf8(output).unwrap();
    assert!(
        human.contains("Graph View: 0/3 done (0%) | ready 1 | active 0 | in_review 0 | blocked 2"),
        "missing state line in:\n{human}"
    );
    assert!(
        human.contains(&format!("○ ready {root} Root work ★ ⏶2")),
        "missing root node with critical marker and pressure in:\n{human}"
    );
    assert!(
        human.contains(&format!("└─blocks─▶ · pending {mid} Middle work ★ ⏶1")),
        "missing nested middle node in:\n{human}"
    );
    assert!(
        human.contains(&format!("   └─blocks─▶ · pending {leaf} Leaf work ★")),
        "missing doubly nested leaf node in:\n{human}"
    );

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["items"].as_array().unwrap().len(), 3);
    assert_eq!(value["links"].as_array().unwrap().len(), 2);
    assert_eq!(
        value["counts"]["ready"], 1,
        "blocked downstream items must not count as ready"
    );
}

#[test]
fn map_show_diagram_renders_shared_routes_and_preserves_json_contract() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Diagram View",
        ])
        .assert()
        .success();

    let root_a = create_test_item(dir.path(), &db, "Root A", "root a");
    let root_b = create_test_item(dir.path(), &db, "Root B", "root b");
    let join = create_test_item_after(dir.path(), &db, "Shared join", "join", &root_a);
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "link",
            "add",
            &root_b,
            &join,
            "--type",
            "hands_to",
        ])
        .assert()
        .success();

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "map",
            "show",
            "--view",
            "diagram",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let human = String::from_utf8(output).unwrap();
    assert!(human.contains("── Diagram View · WORKFLOW MAP"), "{human}");
    assert_eq!(human.matches("component ").count(), 1, "{human}");
    assert!(human.contains("┌────────────────"), "{human}");
    assert!(human.contains("blocks ─▶"), "{human}");
    assert!(human.contains("hands_to ─▶"), "{human}");
    assert!(!human.contains("READY"), "{human}");
    assert!(!human.contains("PENDING"), "{human}");
    assert!(human.contains(&root_a), "{human}");
    assert!(human.contains(&root_b), "{human}");
    assert!(human.contains(&join), "{human}");
    let mut content_rows = 0;
    let mut boxes = 0;
    for line in human.lines() {
        if line.contains('┌') {
            content_rows = 0;
        } else if line.contains('└') && line.contains('┘') {
            assert!(
                (1..=2).contains(&content_rows),
                "compact box contained {content_rows} content rows:\n{human}"
            );
            boxes += 1;
        } else if line.contains('│') && line.ends_with('│') {
            content_rows += 1;
        }
    }
    assert!(boxes >= 3, "expected compact boxes in:\n{human}");

    let full = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "map",
            "show",
            "--view",
            "diagram",
            "--full",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let full = String::from_utf8(full).unwrap();
    assert!(full.contains("○ READY"), "{full}");
    assert!(
        full.contains("↳ joins a node already shown above"),
        "{full}"
    );

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "map", "show", "--full"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--view diagram"));

    let default_json = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let diagram_json = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "map",
            "show",
            "--view",
            "diagram",
            "--full",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(default_json, diagram_json);

    planr()
        .args(["map", "show", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("for human supervision only"))
        .stdout(predicate::str::contains(
            "agents should use `tree` or `--json`",
        ));
    planr()
        .args(["map", "watch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Human-only live observer"))
        .stdout(predicate::str::contains("map show --json"));
}

#[test]
fn map_human_edges_distinguish_satisfied_dependencies_without_changing_json() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", db_arg, "project", "init", "Edge State"])
        .assert()
        .success();

    let source = create_test_item(dir.path(), &db, "Source", "source");
    let target = create_test_item_after(dir.path(), &db, "Target", "target", &source);
    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE items SET status = 'closed' WHERE id = ?1",
        [&source],
    )
    .unwrap();

    planr()
        .current_dir(dir.path())
        .args(["--db", db_arg, "map", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "blocks✓─▶ · pending {target}"
        )))
        .stdout(predicate::str::contains("blocks─▶").not());
    planr()
        .current_dir(dir.path())
        .args(["--db", db_arg, "map", "show", "--view", "diagram"])
        .assert()
        .success()
        .stdout(predicate::str::contains("└─ then ─▶"))
        .stdout(predicate::str::contains("└─ blocks ─▶").not());
    planr()
        .current_dir(dir.path())
        .env_remove("NO_COLOR")
        .env("TERM", "xterm")
        .env("PLANR_FORCE_COLOR", "1")
        .args(["--db", db_arg, "map", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\x1b[2mblocks✓─▶\x1b[0m"))
        .stdout(predicate::str::contains("\x1b[31mblocks✓─▶").not());

    let json = planr()
        .current_dir(dir.path())
        .args(["--db", db_arg, "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(json["links"][0]["kind"], "blocks");
    assert!(!serde_json::to_string(&json).unwrap().contains("blocks✓"));
    assert!(!serde_json::to_string(&json).unwrap().contains("then"));

    conn.execute(
        "UPDATE items SET status = 'cancelled' WHERE id = ?1",
        [&source],
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .env_remove("NO_COLOR")
        .env("TERM", "xterm")
        .env("PLANR_FORCE_COLOR", "1")
        .args(["--db", db_arg, "map", "show", "--view", "diagram"])
        .assert()
        .success()
        .stdout(predicate::str::contains("└─ \x1b[31mblocks ─▶\x1b[0m"))
        .stdout(predicate::str::contains("└─ \x1b[2mthen ─▶").not());
}

#[test]
fn map_colors_cover_states_and_all_opt_outs_remain_plain() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", db_arg, "project", "init", "Color View"])
        .assert()
        .success();

    let states = [
        ("Ready", "ready"),
        ("Pending", "pending"),
        ("Picked", "picked"),
        ("Running", "running"),
        ("Review", "in_review"),
        ("Blocked", "blocked"),
        ("Closed", "closed"),
        ("Failed", "failed"),
        ("Cancelled", "cancelled"),
    ];
    let items = states
        .iter()
        .map(|(title, status)| (create_test_item(dir.path(), &db, title, status), *status))
        .collect::<Vec<_>>();
    let conn = Connection::open(&db).unwrap();
    for (id, status) in items {
        conn.execute(
            "UPDATE items SET status = ?1, worker_id = CASE WHEN ?1 IN ('picked','running') THEN 'worker-color' ELSE NULL END WHERE id = ?2",
            rusqlite::params![status, id],
        )
        .unwrap();
    }

    let forced = planr()
        .current_dir(dir.path())
        .env_remove("NO_COLOR")
        .env("TERM", "xterm")
        .env("PLANR_FORCE_COLOR", "1")
        .args(["--db", db_arg, "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let forced = String::from_utf8(forced).unwrap();
    for token in [
        "\x1b[36m○ ready\x1b[0m",
        "\x1b[2m· pending\x1b[0m",
        "\x1b[33m◎ picked\x1b[0m",
        "\x1b[1;33m◉ running\x1b[0m",
        "\x1b[35m◇ in_review\x1b[0m",
        "\x1b[31m⊖ blocked\x1b[0m",
        "\x1b[32m✓ closed\x1b[0m",
        "\x1b[1;31m✗ failed\x1b[0m",
        "\x1b[2m⊘ cancelled\x1b[0m",
    ] {
        assert!(forced.contains(token), "missing {token:?} in {forced:?}");
    }

    let plain = |extra: &[&str], envs: &[(&str, &str)]| {
        let mut command = planr();
        command
            .current_dir(dir.path())
            .env_remove("NO_COLOR")
            .env("TERM", "xterm")
            .env("PLANR_FORCE_COLOR", "1")
            .args(["--db", db_arg])
            .args(extra);
        for (name, value) in envs {
            command.env(name, value);
        }
        command
            .assert()
            .success()
            .stdout(predicate::str::contains("Color View"))
            .stdout(predicate::str::contains("\x1b[").not());
    };
    plain(&["--no-color", "map", "show"], &[]);
    plain(&["map", "show"], &[("NO_COLOR", "")]);
    plain(&["map", "show"], &[("TERM", "dumb")]);

    planr()
        .current_dir(dir.path())
        .env_remove("NO_COLOR")
        .env_remove("PLANR_FORCE_COLOR")
        .env("TERM", "xterm")
        .args(["--db", db_arg, "map", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\x1b[").not());

    let json_output = planr()
        .current_dir(dir.path())
        .env_remove("NO_COLOR")
        .env("TERM", "xterm")
        .env("PLANR_FORCE_COLOR", "1")
        .args(["--db", db_arg, "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(!json_output.windows(2).any(|pair| pair == b"\x1b["));
    serde_json::from_slice::<Value>(&json_output).unwrap();
}

#[test]
fn map_watch_observes_an_external_process_change_and_stays_machine_safe() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Live View"])
        .assert()
        .success();
    let item = create_test_item(dir.path(), &db, "Observed work", "watch me");

    let bin = assert_cmd::cargo::cargo_bin("planr");
    let mut watcher = StdCommand::new(&bin)
        .current_dir(dir.path())
        .env("NO_COLOR", "1")
        .args([
            "--db",
            &db_arg,
            "map",
            "watch",
            "--view",
            "tree",
            "--interval-ms",
            "100",
            "--iterations",
            "20",
            "--no-clear",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = watcher.stdout.take().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut human = String::new();
        let mut saw_first_update = false;
        let mut readiness_sent = false;
        for line in BufReader::new(stdout).lines() {
            let line = line.unwrap();
            saw_first_update |= line.contains("update 1");
            if saw_first_update && line.contains("○ ready") && !readiness_sent {
                readiness_sent = true;
                let _ = ready_tx.send(());
            }
            human.push_str(&line);
            human.push('\n');
        }
        human
    });
    if let Err(error) = ready_rx.recv_timeout(Duration::from_secs(5)) {
        let _ = watcher.kill();
        let _ = watcher.wait();
        let human = reader.join().unwrap();
        let mut stderr = String::new();
        watcher
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        panic!(
            "watcher did not emit its ready frame: {error}; stdout={human:?}; stderr={stderr:?}"
        );
    }

    let pick = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "external-agent")
        .args(["--db", &db_arg, "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pick: Value = serde_json::from_slice(&pick).unwrap();
    assert_eq!(pick["item"]["id"], item);

    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match watcher.try_wait().unwrap() {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = watcher.kill();
                let status = watcher.wait().unwrap();
                let human = reader.join().unwrap();
                let mut stderr = String::new();
                watcher
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                panic!(
                    "watcher exceeded its deadline ({status}); stdout={human:?}; stderr={stderr:?}"
                );
            }
            None => thread::sleep(Duration::from_millis(10)),
        }
    };
    let human = reader.join().unwrap();
    let mut stderr = String::new();
    watcher
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(status.success(), "{stderr}");
    assert_eq!(human.matches("watching map").count(), 2, "{human}");
    assert!(human.contains("update 1"), "{human}");
    assert!(human.contains("update 2"), "{human}");
    assert!(human.contains("○ ready"), "{human}");
    assert!(human.contains("◎ picked"), "{human}");
    assert!(human.contains("external-agent"), "{human}");
    assert!(!human.contains("\x1b[2J"), "{human:?}");

    planr()
        .current_dir(dir.path())
        .env("NO_COLOR", "1")
        .args([
            "--db",
            &db_arg,
            "map",
            "watch",
            "--iterations",
            "1",
            "--no-clear",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "◎ {item} → Observed work"
        )))
        .stdout(predicate::str::contains("PICKED").not())
        .stdout(predicate::str::contains("worker: external-agent").not());

    planr()
        .current_dir(dir.path())
        .env("NO_COLOR", "1")
        .args([
            "--db",
            &db_arg,
            "map",
            "watch",
            "--iterations",
            "1",
            "--no-clear",
            "--full",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("◎ PICKED"))
        .stdout(predicate::str::contains("worker: external-agent"));

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "--json",
            "map",
            "watch",
            "--iterations",
            "1",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("bad_request"))
        .stdout(predicate::str::contains("/v1/events/stream"));
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "map", "watch", "--interval-ms", "99"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value '99'"));
}

#[test]
fn map_graph_intelligence_reports_cycles_before_critical_output() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Cycle"])
        .assert()
        .success();

    let alpha = create_test_item(dir.path(), &db, "Alpha", "alpha");
    let beta = create_test_item_after(dir.path(), &db, "Beta", "beta", &alpha);
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "link", "add", &beta, &alpha])
        .assert()
        .success();

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "map",
            "lane",
            "--critical",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("cycle detected"));
}

fn create_test_item(
    dir: &std::path::Path,
    db: &std::path::Path,
    title: &str,
    description: &str,
) -> String {
    let output = planr()
        .current_dir(dir)
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            title,
            "--description",
            description,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    value["item"]["id"].as_str().unwrap().to_string()
}

fn create_test_item_after(
    dir: &std::path::Path,
    db: &std::path::Path,
    title: &str,
    description: &str,
    after: &str,
) -> String {
    let output = planr()
        .current_dir(dir)
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            title,
            "--description",
            description,
            "--after",
            after,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    value["item"]["id"].as_str().unwrap().to_string()
}

fn item_status(db: &std::path::Path, item_id: &str) -> String {
    let conn = Connection::open(db).unwrap();
    conn.query_row("SELECT status FROM items WHERE id = ?1", [item_id], |row| {
        row.get(0)
    })
    .unwrap()
}

fn child_item_ids(db: &std::path::Path, parent_id: &str) -> Vec<String> {
    let conn = Connection::open(db).unwrap();
    let mut stmt = conn
        .prepare("SELECT id FROM items WHERE parent_item_id = ?1 ORDER BY created_at")
        .unwrap();
    stmt.query_map([parent_id], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn breakdown_accepts_repeated_flags_and_newlines_and_reports_chain_and_next() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Breakdown"])
        .assert()
        .success();
    let parent = create_test_item(dir.path(), &db, "Coarse slice", "breakdown contract");

    // The delimiter contract: repeated --into flags and newline-packed
    // values parse identically, so agents never guess the separator (the
    // Codex dogfood passed newlines and got one swallowed mega-title).
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "breakdown",
            &parent,
            "--into",
            "Step A",
            "--into",
            "Step B\nStep C",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    let items = value["items"].as_array().unwrap();
    let titles = items
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(titles, ["Step A", "Step B", "Step C"]);
    assert_eq!(
        items[0]["status"], "ready",
        "first child must be ready: {value}"
    );
    assert_eq!(
        items[1]["status"], "pending",
        "chained children must wait on the chain: {value}"
    );
    assert_eq!(
        value["links"].as_array().unwrap().len(),
        2,
        "breakdown must report the chain links: {value}"
    );
    assert_eq!(value["item"]["status"], "blocked", "parent parks as a gate");
    assert_eq!(value["next"], "planr pick --json");

    // The human output names every child and the next command instead of a
    // bare count.
    let human = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "breakdown",
            &create_test_item(dir.path(), &db, "Second slice", "human output"),
            "--into",
            "Only child",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let human = String::from_utf8(human).unwrap();
    assert!(human.contains("Only child"), "human output: {human}");
    assert!(human.contains("next: planr pick"), "human output: {human}");

    // Without --next, done still ends in an action: the response names the
    // exact follow-up command.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pick: Value = serde_json::from_slice(&output).unwrap();
    let picked = pick["item"]["id"].as_str().unwrap().to_string();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &picked,
            "--summary",
            "picked slice done",
            "--review",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        done["next"], "planr pick --work-type review --json",
        "done --review without --next must name the follow-up command: {done}"
    );
}

#[test]
fn done_on_never_picked_item_adopts_lease_and_review_mode_stays_attributed() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Adopt"])
        .assert()
        .success();
    let item = create_test_item(
        dir.path(),
        &db,
        "Upload flow",
        "dogfood adoption regression",
    );

    // `done --review` on a ready item that was never picked: the second
    // Codex dogfood hit the silent-skip variant of this — the target stayed
    // `ready` and the review closed `unattributed` despite a worker id.
    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-1")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &item,
            "--summary",
            "implemented upload flow",
            "--review",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        done["item"]["status"], "in_review",
        "adopted item must transition to in_review: {done}"
    );
    assert_eq!(
        done["item"]["worker_id"], "maker-1",
        "adoption must record the worker lease: {done}"
    );
    let review_id = done["review"]["id"].as_str().unwrap().to_string();

    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-1")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "close",
            &review_id,
            "--verdict",
            "complete",
            "--close-target",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let closed: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        closed["review_mode"], "single_agent",
        "same identity on both sides must record single_agent, never unattributed: {closed}"
    );
}

#[test]
fn parent_gate_rolls_up_and_auto_closes_when_children_settle() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Gates"])
        .assert()
        .success();
    let parent = create_test_item(dir.path(), &db, "Parent feature", "parent gate rollup");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "breakdown",
            &parent,
            "--into",
            "Child A,Child B",
        ])
        .assert()
        .success();
    assert_eq!(item_status(&db, &parent), "blocked");
    let children = child_item_ids(&db, &parent);
    assert_eq!(children.len(), 2);

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "close",
            &children[0],
            "--summary",
            "child a done",
        ])
        .assert()
        .success();
    assert_eq!(
        item_status(&db, &parent),
        "blocked",
        "parent must stay blocked while a child is open"
    );

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "close",
            &children[1],
            "--summary",
            "child b done",
        ])
        .assert()
        .success();
    assert_eq!(
        item_status(&db, &parent),
        "closed",
        "parent gate must auto-close once all children are closed"
    );

    let conn = Connection::open(&db).unwrap();
    let completion_logs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM logs WHERE item_id = ?1 AND kind = 'completion'",
            [&parent],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(completion_logs, 1, "auto-close must write a completion log");

    // A cancelled child downgrades the parent gate to closed_partial.
    let partial_parent = create_test_item(dir.path(), &db, "Partial parent", "partial rollup");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "breakdown",
            &partial_parent,
            "--into",
            "Keep child,Drop child",
        ])
        .assert()
        .success();
    let partial_children = child_item_ids(&db, &partial_parent);
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "close",
            &partial_children[0],
            "--summary",
            "kept child done",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "cancel",
            &partial_children[1],
            "--confirm",
        ])
        .assert()
        .success();
    assert_eq!(item_status(&db, &partial_parent), "closed_partial");
}

#[test]
fn parent_gate_waits_for_open_review_then_auto_closes() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Gates"])
        .assert()
        .success();
    let parent = create_test_item(dir.path(), &db, "Reviewed parent", "gate with review");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "breakdown",
            &parent,
            "--into",
            "Only child",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "request",
            &parent,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let review: Value = serde_json::from_slice(&output).unwrap();
    let review_id = review["review"]["id"].as_str().unwrap().to_string();

    let child = child_item_ids(&db, &parent)[0].clone();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "close",
            &child,
            "--summary",
            "child done",
        ])
        .assert()
        .success();
    assert_eq!(
        item_status(&db, &parent),
        "ready",
        "parent with open review must become ready, not auto-close"
    );

    // Parent gates are not pickable work even when ready.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let picked: Value = serde_json::from_slice(&output).unwrap();
    assert_ne!(
        picked["item"]["id"].as_str(),
        Some(parent.as_str()),
        "pick must not claim a parent gate"
    );

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "review",
            "close",
            &review_id,
            "--verdict",
            "complete",
        ])
        .assert()
        .success();
    assert_eq!(
        item_status(&db, &parent),
        "closed",
        "parent gate must auto-close once its review closes"
    );
}

#[test]
fn log_files_flag_is_repeatable_and_artifact_name_works_as_flag() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Friction"])
        .assert()
        .success();
    let item_id = create_test_item(dir.path(), &db, "Friction item", "cli friction test");

    // --files must accept repetition and comma-separated values together.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "touched files",
            "--files",
            "src/a.rs",
            "--files",
            "src/b.rs,src/c.rs",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    let files = value["log"]["files"].as_array().unwrap();
    let files: Vec<_> = files.iter().filter_map(Value::as_str).collect();
    assert_eq!(files, vec!["src/a.rs", "src/b.rs", "src/c.rs"]);

    // artifact add must accept the name via --name anywhere in the command.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "artifact",
            "add",
            "--item",
            &item_id,
            "--name",
            "flag-named artifact",
            "--content",
            "evidence",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["artifact"]["name"], "flag-named artifact");

    // Missing name must produce an actionable error, not a clap usage puzzle.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "artifact",
            "add",
            "--item",
            &item_id,
            "--content",
            "evidence",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("artifact name is required"));
}

#[test]
fn done_command_collapses_log_review_close_and_next_pick() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Flow"])
        .assert()
        .success();
    let first = create_test_item(dir.path(), &db, "First slice", "compound flow test");
    let second = create_test_item(dir.path(), &db, "Second slice", "compound flow test");

    // Pick the first item, then finish it with review + next in one command.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let picked: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(picked["item"]["id"], first);
    assert!(
        picked["runtime"]["pick_token"].is_string(),
        "pick must include the flat work packet runtime: {picked}"
    );
    assert!(
        picked.get("context").is_none() && picked.get("trace").is_none(),
        "pick packet must be flat without nested context/trace duplication: {picked}"
    );
    let counts = picked["remaining"]["counts"].as_object().unwrap();
    for status in [
        "pending",
        "ready",
        "picked",
        "running",
        "in_review",
        "blocked",
        "failed",
        "cancelled",
        "closed",
        "closed_partial",
    ] {
        assert!(
            counts.contains_key(status),
            "remaining.counts must carry explicit zero for {status}: {picked}"
        );
    }

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &first,
            "--summary",
            "implemented first slice",
            "--files",
            "src/a.rs,src/b.rs",
            "--cmd",
            "cargo test",
            "--review",
            "--next",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&output).unwrap();
    let review_id = done["review"]["id"].as_str().unwrap().to_string();
    assert_eq!(done["closed"], Value::Null, "--review must not close");
    assert!(
        done["remaining"]["total"].as_i64().unwrap_or(0) > 0,
        "done must report board progress for the loop stop condition"
    );
    assert_eq!(
        done["next"]["item"]["id"], second,
        "--next must pick the following ready item"
    );
    assert_eq!(
        item_status(&db, &first),
        "in_review",
        "done --review must surface the review-wait state instead of running"
    );

    // A reviewer's first trace of the review item must already contain the
    // target's evidence, and the human mode must render the packet.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "trace",
            "item",
            &review_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let trace: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(trace["target"]["item"]["id"], first);
    assert!(
        !trace["target"]["logs"].as_array().unwrap().is_empty(),
        "review trace must inline the target completion log"
    );
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "trace", "item", &review_id])
        .assert()
        .success()
        .stdout(predicate::str::contains(&first))
        .stdout(predicate::str::contains("target log"));

    // The reviewer closes review and target in one command.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "close",
            &review_id,
            "--verdict",
            "complete",
            "--close-target",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let closed: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(closed["closed_target"]["id"], first);
    assert!(
        closed["remaining"]["total"].as_i64().unwrap_or(0) > 0,
        "review close must report board progress"
    );
    assert_eq!(item_status(&db, &first), "closed");

    // done without --review closes directly.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "done",
            &second,
            "--summary",
            "second slice done",
        ])
        .assert()
        .success();
    assert_eq!(item_status(&db, &second), "closed");

    // --next must never hand the worker its own freshly requested review.
    let third = create_test_item(dir.path(), &db, "Third slice", "maker checker split");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "pick"])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &third,
            "--summary",
            "third slice done",
            "--review",
            "--next",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        done["next"]["item"],
        Value::Null,
        "worker must not pick its own review; only the review was ready"
    );
}

#[test]
fn independent_review_stamp_requires_explicit_reviewer_identity() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Stamps"])
        .assert()
        .success();

    let mut review_ids = Vec::new();
    for title in ["Slice A", "Slice B"] {
        let item = create_test_item(dir.path(), &db, title, "stamp check");
        planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "maker-1")
            .args(["--db", db.to_str().unwrap(), "pick"])
            .assert()
            .success();
        let output = planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "maker-1")
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "done",
                &item,
                "--summary",
                "built",
                "--review",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let done: Value = serde_json::from_slice(&output).unwrap();
        review_ids.push(done["review"]["id"].as_str().unwrap().to_string());
    }

    // Anonymous reviewer (fallback identity): the string differs from
    // maker-1, but that proves nothing — the stamp must be single_agent,
    // never independent by luck.
    let output = planr()
        .current_dir(dir.path())
        .env_remove("PLANR_WORKER_ID")
        .env_remove("PLANR_SESSION_ID")
        .env_remove("CODEX_SESSION_ID")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "close",
            &review_ids[0],
            "--verdict",
            "complete",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let closed: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        closed["review_mode"], "single_agent",
        "anonymous reviewer must not stamp independent: {closed}"
    );

    // A blank --reviewer is not an identity: it must stamp single_agent
    // exactly like the anonymous fallback (GPT-5.5 review finding).
    let item = create_test_item(dir.path(), &db, "Slice C", "stamp check");
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-1")
        .args(["--db", db.to_str().unwrap(), "pick"])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-1")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &item,
            "--summary",
            "built",
            "--review",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&output).unwrap();
    let blank_review = done["review"]["id"].as_str().unwrap().to_string();
    let output = planr()
        .current_dir(dir.path())
        .env_remove("PLANR_WORKER_ID")
        .env_remove("PLANR_SESSION_ID")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "close",
            &blank_review,
            "--verdict",
            "complete",
            "--reviewer",
            "   ",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let closed: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        closed["review_mode"], "single_agent",
        "blank --reviewer must not stamp independent: {closed}"
    );

    // Explicit --reviewer earns the independent stamp as before.
    let output = planr()
        .current_dir(dir.path())
        .env_remove("PLANR_WORKER_ID")
        .env_remove("PLANR_SESSION_ID")
        .env_remove("CODEX_SESSION_ID")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "close",
            &review_ids[1],
            "--verdict",
            "complete",
            "--reviewer",
            "checker-9",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let closed: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(closed["review_mode"], "independent");
    assert_eq!(closed["reviewer"], "checker-9");
}

#[test]
fn review_close_guard_reviewer_identity_and_role_aware_picks() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "FixPack"])
        .assert()
        .success();
    let first = create_test_item(dir.path(), &db, "Build the slice", "fix pack run");
    let second = create_test_item(dir.path(), &db, "Downstream slice", "fix pack run");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "link", "add", &first, &second])
        .assert()
        .success();

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "pick"])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &first,
            "--summary",
            "built the slice",
            "--review",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&output).unwrap();
    let review_id = done["review"]["id"].as_str().unwrap().to_string();

    // Role-aware pick: a maker asking for code work must not lease the
    // review, and the null pick must explain itself instead of being blind.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "--work-type",
            "code",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let null_pick: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(null_pick["item"], Value::Null);
    assert_eq!(null_pick["reason"], "ready_items_excluded_by_filter");
    assert!(
        null_pick["remaining"]["counts"]["ready"].as_i64().unwrap() > 0,
        "null pick must carry the remaining snapshot: {null_pick}"
    );
    let excluded = null_pick["excluded"].as_array().unwrap();
    assert!(
        excluded.iter().any(|entry| {
            entry["work_type"] == "review"
                && entry["cause"]
                    .as_str()
                    .unwrap()
                    .contains("--work-type code")
        }),
        "exclusions must name the work_type mismatch: {null_pick}"
    );
    assert!(
        null_pick["repair"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cmd| cmd == "planr pick --json"),
        "filter exclusion must carry the exact repair command: {null_pick}"
    );

    // A checker asking for review work leases exactly the review item, and
    // its close effect previews the --close-target cascade: closing this
    // review settles the target, which unlocks the downstream item.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "--work-type",
            "review",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let review_pick: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(review_pick["item"]["id"], review_id);
    assert!(
        review_pick.get("worker_id").is_none(),
        "worker identity lives in item/runtime, not a third top-level copy: {review_pick}"
    );
    let unlocked = review_pick["close_effect"]["would_unlock"]
        .as_array()
        .unwrap();
    assert!(
        unlocked.iter().any(|item| item["id"] == second),
        "review close effect must model the --close-target cascade: {review_pick}"
    );

    // Reviewer identity is recorded on the response and the artifact.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "close",
            &review_id,
            "--verdict",
            "complete",
            "--reviewer",
            "checker-1",
            "--close-target",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let closed: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(closed["reviewer"], "checker-1");
    assert_eq!(closed["closed_target"]["id"], first);
    let artifact = fs::read_to_string(
        dir.path()
            .join(".planr/reviews")
            .join(format!("{review_id}.review.md")),
    )
    .unwrap();
    assert!(
        artifact.contains("Reviewer: checker-1"),
        "artifact must attribute the checker: {artifact}"
    );
    // The artifact is evidence: it must show the final target status after
    // --close-target, not the pre-close in_review snapshot.
    assert!(
        artifact.contains(&format!("Target item: {first} (closed)")),
        "artifact must snapshot the target after --close-target: {artifact}"
    );

    // Double close must fail instead of silently duplicating evidence logs.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "review",
            "close",
            &review_id,
            "--verdict",
            "complete",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already_closed"));

    // In JSON mode the same failure carries a machine-readable error code
    // instead of the generic internal_error.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "close",
            &review_id,
            "--verdict",
            "complete",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let error: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(error["error"]["code"], "already_closed");

    // map show reports the same explicit-zero counts vocabulary as the
    // remaining snapshot, plus settled/total.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let map: Value = serde_json::from_slice(&output).unwrap();
    let counts = map["counts"].as_object().unwrap();
    for status in [
        "pending",
        "ready",
        "picked",
        "running",
        "in_review",
        "blocked",
        "failed",
        "cancelled",
        "closed",
        "closed_partial",
    ] {
        assert!(
            counts.contains_key(status),
            "map counts must carry explicit zero for {status}: {map}"
        );
    }
    assert!(map["settled"].is_number() && map["total"].is_number());

    // Settle the rest, then the null pick explains the finished board.
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "pick"])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "done",
            &second,
            "--summary",
            "downstream slice done",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let final_pick: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(final_pick["item"], Value::Null);
    assert_eq!(final_pick["reason"], "all_settled");
}

#[test]
fn cosmetic_batch_stable_shapes_ids_and_worker_identity() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Polish"])
        .assert()
        .success();

    // Item ids never contain a double dash, even when the 32-char slug
    // truncation lands on a hyphen.
    let long_title = format!("{} b suffix", "a".repeat(31));
    let id = create_test_item(dir.path(), &db, &long_title, "slug truncation");
    assert!(!id.contains("--"), "item id must not contain '--': {id}");

    // PLANR_WORKER_ID attributes the lease to the agent, not client:host:user.
    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-7")
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pick: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(pick["item"]["worker_id"], "maker-7");

    // deeper_reads hints consistently carry the --json flag.
    for hint in pick["deeper_reads"].as_array().unwrap() {
        assert!(
            hint.as_str().unwrap().contains("--json"),
            "deeper read hints must be JSON-mode commands: {hint}"
        );
    }

    // Log list shapes are stable: list fields are [] instead of null even on
    // logs that never set files/commands/tests (e.g. review logs).
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &id,
            "--summary",
            "slice done",
            "--review",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&output).unwrap();
    let review_id = done["review"]["id"].as_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "review",
            "close",
            &review_id,
            "--verdict",
            "complete",
            "--close-target",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "log",
            "list",
            "--item",
            &review_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let logs: Value = serde_json::from_slice(&output).unwrap();
    let review_log = logs["logs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|log| log["kind"] == "review")
        .expect("review log exists");
    for field in ["files", "commands", "tests"] {
        assert!(
            review_log[field].is_array(),
            "log {field} must be [] instead of null: {review_log}"
        );
    }

    // Plan split does not duplicate the source title in the build plan slug
    // when the slice already repeats it.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "new",
            "Habit MVP",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let plan: Value = serde_json::from_slice(&output).unwrap();
    let plan_id = plan["plan"]["id"].as_str().unwrap().to_string();
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "split",
            &plan_id,
            "--slice",
            "Habit MVP build slice",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let build: Value = serde_json::from_slice(&output).unwrap();
    let path = build["plan"]["path"].as_str().unwrap();
    assert!(
        path.ends_with("habit-mvp-build-slice.plan.md"),
        "build plan filename must not duplicate the source title: {path}"
    );
}

#[test]
fn plan_scoped_pick_never_leases_items_of_another_plan() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Scoped"])
        .assert()
        .success();
    let new_plan = |title: &str| {
        let output = planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "--json", "plan", "new", title])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let plan: Value = serde_json::from_slice(&output).unwrap();
        (
            plan["plan"]["id"].as_str().unwrap().to_string(),
            plan["plan"]["path"].as_str().unwrap().to_string(),
        )
    };
    let map_build = |plan_id: &str| {
        planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "map",
                "build",
                "--from",
                plan_id,
            ])
            .assert()
            .success();
    };
    let (alpha_id, _) = new_plan("Alpha");
    let (beta_id, beta_path) = new_plan("Beta");
    let (gamma_id, _) = new_plan("Gamma");
    map_build(&alpha_id);
    map_build(&beta_id);

    // Alpha's item is older and would win an unscoped pick; --plan must
    // lease Beta's item instead.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "--plan",
            &beta_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pick: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        pick["item"]["plan_path"].as_str().unwrap(),
        beta_path,
        "plan-scoped pick leased an item outside the plan: {pick}"
    );

    // Gamma has no mapped items: the null pick names the plan scope as the
    // reason even though other plans still have ready work.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "--plan",
            &gamma_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let null_pick: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(null_pick["item"], Value::Null);
    assert_eq!(null_pick["reason"], "ready_items_excluded_by_filter");
    assert!(
        null_pick["excluded"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["cause"].as_str().unwrap().contains("--plan")),
        "plan-scope exclusions must name the plan filter: {null_pick}"
    );
    assert!(
        null_pick["repair"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cmd| cmd == "planr pick --json"),
        "plan-scope exclusion must carry the unscoped repair command: {null_pick}"
    );

    // An unknown plan id is an error, not a silent unscoped pick.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "pick",
            "--plan",
            "pln-does-not-exist",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("plan not found"));
}

#[test]
fn close_target_requires_completion_log_and_complete_verdict() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Gate"])
        .assert()
        .success();
    let item = create_test_item(dir.path(), &db, "Unlogged item", "close target guard");
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "request",
            &item,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let review_id = serde_json::from_slice::<Value>(&output).unwrap()["review"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "review",
            "close",
            &review_id,
            "--verdict",
            "not-complete",
            "--findings",
            "missing tests",
            "--close-target",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--close-target requires"));

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "review",
            "close",
            &review_id,
            "--verdict",
            "complete",
            "--close-target",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no completion log"));
    assert_ne!(item_status(&db, &item), "closed");
}

#[test]
fn log_add_refreshes_heartbeat_for_the_owner() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "HB"])
        .assert()
        .success();
    let item = create_test_item(dir.path(), &db, "Heartbeat item", "heartbeat folding");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success();
    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE items SET last_heartbeat_at = datetime('now', '-1 hour') WHERE id = ?1",
        [&item],
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item,
            "--summary",
            "progress evidence",
        ])
        .assert()
        .success();
    let (status, fresh): (String, i64) = conn
        .query_row(
            "SELECT status, last_heartbeat_at >= datetime('now', '-60 seconds') FROM items WHERE id = ?1",
            [&item],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        status, "running",
        "owner log must promote picked to running"
    );
    assert_eq!(fresh, 1, "owner log must refresh the heartbeat");
}

#[test]
fn map_build_is_idempotent_per_plan() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Idem"])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "plan", "new", "App"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let plan_id = serde_json::from_slice::<Value>(&output).unwrap()["plan"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let build = |expect_created: usize| {
        let output = planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "map",
                "build",
                "--from",
                &plan_id,
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let value: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            value["created"].as_array().unwrap().len(),
            expect_created,
            "unexpected created count on map build"
        );
        value
    };
    let first = build(1);
    assert!(
        first["hint"].as_str().unwrap().contains("breakdown"),
        "single coarse item must carry a breakdown hint"
    );
    let second = build(0);
    assert!(
        second["hint"].as_str().unwrap().contains("already mapped"),
        "re-running map build must say the plan is already mapped"
    );

    let conn = Connection::open(&db).unwrap();
    let item_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
        .unwrap();
    assert_eq!(item_count, 1, "repeat map build must not duplicate items");
}

#[test]
fn heartbeat_and_ownership_survive_new_processes_without_session_id() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Workers"])
        .assert()
        .success();
    let item_id = create_test_item(dir.path(), &db, "Stable worker item", "worker id test");

    let output = planr()
        .current_dir(dir.path())
        .env_remove("PLANR_SESSION_ID")
        .env_remove("CODEX_SESSION_ID")
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let picked: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(picked["item"]["id"], item_id);

    // A different process without a session id must still own the pick:
    // agent sessions spawn a new process per tool call.
    let output = planr()
        .current_dir(dir.path())
        .env_remove("PLANR_SESSION_ID")
        .env_remove("CODEX_SESSION_ID")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "heartbeat",
            &item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let heartbeat: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(heartbeat["item"]["status"], "running");
}

#[test]
fn graph_adaptation_primitives_preview_rewire_and_replan() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Graph"])
        .assert()
        .success();

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "First",
            "--description",
            "first item",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let first: Value = serde_json::from_slice(&output).unwrap();
    let first_id = first["item"]["id"].as_str().unwrap();

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Second",
            "--description",
            "second item",
            "--after",
            first_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let second: Value = serde_json::from_slice(&output).unwrap();
    let second_id = second["item"]["id"].as_str().unwrap();

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "map",
            "preview",
            "--close",
            first_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(preview["mode"], "preview");
    assert_eq!(preview["would_unlock"][0]["id"], second_id);

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "insert",
            "Middle",
            "--description",
            "inserted between first and second",
            "--after",
            first_id,
            "--before",
            second_id,
            "--preview",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"mode\": \"preview\""));

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "insert",
            "Middle",
            "--description",
            "inserted between first and second",
            "--after",
            first_id,
            "--before",
            second_id,
            "--confirm",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let inserted: Value = serde_json::from_slice(&output).unwrap();
    let middle_id = inserted["item"]["id"].as_str().unwrap();

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let map: Value = serde_json::from_slice(&output).unwrap();
    let links = map["links"].as_array().unwrap();
    assert!(
        links
            .iter()
            .any(|link| link["from"] == first_id && link["to"] == middle_id)
    );
    assert!(
        links
            .iter()
            .any(|link| link["from"] == middle_id && link["to"] == second_id)
    );
    assert!(
        !links
            .iter()
            .any(|link| link["from"] == first_id && link["to"] == second_id)
    );

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "amend",
            middle_id,
            "--note",
            "Future work needs extra verification.",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("item amended"));

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "map", "unlocks", first_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("would unlock"));
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "map",
            "lookahead",
            "--from",
            first_id,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("lookahead"));
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "map", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("settled |"));

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Parent Gate",
            "--description",
            "parent",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parent: Value = serde_json::from_slice(&output).unwrap();
    let parent_id = parent["item"]["id"].as_str().unwrap();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "breakdown",
            parent_id,
            "--into",
            "Old A, Old B",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "replan",
            parent_id,
            "--into",
            "New A, New B",
            "--preview",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"mode\": \"preview\""));
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "replan",
            parent_id,
            "--into",
            "New A, New B",
            "--confirm",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let replan: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(replan["created"].as_array().unwrap().len(), 2);
    assert!(!replan["cancelled"].as_array().unwrap().is_empty());
    let cancelled_ids = replan["cancelled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let map_after_replan: Value = serde_json::from_slice(&output).unwrap();
    for link in map_after_replan["links"].as_array().unwrap() {
        assert!(
            !cancelled_ids
                .iter()
                .any(|id| link["from"] == id.as_str() || link["to"] == id.as_str()),
            "cancelled replan child should not keep graph links: {link}"
        );
    }
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "search", "New A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("result"));

    let mut mcp = planr();
    let mcp_input = format!(
        "{}\n{}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}",
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "planr_map_unlocks",
                "arguments": {"item_id": first_id}
            }
        })
    );
    mcp.current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(mcp_input)
        .assert()
        .success()
        .stdout(predicate::str::contains("planr_item_insert"))
        .stdout(predicate::str::contains("would_unlock"));

    let port = free_port();
    let bin = assert_cmd::cargo::cargo_bin("planr");
    let mut server = StdCommand::new(&bin)
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(150));
    let status = http_request(port, "GET", "/v1/projects/current/map/status", "");
    assert!(status.contains("\"ready\""), "{status}");
    let unlocks = http_request(port, "GET", &format!("/v1/items/{first_id}/unlocks"), "");
    assert!(unlocks.contains("would_unlock"), "{unlocks}");
    let insert_preview = http_request(
        port,
        "POST",
        &format!("/v1/items/{first_id}/insert"),
        "{\"title\":\"HTTP Insert\",\"description\":\"preview\"}",
    );
    assert!(
        insert_preview.contains("\"mode\":\"preview\""),
        "{insert_preview}"
    );
    let http_a = http_json(&http_request(
        port,
        "POST",
        "/v1/projects/current/items",
        "{\"title\":\"HTTP A\",\"description\":\"a\"}",
    ));
    let http_b = http_json(&http_request(
        port,
        "POST",
        "/v1/projects/current/items",
        "{\"title\":\"HTTP B\",\"description\":\"b\"}",
    ));
    let http_a_id = http_a["item"]["id"].as_str().unwrap();
    let http_b_id = http_b["item"]["id"].as_str().unwrap();
    let insert_confirm = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{http_a_id}/insert"),
        &format!(
            "{{\"title\":\"HTTP Middle\",\"description\":\"m\",\"before\":\"{http_b_id}\",\"confirm\":true}}"
        ),
    ));
    assert_eq!(insert_confirm["item"]["title"], "HTTP Middle");
    let map_body = http_json(&http_request(port, "GET", "/v1/projects/current/map", ""));
    let http_b_after = map_body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == http_b_id)
        .unwrap();
    assert_eq!(http_b_after["status"], "pending");
    server.kill().unwrap();
    server.wait().unwrap();
}

#[test]
fn local_http_api_smoke_uses_same_core_engine() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Http"])
        .assert()
        .success();

    let port = free_port();
    let bin = assert_cmd::cargo::cargo_bin("planr");
    let mut server = StdCommand::new(&bin)
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(150));

    let create = http_request(
        port,
        "POST",
        "/v1/projects/p/items",
        "{\"title\":\"HTTP Item\",\"description\":\"from http\"}",
    );
    assert!(create.contains("HTTP Item"), "{create}");
    let create_json = http_json(&create);
    let created_item_id = create_json["item"]["id"].as_str().unwrap();

    let unpicked_progress = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{created_item_id}/progress"),
        "{\"percent\":1}",
    ));
    assert_eq!(unpicked_progress["error"]["code"], "invalid_transition");

    let pick = http_json(&http_request(port, "POST", "/v1/pick", "{}"));
    assert!(pick["runtime"].is_object(), "{pick}");
    assert!(pick["remaining"]["counts"].is_object(), "{pick}");
    let item_id = pick["item"]["id"].as_str().unwrap();

    let heartbeat = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{item_id}/heartbeat"),
        "{}",
    ));
    assert_eq!(heartbeat["item"]["status"], "running");
    assert!(heartbeat["runtime"]["last_heartbeat_at"].is_string());

    let progress = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{item_id}/progress"),
        "{\"percent\":67,\"note\":\"http progress\"}",
    ));
    assert_eq!(progress["runtime"]["progress_percent"], 67);
    assert_eq!(progress["runtime"]["progress_note"], "http progress");

    let approval = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{item_id}/approval/request"),
        "{\"reason\":\"http gate\"}",
    ));
    assert_eq!(approval["approval"]["status"], "requested");
    let approvals = http_json(&http_request(port, "GET", "/v1/approvals?open=true", ""));
    assert_eq!(approvals["approvals"][0]["item"]["id"], item_id);
    let approved = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{item_id}/approval/approve"),
        "{\"by\":\"http-qa\",\"comment\":\"ok\"}",
    ));
    assert_eq!(approved["approval"]["status"], "approved");

    let search = http_request(port, "GET", "/v1/search?q=HTTP", "");
    assert!(search.contains("HTTP Item"), "{search}");

    server.kill().unwrap();
    server.wait().unwrap();
}

#[test]
fn artifacts_events_and_debug_bundle_are_persisted() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Events"])
        .assert()
        .success();
    let created = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Observed Item",
            "--description",
            "event coverage",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&created).unwrap();
    let item_id = created["item"]["id"].as_str().unwrap();

    let artifact = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "artifact",
            "add",
            "test evidence",
            "--item",
            item_id,
            "--kind",
            "verification",
            "--content",
            "small inline proof",
            "--mime",
            "text/plain",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let artifact: Value = serde_json::from_slice(&artifact).unwrap();
    let artifact_id = artifact["artifact"]["id"].as_str().unwrap();
    assert_eq!(artifact["artifact"]["item_id"], item_id);

    // Path artifacts without --mime infer the type from the extension; a
    // screenshot must never land as text/plain in the audit trail.
    std::fs::write(dir.path().join("proof.png"), b"png-bytes").unwrap();
    let screenshot = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "artifact",
            "add",
            "screenshot",
            "--item",
            item_id,
            "--path",
            "proof.png",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let screenshot: Value = serde_json::from_slice(&screenshot).unwrap();
    assert_eq!(
        screenshot["artifact"]["mime_type"], "image/png",
        "mime must be inferred from the path extension: {screenshot}"
    );

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "artifact",
            "show",
            artifact_id,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("small inline proof"));
    let artifacts = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "artifact",
            "list",
            "--item",
            item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let artifacts: Value = serde_json::from_slice(&artifacts).unwrap();
    assert_eq!(artifacts["artifacts"][0]["id"], artifact_id);

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            item_id,
            "--summary",
            "evented log",
            "--cmd",
            "cargo test",
        ])
        .assert()
        .success();
    let events = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "event",
            "list",
            "--item",
            item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let events: Value = serde_json::from_slice(&events).unwrap();
    let event_types = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["event_type"].as_str())
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"artifact_created"), "{event_types:?}");
    assert!(event_types.contains(&"log_created"), "{event_types:?}");
    assert!(event_types.contains(&"item_created"), "{event_types:?}");

    let bundle = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "debug",
            "bundle",
            "--item",
            item_id,
            "--preview",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let bundle: Value = serde_json::from_slice(&bundle).unwrap();
    assert_eq!(bundle["mode"], "preview");
    assert_eq!(bundle["privacy"]["source_file_content_included"], false);
    assert_eq!(bundle["counts"]["artifacts"], 2);

    let mut mcp = planr();
    mcp.current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n\
             {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"planr_event_list\",\"arguments\":{\"limit\":5}}}\n",
        )
        .assert()
        .success()
        .stdout(predicate::str::contains("planr_artifact_add"))
        .stdout(predicate::str::contains("planr_event_list"))
        .stdout(predicate::str::contains("events"));

    let port = free_port();
    let bin = assert_cmd::cargo::cargo_bin("planr");
    let mut server = StdCommand::new(&bin)
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(150));
    let http_artifact = http_json(&http_request(
        port,
        "POST",
        "/v1/artifacts",
        &format!(
            "{{\"name\":\"http artifact\",\"item\":\"{item_id}\",\"content\":\"http proof\"}}"
        ),
    ));
    assert_eq!(http_artifact["artifact"]["name"], "http artifact");
    let http_log = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{item_id}/log"),
        "{\"summary\":\"http event log\",\"commands\":[\"cargo test\"]}",
    ));
    assert_eq!(http_log["log"]["summary"], "http event log");
    let http_review = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{item_id}/reviews"),
        "{}",
    ));
    assert!(http_review["review"]["id"].is_string());
    let http_events = http_json(&http_request(port, "GET", "/v1/events", ""));
    let http_event_types = http_events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["event_type"].as_str())
        .collect::<Vec<_>>();
    assert!(http_event_types.contains(&"artifact_created"));
    assert!(http_event_types.contains(&"log_created"));
    assert!(http_event_types.contains(&"review_requested"));
    let sse = http_sse_read_until(port, "/v1/events/stream", "event: artifact_created");
    assert!(sse.contains("text/event-stream"), "{sse}");
    assert!(sse.contains("event: artifact_created"), "{sse}");
    let http_bundle = http_json(&http_request(port, "GET", "/v1/debug/bundle", ""));
    assert_eq!(
        http_bundle["privacy"]["prompt_or_response_content_included"],
        false
    );
    server.kill().unwrap();
    server.wait().unwrap();
}

#[test]
fn recovery_sweep_recovers_timed_out_and_retryable_work() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Recovery"])
        .assert()
        .success();

    let timed_out = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Timed out work",
            "--description",
            "picked work that should be released",
            "--timeout-seconds",
            "1",
            "--pre",
            "Confirm environment is bootstrapped",
            "--post",
            "Attach recovery evidence",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let timed_out: Value = serde_json::from_slice(&timed_out).unwrap();
    let timed_out_id = timed_out["item"]["id"].as_str().unwrap();

    let pick = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pick: Value = serde_json::from_slice(&pick).unwrap();
    assert_eq!(pick["item"]["id"], timed_out_id);
    assert_eq!(
        pick["conditions"]["pre"],
        "Confirm environment is bootstrapped"
    );
    assert_eq!(pick["recovery"]["timeout_seconds"], 1);

    let retryable = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Retryable failed work",
            "--description",
            "failed work that should be retried",
            "--max-retries",
            "2",
            "--retry-delay-ms",
            "0",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let retryable: Value = serde_json::from_slice(&retryable).unwrap();
    let retryable_id = retryable["item"]["id"].as_str().unwrap();

    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE items SET picked_at = datetime('now','-5 seconds'), last_heartbeat_at = datetime('now','-5 seconds'), updated_at = datetime('now','-5 seconds') WHERE id = ?1",
        [timed_out_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE items SET status = 'failed', retry_count = 0, updated_at = datetime('now','-5 seconds') WHERE id = ?1",
        [retryable_id],
    )
    .unwrap();

    let preview = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "recover",
            "sweep",
            "--older-than-seconds",
            "2",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&preview).unwrap();
    assert_eq!(preview["mode"], "preview");
    assert_eq!(preview["released"], 0);
    assert_eq!(preview["retried"], 0);
    assert_eq!(preview["timed_out"][0]["item"]["id"], timed_out_id);
    assert_eq!(preview["retryable"][0]["item"]["id"], retryable_id);

    let applied = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "recover",
            "sweep",
            "--older-than-seconds",
            "2",
            "--apply",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let applied: Value = serde_json::from_slice(&applied).unwrap();
    assert_eq!(applied["mode"], "apply");
    assert_eq!(applied["released"], 1);
    assert_eq!(applied["retried"], 1);

    let close_preview = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "map",
            "preview",
            "--close",
            timed_out_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let close_preview: Value = serde_json::from_slice(&close_preview).unwrap();
    assert_eq!(
        close_preview["conditions"]["post"],
        "Attach recovery evidence"
    );
    assert_eq!(close_preview["post_condition_unverified"], true);

    let retry_count: i64 = conn
        .query_row(
            "SELECT retry_count FROM items WHERE id = ?1",
            [retryable_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retry_count, 1);
}

#[test]
fn scrub_confirm_redacts_stored_secret_values() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Scrub"])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "context",
            "add",
            "deploy with key sk-test123SECRET against staging",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "context",
            "add",
            "ordinary risk-free note that must survive untouched",
        ])
        .assert()
        .success();

    let preview = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "scrub"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&preview).unwrap();
    assert_eq!(preview["mode"], "preview");
    assert_eq!(preview["findings"].as_array().unwrap().len(), 1);

    let confirmed = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "scrub", "--confirm"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let confirmed: Value = serde_json::from_slice(&confirmed).unwrap();
    assert_eq!(confirmed["mode"], "confirm");
    assert_eq!(confirmed["scrubbed"], 1);

    let conn = Connection::open(&db).unwrap();
    let contents: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT content FROM contexts ORDER BY created_at")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert!(
        contents.iter().any(|c| c.contains("[REDACTED]")),
        "{contents:?}"
    );
    assert!(
        !contents.iter().any(|c| c.contains("sk-test123SECRET")),
        "secret survived scrub: {contents:?}"
    );
    assert!(
        contents.iter().any(|c| c.contains("risk-free")),
        "false positive nuked normal content: {contents:?}"
    );

    // The search index must not return the secret anymore.
    let indexed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM search_index WHERE body LIKE '%sk-test123SECRET%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(indexed, 0);

    // Scrub writes are event-backed.
    let scrub_events: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE event_type = 'secret_scrubbed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(scrub_events, 1);
}

#[test]
fn recovery_timeout_marks_failed_then_retries_until_exhausted() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Lifecycle"])
        .assert()
        .success();
    let item = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Flaky work",
            "--description",
            "times out, retries once, then exhausts",
            "--timeout-seconds",
            "1",
            "--max-retries",
            "1",
            "--retry-delay-ms",
            "0",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let item: Value = serde_json::from_slice(&item).unwrap();
    let item_id = item["item"]["id"].as_str().unwrap().to_string();

    let conn = Connection::open(&db).unwrap();
    let sweep = |apply: bool| -> Value {
        let mut args = vec![
            "--db".to_string(),
            db.to_str().unwrap().to_string(),
            "--json".to_string(),
            "recover".to_string(),
            "sweep".to_string(),
            "--older-than-seconds".to_string(),
            "900".to_string(),
        ];
        if apply {
            args.push("--apply".to_string());
        }
        let output = planr()
            .current_dir(dir.path())
            .args(&args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice(&output).unwrap()
    };
    let pick_and_backdate = |conn: &Connection| {
        planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "--json", "pick"])
            .assert()
            .success();
        conn.execute(
            "UPDATE items SET picked_at = datetime('now','-5 seconds'), last_heartbeat_at = datetime('now','-5 seconds'), updated_at = datetime('now','-5 seconds') WHERE id = ?1",
            [item_id.as_str()],
        )
        .unwrap();
    };
    let status = |conn: &Connection| -> String {
        conn.query_row(
            "SELECT status FROM items WHERE id = ?1",
            [item_id.as_str()],
            |row| row.get(0),
        )
        .unwrap()
    };

    // Round 1: timeout marks the item failed (retry budget exists).
    pick_and_backdate(&conn);
    let applied = sweep(true);
    assert_eq!(applied["failed"], 1, "{applied}");
    assert_eq!(status(&conn), "failed");

    // Round 2: the failed item is retryable and goes back to ready.
    let retry = sweep(true);
    assert_eq!(retry["retried"], 1, "{retry}");
    assert_eq!(status(&conn), "ready");

    // Round 3: second timeout exhausts the budget; item stays failed.
    pick_and_backdate(&conn);
    let second_fail = sweep(true);
    assert_eq!(second_fail["failed"], 1, "{second_fail}");
    let exhausted = sweep(true);
    assert_eq!(exhausted["retried"], 0, "{exhausted}");
    assert_eq!(exhausted["exhausted"][0]["item"]["id"], item_id);
    assert_eq!(status(&conn), "failed");

    // The full lifecycle is event-backed.
    let events: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT event_type FROM events WHERE item_id = ?1 ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([item_id.as_str()], |row| row.get::<_, String>(0))
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    assert!(events.contains(&"item_timed_out".to_string()), "{events:?}");
    assert!(
        events.contains(&"recovery_retry_scheduled".to_string()),
        "{events:?}"
    );
}

#[test]
fn recovery_sweep_is_available_through_mcp_and_http() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Recovery API",
        ])
        .assert()
        .success();

    let mcp_input = [
        json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"tools/call",
            "params":{
                "name":"planr_item_create",
                "arguments":{
                    "title":"MCP timed out work",
                    "description":"created through mcp",
                    "timeout_seconds":1
                }
            }
        })
        .to_string(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"planr_pick_item","arguments":{}}}).to_string(),
    ]
    .join("\n")
        + "\n";
    let mcp_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(mcp_input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let responses = String::from_utf8(mcp_output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let pick_text = responses[1]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let pick_value: Value = serde_json::from_str(pick_text).unwrap();
    let item_id = pick_value["item"]["id"].as_str().unwrap();

    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE items SET picked_at = datetime('now','-5 seconds'), last_heartbeat_at = datetime('now','-5 seconds'), updated_at = datetime('now','-5 seconds') WHERE id = ?1",
        [item_id],
    )
    .unwrap();
    let mcp_recovery = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"planr_recover_sweep","arguments":{"older_than_seconds":2}}})
                .to_string()
                + "\n",
        )
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_response: Value =
        serde_json::from_str(String::from_utf8(mcp_recovery).unwrap().trim()).unwrap();
    let mcp_text = mcp_response["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let mcp_value: Value = serde_json::from_str(mcp_text).unwrap();
    assert_eq!(mcp_value["mode"], "preview");
    assert_eq!(mcp_value["timed_out"][0]["item"]["id"], item_id);

    let bin = assert_cmd::cargo::cargo_bin("planr");
    let port = free_port();
    let mut server = StdCommand::new(&bin)
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(150));
    let http_recovery = http_json(&http_request(
        port,
        "POST",
        "/v1/recover/sweep",
        "{\"older_than_seconds\":2,\"apply\":true}",
    ));
    assert_eq!(http_recovery["mode"], "apply");
    assert_eq!(http_recovery["released"], 1);
    server.kill().unwrap();
    server.wait().unwrap();
}

#[test]
fn local_review_workspace_serves_browser_ui_and_drives_review_chain() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Review Workspace",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "plan", "new", "Workspace app"])
        .assert()
        .success();
    let item = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Workspace target",
            "--description",
            "target for local browser review",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let item: Value = serde_json::from_slice(&item).unwrap();
    let item_id = item["item"]["id"].as_str().unwrap();

    let bin = assert_cmd::cargo::cargo_bin("planr");
    let port = free_port();
    let mut server = StdCommand::new(&bin)
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(150));

    let review = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{item_id}/reviews"),
        "{}",
    ));
    let review_id = review["review"]["id"].as_str().unwrap();

    let html = http_request(port, "GET", "/review", "");
    assert!(html.contains("content-type: text/html"), "{html}");
    assert!(html.contains("Planr Review Workspace"), "{html}");
    assert!(html.contains("Add Annotation"), "{html}");
    assert!(html.contains("Request Changes"), "{html}");

    let workspace = http_json(&http_request(port, "GET", "/v1/review-workspace", ""));
    assert_eq!(workspace["project"]["name"], "Review Workspace");
    assert_eq!(workspace["reviews"][0]["review"]["id"], review_id);
    assert_eq!(workspace["reviews"][0]["target"]["id"], item_id);
    assert!(!workspace["plans"].as_array().unwrap().is_empty());
    assert_eq!(workspace["diff"]["source_content_included"], false);

    let annotation = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{item_id}/review-annotations"),
        "{\"message\":\"Workspace annotation\",\"severity\":\"blocking\",\"file\":\"src/lib.rs\",\"line\":12}",
    ));
    assert_eq!(annotation["annotation"]["severity"], "blocking");
    let annotated_workspace = http_json(&http_request(port, "GET", "/v1/review-workspace", ""));
    let annotations = annotated_workspace["reviews"][0]["annotations"]
        .as_array()
        .unwrap();
    assert!(annotations.iter().any(|entry| {
        entry["content"]
            .as_str()
            .unwrap()
            .contains("Workspace annotation")
    }));

    let feedback = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/items/{item_id}/review-feedback"),
        "{\"reviewer\":\"workspace\",\"verdict\":\"not-complete\",\"findings\":[\"Workspace finding\"]}",
    ));
    assert_eq!(feedback["auto_closed"], false);

    let artifact = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/reviews/{review_id}/artifact"),
        "{}",
    ));
    assert_eq!(artifact["artifact"]["kind"], "review");

    let close = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/reviews/{review_id}/close"),
        "{\"verdict\":\"not-complete\",\"findings\":[\"Workspace finding\"]}",
    ));
    assert_eq!(close["verdict"], "not-complete");
    assert_eq!(close["created"].as_array().unwrap().len(), 2);

    server.kill().unwrap();
    server.wait().unwrap();
}

#[test]
fn review_evidence_scopes_git_dirty_files_and_pr_context() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/owned.rs"), "fn owned() {}\n").unwrap();
    fs::write(dir.path().join("src/unrelated.rs"), "fn unrelated() {}\n").unwrap();
    StdCommand::new("git")
        .current_dir(dir.path())
        .args(["init"])
        .status()
        .unwrap();
    StdCommand::new("git")
        .current_dir(dir.path())
        .args(["config", "user.email", "planr@example.test"])
        .status()
        .unwrap();
    StdCommand::new("git")
        .current_dir(dir.path())
        .args(["config", "user.name", "Planr Test"])
        .status()
        .unwrap();
    StdCommand::new("git")
        .current_dir(dir.path())
        .args(["add", "."])
        .status()
        .unwrap();
    StdCommand::new("git")
        .current_dir(dir.path())
        .args(["commit", "-m", "baseline"])
        .status()
        .unwrap();

    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Git Evidence",
        ])
        .assert()
        .success();
    let item = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Git scoped item",
            "--description",
            "review git evidence",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let item: Value = serde_json::from_slice(&item).unwrap();
    let item_id = item["item"]["id"].as_str().unwrap();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            item_id,
            "--summary",
            "changed owned file",
            "--files",
            "src/owned.rs",
        ])
        .assert()
        .success();
    StdCommand::new("git")
        .current_dir(dir.path())
        .args(["add", ".planr"])
        .status()
        .unwrap();
    StdCommand::new("git")
        .current_dir(dir.path())
        .args(["commit", "-m", "planr baseline"])
        .status()
        .unwrap();

    let clean = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "evidence",
            item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let clean: Value = serde_json::from_slice(&clean).unwrap();
    assert_eq!(clean["evidence"]["git"]["available"], true);
    assert!(
        clean["evidence"]["git"]["changed_files"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    fs::write(
        dir.path().join("src/owned.rs"),
        "fn owned() { assert!(true); }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/unrelated.rs"),
        "fn unrelated() { assert!(true); }\n",
    )
    .unwrap();

    let dirty = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "evidence",
            item_id,
            "--pr-url",
            "https://github.com/instructa/planr/pull/1",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let dirty: Value = serde_json::from_slice(&dirty).unwrap();
    assert!(
        dirty["evidence"]["git"]["scoped_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "src/owned.rs")
    );
    assert!(
        dirty["evidence"]["git"]["unrelated_dirty_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "src/unrelated.rs")
    );
    assert_eq!(
        dirty["evidence"]["dirty_worktree_safety"]["source_content_included"],
        false
    );
    assert!(
        dirty["evidence"]["provenance"]["pr_urls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "https://github.com/instructa/planr/pull/1")
    );

    let review = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "request",
            item_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let review: Value = serde_json::from_slice(&review).unwrap();
    let review_id = review["review"]["id"].as_str().unwrap();
    let close = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "review",
            "close",
            review_id,
            "--verdict",
            "not-complete",
            "--findings",
            "Tie finding to src/owned.rs:1",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let close: Value = serde_json::from_slice(&close).unwrap();
    let artifact_path = close["artifact"]["path"].as_str().unwrap();
    let artifact = fs::read_to_string(artifact_path).unwrap();
    assert!(artifact.contains("Git And PR Evidence"));
    assert!(artifact.contains("src/owned.rs"));
    assert!(artifact.contains("src/unrelated.rs"));
    assert!(artifact.contains("https://github.com/instructa/planr/pull/1"));
}

#[test]
fn template_export_import_preserves_graph_context_and_review_artifacts() {
    let source = tempdir().unwrap();
    let source_db = source.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(source.path())
        .args([
            "--db",
            source_db.to_str().unwrap(),
            "project",
            "init",
            "Template Source",
        ])
        .assert()
        .success();
    planr()
        .current_dir(source.path())
        .args([
            "--db",
            source_db.to_str().unwrap(),
            "plan",
            "new",
            "Reusable App",
        ])
        .assert()
        .success();
    let first = planr()
        .current_dir(source.path())
        .args([
            "--db",
            source_db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Template first",
            "--description",
            "first imported item",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let first: Value = serde_json::from_slice(&first).unwrap();
    let first_id = first["item"]["id"].as_str().unwrap();
    let second = planr()
        .current_dir(source.path())
        .args([
            "--db",
            source_db.to_str().unwrap(),
            "--json",
            "item",
            "create",
            "Template second",
            "--description",
            "second imported item",
            "--after",
            first_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let second: Value = serde_json::from_slice(&second).unwrap();
    let second_id = second["item"]["id"].as_str().unwrap();
    planr()
        .current_dir(source.path())
        .args([
            "--db",
            source_db.to_str().unwrap(),
            "context",
            "add",
            "Template review annotation context",
            "--item",
            second_id,
            "--tag",
            "review_annotation",
        ])
        .assert()
        .success();
    let review = planr()
        .current_dir(source.path())
        .args([
            "--db",
            source_db.to_str().unwrap(),
            "--json",
            "review",
            "request",
            second_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let review: Value = serde_json::from_slice(&review).unwrap();
    let review_id = review["review"]["id"].as_str().unwrap();
    let close = planr()
        .current_dir(source.path())
        .args([
            "--db",
            source_db.to_str().unwrap(),
            "--json",
            "review",
            "close",
            review_id,
            "--verdict",
            "complete",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let close: Value = serde_json::from_slice(&close).unwrap();
    assert_eq!(close["artifact"]["kind"], "review");

    let package = source.path().join("template.planr.json");
    planr()
        .current_dir(source.path())
        .args([
            "--db",
            source_db.to_str().unwrap(),
            "export",
            "--include-plans",
            "--include-logs",
            "--template-name",
            "Reusable template",
            "--tag",
            "v1.1",
            "--out",
            package.to_str().unwrap(),
        ])
        .assert()
        .success();
    let package_json: Value = serde_json::from_slice(&fs::read(&package).unwrap()).unwrap();
    assert_eq!(package_json["planr_template"]["schema_version"], 1);
    assert_eq!(
        package_json["planr_template"]["requirements"]["requires_confirmed_import"],
        true
    );
    assert_eq!(
        package_json["planr_template"]["encrypted_bundle_strategy"]["hosted_share_required"],
        false
    );

    let target = tempdir().unwrap();
    let target_db = target.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(target.path())
        .args([
            "--db",
            target_db.to_str().unwrap(),
            "project",
            "init",
            "Template Target",
        ])
        .assert()
        .success();
    let preview = planr()
        .current_dir(target.path())
        .args([
            "--db",
            target_db.to_str().unwrap(),
            "--json",
            "import",
            package.to_str().unwrap(),
            "--preview",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&preview).unwrap();
    assert_eq!(preview["mode"], "preview");
    assert!(preview["report"]["would_create"]["items"].as_u64().unwrap() >= 2);
    assert_eq!(preview["report"]["requires_confirm"], true);

    planr()
        .current_dir(target.path())
        .args([
            "--db",
            target_db.to_str().unwrap(),
            "import",
            package.to_str().unwrap(),
            "--confirm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("import applied"));
    let pick = planr()
        .current_dir(target.path())
        .args(["--db", target_db.to_str().unwrap(), "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pick: Value = serde_json::from_slice(&pick).unwrap();
    assert_eq!(pick["item"]["id"], first_id);
    let contexts = planr()
        .current_dir(target.path())
        .args([
            "--db",
            target_db.to_str().unwrap(),
            "--json",
            "context",
            "list",
            "--item",
            second_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let contexts: Value = serde_json::from_slice(&contexts).unwrap();
    assert!(
        contexts["contexts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|context| context["content"]
                .as_str()
                .unwrap()
                .contains("Template review annotation context"))
    );
    assert!(target.path().join(".planr/reviews").exists());
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn http_request(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn http_sse_read_until(port: u16, path: &str, needle: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\n\r\n"
    )
    .unwrap();
    let mut collected = String::new();
    let mut buf = [0u8; 4096];
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                collected.push_str(&String::from_utf8_lossy(&buf[..n]));
                if collected.contains(needle) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    collected
}

fn http_json(response: &str) -> Value {
    let body = response
        .split("\r\n\r\n")
        .nth(1)
        .expect("HTTP response body");
    serde_json::from_str(body).expect(body)
}

#[test]
fn planr_native_skills_are_packaged_and_cli_first() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let skills = [
        "planr",
        "planr-goal",
        "planr-loop",
        "planr-verify-web",
        "planr-task-graph",
        "planr-plan",
        "planr-work",
        "planr-review",
        "planr-status",
        "planr-summary",
    ];
    for skill in skills {
        let path = root
            .join("plugins/planr/skills")
            .join(skill)
            .join("SKILL.md");
        assert!(path.exists(), "missing {skill}");
        let body = fs::read_to_string(path).unwrap();
        assert!(
            body.contains("planr "),
            "{skill} should teach Planr CLI usage"
        );
        assert!(
            !body.contains(&["./", ".planr", "/tooling/", "planr"].concat()),
            "{skill} should use the public Planr CLI"
        );
    }

    // The plugin payload lives in plugins/planr; marketplace manifests at the
    // repo root must point there because Codex ignores root-as-plugin sources.
    for manifest in [
        "plugins/planr/.codex-plugin/plugin.json",
        "plugins/planr/.claude-plugin/plugin.json",
        ".cursor-plugin/plugin.json",
        ".claude-plugin/marketplace.json",
        ".agents/plugins/marketplace.json",
    ] {
        let path = root.join(manifest);
        assert!(path.exists(), "missing plugin file {manifest}");
        let value: Value = serde_json::from_str(&fs::read_to_string(path).unwrap())
            .unwrap_or_else(|e| panic!("{manifest} is not valid JSON: {e}"));
        assert_eq!(value["name"], "planr", "{manifest} must be named planr");
    }
    let codex_marketplace =
        fs::read_to_string(root.join(".agents/plugins/marketplace.json")).unwrap();
    assert!(
        codex_marketplace.contains("./plugins/planr"),
        "Codex marketplace must point at the plugins/planr subdirectory"
    );
    let claude_marketplace =
        fs::read_to_string(root.join(".claude-plugin/marketplace.json")).unwrap();
    assert!(
        claude_marketplace.contains("./plugins/planr"),
        "Claude marketplace must point at the plugins/planr subdirectory"
    );
    let codex_manifest: Value = serde_json::from_slice(
        &fs::read(root.join("plugins/planr/.codex-plugin/plugin.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(codex_manifest["skills"], "./skills/");
    assert!(
        codex_manifest.get("agents").is_none(),
        "Codex plugin must provide skills without claiming project agents"
    );
    let cursor_manifest: Value =
        serde_json::from_slice(&fs::read(root.join(".cursor-plugin/plugin.json")).unwrap())
            .unwrap();
    assert_eq!(cursor_manifest["skills"], "./plugins/planr/skills/");
    assert_eq!(cursor_manifest["agents"].as_array().unwrap().len(), 2);
    for skill in skills {
        assert!(
            root.join("plugins/planr/skills")
                .join(skill)
                .join("SKILL.md")
                .exists(),
            "Claude plugin convention must discover skill {skill}"
        );
    }
    for agent in ["planr-worker", "planr-reviewer"] {
        assert!(
            root.join("plugins/planr/agents")
                .join(format!("{agent}.md"))
                .exists(),
            "missing plugin agent {agent}"
        );
        // Cursor-format subagent roles ship as an independent contract and
        // are registered by the root .cursor-plugin manifest.
        assert!(
            root.join("plugins/planr/skills/planr-loop/agents")
                .join(format!("{agent}.md"))
                .exists(),
            "missing cursor plugin agent {agent}"
        );
    }
    let cursor_manifest = fs::read_to_string(root.join(".cursor-plugin/plugin.json")).unwrap();
    assert!(
        cursor_manifest.contains("planr-loop/agents/planr-worker.md"),
        "Cursor plugin manifest must register the worker subagent"
    );
    for removed in [
        "plugins/planr/skills/planr-loop/agents/planr-worker.toml",
        "plugins/planr/skills/planr-loop/agents/planr-reviewer.toml",
    ] {
        assert!(
            !root.join(removed).exists(),
            "retired Codex fallback remains: {removed}"
        );
    }
    let loop_skill =
        fs::read_to_string(root.join("plugins/planr/skills/planr-loop/SKILL.md")).unwrap();
    for reference in [
        "references/host-dispatch.md",
        "references/recovery-and-verification.md",
    ] {
        assert!(
            loop_skill.contains(&format!("]({reference})")),
            "planr-loop must link its progressive-disclosure reference {reference}"
        );
        assert!(
            root.join("plugins/planr/skills/planr-loop")
                .join(reference)
                .is_file(),
            "plugin skill reference must resolve: {reference}"
        );
    }
    assert!(
        loop_skill.contains("Pick packets expose provider-neutral `routing.profile`; they do not expose a host-owned `routing.agent_type`"),
        "planr-loop must document that pick packets expose routing.profile, not routing.agent_type"
    );
    assert!(
        loop_skill
            .contains("dispatch that profile identifier as the host-native role/`agent_type`"),
        "planr-loop must use matching external profile identifiers as native agent_type"
    );
    assert!(
        loop_skill.contains(
            "If no matching repository role exists, keep the host's default dispatch contract"
        ),
        "planr-loop must preserve default host dispatch when no matching role exists"
    );
    assert!(
        loop_skill.contains("Model, effort, profile, client, and fallback fields are advisory declarations and evidence labels only"),
        "planr-loop must keep model/profile/fallback fields advisory"
    );
    assert!(
        loop_skill.contains("attach route observations when available"),
        "planr-loop must preserve effective route evidence recording"
    );
    for removed in [
        "dispatch through the routing skill",
        "resolved native `agent_type`",
        "named profile's client and model",
        "move down the `fallbacks` chain",
    ] {
        assert!(
            !loop_skill.contains(removed),
            "planr-loop reintroduced Planr-owned routing dispatch language: {removed}"
        );
    }
}

#[test]
fn project_init_and_install_provision_loop_agent_roles() {
    fn assert_cursor_loop_references_resolve(root: &std::path::Path) {
        let skill_path = root.join(".cursor/skills/planr-loop/SKILL.md");
        let skill = fs::read_to_string(&skill_path).unwrap();
        for reference in [
            "references/host-dispatch.md",
            "references/recovery-and-verification.md",
        ] {
            assert!(
                skill.contains(&format!("]({reference})")),
                "installed planr-loop must link {reference}"
            );
            assert!(
                skill_path.parent().unwrap().join(reference).is_file(),
                "installed planr-loop reference must resolve: {reference}"
            );
        }
    }

    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Roles",
            "--client",
            "all",
        ])
        .assert()
        .success();
    for role in [
        ".claude/agents/planr-worker.md",
        ".claude/agents/planr-reviewer.md",
        ".cursor/agents/planr-worker.md",
        ".cursor/agents/planr-reviewer.md",
    ] {
        assert!(
            dir.path().join(role).exists(),
            "project init --client all should provision {role}"
        );
    }
    assert!(!dir.path().join(".codex/agents/planr-worker.toml").exists());
    assert!(
        !dir.path()
            .join(".codex/agents/planr-reviewer.toml")
            .exists()
    );
    // Cursor owns project roles and skills even when MCP is omitted.
    let no_mcp = tempdir().unwrap();
    let no_mcp_db = no_mcp.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(no_mcp.path())
        .args([
            "--db",
            no_mcp_db.to_str().unwrap(),
            "project",
            "init",
            "NoMcp",
        ])
        .assert()
        .success();
    planr()
        .current_dir(no_mcp.path())
        .args([
            "--db",
            no_mcp_db.to_str().unwrap(),
            "install",
            "cursor",
            "--no-mcp",
        ])
        .assert()
        .success();
    for provisioned in [
        ".cursor/agents/planr-worker.md",
        ".cursor/agents/planr-reviewer.md",
        ".cursor/skills/planr/SKILL.md",
        ".cursor/skills/planr-loop/references/host-dispatch.md",
        ".cursor/skills/planr-loop/references/recovery-and-verification.md",
        ".cursor/skills/planr-work/SKILL.md",
    ] {
        assert!(
            no_mcp.path().join(provisioned).exists(),
            "install cursor --no-mcp should write {provisioned}"
        );
    }
    assert_cursor_loop_references_resolve(no_mcp.path());
    let installed_reference = no_mcp
        .path()
        .join(".cursor/skills/planr-loop/references/host-dispatch.md");
    fs::write(&installed_reference, "local customization\n").unwrap();
    planr()
        .current_dir(no_mcp.path())
        .args([
            "--db",
            no_mcp_db.to_str().unwrap(),
            "install",
            "cursor",
            "--no-mcp",
        ])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&installed_reference).unwrap(),
        "local customization\n",
        "ordinary reconciliation must preserve an existing reference"
    );
    planr()
        .current_dir(no_mcp.path())
        .args([
            "--db",
            no_mcp_db.to_str().unwrap(),
            "install",
            "cursor",
            "--no-mcp",
            "--force",
        ])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&installed_reference).unwrap(),
        include_str!("../plugins/planr/skills/planr-loop/references/host-dispatch.md"),
        "forced reconciliation must restore the packaged reference"
    );
    assert!(
        !no_mcp.path().join(".cursor/mcp.json").exists(),
        "install cursor --no-mcp must not write MCP config"
    );
    planr()
        .current_dir(no_mcp.path())
        .args([
            "--db",
            no_mcp_db.to_str().unwrap(),
            "install",
            "claude",
            "--no-mcp",
        ])
        .assert()
        .success();
    assert!(
        no_mcp
            .path()
            .join(".claude/agents/planr-worker.md")
            .exists(),
        "install claude --no-mcp should write subagent roles"
    );
    assert!(
        !no_mcp.path().join(".mcp.json").exists(),
        "install claude --no-mcp must not write MCP config"
    );
    let dry = planr()
        .current_dir(no_mcp.path())
        .args([
            "--db",
            no_mcp_db.to_str().unwrap(),
            "install",
            "cursor",
            "--no-mcp",
            "--dry-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let dry = String::from_utf8(dry).unwrap();
    assert!(
        dry.contains(".cursor/skills/planr/SKILL.md") && !dry.contains("mcpServers"),
        "--no-mcp dry-run should list plugin files, not MCP config"
    );

    // `planr install codex` is MCP-only; optional model-specific role files
    // remain exclusively owned by external routing tools.
    let dir2 = tempdir().unwrap();
    let db2 = dir2.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir2.path())
        .args(["--db", db2.to_str().unwrap(), "project", "init", "Roles2"])
        .assert()
        .success();
    assert!(!dir2.path().join(".codex/agents/planr-worker.toml").exists());
    planr()
        .current_dir(dir2.path())
        .args(["--db", db2.to_str().unwrap(), "install", "codex"])
        .assert()
        .success();
    assert!(!dir2.path().join(".codex/agents/planr-worker.toml").exists());
    assert!(
        dir2.path()
            .join(".planr/integrations/codex-mcp.toml")
            .exists()
    );
}

#[test]
fn client_install_responsibilities_are_exact() {
    let full_contracts: [(&str, &[&str], &[&str]); 3] = [
        (
            "codex",
            &[".planr/integrations/codex-mcp.toml", ".codex/hooks.json"],
            &[
                ".codex/agents/planr-worker.toml",
                ".codex/skills/planr/SKILL.md",
            ],
        ),
        (
            "claude",
            &[
                ".mcp.json",
                ".claude/agents/planr-worker.md",
                ".claude/agents/planr-reviewer.md",
                ".claude/settings.json",
            ],
            &[".claude/skills/planr/SKILL.md"],
        ),
        (
            "cursor",
            &[
                ".cursor/mcp.json",
                ".cursor/agents/planr-worker.md",
                ".cursor/agents/planr-reviewer.md",
                ".cursor/skills/planr/SKILL.md",
                ".cursor/skills/planr-goal/SKILL.md",
                ".cursor/skills/planr-loop/SKILL.md",
                ".cursor/skills/planr-loop/references/host-dispatch.md",
                ".cursor/skills/planr-loop/references/recovery-and-verification.md",
                ".cursor/skills/planr-verify-web/SKILL.md",
                ".cursor/skills/planr-task-graph/SKILL.md",
                ".cursor/skills/planr-plan/SKILL.md",
                ".cursor/skills/planr-work/SKILL.md",
                ".cursor/skills/planr-review/SKILL.md",
                ".cursor/skills/planr-status/SKILL.md",
                ".cursor/skills/planr-summary/SKILL.md",
                ".cursor/hooks.json",
                ".cursor/hooks/planr-evidence-guard.sh",
            ],
            &[],
        ),
    ];
    for (client, present, absent) in full_contracts {
        let dir = tempdir().unwrap();
        let db = dir.path().join(".planr/planr.sqlite");
        let db_arg = db.to_str().unwrap();
        planr()
            .current_dir(dir.path())
            .args(["--db", db_arg, "project", "init", "Install Contract"])
            .assert()
            .success();
        let dry_run = planr()
            .current_dir(dir.path())
            .args(["--db", db_arg, "install", client, "--dry-run"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let dry_run = String::from_utf8(dry_run).unwrap();
        for path in present {
            assert!(
                dry_run.contains(path),
                "planr install {client} --dry-run must preview {path}: {dry_run}"
            );
            assert!(
                !dir.path().join(path).exists(),
                "planr install {client} --dry-run must not write {path}"
            );
        }
        for path in absent {
            assert!(
                !dry_run.contains(path),
                "planr install {client} --dry-run must not claim {path}: {dry_run}"
            );
        }
        planr()
            .current_dir(dir.path())
            .args(["--db", db_arg, "install", client])
            .assert()
            .success();
        for path in present {
            assert!(
                dir.path().join(path).exists(),
                "planr install {client} must provision {path}"
            );
        }
        for path in absent {
            assert!(
                !dir.path().join(path).exists(),
                "planr install {client} must not provision {path}"
            );
        }
    }

    let no_mcp_contracts: [(&str, &[&str], &[&str], &str); 3] = [
        (
            "codex",
            &[".codex/hooks.json"],
            &[
                ".planr/integrations/codex-mcp.toml",
                ".codex/agents/planr-worker.toml",
                ".codex/skills/planr/SKILL.md",
            ],
            "no project MCP, roles, or skills",
        ),
        (
            "claude",
            &[
                ".claude/agents/planr-worker.md",
                ".claude/agents/planr-reviewer.md",
                ".claude/settings.json",
            ],
            &[".mcp.json", ".claude/skills/planr/SKILL.md"],
            "project subagent roles; no project MCP or skills",
        ),
        (
            "cursor",
            &[
                ".cursor/agents/planr-worker.md",
                ".cursor/agents/planr-reviewer.md",
                ".cursor/skills/planr/SKILL.md",
                ".cursor/skills/planr-loop/references/host-dispatch.md",
                ".cursor/skills/planr-loop/references/recovery-and-verification.md",
                ".cursor/hooks.json",
                ".cursor/hooks/planr-evidence-guard.sh",
            ],
            &[".cursor/mcp.json"],
            "project subagent roles and skills; no project MCP",
        ),
    ];
    for (client, present, absent, dry_run_contract) in no_mcp_contracts {
        let dir = tempdir().unwrap();
        let db = dir.path().join(".planr/planr.sqlite");
        let db_arg = db.to_str().unwrap();
        planr()
            .current_dir(dir.path())
            .args(["--db", db_arg, "project", "init", "No MCP Contract"])
            .assert()
            .success();
        let dry_run = planr()
            .current_dir(dir.path())
            .args(["--db", db_arg, "install", client, "--no-mcp", "--dry-run"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let dry_run = String::from_utf8(dry_run).unwrap();
        assert!(dry_run.contains(dry_run_contract), "{dry_run}");
        for path in present {
            assert!(
                dry_run.contains(path),
                "planr install {client} --no-mcp --dry-run must preview {path}: {dry_run}"
            );
        }
        for path in absent {
            assert!(
                !dry_run.contains(path),
                "planr install {client} --no-mcp --dry-run must not claim {path}: {dry_run}"
            );
        }
        planr()
            .current_dir(dir.path())
            .args(["--db", db_arg, "install", client, "--no-mcp"])
            .assert()
            .success();
        for path in present {
            assert!(
                dir.path().join(path).exists(),
                "planr install {client} --no-mcp must provision {path}"
            );
        }
        for path in absent {
            assert!(
                !dir.path().join(path).exists(),
                "planr install {client} --no-mcp must not provision {path}"
            );
        }
    }

    planr()
        .args(["install", "codex", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Codex installs hooks only"))
        .stdout(predicate::str::contains("subagent roles and skills only").not());

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let install_contract =
        fs::read_to_string(root.join("apps/docs/content/docs/reference/mcp.mdx")).unwrap();
    for required in [
        "complete client-owned MCP, role, skill, and hook-reconciliation paths",
        "Codex reconciles hooks only",
        "Claude Code writes standalone roles and hooks but no project skills",
        "Cursor writes roles, all ten skills, and hooks",
        "`--no-hooks` is the independent hook opt-out",
        "neither path writes Planr project roles or project skills",
    ] {
        assert!(
            install_contract.contains(required),
            "canonical install contract is missing: {required}"
        );
    }
    let skills_doc =
        fs::read_to_string(root.join("apps/docs/content/docs/plugins/index.mdx")).unwrap();
    assert!(skills_doc.contains(
        "writes the provider-neutral .planr/agents.toml registry; it does not generate Codex roles"
    ));
    assert!(!skills_doc.contains("generates canonical native Codex roles"));
}

#[test]
fn public_examples_use_the_router_and_only_plan_bound_goal_drivers() {
    fn collect_public_docs(path: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        if path.is_file() {
            if matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("md" | "mdx" | "json")
            ) {
                files.push(path.to_path_buf());
            }
            return;
        }
        for entry in fs::read_dir(path).unwrap() {
            collect_public_docs(&entry.unwrap().path(), files);
        }
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![root.join("README.md")];
    for public_root in [
        "docs",
        "apps/docs/content",
        "plugins/planr",
        ".planr/plans/product/planr",
    ] {
        collect_public_docs(&root.join(public_root), &mut files);
    }
    for path in files {
        let body = fs::read_to_string(&path).unwrap();
        assert!(
            !body.contains("Use $planr-loop."),
            "{} teaches an unprepared standalone loop",
            path.display()
        );
        for stale_install_claim in [
            "no MCP configuration at all",
            "generates canonical native Codex roles",
            "Plugin-style install: write subagent roles and skills only",
        ] {
            assert!(
                !body.contains(stale_install_claim),
                "{} contains stale install guidance: {stale_install_claim}",
                path.display()
            );
        }
        for line in body
            .lines()
            .filter(|line| line.contains("/goal Use $planr"))
        {
            assert!(
                line.contains("/goal Use $planr-loop on plan "),
                "{} teaches a non-plan-bound /goal driver: {line}",
                path.display()
            );
        }
    }
    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    assert!(readme.contains("Remember one public entry point: `$planr`"));
}

#[test]
fn rust_implementation_has_owned_module_boundaries() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "src/main.rs",
        "src/cli.rs",
        "src/app/mod.rs",
        "src/app/audit.rs",
        "src/app/commands.rs",
        "src/app/flow.rs",
        "src/app/git_review.rs",
        "src/app/mcp.rs",
        "src/app/packages.rs",
        "src/app/http.rs",
        "src/app/repository.rs",
        "src/app/review.rs",
        "src/app/recovery.rs",
        "src/app/review_workspace.rs",
        "src/app/surfaces.rs",
        "src/app/inspection.rs",
        "src/app/application.rs",
        "src/app/repository/item.rs",
        "src/app/repository/plan.rs",
        "src/app/repository/evidence.rs",
        "src/model.rs",
        "src/storage/mod.rs",
        "src/storage/schema.rs",
        "src/storage/rows.rs",
        "src/planpack.rs",
        "src/integrations.rs",
        "src/util.rs",
    ] {
        assert!(
            root.join(file).exists(),
            "missing architecture module {file}"
        );
    }
    for removed_hub in ["src/app.rs", "src/storage.rs", "src/domain", "crates"] {
        assert!(
            !root.join(removed_hub).exists(),
            "{removed_hub} should not return as a monolithic ownership hub"
        );
    }

    let main = fs::read_to_string(root.join("src/main.rs")).unwrap();
    assert!(
        main.lines().count() < 120,
        "src/main.rs should stay a CLI composition root, not the whole product"
    );
    for module in [
        "mod app;",
        "mod cli;",
        "mod integrations;",
        "mod model;",
        "mod planpack;",
        "mod storage;",
        "mod util;",
    ] {
        assert!(
            main.contains(module),
            "src/main.rs should wire module boundary {module}"
        );
    }
    for (file, max_lines) in [
        ("src/cli.rs", 1_050usize),
        ("src/app/mod.rs", 180),
        ("src/app/audit.rs", 200),
        ("src/app/commands.rs", 1_120),
        ("src/app/flow.rs", 320),
        ("src/app/git_review.rs", 350),
        ("src/app/mcp.rs", 900),
        ("src/app/packages.rs", 450),
        ("src/app/http.rs", 900),
        ("src/app/repository.rs", 1_100),
        ("src/app/lease.rs", 300),
        ("src/app/review.rs", 600),
        ("src/app/recovery.rs", 450),
        ("src/app/review_workspace.rs", 500),
        ("src/app/surfaces.rs", 300),
        ("src/app/inspection.rs", 510),
        ("src/app/application.rs", 200),
        ("src/storage/schema.rs", 300),
        ("src/storage/rows.rs", 150),
        ("src/model.rs", 400),
        ("src/planpack.rs", 320),
        ("src/integrations.rs", 500),
        ("src/agents.rs", 800),
        ("src/app/agents.rs", 950),
        ("src/app/agents_init.rs", 800),
        ("src/rolefiles.rs", 400),
    ] {
        let line_count = fs::read_to_string(root.join(file)).unwrap().lines().count();
        assert!(
            line_count <= max_lines,
            "{file} has {line_count} lines; keep ownership split instead of growing a new hub"
        );
    }

    let docs = fs::read_to_string(root.join("docs/ARCHITECTURE.md")).unwrap();
    for owner in [
        "src/main.rs",
        "src/cli.rs",
        "src/app/mod.rs",
        "src/app/audit.rs",
        "src/app/commands.rs",
        "src/app/flow.rs",
        "src/app/git_review.rs",
        "src/app/mcp.rs",
        "src/app/packages.rs",
        "src/app/http.rs",
        "src/app/repository.rs",
        "src/app/lease.rs",
        "src/app/review.rs",
        "src/app/recovery.rs",
        "src/app/review_workspace.rs",
        "src/app/surfaces.rs",
        "src/app/inspection.rs",
        "src/app/application.rs",
        "src/storage/mod.rs",
        "src/storage/schema.rs",
        "src/storage/rows.rs",
        "src/planpack.rs",
        "src/integrations.rs",
        "src/app/agents_init.rs",
    ] {
        assert!(
            docs.contains(owner),
            "architecture docs should document {owner}"
        );
    }
}

#[test]
fn plan_split_with_colon_slice_stays_parseable_and_check_is_honest() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();

    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Example"])
        .assert()
        .success();

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "plan", "new", "Example app"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    let product_id = value["plan"]["id"].as_str().unwrap().to_string();

    // A slice containing colons used to produce unquoted, unparseable YAML frontmatter.
    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "--json",
            "plan",
            "split",
            &product_id,
            "--slice",
            "MVP: add habit, daily check-in: streak display",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        value["plan"]["parse_status"].as_str().unwrap(),
        "ok",
        "split frontmatter must be YAML-safe for colon-bearing slices"
    );
    let build_id = value["plan"]["id"].as_str().unwrap().to_string();
    let build_path = value["plan"]["path"].as_str().unwrap().to_string();

    // A fresh build plan has empty mandatory sections; plan check must say so
    // instead of waving structure-only plans through.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "plan", "check", &build_id])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        value["ok"].as_bool(),
        Some(false),
        "empty mandatory sections must fail plan check"
    );
    for section in ["Scope Decision", "Verification", "Acceptance Criteria"] {
        let warning = value["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["section"].as_str() == Some(section))
            .unwrap_or_else(|| panic!("warnings must name the empty section {section}"));
        assert_eq!(
            warning["file"].as_str(),
            Some(build_path.as_str()),
            "warning must name the exact file to repair"
        );
        assert!(
            warning["fix"]
                .as_str()
                .unwrap_or_default()
                .contains("plan check"),
            "warning must carry the re-run command: {warning}"
        );
    }

    // Fill the mandatory sections and expand the scaffold's placeholder
    // task; plan check must pass afterwards.
    let text = fs::read_to_string(&build_path).unwrap();
    let filled = text
        .replace(
            "## Scope Decision\n",
            "## Scope Decision\n\nShip the MVP slice only.\n",
        )
        .replace(
            "## Verification\n",
            "## Verification\n\nRun the app and exercise the changed flow.\n",
        )
        .replace(
            "## Acceptance Criteria\n",
            "## Acceptance Criteria\n\n- Habit can be added and checked in.\n",
        );
    let implement_line = filled
        .lines()
        .find(|line| line.trim().starts_with("- [ ] Implement "))
        .expect("scaffold must ship the placeholder task")
        .to_string();
    let filled = filled.replace(
        &implement_line,
        "- [ ] Add habit\n- [ ] Daily check-in\n- [ ] Streak display",
    );
    fs::write(&build_path, filled).unwrap();

    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "plan", "check", &build_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("plan check passed"));

    // Corrupt the frontmatter on disk; plan check must fail and refresh parse_status.
    let text = fs::read_to_string(&build_path).unwrap();
    let broken = text.replacen("overview:", "overview: broken: colon: value", 1);
    fs::write(&build_path, broken).unwrap();

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "plan", "check", &build_id])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        value["ok"].as_bool(),
        Some(false),
        "parse error must fail plan check"
    );
    let warnings = value["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w["message"]
            .as_str()
            .unwrap_or_default()
            .contains("frontmatter parse error")),
        "warnings must name the frontmatter parse error, got {warnings:?}"
    );
    assert_eq!(
        value["plan"]["parse_status"].as_str().unwrap(),
        "parse_error",
        "plan check must refresh stored parse_status from disk"
    );
}

#[test]
fn follow_up_review_is_not_ready_while_fix_item_is_open() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();

    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Example"])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "item",
            "create",
            "Demo work",
            "--description",
            "demo",
        ])
        .assert()
        .success();

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "pick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    let item_id = value["item"]["id"].as_str().unwrap().to_string();

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "log",
            "add",
            "--item",
            &item_id,
            "--summary",
            "s",
            "--cmd",
            "c",
        ])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "review", "request", &item_id])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    let review_id = value["review"]["id"].as_str().unwrap().to_string();

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "review",
            "close",
            &review_id,
            "--verdict",
            "not-complete",
            "--findings",
            "finding x",
        ])
        .assert()
        .success();

    let statuses = |raw: &[u8]| -> std::collections::BTreeMap<String, String> {
        let value: Value = serde_json::from_slice(raw).unwrap();
        value["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                (
                    i["id"].as_str().unwrap().to_string(),
                    i["status"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };

    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let by_id = statuses(&output);
    let fix_id = by_id
        .keys()
        .find(|id| id.starts_with("i-fix-findings"))
        .expect("fix item created")
        .clone();
    let follow_up_id = by_id
        .keys()
        .find(|id| id.starts_with("i-follow-up-review"))
        .expect("follow-up review created")
        .clone();
    assert_eq!(by_id[&fix_id], "ready", "fix item must be ready");
    assert_eq!(
        by_id[&follow_up_id], "pending",
        "follow-up review must not be ready while its blocking fix item is open"
    );

    // Closing the fix item must promote the follow-up review.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "log",
            "add",
            "--item",
            &fix_id,
            "--summary",
            "fixed",
            "--cmd",
            "c",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "close", &fix_id, "--summary", "fixed"])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "--json", "map", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let by_id = statuses(&output);
    assert_eq!(
        by_id[&follow_up_id], "ready",
        "follow-up review must become ready once the fix item closes"
    );
    assert_eq!(
        by_id[&item_id], "in_review",
        "target must stay visibly in_review while the review chain is open"
    );

    // The follow-up review gates the same target, so closing it complete
    // with --close-target settles the original item through the chain.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "review",
            "close",
            &follow_up_id,
            "--verdict",
            "complete",
            "--close-target",
        ])
        .assert()
        .success();
    assert_eq!(item_status(&db, &item_id), "closed");
}

#[test]
fn plan_check_flags_unexpanded_scaffold_task_list_before_map_build() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    let run = |args: &[&str]| -> Value {
        let output = planr()
            .current_dir(dir.path())
            .args(["--db", &db_arg, "--json"])
            .args(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice(&output).unwrap()
    };

    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Coarse"])
        .assert()
        .success();
    let value = run(&["plan", "new", "Coarse map app"]);
    let product_id = value["plan"]["id"].as_str().unwrap().to_string();
    let value = run(&["plan", "split", &product_id, "--slice", "MVP"]);
    let build_id = value["plan"]["id"].as_str().unwrap().to_string();
    let build_path = value["plan"]["path"].as_str().unwrap().to_string();

    // Required sections filled, but the scaffold's single "- [ ] Implement
    // MVP" placeholder is untouched: the third dogfood run mapped exactly
    // this and got one coarse item plus a granularity guess.
    let text = fs::read_to_string(&build_path).unwrap();
    let filled = text
        .replace("## Scope Decision\n", "## Scope Decision\n\nMVP only.\n")
        .replace("## Verification\n", "## Verification\n\nRun the flow.\n")
        .replace(
            "## Acceptance Criteria\n",
            "## Acceptance Criteria\n\n- Works end to end.\n",
        );
    fs::write(&build_path, &filled).unwrap();
    let check = run(&["plan", "check", &build_id]);
    assert_eq!(
        check["ok"], false,
        "placeholder must fail the check: {check}"
    );
    let warnings = check["warnings"].as_array().unwrap();
    let task_warning = warnings
        .iter()
        .find(|warning| warning["section"] == "task list")
        .unwrap_or_else(|| panic!("missing task list warning: {check}"));
    assert!(
        task_warning["message"]
            .as_str()
            .unwrap()
            .contains("scaffold placeholder"),
        "warning must name the placeholder: {task_warning}"
    );
    assert!(
        task_warning["fix"]
            .as_str()
            .unwrap()
            .contains("per verifiable slice"),
        "fix must state the granularity contract: {task_warning}"
    );

    // Expanding the task list clears the warning and map build seeds one
    // item per slice instead of a single coarse gate.
    let expanded = filled.replace(
        "- [ ] Implement MVP\n",
        "- [ ] Add schema\n- [ ] Add endpoint\n- [ ] Add page\n",
    );
    fs::write(&build_path, expanded).unwrap();
    let check = run(&["plan", "check", &build_id]);
    assert_eq!(check["ok"], true, "expanded task list must pass: {check}");
    let build = run(&["map", "build", "--from", &build_id]);
    assert_eq!(
        build["created"].as_array().unwrap().len(),
        3,
        "map build must seed one item per slice: {build}"
    );
}

#[test]
fn symmetry_pack_tag_filter_plan_scoped_map_and_audit_next_command() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    let run = |args: &[&str]| -> Value {
        let output = planr()
            .current_dir(dir.path())
            .args(["--db", &db_arg, "--json"])
            .args(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice(&output).unwrap()
    };

    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Symmetry"])
        .assert()
        .success();
    let value = run(&["plan", "new", "Symmetry app"]);
    let product_id = value["plan"]["id"].as_str().unwrap().to_string();
    let value = run(&["plan", "split", &product_id, "--slice", "MVP"]);
    let build_id = value["plan"]["id"].as_str().unwrap().to_string();
    let build_path = value["plan"]["path"].as_str().unwrap().to_string();
    let text = fs::read_to_string(&build_path).unwrap();
    let implement_line = text
        .lines()
        .find(|line| line.trim().starts_with("- [ ] Implement "))
        .unwrap()
        .to_string();
    fs::write(
        &build_path,
        text.replace(&implement_line, "- [ ] First slice\n- [ ] Second slice"),
    )
    .unwrap();
    run(&["map", "build", "--from", &build_id]);
    run(&[
        "item",
        "create",
        "Off-plan chore",
        "--description",
        "outside the contract",
    ]);

    // Write-side tags must be recoverable on the read side: the fifth
    // dogfood agent inferred `context list --tag` and the CLI rejected it.
    run(&[
        "context",
        "add",
        &format!("GOAL CONTRACT {build_id}: DONE when settled."),
        "--tag",
        "goal-contract",
    ]);
    let contexts = run(&["context", "list", "--tag", "goal-contract"]);
    assert_eq!(
        contexts["contexts"].as_array().unwrap().len(),
        1,
        "tag filter must return exactly the contract note: {contexts}"
    );
    assert_eq!(
        run(&["context", "list", "--tag", "nope"])["contexts"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // Plan scope on map show: the off-plan chore and its counts stay out.
    let scoped = run(&["map", "show", "--plan", &build_id]);
    assert_eq!(
        scoped["items"].as_array().unwrap().len(),
        2,
        "plan scope must exclude off-plan items: {scoped}"
    );
    assert_eq!(scoped["total"], 2, "counts must be plan-scoped: {scoped}");
    let unscoped = run(&["map", "show"]);
    assert_eq!(unscoped["items"].as_array().unwrap().len(), 3);

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db", &db_arg, "map", "show", "--plan", &build_id, "--view", "diagram",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let scoped_diagram = String::from_utf8(output).unwrap();
    for item in scoped["items"].as_array().unwrap() {
        let id = item["id"].as_str().unwrap();
        assert!(
            scoped_diagram.contains(id),
            "scoped diagram omitted {id}: {scoped_diagram}"
        );
    }
    assert!(scoped_diagram.contains("blocks ─▶"), "{scoped_diagram}");
    assert!(
        !scoped_diagram.contains("Off-plan chore"),
        "scoped diagram leaked an off-plan item: {scoped_diagram}"
    );
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "--json",
            "map",
            "show",
            "--plan",
            "pln-missing",
        ])
        .assert()
        .failure();

    // An open audit verdict ends in the exact next command.
    let audit = run(&["plan", "audit", &build_id]);
    assert_eq!(audit["holds"], false);
    assert_eq!(
        audit["next"],
        format!("planr pick --plan {build_id} --json"),
        "open audit must name the next command: {audit}"
    );
}

#[test]
fn guess_killer_pack_auto_chain_audit_review_mode_unlocked_and_repair_errors() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    let run = |worker: &str, args: &[&str]| -> Value {
        let output = planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", worker)
            .args(["--db", &db_arg, "--json"])
            .args(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice(&output).unwrap()
    };

    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "GuessKiller"])
        .assert()
        .success();
    let value = run("prep", &["plan", "new", "Guess killer app"]);
    let product_id = value["plan"]["id"].as_str().unwrap().to_string();
    let value = run("prep", &["plan", "split", &product_id, "--slice", "MVP"]);
    let build_id = value["plan"]["id"].as_str().unwrap().to_string();
    let build_path = value["plan"]["path"].as_str().unwrap().to_string();

    // Fill required sections and define three ordered steps.
    let text = fs::read_to_string(&build_path).unwrap();
    let frontmatter_end = text.find("\n---\n").unwrap() + 5;
    let body = format!(
        "{}\n# Build Plan\n\n## Scope Decision\n\nMVP only.\n\n## Verification\n\nRun the flow.\n\n## Acceptance Criteria\n\n- The flow works end to end.\n\n## Steps\n\n### Add schema\n\nCreate the table.\n\n### Add endpoint\n\nServe the data.\n\n### Add page\n\nRender the data.\n",
        &text[..frontmatter_end]
    );
    fs::write(&build_path, body).unwrap();

    // F1: map build chains the created items in plan order with blocks links
    // and the output already carries items, links, and the next command.
    let build = run("prep", &["map", "build", "--from", &build_id]);
    let created = build["created"].as_array().unwrap();
    assert_eq!(created.len(), 3, "expected one item per plan step: {build}");
    let ids: Vec<String> = created
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect();
    let links = build["links"].as_array().unwrap();
    assert_eq!(links.len(), 2, "consecutive items must be chained: {build}");
    assert_eq!(links[0]["from"].as_str(), Some(ids[0].as_str()));
    assert_eq!(links[0]["to"].as_str(), Some(ids[1].as_str()));
    assert_eq!(links[1]["from"].as_str(), Some(ids[1].as_str()));
    assert_eq!(links[1]["to"].as_str(), Some(ids[2].as_str()));
    assert_eq!(created[0]["status"], "ready");
    assert_eq!(
        created[1]["status"], "pending",
        "chained items must not be ready before their blocker settles"
    );
    let rebuilt = run("prep", &["map", "build", "--from", &build_id]);
    assert!(rebuilt["created"].as_array().unwrap().is_empty());
    assert!(rebuilt["links"].as_array().unwrap().is_empty());

    // F3: the audit verdict before any work is an evidence-backed "open".
    let audit = run("prep", &["plan", "audit", &build_id]);
    assert_eq!(audit["holds"], false);
    let clause = |audit: &Value, name: &str| -> Value {
        audit["clauses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|clause| clause["clause"] == name)
            .unwrap_or_else(|| panic!("missing clause {name}: {audit}"))
            .clone()
    };
    let items_clause = clause(&audit, "items_settled");
    assert_eq!(items_clause["pass"], false);
    assert_eq!(items_clause["open"].as_array().unwrap().len(), 3);
    assert_eq!(
        clause(&audit, "verification_logged")["required"],
        false,
        "verification is contract-scoped; no contract stored yet"
    );

    // F7: closing a blocked item names the repair, not just the rejection.
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "close", &ids[2], "--summary", "premature"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("settle its blockers first"));

    // Item 1: maker ships with evidence, an independent checker closes.
    let pick = run("maker-1", &["pick", "--plan", &build_id]);
    assert_eq!(pick["item"]["id"].as_str(), Some(ids[0].as_str()));
    let done = run(
        "maker-1",
        &[
            "done",
            &ids[0],
            "--summary",
            "schema added",
            "--cmd",
            "cargo test schema",
            "--review",
        ],
    );
    let review_id = done["review"]["id"].as_str().unwrap().to_string();
    assert!(
        done["unlocked"]
            .as_array()
            .unwrap()
            .iter()
            .any(|unlocked| unlocked["id"].as_str() == Some(review_id.as_str())),
        "the freshly ready review must be reported as unlocked: {done}"
    );
    assert!(
        done["hint"].is_null(),
        "evidence was attached; no hint expected: {done}"
    );
    let review_pick = run(
        "checker-1",
        &["pick", "--work-type", "review", "--plan", &build_id],
    );
    assert_eq!(review_pick["item"]["id"].as_str(), Some(review_id.as_str()));
    let closed = run(
        "checker-1",
        &[
            "review",
            "close",
            &review_id,
            "--verdict",
            "complete",
            "--close-target",
        ],
    );
    // F4: the maker/checker split is derived from recorded identity.
    assert_eq!(closed["review_mode"], "independent", "{closed}");
    // F6: settling the gate reports the step it unlocked.
    assert!(
        closed["unlocked"]
            .as_array()
            .unwrap()
            .iter()
            .any(|unlocked| unlocked["id"].as_str() == Some(ids[1].as_str())),
        "closing the gate must report the unlocked next step: {closed}"
    );

    // Item 2: settled without evidence while downstream work exists -> hint.
    run("maker-1", &["pick", "--plan", &build_id]);
    let done = run("maker-1", &["done", &ids[1], "--summary", "endpoint added"]);
    assert!(
        done["hint"].as_str().unwrap_or_default().contains("--cmd"),
        "missing evidence with downstream work must hint: {done}"
    );
    assert!(
        done["unlocked"]
            .as_array()
            .unwrap()
            .iter()
            .any(|unlocked| unlocked["id"].as_str() == Some(ids[2].as_str())),
        "{done}"
    );

    // Item 3: the same identity closes its own review -> single_agent.
    run("maker-1", &["pick", "--plan", &build_id]);
    let done = run(
        "maker-1",
        &[
            "done",
            &ids[2],
            "--summary",
            "page added",
            "--cmd",
            "vitest --run page",
            "--review",
        ],
    );
    let review_id = done["review"]["id"].as_str().unwrap().to_string();
    run(
        "maker-1",
        &["pick", "--work-type", "review", "--plan", &build_id],
    );
    let closed = run(
        "maker-1",
        &[
            "review",
            "close",
            &review_id,
            "--verdict",
            "complete",
            "--close-target",
        ],
    );
    assert_eq!(closed["review_mode"], "single_agent", "{closed}");

    // F3 arc: settled board holds without a contract; a stored contract
    // makes verification binding; a verification log satisfies it.
    let audit = run("prep", &["plan", "audit", &build_id]);
    assert_eq!(audit["holds"], true, "all settled, no contract: {audit}");
    run(
        "prep",
        &[
            "context",
            "add",
            &format!(
                "GOAL CONTRACT {build_id}: DONE when all items closed and live verification logged."
            ),
            "--tag",
            "goal-contract",
        ],
    );
    let audit = run("prep", &["plan", "audit", &build_id]);
    assert_eq!(
        audit["holds"], false,
        "a stored contract makes the verification clause binding: {audit}"
    );
    assert!(
        audit["contract"]["content"]
            .as_str()
            .unwrap()
            .contains(&build_id)
    );
    run(
        "maker-1",
        &[
            "log",
            "add",
            "--item",
            &ids[2],
            "--kind",
            "verification",
            "--summary",
            "verified page in browser",
            "--cmd",
            "curl localhost:3000/page",
        ],
    );
    let audit = run("prep", &["plan", "audit", &build_id]);
    assert_eq!(audit["holds"], true, "{audit}");
    let verification = clause(&audit, "verification_logged");
    assert_eq!(verification["pass"], true);
    assert_eq!(verification["logs"].as_array().unwrap().len(), 1);

    // F8: the post condition is echoed at completion time.
    let manual = run(
        "prep",
        &[
            "item",
            "create",
            "Manual gate",
            "--description",
            "standalone",
            "--post",
            "verify the deploy manually",
        ],
    );
    let manual_id = manual["item"]["id"].as_str().unwrap().to_string();
    let closed_manual = run("prep", &["close", &manual_id, "--summary", "done"]);
    assert_eq!(
        closed_manual["post_condition"], "verify the deploy manually",
        "{closed_manual}"
    );
}
