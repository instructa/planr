use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command as StdCommand;
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

fn planr() -> Command {
    Command::cargo_bin("planr").expect("planr binary")
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
    let fixture: Value =
        serde_json::from_slice(&fs::read(root.join("docs/fixtures/mcp-contract.json")).unwrap())
            .unwrap();

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
    assert!(dir
        .path()
        .join(".planr/integrations/codex-mcp.toml")
        .exists());
    assert!(dir.path().join(".mcp.json").exists());
    assert!(dir.path().join(".cursor/mcp.json").exists());

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
    assert!(prompt_cli["prompt"]
        .as_str()
        .unwrap()
        .contains("planr map preview"));

    let prompt_mcp = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "prompt", "mcp"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let prompt_mcp: Value = serde_json::from_slice(&prompt_mcp).unwrap();
    assert!(prompt_mcp["prompt"]
        .as_str()
        .unwrap()
        .contains("\"mcpServers\""));

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

    let cli_reference = fs::read_to_string(root.join("docs/CLI_REFERENCE.md")).unwrap();
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
    let relevant = picked["context"]["relevant_contexts"].as_array().unwrap();
    assert_eq!(relevant.len(), 1);
    assert!(relevant[0]["content"]
        .as_str()
        .unwrap()
        .contains("session cache"));
    assert_eq!(
        picked["context"]["privacy"]["source_file_content_included"],
        false
    );
    assert!(!serde_json::to_string(&picked)
        .unwrap()
        .contains("sk-test-should-not-appear"));
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
    assert!(mcp_response["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("invalid_transition"));

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
    assert!(responses[0]["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("not_found"));

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
    let ids = stmt
        .query_map([parent_id], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    ids
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
        picked["trace"]["links"].is_array(),
        "pick must include the trace work packet"
    );

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
    assert_eq!(item_status(&db, &first), "running");

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
    assert!(links
        .iter()
        .any(|link| link["from"] == first_id && link["to"] == middle_id));
    assert!(links
        .iter()
        .any(|link| link["from"] == middle_id && link["to"] == second_id));
    assert!(!links
        .iter()
        .any(|link| link["from"] == first_id && link["to"] == second_id));

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
        .stdout(predicate::str::contains("status"));

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
    assert!(pick["context"].is_object(), "{pick}");
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
    assert_eq!(bundle["counts"]["artifacts"], 1);

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
        pick["context"]["conditions"]["pre"],
        "Confirm environment is bootstrapped"
    );
    assert_eq!(pick["context"]["recovery"]["timeout_seconds"], 1);

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
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows
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
    assert!(annotations.iter().any(|entry| entry["content"]
        .as_str()
        .unwrap()
        .contains("Workspace annotation")));

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
    assert!(clean["evidence"]["git"]["changed_files"]
        .as_array()
        .unwrap()
        .is_empty());

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
    assert!(dirty["evidence"]["git"]["scoped_files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "src/owned.rs"));
    assert!(dirty["evidence"]["git"]["unrelated_dirty_files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "src/unrelated.rs"));
    assert_eq!(
        dirty["evidence"]["dirty_worktree_safety"]["source_content_included"],
        false
    );
    assert!(dirty["evidence"]["provenance"]["pr_urls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "https://github.com/instructa/planr/pull/1"));

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
    assert!(contexts["contexts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|context| context["content"]
            .as_str()
            .unwrap()
            .contains("Template review annotation context")));
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
    for skill in [
        "planr",
        "planr-loop",
        "planr-verify-web",
        "planr-task-graph",
        "planr-plan",
        "planr-work",
        "planr-review",
        "planr-status",
        "planr-summary",
    ] {
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
    for agent in ["planr-worker", "planr-reviewer"] {
        assert!(
            root.join("plugins/planr/agents")
                .join(format!("{agent}.md"))
                .exists(),
            "missing plugin agent {agent}"
        );
    }
}

#[test]
fn project_init_and_install_provision_loop_agent_roles() {
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
        ".codex/agents/planr-worker.toml",
        ".codex/agents/planr-reviewer.toml",
        ".claude/agents/planr-worker.md",
        ".claude/agents/planr-reviewer.md",
    ] {
        assert!(
            dir.path().join(role).exists(),
            "project init --client all should provision {role}"
        );
    }
    let worker = fs::read_to_string(dir.path().join(".codex/agents/planr-worker.toml")).unwrap();
    assert!(
        worker.contains("planr_worker"),
        "provisioned codex worker role should define the planr_worker agent"
    );

    // `planr install codex` provisions the same roles for projects initialized
    // without a client, and never overwrites user-edited role files.
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
    assert!(dir2.path().join(".codex/agents/planr-worker.toml").exists());
    fs::write(
        dir2.path().join(".codex/agents/planr-worker.toml"),
        "# user-edited\n",
    )
    .unwrap();
    planr()
        .current_dir(dir2.path())
        .args(["--db", db2.to_str().unwrap(), "install", "codex"])
        .assert()
        .success();
    let edited = fs::read_to_string(dir2.path().join(".codex/agents/planr-worker.toml")).unwrap();
    assert_eq!(
        edited, "# user-edited\n",
        "install must not overwrite roles"
    );
}

#[test]
fn rust_implementation_has_owned_module_boundaries() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "src/main.rs",
        "src/cli.rs",
        "src/app/mod.rs",
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
    for removed_hub in ["src/app.rs", "src/storage.rs"] {
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
        ("src/cli.rs", 900usize),
        ("src/app/mod.rs", 180),
        ("src/app/commands.rs", 1_000),
        ("src/app/flow.rs", 250),
        ("src/app/git_review.rs", 350),
        ("src/app/mcp.rs", 900),
        ("src/app/packages.rs", 450),
        ("src/app/http.rs", 900),
        ("src/app/repository.rs", 1_100),
        ("src/app/review.rs", 600),
        ("src/app/recovery.rs", 450),
        ("src/app/review_workspace.rs", 500),
        ("src/app/surfaces.rs", 300),
        ("src/app/inspection.rs", 500),
        ("src/storage/schema.rs", 300),
        ("src/storage/rows.rs", 150),
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
        "src/storage/mod.rs",
        "src/storage/schema.rs",
        "src/storage/rows.rs",
        "src/planpack.rs",
        "src/integrations.rs",
        "single crate",
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
        assert!(
            value["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w.as_str().unwrap_or_default().contains(section)),
            "warnings must name the empty section {section}"
        );
    }

    // Fill the mandatory sections; plan check must pass afterwards.
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
        warnings.iter().any(|w| w
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
}
