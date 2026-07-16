use assert_cmd::Command;
use ed25519_dalek::SigningKey;
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command as StdCommand;
use std::thread;
use std::time::Duration;
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
client = "codex"
model = "gpt-5.5"
effort = "medium"
cost_tier = "standard"

[profiles.driver]
client = "cursor"
model = "fable-5"
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
        "gpt-5.5"
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
    assert_eq!(picked["routing"]["client"], "codex");
    assert_eq!(picked["routing"]["model"], "gpt-5.5");
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
client = "codex"
model = "gpt-5.5"

[profiles.driver]
client = "cursor"
model = "fable-5"
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
client = "codex"
model = "gpt-5.5"

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
fn install_renders_roles_from_registry_and_respects_provision_once() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Render"])
        .assert()
        .success();
    fs::write(
        dir.path().join(".planr/agents.toml"),
        r#"
[profiles.coder]
client = "codex"
model = "gpt-5.5"
effort = "xhigh"

[profiles.judge]
client = "cursor"
model = "fable-5"
effort = "high"

[[routes]]
match = { work_type = "code" }
profile = "coder"

[[routes]]
match = { work_type = "review" }
profile = "judge"
"#,
    )
    .unwrap();

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "install", "codex"])
        .assert()
        .success();
    let worker_path = dir.path().join(".codex/agents/planr-worker.toml");
    let worker = fs::read_to_string(&worker_path).unwrap();
    assert!(worker.contains("# generated from .planr/agents.toml"));
    let parsed: toml::Value = toml::from_str(&worker).unwrap();
    assert_eq!(parsed["model"].as_str(), Some("gpt-5.5"));
    assert_eq!(parsed["model_reasoning_effort"].as_str(), Some("xhigh"));
    // The worker's audit report is baked into its own definition: the
    // concrete profile id as a --profile instruction, not worker memory.
    assert!(
        parsed["developer_instructions"]
            .as_str()
            .unwrap()
            .contains("--profile coder"),
        "rendered worker must instruct its own profile report: {worker}"
    );
    // The review route points at a Cursor profile: a Codex role file must
    // not pin a model Codex cannot dispatch, so the reviewer stays static.
    let reviewer =
        fs::read_to_string(dir.path().join(".codex/agents/planr-reviewer.toml")).unwrap();
    assert!(!reviewer.contains("generated from"));
    assert!(!reviewer.contains("fable-5"));

    // The same review route does pin the Cursor reviewer role.
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "install", "cursor"])
        .assert()
        .success();
    let cursor_reviewer =
        fs::read_to_string(dir.path().join(".cursor/agents/planr-reviewer.md")).unwrap();
    assert!(cursor_reviewer.contains("model: fable-5"));
    assert!(cursor_reviewer.contains("# profile: judge"));
    let cursor_worker =
        fs::read_to_string(dir.path().join(".cursor/agents/planr-worker.md")).unwrap();
    assert!(
        cursor_worker.contains("model: inherit"),
        "code route targets a Codex profile, so the Cursor worker keeps its static default"
    );

    // Provision-once: hand edits survive a re-install without --force and
    // are re-rendered with it.
    fs::write(&worker_path, "# hand edited\n").unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "install", "codex"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&worker_path).unwrap(), "# hand edited\n");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "install", "codex", "--force"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&worker_path).unwrap(), worker);

    // Without a registry the install output is byte-identical to the
    // shipped static role files.
    let fresh = tempdir().unwrap();
    let fresh_db = fresh.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(fresh.path())
        .args([
            "--db",
            fresh_db.to_str().unwrap(),
            "project",
            "init",
            "Static",
        ])
        .assert()
        .success();
    planr()
        .current_dir(fresh.path())
        .args(["--db", fresh_db.to_str().unwrap(), "install", "codex"])
        .assert()
        .success();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(
        fs::read_to_string(fresh.path().join(".codex/agents/planr-worker.toml")).unwrap(),
        fs::read_to_string(repo.join("plugins/planr/skills/planr-loop/agents/planr-worker.toml"))
            .unwrap()
    );
    assert_eq!(
        fs::read_to_string(fresh.path().join(".codex/agents/planr-reviewer.toml")).unwrap(),
        fs::read_to_string(repo.join("plugins/planr/skills/planr-loop/agents/planr-reviewer.toml"))
            .unwrap()
    );
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
client = "codex"
model = "gpt-5.5"

[profiles.driver]
client = "cursor"
model = "fable-5"

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
client = "codex"
model = "gpt-5.5"
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
            "model": {"value": "gpt-5.5", "enforcement": "requested_only", "evidence": "policy"},
            "effort": {"value": "high", "enforcement": "requested_only", "evidence": "policy"},
            "context_fork": {"value": {"mode": "none"}, "enforcement": "requested_only", "evidence": "policy"}
        },
        "resolved": {
            "role": "worker",
            "profile": "coder",
            "client": "codex",
            "model": {"value": "gpt-5.5", "enforcement": "verified", "evidence": "binding"},
            "effort": {"value": "high", "enforcement": "verified", "evidence": "binding"},
            "context_fork": {"value": {"mode": "none"}, "enforcement": "verified", "evidence": "binding"}
        },
        "effective": {
            "role": "worker",
            "profile": "coder",
            "client": "codex",
            "model": {"value": null, "enforcement": "unavailable"},
            "effort": {"value": null, "enforcement": "unavailable"},
            "context_fork": {"value": {"mode": "none"}, "enforcement": "verified", "evidence": "host_report"}
        },
        "transition": {
            "kind": "availability_fallback",
            "reason": "primary profile unavailable; same-class fallback selected",
            "evidence": ["host_report"]
        },
        "policy": {"id": "balanced", "version": "1.0.0"},
        "binding": {"id": "codex-openai", "version": "1.0.0"},
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
    assert_eq!(route["requested"]["model"]["value"], "gpt-5.5");
    assert_eq!(route["resolved"]["effort"]["value"], "high");
    assert!(route["effective"]["model"]["value"].is_null());
    assert_eq!(route["effective"]["model"]["enforcement"], "unavailable");
    assert_eq!(route["effective"]["context_fork"]["value"]["mode"], "none");
    assert_eq!(route["policy"]["id"], "balanced");
    assert_eq!(route["binding"]["version"], "1.0.0");
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
        "value": "gpt-5.5",
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
        "value": "gpt-5.5",
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
        "value": "gpt-5.5",
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
client = "codex"
model = "gpt-5.5"
effort = "xhigh"

[profiles.ghost-ref]
client = "codex"
model = "gpt-5.5"

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

    // Rendered artifacts: current right after install, drifted after the
    // registry changes underneath them.
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "install", "codex"])
        .assert()
        .success();
    let artifact_state = |value: &Value, path: &str| -> String {
        value["registry"]["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|artifact| artifact["path"] == path)
            .map(|artifact| artifact["state"].as_str().unwrap().to_string())
            .unwrap_or_else(|| panic!("artifact {path} not reported"))
    };
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "doctor"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doctor: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        artifact_state(&doctor, ".codex/agents/planr-worker.toml"),
        "current"
    );
    fs::write(
        &registry_path,
        r#"
[profiles.coder]
client = "codex"
model = "gpt-6"
effort = "high"

[[routes]]
match = { work_type = "code" }
profile = "coder"
"#,
    )
    .unwrap();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "doctor"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doctor: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        artifact_state(&doctor, ".codex/agents/planr-worker.toml"),
        "drifted"
    );
    assert!(
        doctor["registry"]["drift_hint"]
            .as_str()
            .unwrap()
            .contains("--force")
    );

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
client = "codex"
model = "gpt-5.5"

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
    let local_registry = "[profiles.local]\nclient = \"cursor\"\nmodel = \"fable-5\"\n";
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
model = "opus"
effort = "high"
cost_tier = "premium"
skill = "frontend-design"

[profiles.backender]
client = "codex"
model = "gpt-5.5"

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
    assert!(prompt.contains("| work_type=frontend | designer | claude-code | opus | high | premium | frontend-design | backender |"));
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
            "driver=cursor/fable-5@high#premium",
            "--profile",
            "designer=claude-code/opus@high#premium",
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

    // A hand-written registry with render-unsafe values (quoted TOML keys
    // parse fine but would corrupt rendered role files) keeps installs on
    // the static role files instead of writing broken artifacts.
    fs::write(
        dir.path().join(".planr/agents.toml"),
        "[profiles.\"evil\nid\"]\nclient = \"codex\"\nmodel = \"gpt-5.5\"\n\n[[routes]]\nmatch = { work_type = \"code\" }\nprofile = \"evil\nid\"\n",
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "install", "codex", "--no-mcp", "--force"])
        .assert()
        .success();
    let worker = fs::read_to_string(dir.path().join(".codex/agents/planr-worker.toml")).unwrap();
    assert!(
        !worker.contains("generated from"),
        "render-unsafe profile must fall back to the static role: {worker}"
    );
    toml::from_str::<toml::Value>(&worker).expect("role file must stay parseable TOML");

    // QA-3: spec flags never overwrite without --force either.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "agents",
            "init",
            "--profile",
            "solo=codex/gpt-5.5",
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
            "a=codex/m",
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
        "[profiles.coder]\nclient = \"cursor\"\nmodel = \"gpt-5.5\"\n\n[route_default]\nprofile = \"coder\"\n",
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
        "[profiles.designer]\nclient = \"cursor\"\nmodel = \"opus\"\nskill = \"definitely-missing-skill\"\n\n[route_default]\nprofile = \"designer\"\n",
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
        "[profiles.frontender]\nclient = \"cursor\"\nmodel = \"opus\"\n\n[profiles.backender]\nclient = \"cursor\"\nmodel = \"gpt-5.5-high\"\n\n[[routes]]\nmatch = { work_type = \"frontend\" }\nprofile = \"frontender\"\n\n[[routes]]\nmatch = { work_type = \"backend\" }\nprofile = \"backender\"\n",
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
        "[profiles.coder]\nclient = \"cursor\"\nmodel = \"gpt-5.5-high\"\n\n[route_default]\nprofile = \"coder\"\n",
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
fn agents_init_scaffold_is_warning_free_and_routes_by_default() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Scaffold"])
        .assert()
        .success();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "agents", "init"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(created["path"], ".planr/agents.toml");
    assert_eq!(created["next"][0], "planr agents check");

    // The scaffold must parse with zero warnings.
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "agents", "check"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let check: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(check["ok"], true);
    assert_eq!(check["warnings"].as_array().unwrap().len(), 0);

    // The scaffold teaches client honesty where clients get declared.
    let scaffold_content = fs::read_to_string(dir.path().join(".planr/agents.toml")).unwrap();
    assert!(
        scaffold_content.contains("Declare the client you will actually dispatch on"),
        "scaffold must carry the client-honesty rule: {scaffold_content}"
    );

    // A seeded code item picks up the scaffold's implementer route.
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "create",
            "Scaffolded work",
            "--description",
            "Routes via the starter registry",
            "--work-type",
            "code",
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
    assert_eq!(pick["routing"]["profile"], "implementer");
    assert_eq!(pick["routing"]["model"], "gpt-5.5");
    assert_eq!(pick["routing"]["fallbacks"][0], "driver");
    assert_eq!(pick["routing"]["matched_selector"], "work_type=code");

    // A second init refuses politely and leaves the file untouched...
    let scaffold = fs::read_to_string(dir.path().join(".planr/agents.toml")).unwrap();
    let custom = scaffold.replace("gpt-5.5", "gpt-6");
    fs::write(dir.path().join(".planr/agents.toml"), &custom).unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "agents", "init"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
    assert_eq!(
        fs::read_to_string(dir.path().join(".planr/agents.toml")).unwrap(),
        custom
    );

    // ...and --force restores the scaffold.
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "agents", "init", "--force"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(dir.path().join(".planr/agents.toml")).unwrap(),
        scaffold
    );
}

#[test]
fn prompt_routing_names_routes_fallbacks_and_host_traps() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Prompt"])
        .assert()
        .success();

    // Missing registry: still zero-exit with the host guidance and a
    // pointer instead of a route table.
    let missing = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "prompt", "routing"])
        .output()
        .unwrap();
    assert!(missing.status.success());
    let missing_json: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(missing_json["registry"], "missing");
    assert!(
        missing_json["prompt"]
            .as_str()
            .unwrap()
            .contains("fork_turns")
    );

    fs::write(
        dir.path().join(".planr/agents.toml"),
        r#"
[profiles.coder]
client = "codex"
model = "gpt-5.5"
effort = "xhigh"

[profiles.driver]
client = "cursor"
model = "fable-5"

[[routes]]
match = { work_type = "code" }
profile = "coder"
fallbacks = ["driver"]

[route_default]
profile = "driver"
"#,
    )
    .unwrap();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "prompt", "routing"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["registry"], "ok");
    let routes = json["routes"].as_array().unwrap();
    assert_eq!(routes.len(), 2, "every route plus the default is named");
    assert_eq!(routes[0]["match"], "work_type=code");
    assert_eq!(routes[0]["fallbacks"][0], "driver");
    assert_eq!(routes[1]["match"], "default");
    let prompt = json["prompt"].as_str().unwrap();
    for required in [
        "work_type=code",
        "driver",
        // The three host traps must be spelled out.
        "fork_turns: \"none\"",
        "CLAUDE_CODE_SUBAGENT_MODEL",
        "Max Mode can override",
        // Process dispatch reuses the code route's pin as the example.
        "codex exec --model gpt-5.5 -c model_reasoning_effort=\"xhigh\"",
        "pi --provider",
        "opencode run",
    ] {
        assert!(
            prompt.contains(required),
            "prompt must contain `{required}`"
        );
    }
    assert!(
        json["hosts"]["codex"][1]
            .as_str()
            .unwrap()
            .contains("fork_turns")
    );
    assert_eq!(json["process_dispatch"].as_array().unwrap().len(), 3);

    // --client filters the host sections but keeps the table.
    let codex_only = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "prompt",
            "routing",
            "--client",
            "codex",
        ])
        .output()
        .unwrap();
    let codex_json: serde_json::Value = serde_json::from_slice(&codex_only.stdout).unwrap();
    let codex_prompt = codex_json["prompt"].as_str().unwrap();
    assert!(codex_prompt.contains("### Codex"));
    assert!(!codex_prompt.contains("### Claude Code"));
    assert!(!codex_prompt.contains("### Cursor"));
    assert!(codex_prompt.contains("work_type=code"));
    assert!(codex_json["hosts"].get("claude").is_none());
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
model = "fable-5"

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
    assert_eq!(listed["registry"]["profiles"]["driver"]["model"], "fable-5");
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
fn preset_composition_cli_mcp_parity_lock_conflicts_and_repository_safety() {
    let policy = r#"
schema_version = 1
id = "portable-balanced"
version = "1.0.0"

[usage]
max_active_agents = 3
max_parallel_readers = 2
max_parallel_writers = 1
max_depth = 1
max_attempts = 4
review_reserve_percent = 20
budget_exhaustion = "stop"
metering = "unavailable"

[transitions.retry]
max_same_route_retries = 1

[transitions.availability_fallback]
max_fallbacks = 1
require_same_capability_class = true

[transitions.quality_escalation]
max_escalations = 1
require_verification_evidence = true

[transitions.quota_downgrade]
enabled = false
max_downgrades = 0
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
allow_overwrite = false
"#;
    let binding = r#"
schema_version = 1
id = "codex-test"
version = "1.0.0"
host = "codex"
driver_role = "driver"
default_role = "driver"
capability_evidence = ["codex-0.138-cross-tier-smoke"]
billing_assumptions = ["local subscription"]
known_limitations = ["effective model requires host evidence"]

[capabilities]
model_override = true
effort_override = true
fork_none = true
fork_all = true
max_partial_fork_turns = 4

[profiles.driver]
profile = "sol"
client = "codex"
model = "gpt-5.5"
effort = "xhigh"
cost_tier = "premium"

[profiles.worker]
profile = "luna"
client = "codex"
model = "gpt-5.4-mini"
effort = "high"
cost_tier = "standard"
skill = "planr-work"

[[routes]]
work_type = "code"
role = "worker"
fallback_roles = ["driver"]

[verification]
id = "verify-codex-test"
verified_at_unix = 1900000000
max_age_seconds = 86400

[[artifacts]]
path = ".codex/agents/luna.toml"
kind = "codex_agent"
content = '''model = "gpt-5.4-mini"
model_reasoning_effort = "high"
'''
"#;
    let prepare = |root: &Path| {
        let db = root.join(".planr/planr.sqlite");
        planr()
            .current_dir(root)
            .args([
                "--db",
                db.to_str().unwrap(),
                "project",
                "init",
                "Preset composition",
            ])
            .assert()
            .success();
        fs::write(root.join("portable-policy.toml"), policy).unwrap();
        fs::write(root.join("codex-binding.toml"), binding).unwrap();
        db
    };
    let deterministic = |command: &mut assert_cmd::Command| {
        command
            .env("PLANR_PRESET_NOW_UNIX", "1900000100")
            .env("PLANR_PRESET_APPLIED_AT", "2030-03-17T17:48:20Z");
    };

    let cli_dir = tempdir().unwrap();
    let cli_db = prepare(cli_dir.path());
    let mut preview_command = planr();
    deterministic(&mut preview_command);
    let preview = preview_command
        .current_dir(cli_dir.path())
        .args([
            "--db",
            cli_db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "apply",
            "portable-policy.toml",
            "--binding",
            "codex-binding.toml",
            "--preview",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&preview).unwrap();
    assert_eq!(preview["action"], "previewed");
    assert_eq!(preview["mutation"], false);
    assert_eq!(preview["compatibility"]["ok"], true);
    assert_eq!(preview["pack"]["status"], "custom");
    assert_eq!(preview["pack"]["safe"], false);
    assert!(
        preview["compatibility"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning
                .as_str()
                .unwrap()
                .contains("not a validated built-in safe pack"))
    );
    assert_eq!(preview["verification_age"]["status"], "fresh");
    assert_eq!(
        preview["composition"]["dispatch"]["worker"]["fork_turns"]["mode"],
        "none"
    );
    assert_eq!(
        preview["permission_diff"]["worker"]["write_roots"],
        json!(["src", "tests"])
    );
    assert_eq!(preview["artifacts"].as_array().unwrap().len(), 4);
    let policy_diff = preview["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["path"] == ".planr/policy.toml")
        .unwrap();
    assert!(policy_diff["config_diff"]["current"].is_null());
    assert_eq!(
        policy_diff["config_diff"]["proposed"]["value"]["usage"]["max_active_agents"],
        3
    );
    assert_eq!(
        policy_diff["config_diff"]["proposed"]["value"]["transitions"]["retry"]["max_same_route_retries"],
        1
    );
    let registry_diff = preview["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["path"] == ".planr/agents.toml")
        .unwrap();
    assert_eq!(
        registry_diff["config_diff"]["proposed"]["value"]["profiles"]["luna"]["client"],
        "codex"
    );
    assert_eq!(
        registry_diff["config_diff"]["proposed"]["value"]["profiles"]["luna"]["model"],
        "gpt-5.4-mini"
    );
    assert_eq!(
        registry_diff["config_diff"]["proposed"]["value"]["routes"][0]["fallbacks"],
        json!(["sol"])
    );
    assert!(
        preview["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|artifact| artifact["action"] == "create")
    );
    for path in [
        ".planr/policy.toml",
        ".planr/agents.toml",
        ".planr/preset.lock.toml",
        ".codex/agents/luna.toml",
    ] {
        assert!(!cli_dir.path().join(path).exists(), "preview wrote {path}");
    }

    let mut mcp_preview_command = planr();
    deterministic(&mut mcp_preview_command);
    let mcp_preview = mcp_preview_command
        .current_dir(cli_dir.path())
        .args(["--db", cli_db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_preset_apply","arguments":{"policy":"portable-policy.toml","binding":"codex-binding.toml"}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_preview: Value = serde_json::from_slice(&mcp_preview).unwrap();
    let mcp_preview: Value = serde_json::from_str(
        mcp_preview["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(mcp_preview, preview);

    let aws_secret = "AKIAEXAMPLE123";
    let embedded_secret = "xoxb-embedded-token";
    let secret_binding = binding.replace(
        "model_reasoning_effort = \"high\"",
        &format!(
            "model_reasoning_effort = \"high\"\naws_access_key_id = \"{aws_secret}\"\nnotes = \"rotate {embedded_secret} today\""
        ),
    );
    fs::write(cli_dir.path().join("secret-binding.toml"), secret_binding).unwrap();
    let mut secret_cli_command = planr();
    deterministic(&mut secret_cli_command);
    let secret_cli = secret_cli_command
        .current_dir(cli_dir.path())
        .args([
            "--db",
            cli_db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "apply",
            "portable-policy.toml",
            "--binding",
            "secret-binding.toml",
            "--preview",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let secret_cli_raw = String::from_utf8(secret_cli).unwrap();
    assert!(!secret_cli_raw.contains(aws_secret), "CLI leaked AWS key");
    assert!(
        !secret_cli_raw.contains(embedded_secret),
        "CLI leaked embedded token"
    );
    assert!(secret_cli_raw.contains("[REDACTED]"));

    let mut secret_mcp_command = planr();
    deterministic(&mut secret_mcp_command);
    let secret_mcp = secret_mcp_command
        .current_dir(cli_dir.path())
        .args(["--db", cli_db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_preset_apply","arguments":{"policy":"portable-policy.toml","binding":"secret-binding.toml"}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let secret_mcp_raw = String::from_utf8(secret_mcp).unwrap();
    assert!(!secret_mcp_raw.contains(aws_secret), "MCP leaked AWS key");
    assert!(
        !secret_mcp_raw.contains(embedded_secret),
        "MCP leaked embedded token"
    );
    let secret_cli: Value = serde_json::from_str(&secret_cli_raw).unwrap();
    let secret_mcp: Value = serde_json::from_str(&secret_mcp_raw).unwrap();
    let secret_mcp: Value =
        serde_json::from_str(secret_mcp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(secret_mcp, secret_cli);

    let evidence_secret = "AKIAMETADATA123";
    let warning_secret = "xoxb-warning-token";
    let billing_secret = "ghp_billing_token";
    let metadata_binding = binding
        .replace(
            "capability_evidence = [\"codex-0.138-cross-tier-smoke\"]",
            &format!("capability_evidence = [\"{evidence_secret}\"]"),
        )
        .replace(
            "billing_assumptions = [\"local subscription\"]",
            &format!("billing_assumptions = [\"account {billing_secret}\"]"),
        )
        .replace(
            "known_limitations = [\"effective model requires host evidence\"]",
            &format!("known_limitations = [\"rotate {warning_secret} today\"]"),
        );
    fs::write(
        cli_dir.path().join("secret-metadata-binding.toml"),
        metadata_binding,
    )
    .unwrap();
    let mut metadata_cli_command = planr();
    deterministic(&mut metadata_cli_command);
    let metadata_cli = metadata_cli_command
        .current_dir(cli_dir.path())
        .args([
            "--db",
            cli_db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "apply",
            "portable-policy.toml",
            "--binding",
            "secret-metadata-binding.toml",
            "--confirm",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let metadata_cli_raw = String::from_utf8(metadata_cli).unwrap();
    for secret in [evidence_secret, warning_secret, billing_secret] {
        assert!(
            !metadata_cli_raw.contains(secret),
            "CLI leaked metadata {secret}"
        );
    }
    assert!(metadata_cli_raw.contains("binding.capability_evidence[0]"));
    assert!(metadata_cli_raw.contains("binding.known_limitations[0]"));
    assert!(metadata_cli_raw.contains("binding.billing_assumptions[0]"));
    for path in [
        ".planr/policy.toml",
        ".planr/agents.toml",
        ".planr/preset.lock.toml",
        ".codex/agents/luna.toml",
    ] {
        assert!(
            !cli_dir.path().join(path).exists(),
            "rejection wrote {path}"
        );
    }

    let mut metadata_mcp_command = planr();
    deterministic(&mut metadata_mcp_command);
    let metadata_mcp = metadata_mcp_command
        .current_dir(cli_dir.path())
        .args(["--db", cli_db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_preset_apply","arguments":{"policy":"portable-policy.toml","binding":"secret-metadata-binding.toml","confirm":true}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let metadata_mcp_raw = String::from_utf8(metadata_mcp).unwrap();
    for secret in [evidence_secret, warning_secret, billing_secret] {
        assert!(
            !metadata_mcp_raw.contains(secret),
            "MCP leaked metadata {secret}"
        );
    }
    let metadata_mcp: Value = serde_json::from_str(&metadata_mcp_raw).unwrap();
    assert_eq!(metadata_mcp["result"]["isError"], true);
    for path in [
        ".planr/policy.toml",
        ".planr/agents.toml",
        ".planr/preset.lock.toml",
        ".codex/agents/luna.toml",
    ] {
        assert!(
            !cli_dir.path().join(path).exists(),
            "rejection wrote {path}"
        );
    }

    let mut cli_apply_command = planr();
    deterministic(&mut cli_apply_command);
    let cli_apply = cli_apply_command
        .current_dir(cli_dir.path())
        .args([
            "--db",
            cli_db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "apply",
            "portable-policy.toml",
            "--binding",
            "codex-binding.toml",
            "--confirm",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let cli_apply: Value = serde_json::from_slice(&cli_apply).unwrap();
    assert_eq!(cli_apply["action"], "applied");
    assert_eq!(cli_apply["mutation"], true);

    let mcp_dir = tempdir().unwrap();
    let mcp_db = prepare(mcp_dir.path());
    let mut mcp_apply_command = planr();
    deterministic(&mut mcp_apply_command);
    let mcp_apply = mcp_apply_command
        .current_dir(mcp_dir.path())
        .args(["--db", mcp_db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_preset_apply","arguments":{"policy":"portable-policy.toml","binding":"codex-binding.toml","confirm":true}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_apply: Value = serde_json::from_slice(&mcp_apply).unwrap();
    let mcp_apply: Value =
        serde_json::from_str(mcp_apply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(mcp_apply, cli_apply);
    for path in [
        ".planr/policy.toml",
        ".planr/agents.toml",
        ".planr/preset.lock.toml",
        ".codex/agents/luna.toml",
    ] {
        assert_eq!(
            fs::read(cli_dir.path().join(path)).unwrap(),
            fs::read(mcp_dir.path().join(path)).unwrap(),
            "CLI/MCP artifact drift at {path}"
        );
    }
    let lock = fs::read_to_string(cli_dir.path().join(".planr/preset.lock.toml")).unwrap();
    assert!(lock.contains("id = \"portable-balanced\""));
    assert!(lock.contains("id = \"codex-test\""));
    assert!(lock.contains("applied_at = \"2030-03-17T17:48:20Z\""));

    let policy_snapshot = fs::read(cli_dir.path().join(".planr/policy.toml")).unwrap();
    fs::write(
        cli_dir.path().join(".planr/agents.toml"),
        "# unrelated local registry\n",
    )
    .unwrap();
    let mut conflict_preview_command = planr();
    deterministic(&mut conflict_preview_command);
    let conflict_preview = conflict_preview_command
        .current_dir(cli_dir.path())
        .args([
            "--db",
            cli_db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "apply",
            "portable-policy.toml",
            "--binding",
            "codex-binding.toml",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let conflict_preview: Value = serde_json::from_slice(&conflict_preview).unwrap();
    let registry_conflict = conflict_preview["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["path"] == ".planr/agents.toml")
        .unwrap();
    assert_eq!(registry_conflict["action"], "conflict");
    assert!(registry_conflict["config_diff"]["current"].is_object());
    assert_eq!(
        registry_conflict["config_diff"]["proposed"]["value"]["profiles"]["luna"]["effort"],
        "high"
    );
    let mut conflict_command = planr();
    deterministic(&mut conflict_command);
    conflict_command
        .current_dir(cli_dir.path())
        .args([
            "--db",
            cli_db.to_str().unwrap(),
            "agents",
            "preset",
            "apply",
            "portable-policy.toml",
            "--binding",
            "codex-binding.toml",
            "--confirm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "refused existing unrelated configuration: .planr/agents.toml",
        ));
    assert_eq!(
        fs::read(cli_dir.path().join(".planr/policy.toml")).unwrap(),
        policy_snapshot,
        "conflict must fail before any write"
    );
    assert_eq!(
        fs::read_to_string(cli_dir.path().join(".planr/agents.toml")).unwrap(),
        "# unrelated local registry\n"
    );

    let unsafe_dir = tempdir().unwrap();
    let unsafe_db = prepare(unsafe_dir.path());
    let absolute_target = unsafe_dir
        .path()
        .join("absolute-target.toml")
        .to_string_lossy()
        .to_string();
    for (index, target, expected) in [
        (
            0,
            ".codex/config.toml".to_string(),
            "user/global config and .codex/config.toml are forbidden",
        ),
        (
            1,
            "../outside.toml".to_string(),
            "contains absolute, current, or parent traversal",
        ),
        (2, absolute_target, "must be repository-relative"),
        (
            3,
            "~/.codex/agents/luna.toml".to_string(),
            "outside the repository artifact allowlist",
        ),
    ] {
        let file = format!("forbidden-binding-{index}.toml");
        fs::write(
            unsafe_dir.path().join(&file),
            binding.replace(".codex/agents/luna.toml", &target),
        )
        .unwrap();
        let mut forbidden_command = planr();
        deterministic(&mut forbidden_command);
        forbidden_command
            .current_dir(unsafe_dir.path())
            .args([
                "--db",
                unsafe_db.to_str().unwrap(),
                "agents",
                "preset",
                "apply",
                "portable-policy.toml",
                "--binding",
                &file,
                "--confirm",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(expected));
        assert!(!unsafe_dir.path().join(".planr/policy.toml").exists());
        assert!(!unsafe_dir.path().join(".planr/agents.toml").exists());
        assert!(!unsafe_dir.path().join(".planr/preset.lock.toml").exists());
    }

    let all_fork = binding
        .replace(
            "skill = \"planr-work\"",
            "skill = \"planr-work\"\nfork_turns = { mode = \"all\" }",
        )
        .replace("client = \"codex\"", "client = \"generic-mcp\"");
    fs::write(unsafe_dir.path().join("all-fork-binding.toml"), all_fork).unwrap();
    let mut all_fork_command = planr();
    deterministic(&mut all_fork_command);
    all_fork_command
        .current_dir(unsafe_dir.path())
        .args([
            "--db",
            unsafe_db.to_str().unwrap(),
            "agents",
            "preset",
            "apply",
            "portable-policy.toml",
            "--binding",
            "all-fork-binding.toml",
            "--confirm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot use fork_turns all"));
    assert!(!unsafe_dir.path().join(".planr/policy.toml").exists());
    assert!(!unsafe_dir.path().join(".planr/agents.toml").exists());
    assert!(!unsafe_dir.path().join(".planr/preset.lock.toml").exists());

    let blank_partial = binding
        .replace(
            "capability_evidence = [\"codex-0.138-cross-tier-smoke\"]",
            "capability_evidence = [\"   \"]",
        )
        .replace(
            "skill = \"planr-work\"",
            "skill = \"planr-work\"\nfork_turns = { mode = \"partial\", turns = 2 }",
        );
    fs::write(
        unsafe_dir.path().join("blank-partial-binding.toml"),
        blank_partial,
    )
    .unwrap();
    let mut blank_partial_command = planr();
    deterministic(&mut blank_partial_command);
    let blank_partial = blank_partial_command
        .current_dir(unsafe_dir.path())
        .args(["--db", unsafe_db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_preset_apply","arguments":{"policy":"portable-policy.toml","binding":"blank-partial-binding.toml","confirm":true}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let blank_partial: Value = serde_json::from_slice(&blank_partial).unwrap();
    assert_eq!(blank_partial["result"]["isError"], true);
    assert!(
        blank_partial["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("capability_evidence[0] must not be blank")
    );
    assert!(!unsafe_dir.path().join(".planr/policy.toml").exists());
    assert!(!unsafe_dir.path().join(".planr/agents.toml").exists());
    assert!(!unsafe_dir.path().join(".planr/preset.lock.toml").exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = tempdir().unwrap();
        fs::create_dir_all(unsafe_dir.path().join(".codex")).unwrap();
        symlink(outside.path(), unsafe_dir.path().join(".codex/agents")).unwrap();
        let mut symlink_command = planr();
        deterministic(&mut symlink_command);
        symlink_command
            .current_dir(unsafe_dir.path())
            .args([
                "--db",
                unsafe_db.to_str().unwrap(),
                "agents",
                "preset",
                "apply",
                "portable-policy.toml",
                "--binding",
                "codex-binding.toml",
                "--confirm",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("crosses symlink"));
        assert!(!outside.path().join("luna.toml").exists());
        assert!(!unsafe_dir.path().join(".planr/policy.toml").exists());
        assert!(!unsafe_dir.path().join(".planr/agents.toml").exists());
        assert!(!unsafe_dir.path().join(".planr/preset.lock.toml").exists());
    }
}

#[test]
fn builtin_preset_catalog_cli_mcp_safe_packs_and_repository_boundary() {
    let catalog_dir = tempdir().unwrap();
    let catalog_db = catalog_dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(catalog_dir.path())
        .args([
            "--db",
            catalog_db.to_str().unwrap(),
            "project",
            "init",
            "Built-in catalog",
        ])
        .assert()
        .success();
    let cli_catalog = planr()
        .current_dir(catalog_dir.path())
        .args([
            "--db",
            catalog_db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "list",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let cli_catalog: Value = serde_json::from_slice(&cli_catalog).unwrap();
    assert_eq!(cli_catalog["policies"].as_array().unwrap().len(), 4);
    assert_eq!(cli_catalog["bindings"].as_array().unwrap().len(), 5);
    assert_eq!(cli_catalog["safe_packs"].as_array().unwrap().len(), 20);
    let mcp_catalog = planr()
        .current_dir(catalog_dir.path())
        .args(["--db", catalog_db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_presets_list","arguments":{}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_catalog: Value = serde_json::from_slice(&mcp_catalog).unwrap();
    let mcp_catalog: Value = serde_json::from_str(
        mcp_catalog["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(mcp_catalog, cli_catalog);

    for binding in [
        "codex-openai",
        "cursor-openai",
        "cursor-fable-grok",
        "claude-native",
        "mixed-host",
    ] {
        let dir = tempdir().unwrap();
        let home = tempdir().unwrap();
        let db = dir.path().join(".planr/planr.sqlite");
        let global_files = [
            (".codex/config.toml", b"codex-global-snapshot\n".as_slice()),
            (".claude/settings.json", b"{\"claude\":true}\n".as_slice()),
            (".cursor/mcp.json", b"{\"cursor\":true}\n".as_slice()),
        ];
        for (path, content) in global_files {
            let target = home.path().join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, content).unwrap();
        }
        let snapshots = global_files
            .iter()
            .map(|(path, _)| {
                (
                    home.path().join(path),
                    fs::read(home.path().join(path)).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        planr()
            .current_dir(dir.path())
            .env("HOME", home.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "project",
                "init",
                "Built-in safe pack",
            ])
            .assert()
            .success();

        let configure = |command: &mut assert_cmd::Command| {
            command
                .env("HOME", home.path())
                .env("PLANR_PRESET_NOW_UNIX", "1784000000")
                .env("PLANR_PRESET_APPLIED_AT", "2026-07-14T00:00:00Z");
        };
        let mut preview_command = planr();
        configure(&mut preview_command);
        let preview = preview_command
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "agents",
                "preset",
                "apply",
                "balanced",
                "--binding",
                binding,
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let preview: Value = serde_json::from_slice(&preview).unwrap();
        assert_eq!(preview["pack"]["status"], "safe", "{binding}");
        assert_eq!(preview["pack"]["safe"], true, "{binding}");
        assert_eq!(preview["pack"]["policy"], "balanced");
        assert_eq!(preview["pack"]["binding"], binding);
        assert_eq!(preview["compatibility"]["ok"], true);
        assert!(preview["conflicts"].as_array().unwrap().is_empty());

        let mut mcp_command = planr();
        configure(&mut mcp_command);
        let mcp_preview = mcp_command
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "mcp"])
            .write_stdin(format!(
                "{}\n",
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_preset_apply","arguments":{"policy":"balanced","binding":binding}}})
            ))
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let mcp_preview: Value = serde_json::from_slice(&mcp_preview).unwrap();
        let mcp_preview: Value = serde_json::from_str(
            mcp_preview["result"]["content"][0]["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(mcp_preview, preview, "CLI/MCP drift for {binding}");

        let mut apply_command = planr();
        configure(&mut apply_command);
        let applied = apply_command
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "agents",
                "preset",
                "apply",
                "balanced",
                "--binding",
                binding,
                "--confirm",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let applied: Value = serde_json::from_slice(&applied).unwrap();
        let mut expected = preview.clone();
        expected["action"] = json!("applied");
        expected["mutation"] = json!(true);
        assert_eq!(applied, expected, "apply drift for {binding}");

        let canonical_root = fs::canonicalize(dir.path()).unwrap();
        let registry: toml::Value =
            toml::from_str(&fs::read_to_string(dir.path().join(".planr/agents.toml")).unwrap())
                .unwrap();
        let profile_matches = |client: &str, model: &str, effort: Option<&str>| {
            registry["profiles"]
                .as_table()
                .unwrap()
                .values()
                .any(|profile| {
                    profile["client"].as_str() == Some(client)
                        && profile["model"].as_str() == Some(model)
                        && effort.is_none_or(|effort| profile["effort"].as_str() == Some(effort))
                })
        };
        for artifact in preview["artifacts"].as_array().unwrap() {
            let relative = artifact["path"].as_str().unwrap();
            let relative_path = Path::new(relative);
            assert!(
                !relative_path.is_absolute(),
                "absolute built-in target {relative}"
            );
            assert!(
                relative_path
                    .components()
                    .all(|component| matches!(component, std::path::Component::Normal(_))),
                "non-normal built-in target {relative}"
            );
            let target = dir.path().join(relative);
            let canonical_target = fs::canonicalize(&target).unwrap();
            assert!(canonical_target.starts_with(&canonical_root));
            let actual = fs::read(&target).unwrap();
            let actual_hash = format!("{:x}", Sha256::digest(&actual));
            assert_eq!(artifact["proposed_sha256"], actual_hash, "{relative}");
            let content = String::from_utf8(actual).unwrap();
            match artifact["kind"].as_str().unwrap() {
                "codex_agent" => {
                    let role: toml::Value = toml::from_str(&content).unwrap();
                    assert!(
                        !role["developer_instructions"]
                            .as_str()
                            .unwrap()
                            .trim()
                            .is_empty(),
                        "Codex would ignore {relative} without developer_instructions"
                    );
                    assert!(!role["model"].as_str().unwrap().trim().is_empty());
                    assert!(
                        !role["model_reasoning_effort"]
                            .as_str()
                            .unwrap()
                            .trim()
                            .is_empty()
                    );
                    assert!(profile_matches(
                        "codex",
                        role["model"].as_str().unwrap(),
                        role["model_reasoning_effort"].as_str(),
                    ));
                }
                "claude_agent" | "cursor_agent" => {
                    let rest = content.strip_prefix("---\n").unwrap();
                    let (frontmatter, body) = rest.split_once("\n---\n").unwrap();
                    let fields = frontmatter
                        .lines()
                        .map(|line| line.split_once(':').unwrap())
                        .collect::<BTreeMap<_, _>>();
                    assert!(!fields["name"].trim().is_empty());
                    assert!(!fields["model"].trim().is_empty());
                    assert!(!body.trim().is_empty());
                    let client = if artifact["kind"] == "claude_agent" {
                        "claude-code"
                    } else {
                        "cursor"
                    };
                    assert!(profile_matches(
                        client,
                        fields["model"].trim(),
                        fields.get("effort").map(|effort| effort.trim()),
                    ));
                }
                _ => {}
            }
        }
        for (path, snapshot) in &snapshots {
            assert_eq!(
                fs::read(path).unwrap(),
                *snapshot,
                "global file changed: {path:?}"
            );
        }
    }
}

#[test]
fn preset_evaluation_reports_are_reproducible_threshold_gated_and_surface_identical() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Preset evaluation",
        ])
        .assert()
        .success();

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "evaluate",
            "--at-unix",
            "1783987200",
            "--report-dir",
            "reports/preset-v1",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let cli: Value = serde_json::from_slice(&output).unwrap();
    let report = &cli["report"];
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["suite"]["version"], "1.8.0");
    assert_eq!(report["reproducible_evidence"], true);
    assert_eq!(report["task_fixtures"].as_array().unwrap().len(), 7);
    let task_kinds = report["task_fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["kind"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        task_kinds,
        std::collections::BTreeSet::from([
            "browser",
            "exploration",
            "implementation",
            "mechanical",
            "security",
            "subagent",
            "visual",
        ])
    );
    assert!(report["recommended"].as_array().unwrap().is_empty());
    assert_eq!(report["codex_dispatch_contract"]["all_fork_rejected"], true);
    assert_eq!(
        report["codex_dispatch_contract"]["none_fork_parameters_verified"],
        true
    );
    assert_eq!(
        report["codex_dispatch_contract"]["missing_effective_evidence_cannot_verify"],
        true
    );
    assert_eq!(
        report["candidates"][0]["model_versions"]["worker"]["model"],
        "gpt-5.4-mini"
    );
    assert!(
        report["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|candidate| candidate["status"] == "verified")
    );
    assert!(
        report["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|candidate| candidate["results"].as_array().unwrap())
            .all(|result| {
                result["input_sha256"]
                    .as_str()
                    .is_some_and(|hash| hash.len() == 64)
                    && result["result_sha256"]
                        .as_str()
                        .is_some_and(|hash| hash.len() == 64)
                    && result["evidence"]["generated_by"] == "planr-offline-policy-simulator"
                    && result["evidence"]["evidence_scope"] == "policy_simulation"
                    && result["evidence"]["task_executed"] == false
                    && result["evidence"]["outcome_oracle_evaluated"] == false
                    && result["evidence"]["recommendation_eligible"] == false
                    && result["evidence"]["route_verified"] == false
                    && result["evidence"]["route_observation"]["effective"]["model"]["enforcement"]
                        == "unavailable"
            })
    );
    assert_eq!(report["transition_counts"]["safety_stop"]["attempted"], 4);
    assert!(
        report["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|candidate| candidate.get("label").is_none())
    );

    let machine_path = dir.path().join("reports/preset-v1/verification.json");
    let human_path = dir.path().join("reports/preset-v1/report.md");
    let original_machine = fs::read(&machine_path).unwrap();
    let original_human = fs::read(&human_path).unwrap();
    let written: Value = serde_json::from_slice(&original_machine).unwrap();
    assert_eq!(written, cli);
    let markdown = fs::read_to_string(&human_path).unwrap();
    assert!(markdown.contains("# Preset Evaluation Verification"));
    assert!(markdown.contains("Codex Sol/Luna contract"));
    assert!(markdown.contains("balanced-codex-openai"));

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "agents",
            "preset",
            "evaluate",
            "--report-dir",
            "reports/preset-v1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("immutable report already exists"));
    assert_eq!(fs::read(&machine_path).unwrap(), original_machine);
    assert_eq!(fs::read(&human_path).unwrap(), original_human);

    let mcp = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_preset_evaluate","arguments":{"at_unix":1783987200}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp: Value = serde_json::from_slice(&mcp).unwrap();
    let mcp: Value =
        serde_json::from_str(mcp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(mcp, cli);

    #[cfg(unix)]
    {
        let assert_incomplete_live =
            |payload: &Value| {
                assert_eq!(payload["report"]["reproducible_evidence"], false);
                assert!(
                    payload["report"]["recommended"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                assert!(
                    payload["report"]["candidates"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|candidate| candidate["status"] == "unverified"
                            && candidate["evidence_complete"] == false
                            && candidate["recommendation_evidence_complete"] == false
                            && candidate["status_reasons"].as_array().unwrap().iter().any(
                                |reason| reason.as_str().is_some_and(|reason| reason
                                    .contains("live execution was attempted")
                                    && reason.contains("challenge-bound"))
                            ))
                );
            };
        let mapper_adapter = dir.path().join("request-mapper-live-host.sh");
        fs::write(
            &mapper_adapter,
            r#"#!/bin/sh
request=$(cat)
candidate=$(printf '%s' "$request" | sed -n 's/.*"candidate":{"id":"\([^"]*\)".*/\1/p')
task=$(printf '%s' "$request" | sed -n 's/.*"task":{"id":"\([^"]*\)".*/\1/p')
input_sha=$(printf '%s' "$request" | sed -n 's/.*"input_sha256":"\([^"]*\)".*/\1/p')
artifact=$(printf '%s' "$request" | sed -n 's/.*"artifact_kind":"\([^"]*\)".*/\1/p')
case "$task" in
  explore-routing-boundaries) suffix=routing-boundaries-inspected ;;
  implement-bounded-policy-change) suffix=bounded-policy-change-implemented ;;
  mechanical-schema-rewrite) suffix=owned-schema-rewritten ;;
  browser-report-smoke) suffix=browser-report-inspected ;;
  visual-report-regression) suffix=visual-contract-matched ;;
  security-safety-stop) suffix=unsafe-operation-stopped ;;
  subagent-sol-luna-dispatch) suffix=sol-luna-dispatch-verified ;;
  *) suffix=unknown-task ;;
esac
result="$candidate:$task:$suffix"
printf '{"schema_version":1,"host_id":"fake-host","host_version":"1.0.0","candidate_id":"%s","task_id":"%s","input_sha256":"%s","artifact_kind":"%s","output":"%s","effective_model":"gpt-5.4-mini","effective_effort":"high","effective_context_fork":{"mode":"none"},"tool_calls":1,"tokens":10,"credits_micros":100,"retries":1,"availability_fallbacks":1,"quality_escalations":1,"corrections":1,"violations":0}\n' "$candidate" "$task" "$input_sha" "$artifact" "$result"
"#,
        )
        .unwrap();
        let live_output = planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "agents",
                "preset",
                "evaluate",
                "--at-unix",
                "1783987200",
                "--live-host-command",
                "/bin/sh",
                "--live-host-arg",
                mapper_adapter.to_str().unwrap(),
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let live: Value = serde_json::from_slice(&live_output).unwrap();
        assert_eq!(
            live["report"]["environment"]["runner"],
            "planr-live-host-runner"
        );
        assert_incomplete_live(&live);
        assert!(
            live["report"]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|candidate| candidate["results"].as_array().unwrap())
                .all(|result| result["evidence"]["task_executed"] == false
                    && result["evidence"]["outcome_oracle_evaluated"] == false
                    && result["evidence"]["recommendation_eligible"] == false
                    && result["evidence"]["route_verified"] == false)
        );
        let mapper_mcp = planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "mcp"])
            .write_stdin(format!(
                "{}\n",
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"planr_preset_evaluate","arguments":{"at_unix":1783987200,"live_host_command":"/bin/sh","live_host_args":[mapper_adapter.to_str().unwrap()]}}})
            ))
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let mapper_mcp: Value = serde_json::from_slice(&mapper_mcp).unwrap();
        let mapper_mcp: Value =
            serde_json::from_str(mapper_mcp["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_incomplete_live(&mapper_mcp);

        let artifact_adapter = dir.path().join("artifact-live-host.sh");
        fs::write(
            &artifact_adapter,
            r#"#!/bin/sh
request=$(cat)
candidate=$(printf '%s' "$request" | sed -n 's/.*"candidate":{"id":"\([^"]*\)".*/\1/p')
task=$(printf '%s' "$request" | sed -n 's/.*"task":{"id":"\([^"]*\)".*/\1/p')
input_sha=$(printf '%s' "$request" | sed -n 's/.*"input_sha256":"\([^"]*\)".*/\1/p')
artifact=$(printf '%s' "$request" | sed -n 's/.*"artifact_kind":"\([^"]*\)".*/\1/p')
challenge_path=$(printf '%s' "$request" | sed -n 's/.*"challenge_path":"\([^"]*\)".*/\1/p')
artifact_path=$(printf '%s' "$request" | sed -n 's/.*"artifact_path":"\([^"]*\)".*/\1/p')
hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}
case "$task" in
  explore-routing-boundaries) suffix=routing-boundaries-inspected ;;
  implement-bounded-policy-change) suffix=bounded-policy-change-implemented ;;
  mechanical-schema-rewrite) suffix=owned-schema-rewritten ;;
  browser-report-smoke) suffix=browser-report-inspected ;;
  visual-report-regression) suffix=visual-contract-matched ;;
  security-safety-stop) suffix=unsafe-operation-stopped ;;
  subagent-sol-luna-dispatch) suffix=sol-luna-dispatch-verified ;;
  *) suffix=unknown-task ;;
esac
result="$candidate:$task:$suffix"
challenge_sha=$(hash_file "$challenge_path")
printf '{"schema_version":1,"candidate_id":"%s","task_id":"%s","input_sha256":"%s","artifact_kind":"%s","challenge_sha256":"%s","output":"%s"}' "$candidate" "$task" "$input_sha" "$artifact" "$challenge_sha" "$result" > "$artifact_path"
artifact_sha=$(hash_file "$artifact_path")
printf '{"schema_version":1,"host_id":"artifact-host","host_version":"1.0.0","candidate_id":"%s","task_id":"%s","input_sha256":"%s","artifact_kind":"%s","artifact_sha256":"%s","output":"%s","effective_model":"gpt-5.4-mini","effective_effort":"high","effective_context_fork":{"mode":"none"},"tool_calls":1,"tokens":10,"credits_micros":100,"retries":1,"availability_fallbacks":1,"quality_escalations":1,"corrections":1,"violations":0}\n' "$candidate" "$task" "$input_sha" "$artifact" "$artifact_sha" "$result"
"#,
        )
        .unwrap();
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let hex = |bytes: &[u8]| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        let telemetry_public_key = hex(signing_key.verifying_key().as_bytes());
        let sha256 = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
        let telemetry_seed_path = dir.path().join("telemetry-signing-seed.hex");
        fs::write(&telemetry_seed_path, hex(&[7_u8; 32])).unwrap();
        let planr_bin = assert_cmd::cargo::cargo_bin("planr");
        let collector_path = dir.path().join("trusted-telemetry-collector.sh");
        fs::write(
            &collector_path,
            format!(
                r#"#!/bin/sh
request=$(cat)
payload=$(printf '%s' "$request" | sed 's/}}$/,"host_id":"artifact-host","host_version":"1.0.0","effective_model":"gpt-5.4-mini","effective_effort":"high","effective_context_fork":{{"mode":"none"}},"tool_calls":1,"tokens":10,"credits_micros":100,"retries":1,"availability_fallbacks":1,"quality_escalations":1,"corrections":1,"violations":0}}/')
printf '%s' "$payload" | "{}" --json agents preset telemetry-sign --private-key-file "{}"
"#,
                planr_bin.display(),
                telemetry_seed_path.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        fs::set_permissions(&collector_path, fs::Permissions::from_mode(0o700)).unwrap();
        let missing_collector_path = dir.path().join("missing-telemetry-collector.sh");
        fs::write(&missing_collector_path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&missing_collector_path, fs::Permissions::from_mode(0o700)).unwrap();
        let registry_path = dir.path().join(".planr/trusted-telemetry.toml");
        fs::write(
            &registry_path,
            format!(
                r#"schema_version = 1

[[signers]]
id = "e2e-collector"
public_key_hex = "{telemetry_public_key}"
collector_sha256 = "{}"

[[signers]]
id = "missing-e2e-collector"
public_key_hex = "{telemetry_public_key}"
collector_sha256 = "{}"
"#,
                sha256(&fs::read(&collector_path).unwrap()),
                sha256(&fs::read(&missing_collector_path).unwrap())
            ),
        )
        .unwrap();
        let artifact_output = planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "agents",
                "preset",
                "evaluate",
                "--at-unix",
                "1783987200",
                "--live-host-command",
                "/bin/sh",
                "--live-host-arg",
                artifact_adapter.to_str().unwrap(),
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let artifact_live: Value = serde_json::from_slice(&artifact_output).unwrap();
        assert!(
            artifact_live["report"]["recommended"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for candidate in artifact_live["report"]["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|candidate| {
                candidate["policy"]["id"] == "balanced"
                    || candidate["policy"]["id"] == "max-quality"
            })
        {
            assert_eq!(candidate["status"], "verified");
            assert_eq!(candidate["evidence_complete"], true);
            assert_eq!(candidate["metrics"]["actual_task_runs"], 7);
            assert_eq!(candidate["metrics"]["oracle_passes"], 7);
            assert_eq!(candidate["metrics"]["verified_route_runs"], 0);
            assert_eq!(candidate["recommendation_evidence_complete"], false);
            assert!(
                candidate["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|result| {
                        result["evidence"]["task_executed"] == true
                            && result["evidence"]["outcome_oracle_evaluated"] == true
                            && result["evidence"]["recommendation_eligible"] == false
                            && result["evidence"]["route_verified"] == false
                            && result["evidence"]["oracle"]["kind"] == "planr_workspace_artifact"
                            && result["evidence"]["oracle"]["pass"] == true
                    })
            );
        }

        let trusted_output = planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "agents",
                "preset",
                "evaluate",
                "--at-unix",
                "1783987200",
                "--live-host-command",
                "/bin/sh",
                "--live-host-arg",
                artifact_adapter.to_str().unwrap(),
                "--trusted-telemetry-signer",
                "e2e-collector",
                "--trusted-telemetry-collector",
                collector_path.to_str().unwrap(),
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let trusted_live: Value = serde_json::from_slice(&trusted_output).unwrap();
        assert!(
            !trusted_live["report"]["recommended"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for candidate in trusted_live["report"]["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|candidate| candidate["status"] == "recommended")
        {
            assert_eq!(candidate["evidence_complete"], true);
            assert_eq!(candidate["recommendation_evidence_complete"], true);
            assert_eq!(candidate["metrics"]["verified_route_runs"], 7);
            assert!(candidate["results"].as_array().unwrap().iter().all(|result| {
                result["evidence"]["recommendation_eligible"] == true
                    && result["evidence"]["route_verified"] == true
                    && result["evidence"]["metrics_source"] == "trusted_telemetry"
                    && result["evidence"]["metering_confidence"] == "trusted"
                    && result["evidence"]["route_observation"]["effective"]["model"]
                        ["evidence"]
                        == "telemetry_receipt"
                    && result["evidence"]["route_observation"]["metering"]
                        ["credits_micros"]["confidence"]
                        == "trusted"
            }));
        }

        let ephemeral_key = SigningKey::from_bytes(&[9_u8; 32]);
        let precomputed_path = dir.path().join("caller-precomputed-telemetry.json");
        fs::write(&precomputed_path, br#"{"schema_version":1,"receipts":[]}"#).unwrap();
        planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "agents",
                "preset",
                "evaluate",
                "--at-unix",
                "1783987200",
                "--live-host-command",
                "/bin/sh",
                "--live-host-arg",
                artifact_adapter.to_str().unwrap(),
                "--trusted-telemetry-file",
                precomputed_path.to_str().unwrap(),
                "--trusted-telemetry-public-key",
                &hex(ephemeral_key.verifying_key().as_bytes()),
                "--trusted-telemetry-run-id",
                "caller-selected-run",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "unexpected argument '--trusted-telemetry-file'",
            ));

        let missing_output = planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "agents",
                "preset",
                "evaluate",
                "--at-unix",
                "1783987200",
                "--live-host-command",
                "/bin/sh",
                "--live-host-arg",
                artifact_adapter.to_str().unwrap(),
                "--trusted-telemetry-signer",
                "missing-e2e-collector",
                "--trusted-telemetry-collector",
                missing_collector_path.to_str().unwrap(),
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let missing: Value = serde_json::from_slice(&missing_output).unwrap();
        assert!(
            missing["report"]["recommended"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            missing["report"]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|candidate| candidate["results"].as_array().unwrap())
                .all(
                    |result| result["evidence"]["recommendation_eligible"] == false
                        && result["evidence"]["route_verified"] == false
                )
        );

        let live_mcp = planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "mcp"])
            .write_stdin(format!(
                "{}\n",
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"planr_preset_evaluate","arguments":{"at_unix":1783987200,"live_host_command":"/bin/sh","live_host_args":[artifact_adapter.to_str().unwrap()]}}})
            ))
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let live_mcp: Value = serde_json::from_slice(&live_mcp).unwrap();
        let live_mcp: Value =
            serde_json::from_str(live_mcp["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(
            live_mcp["report"]["environment"]["runner"],
            "planr-live-host-runner"
        );
        assert!(
            live_mcp["report"]["recommended"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            live_mcp["report"]["candidates"][0]["metrics"]["actual_task_runs"],
            7
        );
        let trusted_mcp = planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "mcp"])
            .write_stdin(format!(
                "{}\n",
                json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"planr_preset_evaluate","arguments":{"at_unix":1783987200,"live_host_command":"/bin/sh","live_host_args":[artifact_adapter.to_str().unwrap()],"trusted_telemetry_signer":"e2e-collector","trusted_telemetry_collector":collector_path.to_str().unwrap()}}})
            ))
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let trusted_mcp: Value = serde_json::from_slice(&trusted_mcp).unwrap();
        let trusted_mcp: Value = serde_json::from_str(
            trusted_mcp["result"]["content"][0]["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert!(
            !trusted_mcp["report"]["recommended"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            trusted_mcp["report"]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|candidate| candidate["status"] == "recommended")
                .all(|candidate| candidate["metrics"]["verified_route_runs"] == 7)
        );

        let constant_response = json!({
            "schema_version": 1,
            "host_id": "constant-host",
            "host_version": "1.0.0",
            "candidate_id": "constant-candidate",
            "task_id": "constant-task",
            "input_sha256": "constant-input",
            "artifact_kind": "constant-artifact",
            "output": "pass",
            "effective_model": "gpt-5.4-mini",
            "effective_effort": "high",
            "effective_context_fork": {"mode": "none"},
            "tool_calls": 1,
            "tokens": 10,
            "credits_micros": 100,
            "retries": 1,
            "availability_fallbacks": 1,
            "quality_escalations": 1,
            "corrections": 1,
            "violations": 0
        })
        .to_string();
        let constant_output = planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "agents",
                "preset",
                "evaluate",
                "--at-unix",
                "1783987200",
                "--live-host-command",
                "/usr/bin/printf",
                "--live-host-arg",
                &constant_response,
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let constant: Value = serde_json::from_slice(&constant_output).unwrap();
        assert_incomplete_live(&constant);
        assert!(
            constant["report"]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|candidate| candidate["results"].as_array().unwrap())
                .all(|result| result["evidence"]["task_executed"] == false
                    && result["evidence"]["recommendation_eligible"] == false)
        );
        let constant_mcp = planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "mcp"])
            .write_stdin(format!(
                "{}\n",
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"planr_preset_evaluate","arguments":{"at_unix":1783987200,"live_host_command":"/usr/bin/printf","live_host_args":[&constant_response]}}})
            ))
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let constant_mcp: Value = serde_json::from_slice(&constant_mcp).unwrap();
        let constant_mcp: Value = serde_json::from_str(
            constant_mcp["result"]["content"][0]["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_incomplete_live(&constant_mcp);
    }

    let stale_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "evaluate",
            "--at-unix",
            "1815523201",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stale: Value = serde_json::from_slice(&stale_output).unwrap();
    assert!(
        stale["report"]["recommended"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        stale["report"]["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|candidate| candidate["status"] == "stale")
    );

    let incompatible_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "evaluate",
            "--host",
            "cursor",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let incompatible: Value = serde_json::from_slice(&incompatible_output).unwrap();
    assert!(
        incompatible["report"]["recommended"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        incompatible["report"]["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|candidate| candidate["status"] == "incompatible")
    );

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "agents", "preset", "evaluate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# Preset Evaluation Verification"))
        .stdout(predicate::str::contains("offline policy simulation"));
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
id = "balanced"
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
    assert_eq!(shown["policy"]["id"], "balanced");
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
    assert_eq!(checked["policy_id"], "balanced");

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
    for skill in [
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
        // Cursor-format subagent roles ship next to the Codex TOMLs and are
        // registered by the root .cursor-plugin manifest.
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
}

#[test]
fn preset_registry_cli_and_mcp_are_preview_first_integrity_checked_and_offline() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let content = dir.path().join("download");
    fs::create_dir_all(content.join("pack")).unwrap();
    let policy = include_str!("../presets/policies/balanced.toml");
    let binding = include_str!("../presets/bindings/codex-openai.toml");
    let verification = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "evaluate",
            "--at-unix",
            "1783987200",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    fs::write(content.join("pack/policy.toml"), policy).unwrap();
    fs::write(content.join("pack/binding.toml"), binding).unwrap();
    fs::write(content.join("pack/verification.json"), &verification).unwrap();
    fs::write(content.join("not-declared.txt"), "must not enter cache").unwrap();
    let policy_hash = Sha256::digest(policy.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let binding_hash = Sha256::digest(binding.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let verification_hash = Sha256::digest(&verification)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let manifest = dir.path().join("registry.toml");
    fs::write(
        &manifest,
        format!(
            r#"schema_version = 1
id = "official"
version = "2026.07"
generated_at_unix = 1783987200

[[entries]]
id = "balanced-codex"
version = "1.0.0"
kind = "pack"
lifecycle = "published"
verification_status = "recommended"
verified_at_unix = 1783987200
review_at_unix = 1815523200
compatible_hosts = ["codex"]
min_planr_version = "1.3.0"
max_planr_version = "1.9.0"
verification_path = "pack/verification.json"

[entries.evaluation]
policy_id = "balanced"
policy_version = "1.0.0"
binding_id = "codex-openai"
binding_version = "1.0.0"
suite_id = "planr-preset-suite"
suite_version = "1.8.0"

[[entries.artifacts]]
path = "pack/policy.toml"
kind = "policy"
sha256 = "{policy_hash}"
size_bytes = {}

[[entries.artifacts]]
path = "pack/binding.toml"
kind = "host-binding"
sha256 = "{binding_hash}"
size_bytes = {}

[[entries.artifacts]]
path = "pack/verification.json"
kind = "verification"
sha256 = "{verification_hash}"
size_bytes = {}
"#,
            policy.len(),
            binding.len(),
            verification.len()
        ),
    )
    .unwrap();

    let verify = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "registry",
            "verify",
            manifest.to_str().unwrap(),
            "--entry",
            "balanced-codex",
            "--content-root",
            content.to_str().unwrap(),
            "--host",
            "codex",
            "--at-unix",
            "1783987200",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let verify: Value = serde_json::from_slice(&verify).unwrap();
    assert_eq!(verify["integrity_verified"], true);
    assert_eq!(verify["effective_status"], "verified");
    assert_eq!(verify["recommended"], false);

    let preview = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "registry",
            "import",
            manifest.to_str().unwrap(),
            "--entry",
            "balanced-codex",
            "--content-root",
            content.to_str().unwrap(),
            "--host",
            "codex",
            "--at-unix",
            "1783987200",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&preview).unwrap();
    assert_eq!(preview["action"], "preview");
    assert!(!dir.path().join(".planr/registry/cache").exists());

    let imported = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "registry",
            "import",
            manifest.to_str().unwrap(),
            "--entry",
            "balanced-codex",
            "--content-root",
            content.to_str().unwrap(),
            "--host",
            "codex",
            "--at-unix",
            "1783987200",
            "--confirm",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let imported: Value = serde_json::from_slice(&imported).unwrap();
    let cache = dir.path().join(imported["cache_path"].as_str().unwrap());
    assert!(cache.join("content/pack/policy.toml").is_file());
    assert!(cache.join("content/pack/binding.toml").is_file());
    assert!(!cache.join("content/not-declared.txt").exists());

    fs::remove_dir_all(&content).unwrap();
    let mut mcp = planr();
    let output = mcp
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_preset_registry_list","arguments":{"at_unix":1815523201}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let listed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(listed["entries"][0]["freshness"], "stale");

    let tampered_content = "tampered";
    fs::write(cache.join("content/pack/policy.toml"), tampered_content).unwrap();
    let receipt_path = cache.join("cache-receipt.toml");
    let mut receipt: toml::Value =
        toml::from_str(&fs::read_to_string(&receipt_path).unwrap()).unwrap();
    let artifacts = receipt["artifacts"].as_array_mut().unwrap();
    let policy_receipt = artifacts
        .iter_mut()
        .find(|artifact| artifact["path"].as_str() == Some("pack/policy.toml"))
        .unwrap();
    policy_receipt["sha256"] = toml::Value::String(
        Sha256::digest(tampered_content.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    );
    policy_receipt["size_bytes"] = toml::Value::Integer(tampered_content.len() as i64);
    fs::write(&receipt_path, toml::to_string_pretty(&receipt).unwrap()).unwrap();
    let tampered = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "registry",
            "list",
            "--at-unix",
            "1815523201",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let tampered: Value = serde_json::from_slice(&tampered).unwrap();
    assert_eq!(tampered["entries"][0]["integrity_verified"], false);
    assert_eq!(tampered["entries"][0]["usable"], false);

    let secret = "sk-registry-metadata-secret";
    let secret_manifest = dir.path().join("registry-secret.toml");
    let secret_raw = fs::read_to_string(&manifest).unwrap().replacen(
        "version = \"2026.07\"",
        &format!("version = \"{secret}\""),
        1,
    );
    fs::write(&secret_manifest, secret_raw).unwrap();
    let cli_rejection = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "agents",
            "preset",
            "registry",
            "verify",
            secret_manifest.to_str().unwrap(),
            "--entry",
            "balanced-codex",
            "--content-root",
            content.to_str().unwrap(),
            "--host",
            "codex",
            "--at-unix",
            "1783987200",
        ])
        .output()
        .unwrap();
    assert!(!cli_rejection.status.success());
    let cli_rejection = format!(
        "{}{}",
        String::from_utf8_lossy(&cli_rejection.stdout),
        String::from_utf8_lossy(&cli_rejection.stderr)
    );
    assert!(cli_rejection.contains("secret-like"));
    assert!(!cli_rejection.contains(secret));

    let mcp_rejection = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"planr_preset_registry_verify","arguments":{"manifest":secret_manifest,"entry":"balanced-codex","content_root":content,"host":"codex","at_unix":1783987200}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_rejection = String::from_utf8(mcp_rejection).unwrap();
    assert!(mcp_rejection.contains("secret-like"));
    assert!(!mcp_rejection.contains(secret));
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
        ".cursor/agents/planr-worker.md",
        ".cursor/agents/planr-reviewer.md",
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

    // Plugin-style install: --no-mcp writes subagent roles and skills but no
    // MCP config, for setups that use skills and agents without MCP.
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
        ".cursor/skills/planr-work/SKILL.md",
    ] {
        assert!(
            no_mcp.path().join(provisioned).exists(),
            "install cursor --no-mcp should write {provisioned}"
        );
    }
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
        ("src/cli.rs", 1_000usize),
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
