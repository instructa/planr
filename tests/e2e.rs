use assert_cmd::Command;
use planr::codex_compat::{
    CODEX_0145_HOOK_EVENTS, CODEX_0145_PERMISSION_MODES, validate_codex_0145_stop_input,
    validate_codex_0145_stop_output,
};
use planr::usage_policy::{BudgetProvenance, FeatureRunBudgetContract, MeteringProvenance};
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

fn planr() -> Command {
    let mut cmd = Command::cargo_bin("planr").expect("planr binary");
    scrub_planr_test_environment(&mut cmd);
    cmd
}

#[test]
fn inconsistent_verification_restart_retires_atomically_and_successor_starts_only_on_ordinary_pick()
{
    let dir = tempdir().unwrap();
    let db = dir.path().join("inconsistent-verification.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "project",
            "init",
            "Inconsistent verification",
        ])
        .assert()
        .success();
    let product = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                &db_arg,
                "--json",
                "plan",
                "new",
                "Inconsistent verification plan",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let build = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                &db_arg,
                "--json",
                "plan",
                "split",
                product["plan"]["id"].as_str().unwrap(),
                "--slice",
                "Inconsistent verification build",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let plan_id = build["plan"]["id"].as_str().unwrap().to_string();
    let plan_path = build["plan"]["path"].as_str().unwrap().to_string();
    let criterion_id = "criterion-inconsistent-verification-build";
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let contract = test_unbounded_feature_run_contract("run-inconsistent");
    conn.execute("INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, budget_contract_digest, source_revision, outcomes_settled, batch_outcome_count) VALUES ('run-inconsistent', ?1, ?2, 'active', 'verification', 'sha256:policy', ?3, 'source-current', 1, 1)", rusqlite::params![project_id, plan_id, contract.digest]).unwrap();
    insert_test_feature_run_contract(&conn, &contract);
    conn.execute("INSERT INTO execution_batches(id, run_id, maker_worker_id, status) VALUES ('batch-inconsistent', 'run-inconsistent', 'historical-maker', 'active')", []).unwrap();
    conn.execute("UPDATE feature_runs SET active_batch_id = 'batch-inconsistent' WHERE id = 'run-inconsistent'", []).unwrap();
    conn.execute("INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation) VALUES ('run-inconsistent', 'verifier', 'verifier-current', 1)", []).unwrap();
    conn.execute("INSERT INTO feature_run_source_freezes(id, run_id, source_revision, source_digest, status) VALUES ('freeze-inconsistent', 'run-inconsistent', 'source-current', 'sha256:source', 'active')", []).unwrap();
    conn.execute("INSERT INTO feature_run_budget_reservations(id, run_id, contract_digest, phase, boundary_key, owner_role, owner_worker_id, lease_generation, status, started_at_unix_ms, provenance) VALUES ('reservation-inconsistent', 'run-inconsistent', ?1, 'verification', 'verification:current', 'verifier', 'verifier-current', 1, 'active', 1700000000000, 'test.inconsistent')", [&contract.digest]).unwrap();
    conn.execute("INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at) VALUES ('item-verification-current', ?1, 'Verify', 'current projection', 'picked', 'verification', 'verifier-current', ?2, datetime('now'), datetime('now'))", rusqlite::params![project_id, plan_path]).unwrap();
    conn.execute("INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at) VALUES ('item-successor', ?1, 'Continue', 'ordinary successor owner', 'ready', 'code', ?2, datetime('now'), datetime('now'))", rusqlite::params![project_id, plan_path]).unwrap();
    conn.execute("INSERT INTO execution_run_outcomes(id, run_id, batch_id, item_id, ordinal, outcome_json) VALUES ('outcome-preserved', 'run-inconsistent', 'batch-inconsistent', 'historical-item', 1, '{}')", []).unwrap();
    conn.execute("INSERT INTO proof_obligations(id, project_id, plan_id, criterion_id, obligation_version, title, binding, observation_requirements_json, fixture_policy_json, freshness_policy_json, assurance_policy_json, policy_digest, config_digest, created_at) VALUES ('pob-preserved', ?1, ?2, ?3, 1, 'Preserved Evidence', 1, '[]', '{}', '{}', '{}', 'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', datetime('now'))", rusqlite::params![project_id, plan_id, criterion_id]).unwrap();
    let admission = json!({"plan_id":plan_id,"run_id":"run-inconsistent","freeze_id":"freeze-inconsistent","run_revision":0,"verifier_worker_id":"verifier-current","verifier_lease_generation":1,"verification_item_id":"item-verification-current","run_index_digest":"sha256:index","sealed_run_index":{"run_index_digest":"sha256:index"}});
    conn.execute("INSERT INTO events(project_id, item_id, worker_id, event_type, payload, timestamp) VALUES (?1, 'item-verification-current', 'verifier-current', 'feature_run_verification_admitted', ?2, datetime('now'))", rusqlite::params![project_id, admission.to_string()]).unwrap();
    drop(conn);

    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "verifier-current")
        .args([
            "--db",
            &db_arg,
            "--json",
            "run",
            "restart",
            "--plan",
            &plan_id,
            "--reason",
            "inconsistent-verification",
        ])
        .assert()
        .failure();
    let conn = Connection::open(&db).unwrap();
    conn.execute("UPDATE events SET payload = json_set(payload, '$.run_revision', 99) WHERE event_type = 'feature_run_verification_admitted'", []).unwrap();
    drop(conn);
    let retired = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "verifier-current")
            .args([
                "--db",
                &db_arg,
                "--json",
                "run",
                "restart",
                "--plan",
                &plan_id,
                "--reason",
                "inconsistent-verification",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(
        retired["restart"]["facts"]["diagnosis"]["inconsistency"],
        "admission_revision_mismatch"
    );
    assert_eq!(retired["restart"]["batch_effect"]["disposition"], "ended");
    assert_eq!(
        retired["restart"]["released_verification_reservation_ids"],
        json!(["reservation-inconsistent"])
    );
    assert_eq!(
        retired["restart"]["released_verification_item_id"],
        "item-verification-current"
    );
    assert!(retired["restart"]["successor_run_id"].is_null());
    let conn = Connection::open(&db).unwrap();
    let state: Value = serde_json::from_str(&conn.query_row("SELECT json_object('run',(SELECT status||':'||phase||':'||coalesce(active_batch_id,'null') FROM feature_runs WHERE id='run-inconsistent'),'batch',(SELECT status FROM execution_batches WHERE id='batch-inconsistent'),'freeze',(SELECT status FROM feature_run_source_freezes WHERE id='freeze-inconsistent'),'roles',(SELECT COUNT(*) FROM feature_run_role_leases WHERE run_id='run-inconsistent' AND released_at IS NULL),'reservation',(SELECT status FROM feature_run_budget_reservations WHERE id='reservation-inconsistent'),'item',(SELECT status||':'||coalesce(worker_id,'null') FROM items WHERE id='item-verification-current'),'outcomes',(SELECT COUNT(*) FROM execution_run_outcomes WHERE id='outcome-preserved'),'evidence',(SELECT COUNT(*) FROM proof_obligations WHERE id='pob-preserved'),'admissions',(SELECT COUNT(*) FROM events WHERE event_type='feature_run_verification_admitted'),'runs',(SELECT COUNT(*) FROM feature_runs))", [], |row| row.get::<_, String>(0)).unwrap()).unwrap();
    assert_eq!(
        state,
        json!({"run":"cancelled:cancelled:null","batch":"ended","freeze":"invalidated","roles":0,"reservation":"released","item":"ready:null","outcomes":1,"evidence":1,"admissions":1,"runs":1})
    );
    drop(conn);

    let successor = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "ordinary-maker")
            .args([
                "--db",
                &db_arg,
                "--json",
                "pick",
                "--plan",
                &plan_id,
                "--work-type",
                "code",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(successor["work_packet"]["kind"], "outcome");
    assert_eq!(successor["item"]["id"], "item-successor");
    assert_eq!(successor["item"]["status"], "picked");
    let successor_run_id = successor["work_packet"]["execution_state"]["feature_run"]["id"]
        .as_str()
        .expect("ordinary outcome pick must return a successor run id");
    assert_ne!(successor_run_id, "run-inconsistent");
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM feature_runs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn verification_admission_repair_settles_refreezes_and_replays_idempotently() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    write_evidence_policy_fixture(dir.path());
    write_materiality_policy(dir.path());
    let run = |worker: &str, args: &[&str], succeeds: bool| -> Value {
        let mut command = planr();
        command
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", worker)
            .args(["--db", &db_arg, "--json"])
            .args(args);
        let assertion = command.assert();
        let output = if succeeds {
            assertion.success()
        } else {
            assertion.failure()
        }
        .get_output()
        .stdout
        .clone();
        single_json_document(&output)
    };
    run(
        "fixture",
        &["project", "init", "Verification admission settlement"],
        true,
    );
    let plan_path = dir.path().join("verification-admission-settlement.plan.md");
    fs::write(&plan_path, "---\ncriteria:\n  - id: criterion-verification-admission-settlement\n    title: Verification admission settlement\n---\n# Verification admission settlement\n").unwrap();
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute("INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at) VALUES ('plan-verification-admission-settlement', ?1, 'build', ?2, 'Verification admission settlement', 'verification-admission-settlement', 'ok', 'sha256:fixture', datetime('now'), datetime('now'))", rusqlite::params![project_id, plan_path.to_string_lossy()]).unwrap();
    conn.execute("INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at) VALUES ('item-verification-admission-build', ?1, 'Build', 'fixture outcome', 'picked', 'code', 'maker-verification-admission', ?2, datetime('now'), datetime('now'))", rusqlite::params![project_id, plan_path.to_string_lossy()]).unwrap();
    drop(conn);
    let mut obligation = evidence_obligation(
        "pob-verification-admission-settlement",
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        json!({"kind":"local","id":"verification-admission-settlement"}),
    );
    obligation["plan_id"] = json!("plan-verification-admission-settlement");
    obligation["item_id"] = Value::Null;
    obligation["criterion_id"] = json!("criterion-verification-admission-settlement");
    add_evidence_obligation_value(
        dir.path(),
        &db,
        "pob-verification-admission-settlement",
        &obligation,
    );
    init_git_repo(dir.path());

    let done = run(
        "maker-verification-admission",
        &[
            "done",
            "item-verification-admission-build",
            "--summary",
            "built admission repair",
            "--cmd",
            "true",
            "--tests",
            "true",
            "--next",
        ],
        true,
    );
    assert_eq!(
        done["next"]["work_packet"]["verification_item_id"],
        Value::Null
    );
    let held = run(
        "verifier-verification-admission",
        &[
            "pick",
            "--plan",
            "plan-verification-admission-settlement",
            "--work-type",
            "verification",
        ],
        false,
    );
    let details = &held["error"]["details"];
    let request = &details["repair_request"];
    assert_eq!(request["reason"], "readiness-blocked");
    assert!(request["run_index_digest"].is_null());
    assert_eq!(details["execution_state"]["phase"], "held");
    assert_eq!(
        details["execution_state"]["verification_admission_repair"],
        *request
    );
    assert!(
        details["execution_state"]["next_action"]
            .as_str()
            .unwrap()
            .contains("run repair-verification-admission")
    );
    assert!(
        !details
            .to_string()
            .contains("selective_replay_obligation_ids")
    );
    let run_id = request["run_id"].as_str().unwrap().to_string();
    let freeze_id = request["freeze_id"].as_str().unwrap().to_string();
    let revision = request["run_revision"].as_u64().unwrap().to_string();
    let repaired = run(
        "verifier-verification-admission",
        &[
            "run",
            "repair-verification-admission",
            "--plan",
            "plan-verification-admission-settlement",
            "--run",
            &run_id,
            "--freeze",
            &freeze_id,
            "--revision",
            &revision,
            "--reason",
            "readiness-blocked",
        ],
        true,
    );
    assert_eq!(
        repaired["repair"]["repaired_run"]["phase"],
        "implementation"
    );
    let invalidation_id = repaired["repair"]["facts"]["invalidation_id"]
        .as_str()
        .unwrap()
        .to_string();
    let repair_batch_id = repaired["repair"]["facts"]["repair_batch_id"]
        .as_str()
        .unwrap()
        .to_string();
    let packet = run(
        "maker-verification-admission",
        &[
            "pick",
            "--plan",
            "plan-verification-admission-settlement",
            "--work-type",
            "code",
        ],
        true,
    );
    assert_eq!(
        packet["work_packet"]["mode"],
        "verification_admission_repair"
    );
    assert_eq!(packet["work_packet"]["repair_id"], invalidation_id);
    assert_eq!(
        packet["work_packet"]["execution_state"]["next_action"],
        "settle_verification_admission_repair"
    );
    assert!(
        packet["work_packet"]
            .get("selective_replay_obligation_ids")
            .is_none()
    );

    let settle = || {
        run(
            "maker-verification-admission",
            &[
                "run",
                "settle-repair",
                "--plan",
                "plan-verification-admission-settlement",
                "--invalidation",
                &invalidation_id,
                "--summary",
                "settled typed admission repair",
                "--files",
                "tests/e2e.rs",
                "--cmd",
                "cargo check --bin planr",
                "--tests",
                "focused invariant passed",
            ],
            true,
        )
    };
    let first = settle();
    assert_eq!(first["created"], true);
    assert_eq!(first["reason"], "verification_handoff_source_frozen");
    assert_eq!(first["item"], Value::Null);
    assert_eq!(first["work_packet"]["kind"], "verification_handoff");
    assert_eq!(first["work_packet"]["verification_item_id"], Value::Null);
    for forbidden in [
        "mode",
        "repair_id",
        "responsible_maker_id",
        "selective_replay_obligation_ids",
    ] {
        assert!(
            first["work_packet"].get(forbidden).is_none(),
            "{forbidden}: {first}"
        );
    }
    let durable = || -> Value {
        let conn = Connection::open(&db).unwrap();
        let value: String = conn.query_row("SELECT json_object('phase',(SELECT phase FROM feature_runs WHERE id=?1),'active_batch',(SELECT active_batch_id FROM feature_runs WHERE id=?1),'batch_status',(SELECT status FROM execution_batches WHERE id=?2),'settlements',(SELECT COUNT(*) FROM feature_run_verification_admission_repair_settlements WHERE invalidation_id=?3),'settlement_events',(SELECT COUNT(*) FROM events WHERE event_type='feature_run_verification_admission_repair_settled'),'repair_events',(SELECT COUNT(*) FROM events WHERE event_type='feature_run_verification_admission_repaired'),'invalidations',(SELECT COUNT(*) FROM feature_run_evidence_invalidations WHERE run_id=?1),'freezes',(SELECT COUNT(*) FROM feature_run_source_freezes WHERE run_id=?1),'active_freezes',(SELECT COUNT(*) FROM feature_run_source_freezes WHERE run_id=?1 AND status='active'),'batches',(SELECT COUNT(*) FROM execution_batches WHERE run_id=?1),'reservations',(SELECT COUNT(*) FROM feature_run_budget_reservations WHERE run_id=?1),'observations',(SELECT COUNT(*) FROM feature_run_budget_observations WHERE run_id=?1))", rusqlite::params![run_id, repair_batch_id, invalidation_id], |row| row.get(0)).unwrap();
        serde_json::from_str(&value).unwrap()
    };
    let after_first = durable();
    assert_eq!(after_first["phase"], "source_frozen");
    assert_eq!(after_first["active_batch"], repair_batch_id);
    assert_eq!(after_first["batch_status"], "ended");
    assert_eq!(
        (
            after_first["settlements"].clone(),
            after_first["settlement_events"].clone(),
            after_first["repair_events"].clone()
        ),
        (json!(1), json!(1), json!(1))
    );
    assert_eq!(
        (
            after_first["invalidations"].clone(),
            after_first["freezes"].clone(),
            after_first["active_freezes"].clone()
        ),
        (json!(1), json!(2), json!(1))
    );
    let replay = settle();
    assert_eq!(replay["created"], false);
    assert_eq!(
        serde_json::to_vec(&replay["settlement"]).unwrap(),
        serde_json::to_vec(&first["settlement"]).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&replay["work_packet"]).unwrap(),
        serde_json::to_vec(&first["work_packet"]).unwrap()
    );
    assert_eq!(durable(), after_first);
    let no_offer = run(
        "maker-verification-admission",
        &[
            "pick",
            "--plan",
            "plan-verification-admission-settlement",
            "--work-type",
            "code",
        ],
        true,
    );
    assert_eq!(no_offer["item"], Value::Null);
    assert!(
        !no_offer
            .to_string()
            .contains("verification_admission_repair")
    );
    assert_eq!(durable(), after_first);
}

fn planr_from_binary(binary: &Path) -> Command {
    let mut cmd = Command::new(binary);
    scrub_planr_test_environment(&mut cmd);
    cmd
}

const PLANR_TEST_ENV_VARS: [&str; 8] = [
    "CODEX_SANDBOX",
    "CODEX_SESSION_ID",
    "CLAUDECODE",
    "CURSOR_AGENT",
    "CURSOR_INVOKED_AS",
    "PLANR_MCP_CLIENT",
    "PI_CODING_AGENT",
    "PLANR_PROFILE",
];

fn scrub_planr_test_environment(cmd: &mut Command) {
    // Tests may run inside a real host session (Cursor terminal, Codex
    // sandbox); scrub the host-identifying vars observed_client() reads
    // so detection is opt-in per test and results are deterministic.
    for var in PLANR_TEST_ENV_VARS {
        cmd.env_remove(var);
    }
}

fn std_planr_from_binary(binary: &Path) -> StdCommand {
    let mut cmd = StdCommand::new(binary);
    for var in PLANR_TEST_ENV_VARS {
        cmd.env_remove(var);
    }
    cmd
}

fn private_planr_binary(root: &Path) -> PathBuf {
    let source = assert_cmd::cargo::cargo_bin("planr");
    private_planr_binary_from(root, &source)
}

fn private_planr_binary_from(root: &Path, source: &Path) -> PathBuf {
    let destination = root.join(".planr/test-bin/planr");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::copy(source, &destination).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o555)).unwrap();
    #[cfg(not(unix))]
    {
        let mut permissions = fs::metadata(&destination).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&destination, permissions).unwrap();
    }
    assert_eq!(fs::read(&destination).unwrap(), fs::read(source).unwrap());
    #[cfg(unix)]
    assert_eq!(fs::metadata(&destination).unwrap().nlink(), 1);
    destination
}

#[cfg(unix)]
#[test]
fn private_planr_binary_copy_is_single_link_when_cargo_source_has_two_links() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("cargo-planr");
    let second_link = dir.path().join("cargo-planr-hardlink");
    fs::copy(assert_cmd::cargo::cargo_bin("planr"), &source).unwrap();
    fs::hard_link(&source, &second_link).unwrap();
    assert_eq!(fs::metadata(&source).unwrap().nlink(), 2);

    let private = private_planr_binary_from(dir.path(), &source);
    assert_eq!(fs::metadata(private).unwrap().nlink(), 1);
    assert_eq!(fs::metadata(source).unwrap().nlink(), 2);
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn wait_for_http_server(port: u16) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => {
                drop(stream);
                return;
            }
            Err(_) if std::time::Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("HTTP server did not become ready: {error}"),
        }
    }
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

fn single_json_document(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("stdout must be exactly one JSON document")
}

fn test_unbounded_feature_run_contract(run_id: &str) -> FeatureRunBudgetContract {
    FeatureRunBudgetContract::unbounded(
        run_id,
        1_700_000_000_000,
        BudgetProvenance {
            wall_seconds: MeteringProvenance::Trusted,
            tool_calls: MeteringProvenance::Unavailable,
            tokens: MeteringProvenance::Unavailable,
        },
    )
    .expect("valid test budget contract")
}

fn insert_test_feature_run_contract(conn: &Connection, contract: &FeatureRunBudgetContract) {
    conn.execute(
        "INSERT INTO feature_run_budget_contracts(run_id, schema, digest, contract_json) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            contract.run_id,
            contract.schema,
            contract.digest,
            serde_json::to_string(contract).unwrap()
        ],
    )
    .expect("test budget contract");
}

fn codex_stop_envelope(dir: &Path, session: &str, stop_hook_active: bool) -> String {
    json!({
        "session_id": session,
        "transcript_path": dir.join("codex-transcript.jsonl").to_string_lossy(),
        "cwd": dir.to_string_lossy(),
        "hook_event_name": "Stop",
        "model": "gpt-5.6-sol",
        "permission_mode": "plan",
        "turn_id": "turn-1",
        "stop_hook_active": stop_hook_active,
        "last_assistant_message": "done"
    })
    .to_string()
}

fn assert_codex_stop_input_schema(value: &Value) {
    validate_codex_0145_stop_input(value).unwrap_or_else(|err| panic!("{err}: {value}"));
}

fn assert_codex_stop_output_schema(value: &Value, expect_block: bool) {
    validate_codex_0145_stop_output(value, expect_block)
        .unwrap_or_else(|err| panic!("{err}: {value}"));
}

fn assert_codex_hooks_config_schema(value: &Value) {
    let object = value.as_object().expect("Codex hooks config object");
    let hooks = object
        .get("hooks")
        .and_then(Value::as_object)
        .expect("Codex hooks object");
    for key in hooks.keys() {
        assert!(
            CODEX_0145_HOOK_EVENTS.contains(&key.as_str()),
            "unsupported Codex hook event in active config: {key}"
        );
    }
    for (event, entries) in hooks {
        let entries = entries
            .as_array()
            .unwrap_or_else(|| panic!("{event} entries must be array: {value}"));
        for entry in entries {
            let hooks = entry["hooks"]
                .as_array()
                .unwrap_or_else(|| panic!("{event} entry hooks must be array: {entry}"));
            for hook in hooks {
                assert_eq!(hook["type"], "command", "{hook}");
                assert!(hook["command"].as_str().is_some_and(|s| !s.is_empty()));
            }
        }
    }
}

fn assert_unsupported_codex_sidecar_contains(value: &Value, event: &str, command: &str) {
    assert!(
        value[event]
            .as_array()
            .unwrap_or_else(|| panic!("{event} sidecar value must be an array: {value}"))
            .iter()
            .any(|entry| entry
                .pointer("/hooks/0/command")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains(command))),
        "{event} sidecar missing {command}: {value}"
    );
}

fn assert_codex_0145_stop_schema_fixture(dir: &Path) {
    for mode in CODEX_0145_PERMISSION_MODES {
        let mut envelope: Value =
            serde_json::from_str(&codex_stop_envelope(dir, &format!("session-{mode}"), false))
                .unwrap();
        envelope["permission_mode"] = json!(mode);
        assert_codex_stop_input_schema(&envelope);
    }
    let mut invalid: Value =
        serde_json::from_str(&codex_stop_envelope(dir, "session-invalid", false)).unwrap();
    invalid["permission_mode"] = json!("workspace-write");
    assert!(
        validate_codex_0145_stop_input(&invalid).is_err(),
        "workspace-write is not a Codex 0.145 permission mode"
    );
    assert!(CODEX_0145_HOOK_EVENTS.contains(&"PermissionRequest"));
    assert!(CODEX_0145_HOOK_EVENTS.contains(&"PostCompact"));
    assert!(CODEX_0145_HOOK_EVENTS.contains(&"SessionEnd"));
    assert!(CODEX_0145_HOOK_EVENTS.contains(&"SubagentStart"));
    assert!(
        !CODEX_0145_HOOK_EVENTS.contains(&"Notification"),
        "Notification is not supported by Codex 0.145 hooks"
    );
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
        sort_object_array_by_string_field(object, "fixtures", "id");
        sort_scorers_by_id_version(object);
        sort_object_array_by_string_field(object, "cases", "case_id");
        if let Some(cases) = object.get_mut("cases").and_then(Value::as_array_mut) {
            for case in cases {
                let Some(case_object) = case.as_object_mut() else {
                    continue;
                };
                sort_string_array_field(case_object, "fixture_ids");
                sort_string_array_field(case_object, "scorer_ids");
                sort_string_array_field(case_object, "measures");
            }
        }
    }
    let bytes = serde_jcs::to_vec(&value).unwrap();
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    value["digest"] = json!(digest);
    value
}

fn with_authored_order_and_canonical_digest(mut value: Value) -> Value {
    let digest = with_canonical_digest(value.clone())["digest"].clone();
    value["digest"] = digest;
    value
}

fn sort_scorers_by_id_version(object: &mut serde_json::Map<String, Value>) {
    if let Some(values) = object.get_mut("scorers").and_then(Value::as_array_mut) {
        values.sort_by(|left, right| {
            scorer_id_version_key(left)
                .as_str()
                .cmp(scorer_id_version_key(right).as_str())
        });
    }
}

fn scorer_id_version_key(value: &Value) -> String {
    let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("{id}@{version}")
}

fn sort_object_array_by_string_field(
    object: &mut serde_json::Map<String, Value>,
    array_field: &str,
    key_field: &str,
) {
    if let Some(values) = object.get_mut(array_field).and_then(Value::as_array_mut) {
        values.sort_by(|left, right| {
            let left_key = left
                .get(key_field)
                .and_then(Value::as_str)
                .unwrap_or_default();
            let right_key = right
                .get(key_field)
                .and_then(Value::as_str)
                .unwrap_or_default();
            left_key.cmp(right_key)
        });
    }
}

fn sort_string_array_field(object: &mut serde_json::Map<String, Value>, field: &str) {
    if let Some(values) = object.get_mut(field).and_then(Value::as_array_mut) {
        values.sort_by(|left, right| {
            let left = left.as_str().unwrap_or_default();
            let right = right.as_str().unwrap_or_default();
            left.cmp(right)
        });
    }
}

fn with_legacy_eval_digest(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.remove("digest");
    }
    let bytes = serde_json::to_vec(&legacy_sorted_json_value(&value)).unwrap();
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    value["digest"] = json!(digest);
    value
}

fn legacy_sorted_json_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(legacy_sorted_json_value).collect()),
        Value::Object(object) => {
            let mut entries = object
                .iter()
                .map(|(key, value)| (key.clone(), legacy_sorted_json_value(value)))
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

fn mcp_tool_response_with_env(
    dir: &Path,
    db: &Path,
    id: u64,
    name: &str,
    arguments: Value,
    env: &[(&str, &str)],
) -> Value {
    mcp_tool_response_with_env_and_binary(dir, db, id, name, arguments, env, None)
}

fn mcp_tool_response_with_env_and_binary(
    dir: &Path,
    db: &Path,
    id: u64,
    name: &str,
    arguments: Value,
    env: &[(&str, &str)],
    binary: Option<&Path>,
) -> Value {
    let input = format!(
        "{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        })
    );
    let mut command = binary.map_or_else(planr, planr_from_binary);
    command.current_dir(dir);
    command.args(["--db", db.to_str().unwrap(), "mcp"]);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).unwrap()
}

fn mcp_tool_response(dir: &Path, db: &Path, id: u64, name: &str, arguments: Value) -> Value {
    mcp_tool_response_with_env(dir, db, id, name, arguments, &[])
}

fn mcp_tool(dir: &Path, db: &Path, id: u64, name: &str, arguments: Value) -> Value {
    let response = mcp_tool_response(dir, db, id, name, arguments);
    mcp_text_value(&response)
}

fn assert_evidence_envelope(value: &Value, command: &str, ok: bool) {
    assert_eq!(value["schema"], "planr.evidence.command.v1");
    assert_eq!(value["command"], command);
    assert_eq!(value["ok"], ok, "{value}");
    assert!(value["exit"]["code"].is_i64());
    if ok {
        assert!(value.get("object").is_some());
    } else {
        assert!(value["error"]["message"].is_string());
    }
}

fn assert_evidence_error(value: &Value, command: &str, code: &str, message: &str) {
    assert_evidence_envelope(value, command, false);
    assert_eq!(value["error"]["code"], code, "{value}");
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains(message),
        "{value}"
    );
}

fn assert_mcp_evidence_error(response: &Value, command: &str, code: &str, message: &str) -> Value {
    assert_eq!(response["result"]["isError"], true, "{response}");
    let envelope = mcp_text_value(response);
    assert_evidence_error(&envelope, command, code, message);
    envelope
}

fn assert_http_evidence_error(
    response: &str,
    status: &str,
    command: &str,
    code: &str,
    message: &str,
) -> Value {
    assert!(
        response.starts_with(&format!("HTTP/1.1 {status}")),
        "{response}"
    );
    let envelope = http_json(response);
    assert_evidence_error(&envelope, command, code, message);
    envelope
}

fn evidence_obligation_count_like(db: &Path, id_pattern: &str) -> i64 {
    let conn = Connection::open(db).unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM proof_obligations WHERE id LIKE ?1",
        [id_pattern],
        |row| row.get(0),
    )
    .unwrap()
}

fn evidence_attempt_receipt_counts(db: &Path, obligation_id: &str) -> (i64, i64) {
    let conn = Connection::open(db).unwrap();
    let attempts = conn
        .query_row(
            "SELECT COUNT(*) FROM evidence_attempts WHERE obligation_id = ?1",
            [obligation_id],
            |row| row.get(0),
        )
        .unwrap();
    let receipts = conn
        .query_row(
            "SELECT COUNT(*) FROM evidence_receipts WHERE obligation_id = ?1",
            [obligation_id],
            |row| row.get(0),
        )
        .unwrap();
    (attempts, receipts)
}

fn latest_evidence_attempt_for_obligation(db: &Path, obligation_id: &str) -> Value {
    let conn = Connection::open(db).unwrap();
    let attempt_json: String = conn
        .query_row(
            "SELECT attempt_json FROM evidence_attempts WHERE obligation_id = ?1 ORDER BY created_at DESC LIMIT 1",
            [obligation_id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&attempt_json).unwrap()
}

fn evidence_observe_snapshot(db: &Path, item_id: &str) -> Value {
    let conn = Connection::open(db).unwrap();
    let item_ids = [item_id.to_string()];
    let quoted_ids = item_ids
        .iter()
        .map(|id| format!("'{}'", id.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    let item_rows = query_snapshot_rows(
        &conn,
        &format!(
            "SELECT id, project_id, parent_item_id, title, description, status, work_type,
                    priority, worker_id, plan_path, pick_token, picked_at, last_heartbeat_at,
                    progress_percent, progress_note, paused_at, timeout_seconds, max_retries,
                    retry_count, retry_backoff, retry_delay_ms, pre_condition, post_condition,
                    approval_status, approval_requested_at, approved_by, approval_comment,
                    started_at, completed_at, result, error, metadata, created_at, updated_at
             FROM items WHERE id IN ({quoted_ids}) ORDER BY id"
        ),
        |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "project_id": row.get::<_, String>(1)?,
                "parent_item_id": row.get::<_, Option<String>>(2)?,
                "title": row.get::<_, String>(3)?,
                "description": row.get::<_, String>(4)?,
                "status": row.get::<_, String>(5)?,
                "work_type": row.get::<_, String>(6)?,
                "priority": row.get::<_, i64>(7)?,
                "worker_id": row.get::<_, Option<String>>(8)?,
                "plan_path": row.get::<_, Option<String>>(9)?,
                "pick_token": row.get::<_, Option<String>>(10)?,
                "picked_at": row.get::<_, Option<String>>(11)?,
                "last_heartbeat_at": row.get::<_, Option<String>>(12)?,
                "progress_percent": row.get::<_, Option<i64>>(13)?,
                "progress_note": row.get::<_, Option<String>>(14)?,
                "paused_at": row.get::<_, Option<String>>(15)?,
                "timeout_seconds": row.get::<_, Option<i64>>(16)?,
                "max_retries": row.get::<_, i64>(17)?,
                "retry_count": row.get::<_, i64>(18)?,
                "retry_backoff": row.get::<_, String>(19)?,
                "retry_delay_ms": row.get::<_, i64>(20)?,
                "pre_condition": row.get::<_, Option<String>>(21)?,
                "post_condition": row.get::<_, Option<String>>(22)?,
                "approval_status": row.get::<_, Option<String>>(23)?,
                "approval_requested_at": row.get::<_, Option<String>>(24)?,
                "approved_by": row.get::<_, Option<String>>(25)?,
                "approval_comment": row.get::<_, Option<String>>(26)?,
                "started_at": row.get::<_, Option<String>>(27)?,
                "completed_at": row.get::<_, Option<String>>(28)?,
                "result": row.get::<_, Option<String>>(29)?,
                "error": row.get::<_, Option<String>>(30)?,
                "metadata": row.get::<_, Option<String>>(31)?,
                "created_at": row.get::<_, String>(32)?,
                "updated_at": row.get::<_, String>(33)?,
            }))
        },
    );
    let review_links = query_snapshot_rows(
        &conn,
        &format!(
            "SELECT id, from_item, to_item, kind, condition, metadata FROM links
             WHERE (from_item IN ({quoted_ids}) OR to_item IN ({quoted_ids}))
               AND kind IN ('reviews','blocks')
             ORDER BY id"
        ),
        |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "from_item": row.get::<_, String>(1)?,
                "to_item": row.get::<_, String>(2)?,
                "kind": row.get::<_, String>(3)?,
                "condition": row.get::<_, String>(4)?,
                "metadata": row.get::<_, Option<String>>(5)?,
            }))
        },
    );
    let logs = query_snapshot_rows(
        &conn,
        &format!(
            "SELECT id, project_id, item_id, run_id, kind, summary, files, commands,
                    tests, review_findings, blocked_or_unverified, created_at
             FROM logs WHERE item_id IN ({quoted_ids}) ORDER BY id"
        ),
        |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "project_id": row.get::<_, String>(1)?,
                "item_id": row.get::<_, String>(2)?,
                "run_id": row.get::<_, Option<String>>(3)?,
                "kind": row.get::<_, String>(4)?,
                "summary": row.get::<_, String>(5)?,
                "files": row.get::<_, Option<String>>(6)?,
                "commands": row.get::<_, Option<String>>(7)?,
                "tests": row.get::<_, Option<String>>(8)?,
                "review_findings": row.get::<_, Option<String>>(9)?,
                "blocked_or_unverified": row.get::<_, Option<String>>(10)?,
                "created_at": row.get::<_, String>(11)?,
            }))
        },
    );
    let artifacts = query_snapshot_rows(
        &conn,
        &format!(
            "SELECT id, project_id, item_id, name, kind, path, content, mime_type,
                    size_bytes, metadata, created_at
             FROM artifacts WHERE item_id IN ({quoted_ids}) ORDER BY id"
        ),
        |row| {
            let content = row.get::<_, Option<String>>(6)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "project_id": row.get::<_, String>(1)?,
                "item_id": row.get::<_, Option<String>>(2)?,
                "name": row.get::<_, String>(3)?,
                "kind": row.get::<_, Option<String>>(4)?,
                "path": row.get::<_, Option<String>>(5)?,
                "content": content,
                "content_digest": content.as_deref().map(sha256_text),
                "mime_type": row.get::<_, Option<String>>(7)?,
                "size_bytes": row.get::<_, Option<i64>>(8)?,
                "metadata": row.get::<_, Option<String>>(9)?,
                "created_at": row.get::<_, String>(10)?,
            }))
        },
    );
    let contexts = query_snapshot_rows(
        &conn,
        &format!(
            "SELECT id, project_id, item_id, worker_id, kind, content, tags, created_at
             FROM contexts WHERE item_id IN ({quoted_ids}) ORDER BY id"
        ),
        |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "project_id": row.get::<_, String>(1)?,
                "item_id": row.get::<_, Option<String>>(2)?,
                "worker_id": row.get::<_, Option<String>>(3)?,
                "kind": row.get::<_, String>(4)?,
                "content": row.get::<_, String>(5)?,
                "tags": row.get::<_, Option<String>>(6)?,
                "created_at": row.get::<_, String>(7)?,
            }))
        },
    );
    let events = query_snapshot_rows(
        &conn,
        &format!(
            "SELECT id, project_id, item_id, worker_id, event_type, payload, timestamp
             FROM events
             WHERE item_id IN ({quoted_ids})
                OR event_type LIKE 'approval_%'
                OR event_type LIKE 'review_%'
                OR event_type IN ('item_closed','done')
             ORDER BY id"
        ),
        |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "project_id": row.get::<_, Option<String>>(1)?,
                "item_id": row.get::<_, Option<String>>(2)?,
                "worker_id": row.get::<_, Option<String>>(3)?,
                "event_type": row.get::<_, String>(4)?,
                "payload": row.get::<_, Option<String>>(5)?,
                "timestamp": row.get::<_, String>(6)?,
            }))
        },
    );
    json!({
        "item_ids": item_ids,
        "items": item_rows,
        "review_and_block_links": review_links,
        "logs": logs,
        "artifacts": artifacts,
        "contexts": contexts,
        "events": events,
        "excluded_non_authority_tables": [
            "proof_obligations",
            "verification_capability_manifests",
            "verification_capability_instances",
            "evidence_attempts",
            "evidence_receipts",
            "evidence_coverage_history",
            "evidence_validated_imports"
        ],
    })
}

fn query_snapshot_rows<F>(conn: &Connection, sql: &str, mut row_fn: F) -> Vec<Value>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<Value>,
{
    let mut stmt = conn.prepare(sql).unwrap();
    stmt.query_map([], |row| row_fn(row))
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

fn sha256_json(value: &Value) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_jcs::to_vec(value).unwrap())
    )
}

fn sha256_text(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn sha256_json_without_field(value: &Value, field: &str) -> String {
    let mut digest_value = value.clone();
    digest_value.as_object_mut().unwrap().remove(field);
    sha256_json(&digest_value)
}

fn process_adapter_digest(execution: &Value, file_arguments: Vec<Value>) -> String {
    sha256_json(&json!({
        "schema_version": "planr.process_adapter.binding.v1",
        "execution_contract": execution,
        "file_arguments": file_arguments,
    }))
}

fn evidence_runner_manifest() -> (Value, Value, String) {
    let schema = json!({
        "type": "object",
        "required": ["status"],
        "properties": {"status": {"const": "ok"}},
        "additionalProperties": false
    });
    let payload_schema = json!({
        "type": "com.example.health.status",
        "schema_ref": "schema://com.example.health.status",
        "schema_digest": sha256_json(&schema),
    });
    let execution = json!({
        "kind": "process",
        "executable": "sh",
        "args": ["-c", "if [ -n \"${PLANR_FIXTURE_GAP_REASONS:-}\" ]; then printf '{\"planr_adapter_gap_reasons\":%s}\\n' \"$PLANR_FIXTURE_GAP_REASONS\" >&2; exit 2; fi; if [ -n \"${PLANR_FIXTURE_BOUNDARY:-}\" ]; then printf '{\"planr_adapter_boundary\":\"%s\"}\\n' \"$PLANR_FIXTURE_BOUNDARY\" >&2; exit 2; fi; if [ -n \"${PLANR_FIXTURE_EXIT:-}\" ]; then exit \"$PLANR_FIXTURE_EXIT\"; fi; if [ -n \"${PLANR_E2E_WRITE_PLANR_RUNTIME:-}\" ]; then mkdir -p .planr/evidence/runs .planr/evidence/attempts .planr/evidence/receipts .planr/evidence/coverage; printf runtime > .planr/planr.sqlite; printf runtime > .planr/evidence/runs/runtime.txt; printf runtime > .planr/evidence/attempts/runtime.txt; printf runtime > .planr/evidence/receipts/runtime.txt; printf runtime > .planr/evidence/coverage/runtime.txt; fi; if [ -n \"${PLANR_E2E_MUTATE_SOURCE:-}\" ]; then printf mutated > product-source.txt; fi; printf '{\"status\":\"ok\"}'"],
        "working_directory": ".",
        "timeout_ms": 5000,
        "stdout_limit_bytes": 4096,
        "stderr_limit_bytes": 1024,
        "payload_schema": payload_schema,
    });
    let adapter_digest = process_adapter_digest(&execution, vec![]);
    let manifest = json!({
        "id": "verifier-generic-adapter",
        "schema_version": "evidence.contract.v1",
        "version": "1.0.0",
        "adapter_kind": "artifact_import",
        "adapter_digest": adapter_digest,
        "supported_surfaces": ["local-process"],
        "supported_observations": [payload_schema],
        "supported_interactions": ["process", "import"],
        "supported_artifacts": ["stdout", "planr.runner_result.v1"],
        "runtime_targets": [{"kind": "process", "id": "runtime-local"}],
        "provenance_path": "planr_observed_execution",
        "permissions": {"network": "none", "filesystem": "read_workspace"},
        "costs": {},
        "determinism": "deterministic",
        "repeatability": "repeatable",
        "independence": "repository-owned adapter manifest outside Planr core",
        "blind_spots": ["fixture covers public evidence flow"],
        "availability_probe": {
            "kind": "process",
            "execution": execution,
        },
    });
    let manifest_digest = sha256_json(&manifest);
    (schema, manifest, manifest_digest)
}

fn write_evidence_policy_fixture(root: &Path) {
    let evidence_dir = root.join(".planr/evidence");
    fs::create_dir_all(evidence_dir.join("schemas")).unwrap();
    fs::create_dir_all(evidence_dir.join("adapters")).unwrap();
    let (schema, manifest, manifest_digest) = evidence_runner_manifest();
    fs::write(
        evidence_dir
            .join("schemas")
            .join("com.example.health.status.schema.json"),
        serde_json::to_vec_pretty(&schema).unwrap(),
    )
    .unwrap();
    fs::write(
        evidence_dir.join("adapters").join("runner.manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let payload_schema = manifest["supported_observations"][0].clone();
    let execution = manifest["availability_probe"]["execution"].clone();
    let import_payload_schema = json!({
        "type": "planr.runner.result",
        "schema_ref": "schema://planr.runner.result",
        "schema_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
    });
    let import_execution = json!({
        "kind": "process",
        "executable": "true",
        "args": [],
        "working_directory": ".",
        "timeout_ms": 5000,
        "stdout_limit_bytes": 1,
        "stderr_limit_bytes": 1,
        "payload_schema": import_payload_schema.clone(),
    });
    let import_manifest = json!({
        "id": "verifier-runner-import",
        "schema_version": "evidence.contract.v1",
        "version": "1.0.0",
        "adapter_kind": "artifact_import",
        "adapter_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
        "supported_surfaces": ["local-process"],
        "supported_observations": [import_payload_schema],
        "supported_interactions": ["import"],
        "supported_artifacts": ["planr.runner_result.v1"],
        "runtime_targets": [{"kind": "process", "id": "runtime-import"}],
        "provenance_path": "validated_artifact_import",
        "permissions": {"network": "none", "filesystem": "read_workspace"},
        "costs": {},
        "determinism": "deterministic",
        "repeatability": "repeatable",
        "independence": "repository-owned import verifier manifest",
        "blind_spots": ["fixture covers runner import binding"],
        "availability_probe": {
            "kind": "process",
            "execution": import_execution,
        },
    });
    let import_manifest_digest = sha256_json(&import_manifest);
    fs::write(
        evidence_dir
            .join("adapters")
            .join("runner-import.manifest.json"),
        serde_json::to_vec_pretty(&import_manifest).unwrap(),
    )
    .unwrap();
    let mut policy = json!({
        "id": "epolicy-e2e-public-v1",
        "schema_version": "evidence.contract.v1",
        "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "defaults": {
            "preset_id": "health",
            "binding": true,
            "assurance_level": "standard",
        },
        "named_presets": [{
            "id": "health",
            "schema_version": "evidence.contract.v1",
            "namespace": "com.example.health",
            "observations": [{
                "id": "health",
                "type": "com.example.health.status",
                "subject": "public evidence health",
                "expected": {"status": "ok"},
                "target": {"kind": "process", "uri": "local://health"},
            }],
        }],
        "observation_schema_registrations": [{
            "type": "com.example.health.status",
            "schema_ref": "schema://com.example.health.status",
            "schema_digest": sha256_json(&schema),
            "owning_namespace": "com.example.health",
        }],
        "adapter_registrations": [
            {
                "manifest_id": "verifier-generic-adapter",
                "manifest_path": ".planr/evidence/adapters/runner.manifest.json",
                "manifest_digest": manifest_digest,
                "observation_types": ["com.example.health.status"],
                "payload_schemas": [payload_schema],
                "provenance_path": "planr_observed_execution",
                "execution_contract": execution,
            },
            {
                "manifest_id": "verifier-runner-import",
                "manifest_path": ".planr/evidence/adapters/runner-import.manifest.json",
                "manifest_digest": import_manifest_digest,
                "observation_types": ["planr.runner.result"],
                "payload_schemas": [import_manifest["supported_observations"][0].clone()],
                "provenance_path": "validated_artifact_import",
                "execution_contract": import_manifest["availability_probe"]["execution"].clone(),
            },
        ],
        "extension_namespaces": ["com.example.health"],
        "trust_policy": {
            "accepted_provenance": ["planr_observed_execution", "validated_artifact_import"],
            "min_receipt_status": "trusted",
            "allow_user_attestation": false,
        },
        "freshness_policy": {
            "max_age_seconds": 3600,
            "invalidate_on": ["source_change", "target_change", "policy_change"],
        },
        "fixture_policy": {
            "fixtures_allowed": false,
            "mocks_allowed": false,
            "disclosure_required": true,
        },
        "completion_policy": {
            "require_satisfied_or_waived": true,
            "allow_inconclusive_completion": false,
            "require_review_evidence": true,
        },
        "layering_policy": {
            "mode": "monotonic_strengthening",
            "weakening_requires_waiver": true,
            "layers": [{
                "scope": {"kind": "plan", "id": "pln-evidence-public"},
                "policy_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            }],
        },
    });
    policy["policy_digest"] = json!(sha256_json_without_field(&policy, "policy_digest"));
    fs::write(
        root.join(".planr/evidence.yaml"),
        serde_yaml::to_string(&policy).unwrap(),
    )
    .unwrap();
}

fn rewrite_evidence_policy_fixture(root: &Path, update: impl FnOnce(&mut Value)) {
    let path = root.join(".planr/evidence.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let mut policy: Value = serde_yaml::from_str(&text).unwrap();
    update(&mut policy);
    policy["policy_digest"] = json!(sha256_json_without_field(&policy, "policy_digest"));
    fs::write(path, serde_yaml::to_string(&policy).unwrap()).unwrap();
}

fn rewrite_evidence_runner_manifest(root: &Path, update: impl FnOnce(&mut Value)) -> String {
    let path = root.join(".planr/evidence/adapters/runner.manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    update(&mut manifest);
    let digest = sha256_json(&manifest);
    fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    digest
}

fn init_git_repo(root: &Path) {
    for args in [
        vec!["init"],
        vec!["config", "user.email", "planr-test@example.invalid"],
        vec!["config", "user.name", "Planr Test"],
        vec!["add", "."],
        vec!["commit", "-m", "initial evidence fixture"],
    ] {
        let output = StdCommand::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git failed: {output:?}");
    }
}

fn write_materiality_policy(root: &Path) {
    fs::write(
        root.join(".planr/policy.toml"),
        r#"
schema_version = 1
id = "materiality-test"
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
budget_exhaustion = "downgrade_noncritical"
metering = "trusted"

[usage.phase_reserves]
verification_percent = 10
review_percent = 5
repair_percent = 5

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
protected_risks = ["security_or_auth", "secrets_or_crypto", "schema_or_migration", "infrastructure_or_deploy", "public_api", "billing", "concurrency_or_transaction"]
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
}

fn init_materiality_git(root: &Path) {
    init_git_repo(root);
}

fn git_stdout(root: &Path, args: &[&str]) -> String {
    let output = StdCommand::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git failed: {output:?}");
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string()
}

fn evidence_source_binding(root: &Path) -> Value {
    let untracked = source_git_stdout(root, &["ls-files", "--others", "--exclude-standard"])
        .lines()
        .filter(|line| !line.is_empty())
        .map(|relative| {
            json!({
                "path": relative,
                "digest": sha256_prefixed(&fs::read(root.join(relative)).unwrap()),
            })
        })
        .collect::<Vec<_>>();
    let revision = git_stdout(root, &["rev-parse", "--verify", "HEAD"]);
    let head_tree = git_stdout(root, &["rev-parse", "HEAD^{tree}"]);
    let index = source_git_stdout(root, &["ls-files", "-s"]);
    let status = source_git_stdout(root, &["status", "--porcelain=v1"]);
    let diff = source_git_stdout(root, &["diff", "--binary", "HEAD"]);
    let diff_cached = source_git_stdout(root, &["diff", "--cached", "--binary"]);
    json!({
        "revision": revision,
        "tree_digest": sha256_json(&json!({
            "revision": revision,
            "head_tree": head_tree,
            "index": index,
            "status": status,
            "diff": diff,
            "diff_cached": diff_cached,
            "untracked": untracked,
        })),
        "dirty": !status.trim().is_empty(),
    })
}

fn source_git_stdout(root: &Path, args: &[&str]) -> String {
    let mut scoped = args.to_vec();
    scoped.push("--");
    scoped.extend_from_slice(&[
        ".",
        ":(exclude).planr/planr.sqlite",
        ":(exclude).planr/planr.sqlite-shm",
        ":(exclude).planr/planr.sqlite-wal",
        ":(exclude).planr/artifacts/**",
        ":(exclude).planr/verification/**",
        ":(exclude).planr/evidence/runs/**",
        ":(exclude).planr/evidence/attempts/**",
        ":(exclude).planr/evidence/receipts/**",
        ":(exclude).planr/evidence/coverage/**",
    ]);
    git_stdout(root, &scoped)
}

fn evidence_obligation(id: &str, policy_digest: &str, environment: Value) -> Value {
    evidence_obligation_for(
        id,
        policy_digest,
        "com.example.health.status",
        "public evidence health",
        json!({"status": "ok"}),
        json!({"kind": "process", "uri": "local://health"}),
        environment.clone(),
        json!({"kind": "process", "id": "runtime-local"}),
        json!([
            "source_change",
            "target_change",
            "policy_change",
            "adapter_schema_change"
        ]),
        "sha256:8888888888888888888888888888888888888888888888888888888888888888",
    )
}

#[allow(clippy::too_many_arguments)]
fn evidence_obligation_for(
    id: &str,
    _policy_digest: &str,
    observation_type: &str,
    subject: &str,
    expected: Value,
    target: Value,
    _environment: Value,
    _runtime_target: Value,
    invalidate_on: Value,
    _config_digest: &str,
) -> Value {
    json!({
        "id": id,
        "schema_version": "evidence.contract.v1",
        "criterion_id": format!("crit-{id}"),
        "plan_id": "pln-evidence-public",
        "title": format!("Evidence obligation {id}"),
        "binding": true,
        "observations": [{
            "id": format!("obs-{id}"),
            "type": observation_type,
            "subject": subject,
            "expected": expected,
            "target": target,
        }],
        "fixture_policy": {"fixtures_allowed": false, "mocks_allowed": false, "disclosure_required": true},
        "freshness_policy": {"invalidate_on": invalidate_on},
        "assurance_policy": {},
    })
}

fn current_utc_second() -> String {
    OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .unwrap()
        .format(&Rfc3339)
        .unwrap()
}

fn write_fresh_host_capture_envelope_with_producer(
    root: &Path,
    producer_name: &str,
    mutate: impl FnOnce(&mut Value),
) {
    let fixture_root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/evidence/host-capabilities/v1");
    fs::create_dir_all(root.join("artifacts/chrome-browser-client")).unwrap();
    fs::copy(
        fixture_root.join("artifacts/chrome-browser-client/cdp-result.json"),
        root.join("artifacts/chrome-browser-client/cdp-result.json"),
    )
    .unwrap();
    let mut raw: Value = serde_json::from_slice(
        &fs::read(fixture_root.join("observed/exp-chrome-browser-client.json")).unwrap(),
    )
    .unwrap();
    let now = current_utc_second();
    raw.as_object_mut().unwrap().remove("provenance_ref");
    raw["started_at"] = json!(now);
    raw["ended_at"] = json!(now);
    raw["host_identity"]["version"] = json!("Chrome/126.0.0.0");
    raw["result"]["missing_fields"]
        .as_array_mut()
        .unwrap()
        .retain(|field| field != "host_version");
    mutate(&mut raw);
    let envelope = json!({
        "schema_version": "planr.host_capability_external_capture_envelope.v1",
        "producer": {
            "name": producer_name,
            "version": "1.0.0",
            "captured_at": now,
        },
        "suite_id": "codex-host-capability-phase1",
        "captures": [raw],
    });
    fs::write(
        root.join("external-capture-envelope.json"),
        serde_json::to_vec_pretty(&envelope).unwrap(),
    )
    .unwrap();
}

fn write_queue_extension_fixture(root: &Path) {
    let schema = json!({
        "schema_version": "evidence.contract.v1",
        "type": "com.example.queue.depth.v2",
        "schema_ref": "schema://com.example.queue.depth.v2",
        "json_schema": {
            "type": "object",
            "required": ["status"],
            "additionalProperties": false,
            "properties": {"status": {"const": "drained"}}
        }
    });
    fs::write(
        root.join(".planr/evidence/schemas/com.example.queue.depth.v2.schema.json"),
        serde_json::to_vec_pretty(&schema).unwrap(),
    )
    .unwrap();
    let payload_schema = json!({
        "type": "com.example.queue.depth.v2",
        "schema_ref": "schema://com.example.queue.depth.v2",
        "schema_digest": sha256_json(&schema),
    });
    let execution = json!({
        "kind": "process",
        "executable": "sh",
        "args": ["-c", "printf '{\"status\":\"drained\"}'"],
        "working_directory": ".",
        "timeout_ms": 5000,
        "stdout_limit_bytes": 1024,
        "stderr_limit_bytes": 1024,
        "payload_schema": payload_schema,
    });
    let adapter_digest = process_adapter_digest(&execution, vec![]);
    let manifest = json!({
        "id": "verifier-queue-depth-v2",
        "schema_version": "evidence.contract.v1",
        "version": "2.1.0",
        "adapter_kind": "artifact_import",
        "adapter_digest": adapter_digest,
        "supported_surfaces": ["local-process"],
        "supported_observations": [execution["payload_schema"].clone()],
        "supported_interactions": ["process"],
        "supported_artifacts": ["stdout", "planr.generic_adapter_predicate.v1"],
        "runtime_targets": [{"kind": "process", "id": "runtime-queue-v2"}],
        "provenance_path": "planr_observed_execution",
        "permissions": {"network": "none", "filesystem": "read_workspace"},
        "costs": {},
        "determinism": "deterministic",
        "repeatability": "repeatable",
        "independence": "repository-defined queue extension adapter",
        "blind_spots": ["fixture adapter exits successfully without external queue"],
        "availability_probe": {"kind": "process", "execution": execution},
    });
    fs::write(
        root.join(".planr/evidence/adapters/queue-depth.manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let manifest_digest = sha256_json(&manifest);
    rewrite_evidence_policy_fixture(root, |policy| {
        policy["observation_schema_registrations"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "type": "com.example.queue.depth.v2",
                "schema_ref": "schema://com.example.queue.depth.v2",
                "schema_digest": sha256_json(&schema),
                "owning_namespace": "com.example.queue",
            }));
        policy["adapter_registrations"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "manifest_id": "verifier-queue-depth-v2",
                "manifest_path": ".planr/evidence/adapters/queue-depth.manifest.json",
                "manifest_digest": manifest_digest,
                "observation_types": ["com.example.queue.depth.v2"],
                "payload_schemas": [manifest["supported_observations"][0].clone()],
                "provenance_path": "planr_observed_execution",
                "execution_contract": manifest["availability_probe"]["execution"].clone(),
            }));
        policy["extension_namespaces"]
            .as_array_mut()
            .unwrap()
            .push(json!("com.example.queue"));
    });
}

fn write_http_curl_extension_fixture(root: &Path, port: u16) {
    let schema = json!({
        "schema_version": "evidence.contract.v1",
        "type": "com.example.http.status",
        "schema_ref": "schema://com.example.http.status",
        "json_schema": {
            "type": "object",
            "required": ["status"],
            "additionalProperties": false,
            "properties": {"status": {"const": "ok"}}
        }
    });
    fs::write(
        root.join(".planr/evidence/schemas/com.example.http.status.schema.json"),
        serde_json::to_vec_pretty(&schema).unwrap(),
    )
    .unwrap();
    let payload_schema = json!({
        "type": "com.example.http.status",
        "schema_ref": "schema://com.example.http.status",
        "schema_digest": sha256_json(&schema),
    });
    let execution = json!({
        "kind": "process",
        "executable": "curl",
        "args": ["-fsS", format!("http://127.0.0.1:{port}/health")],
        "working_directory": ".",
        "timeout_ms": 5000,
        "stdout_limit_bytes": 4096,
        "stderr_limit_bytes": 4096,
        "payload_schema": payload_schema,
    });
    let adapter_digest = process_adapter_digest(&execution, vec![]);
    let manifest = json!({
        "id": "verifier-http-curl-v1",
        "schema_version": "evidence.contract.v1",
        "version": "1.0.0",
        "adapter_kind": "process",
        "adapter_digest": adapter_digest,
        "supported_surfaces": ["local-process"],
        "supported_observations": [execution["payload_schema"].clone()],
        "supported_interactions": ["process"],
        "supported_artifacts": ["stdout", "planr.generic_adapter_predicate.v1"],
        "runtime_targets": [{"kind": "process", "id": "runtime-http-curl"}],
        "provenance_path": "planr_observed_execution",
        "permissions": {"network": "loopback", "filesystem": "read_workspace"},
        "costs": {},
        "determinism": "deterministic",
        "repeatability": "repeatable",
        "independence": "repository-defined curl HTTP adapter",
        "blind_spots": ["fixture HTTP server returns static health body"],
        "availability_probe": {"kind": "process", "execution": execution},
    });
    fs::write(
        root.join(".planr/evidence/adapters/http-curl.manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let manifest_digest = sha256_json(&manifest);
    rewrite_evidence_policy_fixture(root, |policy| {
        policy["observation_schema_registrations"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "type": "com.example.http.status",
                "schema_ref": "schema://com.example.http.status",
                "schema_digest": sha256_json(&schema),
                "owning_namespace": "com.example.http",
            }));
        policy["adapter_registrations"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "manifest_id": "verifier-http-curl-v1",
                "manifest_path": ".planr/evidence/adapters/http-curl.manifest.json",
                "manifest_digest": manifest_digest,
                "observation_types": ["com.example.http.status"],
                "payload_schemas": [manifest["supported_observations"][0].clone()],
                "provenance_path": "planr_observed_execution",
                "execution_contract": manifest["availability_probe"]["execution"].clone(),
            }));
        policy["extension_namespaces"]
            .as_array_mut()
            .unwrap()
            .push(json!("com.example.http"));
    });
}

#[test]
fn evidence_host_capture_import_uses_fresh_strict_boundary_across_cli_http_and_mcp() {
    const HOST_OBSERVATION: &str = "host.codex.chrome_browser_client";
    let canonical_temp_root = std::env::temp_dir().canonicalize().unwrap();
    let dir = tempfile::tempdir_in(canonical_temp_root).unwrap();
    let canonical_temp_root = dir.path().parent().unwrap().to_path_buf();
    let planr = || {
        let mut command = crate::planr();
        command.env("TMPDIR", &canonical_temp_root);
        command
    };
    let db_dir = tempdir().unwrap();
    let db = db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(dir.path());
    init_evidence_project(dir.path(), &db, "Evidence Host Capture Import");
    rewrite_evidence_policy_fixture(dir.path(), |policy| {
        policy["trust_policy"]["accepted_provenance"].as_array_mut().unwrap().push(json!("verified_host_event"));
    });

    let policy_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "evidence", "policy"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let policy = single_json_document(&policy_output);
    let obligation = evidence_obligation_for(
        "pob-host-capture",
        policy["object"]["digest"].as_str().unwrap(),
        HOST_OBSERVATION,
        "Example Domain Chrome capture",
        json!({
            "final_status": "available",
            "url": "https://example.com/",
            "title": "Example Domain",
            "bodyVisible": true,
        }),
        json!({"kind": "url", "uri": "https://example.com/"}),
        json!({}),
        json!({}),
        json!(["target_change", "policy_change", "adapter_schema_change"]),
        "sha256:7777777777777777777777777777777777777777777777777777777777777777",
    );
    let mut obligation = bind_obligation_to_authored_criterion(
        obligation, "pln-evidence-public", "criterion-evidence-public",
    );
    let conn = Connection::open(&db).unwrap();
    conn.execute("INSERT INTO items(id,project_id,title,description,status,work_type,plan_path,created_at,updated_at) SELECT 'item-host-maker',project_id,'Host maker','settle host capture source','ready','code',path,datetime('now'),datetime('now') FROM plans WHERE id='pln-evidence-public' UNION ALL SELECT 'item-host-verifier',project_id,'Host verifier','verify host capture','ready','verification',path,datetime('now'),datetime('now') FROM plans WHERE id='pln-evidence-public'", []).unwrap();
    drop(conn);
    obligation["item_id"] = json!("item-host-verifier");
    obligation["observations"][0]["payload_schema"] = json!({"schema_ref": "schemas/host-capability-observed-raw.schema.json"});
    add_evidence_obligation_value(dir.path(), &db, "pob-host-capture", &obligation);
    init_git_repo(dir.path());

    let cli = |worker: &str, args: &[&str]| {
        single_json_document(&planr().current_dir(dir.path()).env("PLANR_WORKER_ID", worker).args(["--db", db.to_str().unwrap(), "--json"]).args(args).assert().success().get_output().stdout)
    };
    assert_eq!(cli("host-maker", &["pick", "--plan", "pln-evidence-public", "--work-type", "code"])["item"]["id"], "item-host-maker");
    cli("host-maker", &["done", "item-host-maker", "--summary", "host source settled", "--cmd", "true", "--next"]);
    let verifier = cli("host-verifier", &["pick", "--plan", "pln-evidence-public", "--work-type", "verification"]);
    let authority = &verifier["work_packet"]["verification_admission"];

    let import_root = tempfile::tempdir_in(&canonical_temp_root).unwrap();
    let canonical_import_root = import_root.path().canonicalize().unwrap();
    write_fresh_host_capture_envelope_with_producer(&canonical_import_root, "planr-codex-host-capture", |_| {});
    let request = json!({"schema_version":"planr.evidence.host_capture.admission.v1","plan_id":authority["plan_id"],"run_id":authority["run_id"],"freeze_id":authority["freeze_id"],"run_revision":authority["run_revision"],"obligation_id":"pob-host-capture","import_root":canonical_import_root});
    let admission_path = db_dir.path().join("host-capture-admission.json");
    fs::write(&admission_path, serde_json::to_vec_pretty(&request).unwrap()).unwrap();
    let admitted = cli("host-verifier", &["evidence", "host-capture", "admit", "--input", admission_path.to_str().unwrap()]);
    assert_evidence_envelope(&admitted, "evidence.host_capture.admit", true);
    assert_eq!(admitted["object"]["status"], "pending");
    assert_eq!(admitted["object"]["freeze_id"], authority["freeze_id"]);
    assert_eq!(admitted["object"]["sealed_run_index"]["run_index_digest"], admitted["object"]["run_index_digest"]);
    let digest = admitted["object"]["run_index_digest"].as_str().unwrap();
    let persisted = || Connection::open(&db).unwrap().query_row("SELECT status,(SELECT COUNT(*) FROM verification_capability_instances WHERE id='host-exp-chrome-browser-client'),(SELECT COUNT(*) FROM evidence_attempts WHERE obligation_id='pob-host-capture'),(SELECT COUNT(*) FROM evidence_receipts WHERE obligation_id='pob-host-capture' AND receipt_json LIKE '%verified_host_event%') FROM host_capture_admissions WHERE sealed_run_index_digest=?1", [digest], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?))).unwrap();
    assert_eq!(persisted(), ("pending".to_string(), 0, 0, 0));

    let import_path = db_dir.path().join("host-capture-import.json");
    fs::write(&import_path, serde_json::to_vec_pretty(&admitted["object"]["import_input"]).unwrap()).unwrap();
    let imported = cli("host-verifier", &["evidence", "host-capture", "import", "--input", import_path.to_str().unwrap()]);
    assert_evidence_envelope(&imported, "evidence.host_capture.import", true);
    assert_eq!(imported["object"]["verdict"], "trusted");
    assert_eq!(imported["object"]["receipt"]["provenance"]["source"], "verified_host_event");
    assert_eq!(imported["object"]["receipt"]["vantage_point"]["kind"], "host_capture_import");
    assert_eq!(imported["object"]["receipt"]["vantage_point"]["identity"], "codex/chrome-browser-client");
    assert_eq!(persisted(), ("promoted".to_string(), 1, 1, 1));

    let http_dir = tempfile::tempdir_in(&canonical_temp_root).unwrap();
    let http_db_dir = tempdir().unwrap();
    let http_db = http_db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(http_dir.path());
    init_evidence_project(http_dir.path(), &http_db, "Evidence Host Capture HTTP");
    rewrite_evidence_policy_fixture(http_dir.path(), |policy| {
        policy["trust_policy"]["accepted_provenance"].as_array_mut().unwrap().push(json!("verified_host_event"));
    });
    let conn = Connection::open(&http_db).unwrap();
    conn.execute("INSERT INTO items(id,project_id,title,description,status,work_type,plan_path,created_at,updated_at) SELECT 'item-host-maker',project_id,'Host maker','settle host capture source','ready','code',path,datetime('now'),datetime('now') FROM plans WHERE id='pln-evidence-public' UNION ALL SELECT 'item-host-verifier',project_id,'Host verifier','verify host capture','ready','verification',path,datetime('now'),datetime('now') FROM plans WHERE id='pln-evidence-public'", []).unwrap();
    drop(conn);
    add_evidence_obligation_value(http_dir.path(), &http_db, "pob-host-capture", &obligation);
    init_git_repo(http_dir.path());
    let http_cli = |worker: &str, args: &[&str]| {
        single_json_document(&planr().current_dir(http_dir.path()).env("PLANR_WORKER_ID", worker).args(["--db", http_db.to_str().unwrap(), "--json"]).args(args).assert().success().get_output().stdout)
    };
    assert_eq!(http_cli("http-maker", &["pick", "--plan", "pln-evidence-public", "--work-type", "code"])["item"]["id"], "item-host-maker");
    http_cli("http-maker", &["done", "item-host-maker", "--summary", "http host source settled", "--cmd", "true", "--next"]);
    let http_verifier = http_cli("http-verifier", &["pick", "--plan", "pln-evidence-public", "--work-type", "verification"]);
    let http_authority = &http_verifier["work_packet"]["verification_admission"];
    let http_import_root = tempfile::tempdir_in(&canonical_temp_root).unwrap();
    let canonical_http_import_root = http_import_root.path().canonicalize().unwrap();
    write_fresh_host_capture_envelope_with_producer(&canonical_http_import_root, "planr-codex-host-capture", |_| {});
    let http_admission_request = json!({"schema_version":"planr.evidence.host_capture.admission.v1","plan_id":http_authority["plan_id"],"run_id":http_authority["run_id"],"freeze_id":http_authority["freeze_id"],"run_revision":http_authority["run_revision"],"obligation_id":"pob-host-capture","import_root":canonical_http_import_root});
    let http_port = free_port();
    let mut http_server = std_planr_from_binary(&assert_cmd::cargo::cargo_bin("planr"));
    let mut http_server = http_server
        .current_dir(http_dir.path())
        .env("PLANR_WORKER_ID", "http-verifier")
        .env("TMPDIR", &canonical_temp_root)
        .args(["--db", http_db.to_str().unwrap(), "serve", "--port", &http_port.to_string()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_http_server(http_port);
    let http_admitted = http_json(&http_request(
        http_port, "POST", "/v1/evidence/host-capture/admit", &http_admission_request.to_string(),
    ));
    assert_evidence_envelope(&http_admitted, "evidence.host_capture.admit", true);
    assert_eq!(http_admitted["object"]["status"], admitted["object"]["status"]);
    assert_eq!(http_admitted["object"]["freeze_id"], http_authority["freeze_id"]);
    assert_eq!(http_admitted["object"]["sealed_run_index"]["run_index_digest"], http_admitted["object"]["run_index_digest"]);
    let http_digest = http_admitted["object"]["run_index_digest"].as_str().unwrap();
    let http_persisted = || Connection::open(&http_db).unwrap().query_row("SELECT status,(SELECT COUNT(*) FROM verification_capability_instances WHERE id='host-exp-chrome-browser-client'),(SELECT COUNT(*) FROM evidence_attempts WHERE obligation_id='pob-host-capture'),(SELECT COUNT(*) FROM evidence_receipts WHERE obligation_id='pob-host-capture' AND receipt_json LIKE '%verified_host_event%') FROM host_capture_admissions WHERE sealed_run_index_digest=?1", [http_digest], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?))).unwrap();
    assert_eq!(http_persisted(), ("pending".to_string(), 0, 0, 0));
    let http_imported = http_json(&http_request(
        http_port, "POST", "/v1/evidence/host-capture/import", &http_admitted["object"]["import_input"].to_string(),
    ));
    assert_evidence_envelope(&http_imported, "evidence.host_capture.import", true);
    assert_eq!(http_imported["object"]["verdict"], imported["object"]["verdict"]);
    assert_eq!(http_imported["object"]["receipt"]["provenance"]["source"], "verified_host_event");
    assert_eq!(http_persisted(), ("promoted".to_string(), 1, 1, 1));
    http_server.kill().unwrap();
    http_server.wait().unwrap();
}
struct StaticHttpServer {
    shutdown: mpsc::Sender<()>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Drop for StaticHttpServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn start_static_http_server() -> (u16, StaticHttpServer) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (shutdown, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        loop {
            if receiver.try_recv().is_ok() {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buffer = [0_u8; 1024];
                    let _ = stream.read(&mut buffer);
                    let body = r#"{"status":"ok"}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });
    (
        port,
        StaticHttpServer {
            shutdown,
            handle: Some(handle),
        },
    )
}

fn add_evidence_obligation_value(dir: &Path, db: &Path, id: &str, obligation: &Value) -> Value {
    let migration = json!({
        "schema_version": "planr.evidence.migration.v1",
        "plan_id": obligation["plan_id"],
        "obligations": [obligation],
    });
    let path = dir.join(format!("{id}.migration.json"));
    fs::write(&path, serde_json::to_vec_pretty(&migration).unwrap()).unwrap();
    single_json_document(
        &planr()
            .current_dir(dir)
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "migrate",
                "--input",
                path.to_str().unwrap(),
                "--apply",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    )
}


fn run_evidence_value(
    dir: &Path,
    db: &Path,
    obligation_id: &str,
    capability_instance_id: &str,
    target: Value,
) -> Value {
    run_evidence_value_with_code(dir, db, obligation_id, capability_instance_id, target, 0)
}

fn run_evidence_value_with_code(
    dir: &Path,
    db: &Path,
    obligation_id: &str,
    capability_instance_id: &str,
    target: Value,
    expected_code: i32,
) -> Value {
    run_evidence_value_with_env_code(
        dir,
        db,
        obligation_id,
        capability_instance_id,
        target,
        None,
        expected_code,
    )
}

fn run_evidence_value_with_env_code(
    dir: &Path,
    db: &Path,
    obligation_id: &str,
    capability_instance_id: &str,
    target: Value,
    env: Option<Value>,
    expected_code: i32,
) -> Value {
    run_evidence_value_with_env_fixture_code(
        dir,
        db,
        obligation_id,
        capability_instance_id,
        target,
        env,
        None,
        expected_code,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_evidence_value_with_env_fixture_code(
    dir: &Path,
    db: &Path,
    obligation_id: &str,
    capability_instance_id: &str,
    target: Value,
    env: Option<Value>,
    fixture_disclosure: Option<Value>,
    expected_code: i32,
) -> Value {
    let path = dir.join(format!("{obligation_id}.run.json"));
    let mut input = json!({
        "obligation_id": obligation_id,
        "capability_instance_id": capability_instance_id,
        "target": target,
    });
    if let Some(env) = env {
        input["env"] = env;
    }
    if let Some(fixture_disclosure) = fixture_disclosure {
        input["fixture_disclosure"] = fixture_disclosure;
    }
    fs::write(&path, serde_json::to_vec_pretty(&input).unwrap()).unwrap();
    single_json_document(
        &planr()
            .current_dir(dir)
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "run",
                "--input",
                path.to_str().unwrap(),
            ])
            .assert()
            .code(expected_code)
            .get_output()
            .stdout,
    )
}

fn assert_unavailable_run_identity(
    run: &Value,
    expected_instance: &Value,
    expected_obligation_id: &str,
) -> (String, String) {
    assert_evidence_envelope(run, "evidence.run", true);
    assert_eq!(run["object"]["verdict"], "unavailable");
    let attempt = &run["object"]["attempt"];
    let receipt = &run["object"]["receipt"];
    let capability = &receipt["capability"];
    let expected_capability = &expected_instance["capability"];

    assert_eq!(attempt["status"], "unavailable");
    assert_eq!(attempt["obligation_id"], expected_obligation_id);
    assert_eq!(
        attempt["criterion_id"],
        format!("crit-{expected_obligation_id}")
    );
    assert_eq!(attempt["capability_instance_id"], expected_instance["id"]);
    assert_eq!(attempt["exit"]["error"], "unavailable");
    assert_eq!(attempt["raw_result"]["kind"], "process_error");
    assert_eq!(attempt["raw_result"]["exit"]["error"], "unavailable");
    assert_eq!(
        attempt["resolved_command"]["resolution"]["status"],
        "unavailable"
    );
    assert_eq!(
        attempt["resolved_command"]["command"][0],
        "planr-definitely-missing-adapter"
    );
    assert!(
        attempt["raw_result"]["error_reason"]
            .as_str()
            .unwrap()
            .contains("was not found in captured PATH")
    );

    assert_eq!(receipt["receipt_status"], "trusted");
    assert_eq!(receipt["obligation_id"], expected_obligation_id);
    assert_eq!(
        receipt["criterion_id"],
        format!("crit-{expected_obligation_id}")
    );
    assert_eq!(receipt["provenance"]["source"], "planr_observed_execution");
    assert_eq!(receipt["provenance"]["assigned_by"], "planr");
    assert_eq!(receipt["provenance"]["execution_id"], attempt["id"]);
    assert_eq!(receipt["attempt_ids"], json!([attempt["id"].clone()]));
    assert_eq!(receipt["observations"][0]["outcome"], "unavailable");
    assert_eq!(receipt["proof_gaps"], json!(["missing_capability"]));
    assert_eq!(capability["manifest_id"], expected_instance["manifest_id"]);
    assert_eq!(
        capability["manifest_digest"],
        expected_instance["manifest_digest"]
    );
    assert_eq!(capability["instance_id"], expected_instance["id"]);
    assert!(
        capability["instance_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(
        expected_capability["probe_result"]["probe_execution_id"],
        expected_instance["probe_execution_id"]
    );
    assert_eq!(expected_capability["probe_result"]["outcome"], "passed");
    assert_eq!(expected_capability["availability"]["status"], "available");

    (
        attempt["id"].as_str().unwrap().to_string(),
        receipt["id"].as_str().unwrap().to_string(),
    )
}

fn assert_blocked_missing_capability_coverage(
    coverage: &Value,
    expected_coverage_id: Option<&str>,
    expected_obligation_id: &str,
    expected_receipt_ids: &[String],
) -> String {
    assert_evidence_envelope(coverage, "evidence.coverage", true);
    assert_eq!(coverage["object"]["status"], "blocked");
    assert_eq!(coverage["object"]["verdict"], "blocked");
    let verdict = &coverage["object"]["coverage"];
    assert_eq!(verdict["status"], "blocked");
    assert_eq!(verdict["scope"]["kind"], "criterion");
    assert_eq!(
        verdict["scope"]["id"],
        format!("crit-{expected_obligation_id}")
    );
    assert_eq!(
        verdict["scope"]["criterion_id"],
        format!("crit-{expected_obligation_id}")
    );
    if let Some(expected_coverage_id) = expected_coverage_id {
        assert_eq!(coverage["object"]["coverage_id"], expected_coverage_id);
        assert_eq!(verdict["id"], expected_coverage_id);
    }
    let observation = &verdict["observation_coverage"][0];
    assert_eq!(
        observation["requirement_id"],
        format!("obs-{expected_obligation_id}")
    );
    assert_eq!(observation["status"], "blocked");
    assert_eq!(observation["gap_reason"], "missing_capability");
    let attempted = observation["attempted_receipt_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    for expected_receipt_id in expected_receipt_ids {
        assert!(
            attempted.contains(expected_receipt_id),
            "coverage did not include attempted receipt {expected_receipt_id}: {coverage}"
        );
    }
    coverage["object"]["coverage_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn superseding_obligation(mut base: Value, id: &str, supersedes: &str) -> Value {
    base["id"] = json!(id);
    base["title"] = json!(format!("Evidence obligation {id}"));
    base["supersedes"] = json!(supersedes);
    base
}

fn register_receipt_binding_validator_for_test(conn: &Connection) {
    conn.create_scalar_function(
        "planr_evidence_receipt_binding_is_valid",
        2,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let trusted_binding_json: String = ctx.get(0)?;
            let receipt_json: String = ctx.get(1)?;
            let trusted_binding: Value = serde_json::from_str(&trusted_binding_json)
                .map_err(|err| rusqlite::Error::UserFunctionError(Box::new(err)))?;
            let receipt: Value = serde_json::from_str(&receipt_json)
                .map_err(|err| rusqlite::Error::UserFunctionError(Box::new(err)))?;
            Ok(
                if planr::evidence::trusted_receipt_binding_matches_receipt(
                    trusted_binding,
                    receipt,
                )
                .is_ok()
                {
                    1i64
                } else {
                    0i64
                },
            )
        },
    )
    .unwrap();
}

fn evidence_row_count(db: &Path, table: &str) -> i64 {
    assert!(
        matches!(
            table,
            "evidence_attempts" | "evidence_receipts" | "evidence_validated_imports"
        ),
        "unexpected evidence table {table}"
    );
    Connection::open(db)
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn assert_persisted_coverage_verdict(db: &Path, coverage: &Value) {
    let conn = Connection::open(db).unwrap();
    let persisted: String = conn
        .query_row(
            "SELECT verdict_json FROM coverage_verdicts WHERE id = ?1",
            [coverage["object"]["coverage_id"].as_str().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    let persisted: Value = serde_json::from_str(&persisted).unwrap();
    assert_eq!(persisted, coverage["object"]["coverage"]);
}

fn assert_json_string_set(actual: &Value, expected: &[&str]) {
    let mut actual = actual
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let mut expected = expected
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

fn generic_import_artifact_set_digest(artifacts: &[Value], artifact_bytes: &[&[u8]]) -> String {
    sha256_json(&Value::Array(
        artifacts
            .iter()
            .zip(artifact_bytes)
            .map(|(artifact, bytes)| {
                json!({
                    "id": artifact["id"],
                    "kind": artifact["kind"],
                    "declared_digest": artifact["digest"],
                    "content_digest": sha256_prefixed(bytes),
                })
            })
            .collect(),
    ))
}

fn generic_import_predicate_digest(predicate: &Value) -> String {
    sha256_json(&json!({
        "kind": predicate["kind"],
        "version": predicate["version"],
        "type": predicate["type"],
        "outcome": predicate["outcome"],
        "predicate": predicate["predicate"],
        "actual": predicate["actual"],
    }))
}

fn write_generic_import_validator_fixture(root: &Path) -> String {
    let payload_schema = json!({
        "type": "planr.import.validator.generic_predicate",
        "schema_ref": "schema://planr.import.validator.generic_predicate",
        "schema_digest": "sha256:abababababababababababababababababababababababababababababababab",
    });
    let execution = json!({
        "kind": "process",
        "executable": "sh",
        "args": ["-c", "printf '%s' \"$PLANR_GENERIC_VALIDATOR_RESULT\""],
        "working_directory": ".",
        "timeout_ms": 5000,
        "stdout_limit_bytes": 4096,
        "stderr_limit_bytes": 1024,
        "payload_schema": payload_schema,
    });
    let adapter_digest = process_adapter_digest(&execution, vec![]);
    let manifest = json!({
        "id": "verifier-generic-import-validator",
        "schema_version": "evidence.contract.v1",
        "version": "1.0.0",
        "adapter_kind": "artifact_import",
        "adapter_digest": adapter_digest,
        "supported_surfaces": ["local-process"],
        "supported_observations": [execution["payload_schema"].clone()],
        "supported_interactions": ["process", "import"],
        "supported_artifacts": ["stdout", "planr.generic_adapter_predicate.v1"],
        "runtime_targets": [{"kind": "process", "id": "runtime-local"}],
        "provenance_path": "validated_artifact_import",
        "permissions": {"network": "none", "filesystem": "read_workspace"},
        "costs": {},
        "determinism": "deterministic",
        "repeatability": "repeatable",
        "independence": "repository-owned generic import validator",
        "blind_spots": ["fixture validates generic import trust boundary only"],
        "availability_probe": {"kind": "process", "execution": execution},
    });
    let manifest_digest = sha256_json(&manifest);
    fs::write(
        root.join(".planr/evidence/adapters/generic-import-validator.manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    rewrite_evidence_policy_fixture(root, |policy| {
        policy["adapter_registrations"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "manifest_id": "verifier-generic-import-validator",
                "manifest_path": ".planr/evidence/adapters/generic-import-validator.manifest.json",
                "manifest_digest": manifest_digest,
                "observation_types": ["planr.import.validator.generic_predicate"],
                "payload_schemas": [manifest["supported_observations"][0].clone()],
                "provenance_path": "validated_artifact_import",
                "execution_contract": manifest["availability_probe"]["execution"].clone(),
            }));
    });
    adapter_digest
}

fn doctor_evidence(dir: &Path, db: &Path) -> Value {
    let output = planr()
        .current_dir(dir)
        .args(["--db", db.to_str().unwrap(), "--json", "doctor"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    single_json_document(&output)["evidence"].clone()
}

fn add_evidence_obligation(dir: &Path, db: &Path, id: &str, policy_digest: &str) {
    add_evidence_obligation_with_environment(
        dir,
        db,
        id,
        policy_digest,
        json!({"kind": "local", "id": "placeholder", "digest": "sha256:5555555555555555555555555555555555555555555555555555555555555555"}),
    );
}

fn add_evidence_obligation_with_environment(
    dir: &Path,
    db: &Path,
    id: &str,
    policy_digest: &str,
    environment: Value,
) {
    let mut obligation = bind_obligation_to_authored_criterion(
        evidence_obligation_for(
            id,
            policy_digest,
            "com.example.health.status",
            "public evidence health",
            json!({"status": "ok"}),
            json!({"kind": "process", "uri": "local://health"}),
            environment.clone(),
            json!({"kind": "process", "id": "runtime-local"}),
            json!(["target_change", "policy_change", "adapter_schema_change"]),
            "sha256:8888888888888888888888888888888888888888888888888888888888888888",
        ),
        "pln-evidence-public",
        "criterion-evidence-public",
    );
    obligation["observations"][0]["payload_schema"] =
        json!({"schema_ref": "schema://com.example.health.status"});
    add_evidence_obligation_value(dir, db, id, &obligation);
}

fn capability_instance_environment(db: &Path, instance_id: &str) -> Value {
    let conn = Connection::open(db).unwrap();
    let snapshot: String = conn
        .query_row(
            "SELECT capability_snapshot_json FROM verification_capability_instances WHERE id = ?1",
            [instance_id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str::<Value>(&snapshot).unwrap()["environment"].clone()
}

fn run_evidence_manifest(dir: &Path, db: &Path, obligation_id: &str) -> Value {
    let readiness = single_json_document(
        &planr()
            .current_dir(dir)
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "readiness",
                "--scope",
                "obligation",
                "--id",
                obligation_id,
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_evidence_envelope(&readiness, "evidence.readiness", true);
    assert_eq!(readiness["object"]["status"], "passed");
    let path = dir.join(
        readiness["object"]["run_index"]["repository_path"]
            .as_str()
            .unwrap(),
    );
    single_json_document(
        &planr()
            .current_dir(dir)
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "run",
                "--input",
                path.to_str().unwrap(),
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    )
}

fn seed_capability_instance_variant(
    source_db: &Path,
    target_db: &Path,
    instance_id: &str,
    runtime_target_json: Option<&str>,
    valid_until: Option<&str>,
) {
    seed_capability_instance_with_options(
        source_db,
        target_db,
        instance_id,
        SeedCapabilityInstanceOptions {
            runtime_target_json,
            valid_until,
            environment: None,
            snapshot_manifest_id: None,
        },
    );
}

#[derive(Clone)]
struct SeedCapabilityInstanceOptions<'a> {
    runtime_target_json: Option<&'a str>,
    valid_until: Option<&'a str>,
    environment: Option<Value>,
    snapshot_manifest_id: Option<&'a str>,
}

fn seed_capability_instance_with_options(
    source_db: &Path,
    target_db: &Path,
    instance_id: &str,
    options: SeedCapabilityInstanceOptions<'_>,
) {
    struct SeedCapabilityInstance {
        manifest_id: String,
        manifest_version: String,
        manifest_digest: String,
        availability_status: String,
        runtime_target_json: String,
        host_fingerprint_json: String,
        capability_snapshot_json: String,
        probe_result_json: String,
    }

    let source = Connection::open(source_db).unwrap();
    let target = Connection::open(target_db).unwrap();
    let manifest: (String, String, String, String, String, String, Option<String>, String) = source
        .query_row(
            "SELECT id, version, adapter_kind, adapter_digest, manifest_digest, manifest_json, source_path, created_at
             FROM verification_capability_manifests WHERE id = 'verifier-generic-adapter'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .unwrap();
    target
        .execute(
            "INSERT INTO verification_capability_manifests(
              id, version, adapter_kind, adapter_digest, manifest_digest, manifest_json, source_path, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                manifest.0, manifest.1, manifest.2, manifest.3, manifest.4, manifest.5,
                manifest.6, manifest.7
            ],
        )
        .unwrap();
    let instance: SeedCapabilityInstance = source
        .query_row(
            "SELECT manifest_id, manifest_version, manifest_digest,
                    availability_status, runtime_target_json, host_fingerprint_json,
                    capability_snapshot_json, probe_result_json
             FROM verification_capability_instances
             WHERE manifest_id = 'verifier-generic-adapter'
             ORDER BY created_at DESC, id DESC LIMIT 1",
            [],
            |row| {
                Ok(SeedCapabilityInstance {
                    manifest_id: row.get(0)?,
                    manifest_version: row.get(1)?,
                    manifest_digest: row.get(2)?,
                    availability_status: row.get(3)?,
                    runtime_target_json: row.get(4)?,
                    host_fingerprint_json: row.get(5)?,
                    capability_snapshot_json: row.get(6)?,
                    probe_result_json: row.get(7)?,
                })
            },
        )
        .unwrap();
    let probe_execution_id = format!("probe-{instance_id}");
    let mut snapshot: Value = serde_json::from_str(&instance.capability_snapshot_json).unwrap();
    snapshot["id"] = json!(instance_id);
    if let Some(manifest_id) = options.snapshot_manifest_id {
        snapshot["manifest_id"] = json!(manifest_id);
    }
    if let Some(environment) = options.environment.as_ref() {
        snapshot["environment"] = environment.clone();
    }
    snapshot["probe_result"]["probe_execution_id"] = json!(probe_execution_id);
    let mut probe_result: Value = serde_json::from_str(&instance.probe_result_json).unwrap();
    probe_result["probe_execution_id"] = json!(probe_execution_id);
    let mut host_fingerprint: Value =
        serde_json::from_str(&instance.host_fingerprint_json).unwrap();
    if let Some(environment) = options.environment.as_ref() {
        host_fingerprint["environment"] = environment.clone();
    }
    target
        .execute(
            "INSERT INTO verification_capability_instances(
              id, manifest_id, manifest_version, manifest_digest, probe_execution_id,
              availability_status, runtime_target_json, host_fingerprint_json,
              capability_snapshot_json, probe_result_json, created_at, valid_until
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'), ?11)",
            rusqlite::params![
                instance_id,
                instance.manifest_id,
                instance.manifest_version,
                instance.manifest_digest,
                probe_execution_id,
                instance.availability_status,
                options
                    .runtime_target_json
                    .unwrap_or(&instance.runtime_target_json),
                host_fingerprint.to_string(),
                snapshot.to_string(),
                probe_result.to_string(),
                options.valid_until,
            ],
        )
        .unwrap();
}

fn init_evidence_project(dir: &Path, db: &Path, name: &str) {
    planr()
        .current_dir(dir)
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "project",
            "init",
            name,
        ])
        .assert()
        .success();
    let plan_path = dir.join(".planr/plans/build/evidence-public-fixture.plan.md");
    author_build_plan_criterion(&plan_path, "criterion-evidence-public");
    let conn = rusqlite::Connection::open(db).unwrap();
    let project_id: String = conn
        .query_row(
            "SELECT id FROM projects ORDER BY created_at DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO plans(
          id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at
        ) VALUES (
          'pln-evidence-public', ?1, 'build', ?2, 'Evidence Public Fixture', 'evidence-public-fixture', 'ok', 'fixture-hash', datetime('now'), datetime('now')
        )",
        rusqlite::params![
            project_id,
            plan_path.to_string_lossy().to_string()
        ],
    )
    .unwrap();
}

fn author_build_plan_criterion(path: &Path, criterion_id: &str) {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let body = existing
        .find("\n---\n")
        .map(|end| &existing[end + 5..])
        .unwrap_or(existing.as_str());
    fs::write(
        path,
        format!(
            "---\ncriteria:\n  - id: {criterion_id}\n    title: {criterion_id}\n---\n{body}"
        ),
    )
    .unwrap();
}

fn author_build_plan_criteria(path: &Path, criterion_ids: &[&str]) {
    assert!(!criterion_ids.is_empty(), "fixture criteria must be explicit");
    let existing = fs::read_to_string(path).unwrap_or_default();
    let body = existing
        .find("\n---\n")
        .map(|end| &existing[end + 5..])
        .unwrap_or(existing.as_str());
    let criteria = criterion_ids.iter().map(|id| format!("  - id: {id}\n    title: {id}\n")).collect::<String>();
    fs::write(path, format!("---\ncriteria:\n{criteria}---\n{body}")).unwrap();
}

fn bind_obligation_to_authored_criterion(
    mut obligation: Value,
    plan_id: &str,
    criterion_id: &str,
) -> Value {
    obligation["plan_id"] = json!(plan_id);
    obligation["criterion_id"] = json!(criterion_id);
    obligation
}

#[test]
fn evidence_doctor_reports_degraded_states_and_matches_run_resolution() {
    let missing_dir = tempdir().unwrap();
    let missing_db_dir = tempdir().unwrap();
    let missing_db = missing_db_dir.path().join("planr.sqlite");
    init_evidence_project(missing_dir.path(), &missing_db, "Missing Evidence Policy");
    let missing = doctor_evidence(missing_dir.path(), &missing_db);
    assert_eq!(missing["status"], "warning");
    assert_eq!(missing["policy"]["state"], "missing_policy");

    let malformed_dir = tempdir().unwrap();
    let malformed_db_dir = tempdir().unwrap();
    let malformed_db = malformed_db_dir.path().join("planr.sqlite");
    fs::create_dir_all(malformed_dir.path().join(".planr")).unwrap();
    fs::write(
        malformed_dir.path().join(".planr/evidence.yaml"),
        "schema_version: evidence.contract.v1\npolicy_digest: definitely-not-a-digest\n",
    )
    .unwrap();
    init_evidence_project(
        malformed_dir.path(),
        &malformed_db,
        "Malformed Evidence Policy",
    );
    let malformed = doctor_evidence(malformed_dir.path(), &malformed_db);
    assert_eq!(malformed["status"], "warning");
    assert_eq!(malformed["policy"]["state"], "malformed_policy");
    assert!(
        malformed["policy"]["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("evidence policy invalid")
    );

    let no_adapters_dir = tempdir().unwrap();
    let no_adapters_db_dir = tempdir().unwrap();
    let no_adapters_db = no_adapters_db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(no_adapters_dir.path());
    rewrite_evidence_policy_fixture(no_adapters_dir.path(), |policy| {
        policy["adapter_registrations"] = json!([]);
    });
    init_evidence_project(
        no_adapters_dir.path(),
        &no_adapters_db,
        "No Evidence Adapters",
    );
    let no_adapters = doctor_evidence(no_adapters_dir.path(), &no_adapters_db);
    assert_eq!(no_adapters["status"], "warning");
    assert_eq!(no_adapters["policy"]["state"], "no_adapters");
    assert_eq!(
        no_adapters["policy"]["registry"]["registered_capabilities"],
        0
    );

    let unavailable_dir = tempdir().unwrap();
    let unavailable_db_dir = tempdir().unwrap();
    let unavailable_db = unavailable_db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(unavailable_dir.path());
    let unavailable_digest = rewrite_evidence_runner_manifest(unavailable_dir.path(), |manifest| {
        manifest["availability_probe"]["execution"]["executable"] =
            json!("definitely-not-a-planr-probe");
    });
    rewrite_evidence_policy_fixture(unavailable_dir.path(), |policy| {
        policy["adapter_registrations"][0]["manifest_digest"] = json!(unavailable_digest);
    });
    init_evidence_project(
        unavailable_dir.path(),
        &unavailable_db,
        "Unavailable Evidence Adapter",
    );
    init_git_repo(unavailable_dir.path());
    let unavailable = doctor_evidence(unavailable_dir.path(), &unavailable_db);
    assert_eq!(unavailable["status"], "warning");
    assert_eq!(unavailable["policy"]["state"], "unavailable");
    assert_eq!(
        unavailable["policy"]["registry"]["probes"][0]["availability_status"],
        "unavailable"
    );
    add_evidence_obligation(
        unavailable_dir.path(),
        &unavailable_db,
        "pob-unavailable",
        unavailable["policy"]["digest"].as_str().unwrap(),
    );
    let unavailable_readiness = single_json_document(
        &planr()
            .current_dir(unavailable_dir.path())
            .args([
                "--db",
                unavailable_db.to_str().unwrap(),
                "--json",
                "evidence",
                "readiness",
                "--scope",
                "obligation",
                "--id",
                "pob-unavailable",
            ])
            .assert()
            .failure()
            .code(3)
            .get_output()
            .stdout,
    );
    assert_evidence_envelope(&unavailable_readiness, "evidence.readiness", true);
    assert_eq!(unavailable_readiness["object"]["status"], "blocked");
    assert_eq!(unavailable_readiness["object"]["gaps"][0]["code"], "ProbeUnavailable");
    assert_eq!(unavailable_readiness["object"]["gaps"][0]["obligation_id"], "pob-unavailable");
    assert_eq!(unavailable_readiness["object"]["registry"]["diagnostics"][0]["code"], "ProbeUnavailable");

    let degraded_dir = tempdir().unwrap();
    let degraded_db_dir = tempdir().unwrap();
    let degraded_db = degraded_db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(degraded_dir.path());
    let degraded_digest = rewrite_evidence_runner_manifest(degraded_dir.path(), |manifest| {
        manifest["availability_probe"]["execution"]["args"] = json!(["-c", "exit 2"]);
    });
    rewrite_evidence_policy_fixture(degraded_dir.path(), |policy| {
        policy["adapter_registrations"][0]["manifest_digest"] = json!(degraded_digest);
        policy["adapter_registrations"][0]["execution_contract"]["args"] = json!(["-c", "exit 2"]);
    });
    init_evidence_project(
        degraded_dir.path(),
        &degraded_db,
        "Degraded Evidence Adapter",
    );
    let degraded = doctor_evidence(degraded_dir.path(), &degraded_db);
    assert_eq!(degraded["status"], "warning");
    assert_eq!(degraded["policy"]["state"], "degraded");
    assert_eq!(
        degraded["policy"]["registry"]["probes"][0]["availability_status"],
        "probe_failed"
    );

    let seed_dir = tempdir().unwrap();
    let seed_db_dir = tempdir().unwrap();
    let seed_db = seed_db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(seed_dir.path());
    init_evidence_project(seed_dir.path(), &seed_db, "Seed Evidence Adapter");
    let first = doctor_evidence(seed_dir.path(), &seed_db);
    assert_eq!(first["status"], "ok");
    assert_eq!(first["policy"]["state"], "ready");
    let first_instance = first["policy"]["registry"]["probes"][0]["instance_id"]
        .as_str()
        .unwrap()
        .to_string();

    let expired_dir = tempdir().unwrap();
    let expired_db_dir = tempdir().unwrap();
    let expired_db = expired_db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(expired_dir.path());
    init_git_repo(expired_dir.path());
    init_evidence_project(expired_dir.path(), &expired_db, "Expired Evidence Adapter");
    seed_capability_instance_variant(
        &seed_db,
        &expired_db,
        "vcap-seeded-expired",
        None,
        Some("2020-01-01T00:00:00Z"),
    );
    let expired_recovery = doctor_evidence(expired_dir.path(), &expired_db);
    assert_eq!(expired_recovery["status"], "warning");
    assert_eq!(expired_recovery["policy"]["state"], "recovered");
    assert_eq!(
        expired_recovery["policy"]["registry"]["probes"][0]["resolution"],
        "reprobed_expired"
    );
    let expired_instance = expired_recovery["policy"]["registry"]["probes"][0]["instance_id"]
        .as_str()
        .unwrap();
    assert_ne!(expired_instance, first_instance);
    add_evidence_obligation_with_environment(
        expired_dir.path(),
        &expired_db,
        "pob-recovered-expired",
        first["policy"]["digest"].as_str().unwrap(),
        capability_instance_environment(&expired_db, expired_instance),
    );
    let expired_run =
        run_evidence_manifest(expired_dir.path(), &expired_db, "pob-recovered-expired");
    assert_evidence_envelope(&expired_run, "evidence.run", true);
    assert_eq!(
        expired_run["object"]["results"][0]["attempt"]["capability_instance_id"],
        expired_instance
    );

    let mismatch_dir = tempdir().unwrap();
    let mismatch_db_dir = tempdir().unwrap();
    let mismatch_db = mismatch_db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(mismatch_dir.path());
    init_git_repo(mismatch_dir.path());
    init_evidence_project(
        mismatch_dir.path(),
        &mismatch_db,
        "Mismatch Evidence Adapter",
    );
    seed_capability_instance_variant(
        &seed_db,
        &mismatch_db,
        "vcap-seeded-mismatch",
        Some("[{\"kind\":\"process\",\"id\":\"runtime-other\"}]"),
        None,
    );
    let mismatch_recovery = doctor_evidence(mismatch_dir.path(), &mismatch_db);
    assert_eq!(mismatch_recovery["status"], "warning");
    assert_eq!(mismatch_recovery["policy"]["state"], "recovered");
    assert_eq!(
        mismatch_recovery["policy"]["registry"]["probes"][0]["resolution"],
        "reprobed_runtime_mismatch"
    );
    let mismatch_instance = mismatch_recovery["policy"]["registry"]["probes"][0]["instance_id"]
        .as_str()
        .unwrap();
    assert_ne!(mismatch_instance, first_instance);
    add_evidence_obligation_with_environment(
        mismatch_dir.path(),
        &mismatch_db,
        "pob-recovered-mismatch",
        first["policy"]["digest"].as_str().unwrap(),
        capability_instance_environment(&mismatch_db, mismatch_instance),
    );
    let mismatch_run =
        run_evidence_manifest(mismatch_dir.path(), &mismatch_db, "pob-recovered-mismatch");
    assert_evidence_envelope(&mismatch_run, "evidence.run", true);
    assert_eq!(
        mismatch_run["object"]["results"][0]["attempt"]["capability_instance_id"],
        mismatch_instance
    );

    let environment_dir = tempdir().unwrap();
    let environment_db_dir = tempdir().unwrap();
    let environment_db = environment_db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(environment_dir.path());
    init_git_repo(environment_dir.path());
    init_evidence_project(
        environment_dir.path(),
        &environment_db,
        "Environment Mismatch Evidence Adapter",
    );
    seed_capability_instance_with_options(
        &seed_db,
        &environment_db,
        "vcap-seeded-environment-mismatch",
        SeedCapabilityInstanceOptions {
            runtime_target_json: None,
            valid_until: None,
            environment: Some(json!({
                "kind": "local",
                "id": "planr-local",
                "digest": "sha256:9999999999999999999999999999999999999999999999999999999999999999"
            })),
            snapshot_manifest_id: None,
        },
    );
    let environment_recovery = doctor_evidence(environment_dir.path(), &environment_db);
    assert_eq!(environment_recovery["status"], "warning");
    assert_eq!(environment_recovery["policy"]["state"], "recovered");
    assert_eq!(
        environment_recovery["policy"]["registry"]["probes"][0]["resolution"],
        "reprobed_environment_mismatch"
    );
    let environment_instance =
        environment_recovery["policy"]["registry"]["probes"][0]["instance_id"]
            .as_str()
            .unwrap();
    add_evidence_obligation_with_environment(
        environment_dir.path(),
        &environment_db,
        "pob-recovered-environment",
        first["policy"]["digest"].as_str().unwrap(),
        capability_instance_environment(&environment_db, environment_instance),
    );
    let environment_run = run_evidence_manifest(
        environment_dir.path(),
        &environment_db,
        "pob-recovered-environment",
    );
    assert_evidence_envelope(&environment_run, "evidence.run", true);
    assert_eq!(
        environment_run["object"]["results"][0]["attempt"]["capability_instance_id"],
        environment_instance
    );

    let registration_dir = tempdir().unwrap();
    let registration_db_dir = tempdir().unwrap();
    let registration_db = registration_db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(registration_dir.path());
    init_git_repo(registration_dir.path());
    init_evidence_project(
        registration_dir.path(),
        &registration_db,
        "Registration Mismatch Evidence Adapter",
    );
    seed_capability_instance_with_options(
        &seed_db,
        &registration_db,
        "vcap-seeded-registration-mismatch",
        SeedCapabilityInstanceOptions {
            runtime_target_json: None,
            valid_until: None,
            environment: None,
            snapshot_manifest_id: Some("verifier-generic-adapter-shadow"),
        },
    );
    let registration_doctor = doctor_evidence(registration_dir.path(), &registration_db);
    assert_eq!(registration_doctor["status"], "warning");
    assert_eq!(registration_doctor["policy"]["state"], "recovered");
    assert_eq!(
        registration_doctor["policy"]["registry"]["probes"][0]["resolution"],
        "reprobed_registration_mismatch"
    );
    let registration_instance =
        registration_doctor["policy"]["registry"]["probes"][0]["instance_id"]
            .as_str()
            .unwrap();
    add_evidence_obligation_with_environment(
        registration_dir.path(),
        &registration_db,
        "pob-registration-mismatch",
        first["policy"]["digest"].as_str().unwrap(),
        capability_instance_environment(&registration_db, "vcap-seeded-registration-mismatch"),
    );
    let mut registration_recovered = bind_obligation_to_authored_criterion(
        evidence_obligation_for(
            "pob-registration-recovered",
            first["policy"]["digest"].as_str().unwrap(),
            "com.example.health.status",
            "public evidence health",
            json!({"status": "ok"}),
            json!({"kind": "process", "uri": "local://health"}),
            capability_instance_environment(&registration_db, registration_instance),
            json!({"kind": "process", "id": "runtime-local"}),
            json!(["target_change", "policy_change", "adapter_schema_change"]),
            "sha256:8888888888888888888888888888888888888888888888888888888888888888",
        ),
        "pln-evidence-public",
        "criterion-evidence-public",
    );
    registration_recovered["observations"][0]["payload_schema"] =
        json!({"schema_ref": "schema://com.example.health.status"});
    let registration_recovered = superseding_obligation(
        registration_recovered,
        "pob-registration-recovered",
        "pob-registration-mismatch",
    );
    add_evidence_obligation_value(
        registration_dir.path(),
        &registration_db,
        "pob-registration-recovered",
        &registration_recovered,
    );
    let registration_run = run_evidence_manifest(
        registration_dir.path(),
        &registration_db,
        "pob-registration-recovered",
    );
    assert_evidence_envelope(&registration_run, "evidence.run", true);
    assert_eq!(
        registration_run["object"]["results"][0]["attempt"]["capability_instance_id"],
        registration_instance
    );
}

#[test]
fn evidence_run_enforces_frozen_source_before_receipt_commit() {
    let dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db = db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(dir.path());
    init_evidence_project(dir.path(), &db, "Evidence Frozen Source Boundary");
    init_git_repo(dir.path());

    let policy = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "--json", "evidence", "policy"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let capabilities = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "capability",
                "list",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let instance = capabilities["object"]["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instance| {
            instance["availability_status"] == "available"
                && instance["manifest_id"] == "verifier-generic-adapter"
        })
        .expect("available generic adapter instance");
    let instance_id = instance["id"].as_str().unwrap().to_string();
    let environment = instance["capability"]["environment"].clone();

    let mut obligation = evidence_obligation_for(
        "pob-frozen-source-boundary",
        policy["object"]["digest"].as_str().unwrap(),
        "com.example.health.status",
        "frozen source boundary",
        json!({"status": "ok"}),
        json!({"kind": "process", "uri": "local://health"}),
        environment,
        json!({"kind": "process", "id": "runtime-local"}),
        json!(["source_change", "target_change", "policy_change"]),
        "sha256:9999999999999999999999999999999999999999999999999999999999999999",
    );
    obligation["observations"][0]["payload_schema"] =
        json!({"schema_ref": "schema://com.example.health.status"});
    let obligation = bind_obligation_to_authored_criterion(
        obligation, "pln-evidence-public", "criterion-evidence-public",
    );
    add_evidence_obligation_value(dir.path(), &db, "pob-frozen-source-boundary", &obligation);

    let runtime_write_run = run_evidence_value_with_env_code(
        dir.path(),
        &db,
        "pob-frozen-source-boundary",
        &instance_id,
        json!({"kind": "process", "uri": "local://health"}),
        Some(json!({"PLANR_E2E_WRITE_PLANR_RUNTIME": "1"})),
        0,
    );
    assert_evidence_envelope(&runtime_write_run, "evidence.run", true);
    assert_eq!(runtime_write_run["object"]["verdict"], "passed");
    assert_eq!(
        evidence_attempt_receipt_counts(&db, "pob-frozen-source-boundary"),
        (1, 1),
        "allowed Planr runtime writes must not invalidate the canonical source digest"
    );

    let source_mutation_path = dir.path().join("frozen-source-mutation.run.json");
    fs::write(
        &source_mutation_path,
        serde_json::to_vec_pretty(&json!({
            "obligation_id": "pob-frozen-source-boundary",
            "capability_instance_id": instance_id,
            "target": {"kind": "process", "uri": "local://health"},
            "env": {"PLANR_E2E_MUTATE_SOURCE": "1"},
        }))
        .unwrap(),
    )
    .unwrap();
    let source_mutation = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "run",
                "--input",
                source_mutation_path.to_str().unwrap(),
            ])
            .assert()
            .failure()
            .get_output()
            .stdout,
    );
    assert_evidence_error(
        &source_mutation,
        "evidence.run",
        "internal_error",
        "without trusted receipt",
    );
    assert_eq!(
        evidence_attempt_receipt_counts(&db, "pob-frozen-source-boundary"),
        (2, 1),
        "source mutation must record one failed attempt and zero new trusted receipts"
    );
    let failed_attempt = latest_evidence_attempt_for_obligation(&db, "pob-frozen-source-boundary");
    assert_eq!(failed_attempt["status"], "failed");
    assert_eq!(failed_attempt["exit"]["error"], "stale_source");
    assert_eq!(
        failed_attempt["raw_result"]["planr_adapter_gap_reasons"],
        json!(["stale_source"])
    );

    fs::remove_file(dir.path().join("product-source.txt")).unwrap();
    let readiness_after_fix = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "readiness",
                "--scope",
                "obligation",
                "--id",
                "pob-frozen-source-boundary",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_evidence_envelope(&readiness_after_fix, "evidence.readiness", true);
    let run_index_path = dir.path().join(
        readiness_after_fix["object"]["run_index"]["repository_path"]
            .as_str()
            .unwrap(),
    );
    let mut tampered_run_index: Value =
        serde_json::from_slice(&fs::read(&run_index_path).unwrap()).unwrap();
    assert!(tampered_run_index["source"]["tree_digest"].is_string());
    assert!(tampered_run_index["policy_digest"].is_string());
    assert!(tampered_run_index["runs"][0]["input"]["execution_contract"].is_object());
    tampered_run_index["runs"][0]["input"]["target"]["uri"] = json!("local://tampered");
    let tampered_path = dir
        .path()
        .join(".planr/evidence/runs/tampered-readiness-run-index.json");
    fs::write(
        &tampered_path,
        serde_json::to_vec_pretty(&tampered_run_index).unwrap(),
    )
    .unwrap();
    let tampered = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "run",
                "--input",
                tampered_path.to_str().unwrap(),
            ])
            .assert()
            .failure()
            .get_output()
            .stdout,
    );
    assert_evidence_error(
        &tampered,
        "evidence.run",
        "conflict",
        "run-index seal is invalid",
    );
    assert_eq!(
        evidence_attempt_receipt_counts(&db, "pob-frozen-source-boundary"),
        (2, 1),
        "tampered readiness work must not start an attempt"
    );
    let selective_replay = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "run",
                "--input",
                run_index_path.to_str().unwrap(),
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_evidence_envelope(&selective_replay, "evidence.run", true);
    assert_eq!(
        selective_replay["object"]["schema_version"],
        "planr.evidence.run-index.result.v1"
    );
    assert_eq!(selective_replay["object"]["verdict"], "passed");
    assert_eq!(
        selective_replay["object"]["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        evidence_attempt_receipt_counts(&db, "pob-frozen-source-boundary"),
        (3, 2),
        "maker fix plus readiness/source snapshot should allow selective replay to create exactly one new receipt"
    );
    let reused = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "run",
                "--input",
                run_index_path.to_str().unwrap(),
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(reused["object"]["results"][0]["reused"], true);
    assert!(reused["object"]["results"][0]["reuse_key"].is_string());
    assert_eq!(
        evidence_attempt_receipt_counts(&db, "pob-frozen-source-boundary"),
        (3, 2),
        "an exact hermetic key hit must not execute or persist duplicate evidence"
    );
    let pre_commit_race_path = dir.path().join("frozen-source-pre-commit-race.run.json");
    fs::write(
        &pre_commit_race_path,
        serde_json::to_vec_pretty(&json!({
            "obligation_id": "pob-frozen-source-boundary",
            "capability_instance_id": instance_id,
            "target": {"kind": "process", "uri": "local://health"},
        }))
        .unwrap(),
    )
    .unwrap();
    let pre_commit_race = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env(
                "PLANR_TEST_EVIDENCE_PRE_COMMIT_MUTATE_SOURCE_PATH",
                "product-source.txt",
            )
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "run",
                "--input",
                pre_commit_race_path.to_str().unwrap(),
            ])
            .assert()
            .failure()
            .get_output()
            .stdout,
    );
    assert_evidence_error(
        &pre_commit_race,
        "evidence.run",
        "internal_error",
        "without trusted receipt",
    );
    assert_eq!(
        evidence_attempt_receipt_counts(&db, "pob-frozen-source-boundary"),
        (4, 2),
        "pre-commit source mutation must roll back staged trusted rows and persist only a failed attempt"
    );
    let pre_commit_attempt =
        latest_evidence_attempt_for_obligation(&db, "pob-frozen-source-boundary");
    assert_eq!(pre_commit_attempt["status"], "failed");
    assert_eq!(pre_commit_attempt["exit"]["error"], "stale_source");
    assert_eq!(
        pre_commit_attempt["raw_result"]["planr_adapter_gap_reasons"],
        json!(["stale_source"])
    );
}

#[test]
fn evidence_public_surfaces_share_canonical_service_and_status_codes() {
    let dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db = db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(dir.path());
    let generic_import_validator_digest = write_generic_import_validator_fixture(dir.path());
    init_evidence_project(dir.path(), &db, "Evidence Public Surfaces");
    init_git_repo(dir.path());
    let observed_item_id = create_test_item(
        dir.path(),
        &db,
        "Observe-only target",
        "evidence must not close this",
    );
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "approval",
            "request",
            &observed_item_id,
            "--reason",
            "prove evidence commands are observe-only",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "log",
            "add",
            "--item",
            &observed_item_id,
            "--summary",
            "observe-only baseline completion log",
            "--files",
            "src/app/evidence.rs",
            "--cmd",
            "true",
            "--tests",
            "true",
        ])
        .assert()
        .success();
    let adversarial_baseline = evidence_observe_snapshot(&db, &observed_item_id);
    let conn = Connection::open(&db).unwrap();
    register_receipt_binding_validator_for_test(&conn);
    let baseline_log_blocked: Option<String> = conn
        .query_row(
            "SELECT blocked_or_unverified FROM logs WHERE item_id = ?1 ORDER BY id LIMIT 1",
            [&observed_item_id],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "UPDATE logs SET blocked_or_unverified = 'adversarial blocked mutation' WHERE item_id = ?1",
        [&observed_item_id],
    )
    .unwrap();
    assert_ne!(
        evidence_observe_snapshot(&db, &observed_item_id),
        adversarial_baseline,
        "observe snapshot must detect in-place log blocked_or_unverified mutation"
    );
    conn.execute(
        "UPDATE logs SET blocked_or_unverified = ?1 WHERE item_id = ?2",
        rusqlite::params![baseline_log_blocked, observed_item_id],
    )
    .unwrap();
    assert_eq!(
        evidence_observe_snapshot(&db, &observed_item_id),
        adversarial_baseline
    );

    let observe_before = evidence_observe_snapshot(&db, &observed_item_id);
    let close_preview_before = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "map",
                "preview",
                "--close",
                &observed_item_id,
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );

    let policy_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "evidence", "policy"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let policy = single_json_document(&policy_output);
    assert_evidence_envelope(&policy, "evidence.policy", true);
    assert_eq!(policy["object"]["status"], "valid");
    assert_eq!(policy["object"]["registry"]["registered_capabilities"], 3);

    let capabilities_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "evidence",
            "capability",
            "list",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let capabilities = single_json_document(&capabilities_output);
    assert_evidence_envelope(&capabilities, "evidence.capability.list", true);
    let _instance = capabilities["object"]["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instance| {
            instance["availability_status"] == "available"
                && instance["manifest_id"] == "verifier-generic-adapter"
        })
        .expect("available capability instance");
    let recovered_capabilities_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "evidence",
            "capability",
            "list",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let recovered_capabilities = single_json_document(&recovered_capabilities_output);
    assert!(
        !recovered_capabilities["object"]["instances"]
            .as_array()
            .unwrap()
            .iter()
            .any(|instance| instance["id"] == "host-exp-chrome-browser-client"),
        "capability list must not promote embedded Phase-1 host fixtures to live instances"
    );
    assert!(
        !recovered_capabilities["object"]["manifests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|manifest| manifest["id"] == "host-chrome-browser-client-manifest"),
        "capability list must not promote embedded Phase-1 host fixtures to live manifests"
    );
    let host_show = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "evidence",
            "capability",
            "show",
            "host-exp-chrome-browser-client",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    assert_evidence_error(
        &single_json_document(&host_show),
        "evidence.capability.show",
        "not_found",
        "not found",
    );
    let host_manifest = recovered_capabilities["object"]["manifests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|manifest| manifest["id"] == "host-chrome-browser-client-manifest");
    assert!(host_manifest.is_none());
    let host_fixture_receipts: i64 = Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM evidence_receipts WHERE id = 'receipt-host-exp-chrome-browser-client'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        host_fixture_receipts, 0,
        "Phase-1 host fixture contract vector must not be persisted as live Evidence"
    );
    let recovery_probe = recovered_capabilities["object"]["registry"]["probes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|probe| probe["manifest_id"] == "verifier-generic-adapter")
        .unwrap();
    assert_eq!(recovery_probe["availability_status"], "available");
    assert_eq!(recovery_probe["reused"], true);
    let instance = recovered_capabilities["object"]["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instance| instance["id"] == recovery_probe["instance_id"])
        .expect("recovered capability instance");
    let instance_id = instance["id"].as_str().unwrap().to_string();
    let environment = instance["capability"]["environment"].clone();
    let validator_probe = recovered_capabilities["object"]["registry"]["probes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|probe| probe["manifest_id"] == "verifier-generic-import-validator")
        .unwrap();
    assert_eq!(validator_probe["availability_status"], "available");
    let validator_instance = recovered_capabilities["object"]["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instance| instance["id"] == validator_probe["instance_id"])
        .expect("generic import validator capability instance");
    let validator_instance_id = validator_instance["id"].as_str().unwrap().to_string();

    let obligation_runtime_config_digest =
        "sha256:8888888888888888888888888888888888888888888888888888888888888888";
    let obligation = evidence_obligation_for(
        "pob-public-run",
        policy["object"]["digest"].as_str().unwrap(),
        "com.example.health.status",
        "public evidence health",
        json!({"status": "ok"}),
        json!({"kind": "process", "uri": "local://health"}),
        environment.clone(),
        json!({"kind": "process", "id": "runtime-local"}),
        json!(["target_change", "policy_change", "adapter_schema_change"]),
        obligation_runtime_config_digest,
    );
    let obligation = bind_obligation_to_authored_criterion(
        obligation, "pln-evidence-public", "criterion-evidence-public",
    );
    let cli_add = add_evidence_obligation_value(dir.path(), &db, "pob-public-run", &obligation);
    assert_evidence_envelope(&cli_add, "evidence.migrate", true);
    assert_eq!(cli_add["object"]["created"][0]["id"], "pob-public-run");
    let project_id = cli_add["object"]["created"][0]["project_id"]
        .as_str()
        .unwrap()
        .to_string();

    let cli_show_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "evidence",
            "obligation",
            "show",
            "pob-public-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let cli_show = single_json_document(&cli_show_output);
    assert_evidence_envelope(&cli_show, "evidence.obligation.show", true);

    let mcp_input = format!(
        "{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "planr_evidence_obligation_show",
                "arguments": {"id": "pob-public-run"}
            }
        })
    );
    let mcp_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(mcp_input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_response: Value = serde_json::from_slice(&mcp_output).unwrap();
    let mcp_show = mcp_text_value(&mcp_response);
    assert_eq!(mcp_show, cli_show);

    let run_input_path = dir.path().join("evidence-run.json");
    fs::write(
        &run_input_path,
        serde_json::to_vec_pretty(&json!({
            "obligation_id": "pob-public-run",
            "capability_instance_id": instance_id,
            "target": {"kind": "process", "uri": "local://health"},
        }))
        .unwrap(),
    )
    .unwrap();
    let run_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "evidence",
            "run",
            "--input",
            run_input_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run = single_json_document(&run_output);
    assert_evidence_envelope(&run, "evidence.run", true);
    assert_eq!(run["object"]["verdict"], "passed");
    let attempt_id = run["object"]["attempt"]["id"].as_str().unwrap();
    let receipt_id = run["object"]["receipt"]["id"].as_str().unwrap();

    let mcp_run = mcp_tool(
        dir.path(),
        &db,
        20,
        "planr_evidence_run",
        json!({
            "input": {
                "obligation_id": "pob-public-run",
                "capability_instance_id": instance_id,
                "target": {"kind": "process", "uri": "local://health"}
            }
        }),
    );
    assert_evidence_envelope(&mcp_run, "evidence.run", true);
    assert_eq!(mcp_run["object"]["verdict"], "passed");
    let mcp_attempt_id = mcp_run["object"]["attempt"]["id"].as_str().unwrap();
    let mcp_receipt_id = mcp_run["object"]["receipt"]["id"].as_str().unwrap();

    let attempts_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "evidence",
            "attempts",
            "--obligation",
            "pob-public-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let attempts = single_json_document(&attempts_output);
    assert_evidence_envelope(&attempts, "evidence.attempts", true);
    assert_eq!(attempts["object"]["attempts"][0]["id"], attempt_id);

    let receipts_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "evidence",
            "receipts",
            "--obligation",
            "pob-public-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let receipts = single_json_document(&receipts_output);
    assert_evidence_envelope(&receipts, "evidence.receipts", true);
    assert_eq!(receipts["object"]["receipts"][0]["id"], receipt_id);

    let runner_artifact = json!({
        "command": ["cargo", "test"],
        "exit_code": 0,
        "status": "passed",
        "stdout_digest": "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "stderr_digest": "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "duration_ms": 12,
    });
    let runner_bytes = serde_jcs::to_vec(&runner_artifact).unwrap();
    fs::write(dir.path().join("runner.json"), &runner_bytes).unwrap();
    let import_input = json!({
        "id": "import-runner-public",
        "schema_version": "planr.evidence.import.v1",
        "source_kind": "artifact_import",
        "submitted_at": "2026-07-29T00:00:00Z",
        "format": "planr.runner_result.v1",
        "verifier_identity": {
            "kind": "adapter",
            "id": "verifier-runner-import",
            "name": "verifier-runner-import",
            "version": "1.0.0",
            "digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
        },
        "runner_result": {
            "kind": "planr_runner_result",
            "version": "1.0.0",
            "command": ["cargo", "test"],
            "exit_code": 0,
            "status": "passed",
            "stdout_digest": "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "stderr_digest": "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "duration_ms": 12,
        },
        "artifact_refs": [{
            "id": "artifact-runner",
            "kind": "runner-json",
            "digest": sha256_prefixed(&runner_bytes),
            "uri": "file://runner.json",
        }],
        "producer_metadata": {"client": "fixture-importer"},
    });
    let import_path = dir.path().join("import-runner.json");
    fs::write(
        &import_path,
        serde_json::to_vec_pretty(&import_input).unwrap(),
    )
    .unwrap();
    let import_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "evidence",
            "import",
            "--input",
            import_path.to_str().unwrap(),
            "--artifact-root",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let import = single_json_document(&import_output);
    assert_evidence_envelope(&import, "evidence.import", true);
    assert_eq!(import["object"]["verdict"], "valid");

    let mut mcp_import_input = import_input.clone();
    mcp_import_input["id"] = json!("import-runner-mcp");
    let mcp_import = mcp_tool(
        dir.path(),
        &db,
        21,
        "planr_evidence_import",
        json!({"artifact_root": dir.path().to_str().unwrap(), "input": mcp_import_input}),
    );
    assert_evidence_envelope(&mcp_import, "evidence.import", true);
    assert_eq!(mcp_import["object"]["verdict"], "valid");

    let mcp_attempts = mcp_tool(
        dir.path(),
        &db,
        22,
        "planr_evidence_attempts",
        json!({"obligation": "pob-public-run"}),
    );
    assert_evidence_envelope(&mcp_attempts, "evidence.attempts", true);
    assert!(
        mcp_attempts["object"]["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|attempt| attempt["id"] == attempt_id)
    );

    let mcp_receipts = mcp_tool(
        dir.path(),
        &db,
        23,
        "planr_evidence_receipts",
        json!({"obligation": "pob-public-run"}),
    );
    assert_evidence_envelope(&mcp_receipts, "evidence.receipts", true);
    assert!(
        mcp_receipts["object"]["receipts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|receipt| receipt["id"] == receipt_id)
    );

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
    wait_for_http_server(port);
    let http_show = http_json(&http_request(
        port,
        "GET",
        "/v1/evidence/obligations/pob-public-run",
        "",
    ));
    assert_eq!(http_show, cli_show);

    let http_capabilities = http_json(&http_request(port, "GET", "/v1/evidence/capabilities", ""));
    assert_evidence_envelope(&http_capabilities, "evidence.capability.list", true);
    assert_eq!(
        http_capabilities["object"]["registry"]["registered_capabilities"],
        recovered_capabilities["object"]["registry"]["registered_capabilities"]
    );
    assert!(
        http_capabilities["object"]["instances"]
            .as_array()
            .unwrap()
            .iter()
            .all(|instance| instance["id"] != "host-exp-chrome-browser-client"),
        "{http_capabilities}"
    );
    let mcp_capabilities = mcp_tool(
        dir.path(),
        &db,
        44,
        "planr_evidence_capability_list",
        json!({}),
    );
    assert_evidence_envelope(&mcp_capabilities, "evidence.capability.list", true);
    assert!(
        mcp_capabilities["object"]["instances"]
            .as_array()
            .unwrap()
            .iter()
            .all(|instance| instance["id"] != "host-exp-chrome-browser-client"),
        "{mcp_capabilities}"
    );

    let http_run = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/run",
        &json!({
            "obligation_id": "pob-public-run",
            "capability_instance_id": instance_id,
            "target": {"kind": "process", "uri": "local://health"}
        })
        .to_string(),
    ));
    assert_evidence_envelope(&http_run, "evidence.run", true);
    assert_eq!(http_run["object"]["verdict"], "passed");
    let http_attempt_id = http_run["object"]["attempt"]["id"].as_str().unwrap();
    let http_receipt_id = http_run["object"]["receipt"]["id"].as_str().unwrap();

    let rejected_run_trust_fields = [
        "attempt",
        "receipt",
        "receipt_json",
        "trusted_binding_json",
        "trusted_receipt",
        "receipt_status",
        "provenance",
    ];
    for (index, field) in rejected_run_trust_fields.iter().enumerate() {
        let forged_value = match *field {
            "receipt_status" => json!("trusted"),
            "provenance" => json!({"source": "agent_supplied"}),
            _ => json!({"id": format!("forged-{field}")}),
        };
        let mut forged_run_input = json!({
            "obligation_id": "pob-public-run",
            "capability_instance_id": instance_id,
            "target": {"kind": "process", "uri": "local://health"},
        });
        forged_run_input[*field] = forged_value;
        let before_counts = evidence_attempt_receipt_counts(&db, "pob-public-run");

        let cli_forged_path = dir.path().join(format!("evidence-run-forged-{field}.json"));
        fs::write(
            &cli_forged_path,
            serde_json::to_vec_pretty(&forged_run_input).unwrap(),
        )
        .unwrap();
        let cli_forged = single_json_document(
            &planr()
                .current_dir(dir.path())
                .args([
                    "--db",
                    db.to_str().unwrap(),
                    "--json",
                    "evidence",
                    "run",
                    "--input",
                    cli_forged_path.to_str().unwrap(),
                ])
                .assert()
                .failure()
                .get_output()
                .stdout,
        );
        assert_evidence_error(
            &cli_forged,
            "evidence.run",
            "bad_request",
            &format!("trusted receipt field: {field}"),
        );
        assert_eq!(
            evidence_attempt_receipt_counts(&db, "pob-public-run"),
            before_counts,
            "CLI forged run field {field} must not persist attempts or receipts"
        );

        let mcp_forged = mcp_tool_response(
            dir.path(),
            &db,
            100 + index as u64,
            "planr_evidence_run",
            json!({"input": forged_run_input.clone()}),
        );
        assert_mcp_evidence_error(
            &mcp_forged,
            "evidence.run",
            "bad_request",
            &format!("trusted receipt field: {field}"),
        );
        assert_eq!(
            evidence_attempt_receipt_counts(&db, "pob-public-run"),
            before_counts,
            "MCP forged run field {field} must not persist attempts or receipts"
        );

        let http_forged = http_request(
            port,
            "POST",
            "/v1/evidence/run",
            &forged_run_input.to_string(),
        );
        assert_http_evidence_error(
            &http_forged,
            "400 Bad Request",
            "evidence.run",
            "bad_request",
            &format!("trusted receipt field: {field}"),
        );
        assert_eq!(
            evidence_attempt_receipt_counts(&db, "pob-public-run"),
            before_counts,
            "HTTP forged run field {field} must not persist attempts or receipts"
        );
    }

    let mut http_import_input = import_input.clone();
    http_import_input["id"] = json!("import-runner-http");
    let http_import = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/import",
        &json!({"artifact_root": dir.path().to_str().unwrap(), "input": http_import_input})
            .to_string(),
    ));
    assert_evidence_envelope(&http_import, "evidence.import", true);
    assert_eq!(http_import["object"]["verdict"], "valid");

    let http_attempts = http_json(&http_request(
        port,
        "GET",
        "/v1/evidence/attempts?obligation=pob-public-run",
        "",
    ));
    assert_evidence_envelope(&http_attempts, "evidence.attempts", true);
    let http_attempt_ids = http_attempts["object"]["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|attempt| attempt["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    for expected in [attempt_id, mcp_attempt_id, http_attempt_id] {
        assert!(http_attempt_ids.contains(&expected.to_string()));
    }

    let http_receipts = http_json(&http_request(
        port,
        "GET",
        "/v1/evidence/receipts?obligation=pob-public-run",
        "",
    ));
    assert_evidence_envelope(&http_receipts, "evidence.receipts", true);
    let http_receipt_ids = http_receipts["object"]["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|receipt| receipt["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    for expected in [receipt_id, mcp_receipt_id, http_receipt_id] {
        assert!(http_receipt_ids.contains(&expected.to_string()));
    }

    let cli_readiness = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "readiness",
                "--scope",
                "obligation",
                "--id",
                "pob-public-run",
            ])
            .assert()
            .code(3)
            .get_output()
            .stdout,
    );
    let mcp_readiness = mcp_tool(
        dir.path(),
        &db,
        240,
        "planr_evidence_readiness",
        json!({"scope": "obligation", "id": "pob-public-run"}),
    );
    let http_readiness = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/readiness",
        r#"{"scope":"obligation","id":"pob-public-run"}"#,
    ));
    for readiness in [&cli_readiness, &mcp_readiness, &http_readiness] {
        assert_evidence_envelope(readiness, "evidence.readiness", true);
        assert_eq!(readiness["object"]["status"], "blocked");
        assert_eq!(
            readiness["object"]["active_obligation_ids"],
            json!(["pob-public-run"])
        );
    }

    let coverage_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "evidence",
            "coverage",
            "--scope",
            "obligation",
            "--id",
            "pob-public-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let coverage = single_json_document(&coverage_output);
    assert_evidence_envelope(&coverage, "evidence.coverage", true);
    assert_eq!(coverage["object"]["verdict"], "satisfied");
    assert_eq!(coverage["exit"]["code"], 0);

    let mcp_coverage = mcp_tool(
        dir.path(),
        &db,
        24,
        "planr_evidence_coverage",
        json!({"scope": "obligation", "id": "pob-public-run"}),
    );
    assert_evidence_envelope(&mcp_coverage, "evidence.coverage", true);
    assert_eq!(
        mcp_coverage["object"]["verdict"],
        coverage["object"]["verdict"]
    );
    assert_eq!(
        mcp_coverage["object"]["coverage"]["observation_coverage"][0]["status"],
        coverage["object"]["coverage"]["observation_coverage"][0]["status"]
    );

    let http_coverage = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/coverage",
        r#"{"scope":"obligation","id":"pob-public-run"}"#,
    ));
    assert_evidence_envelope(&http_coverage, "evidence.coverage", true);
    assert_eq!(
        http_coverage["object"]["verdict"],
        coverage["object"]["verdict"]
    );

    let http_explain = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/explain",
        r#"{"scope":"plan","id":"pln-evidence-public"}"#,
    ));
    assert_evidence_envelope(&http_explain, "evidence.explain", true);
    assert_eq!(http_explain["object"]["verdict"], "satisfied");
    assert_eq!(
        http_explain["object"]["explain"]["obligation_ids"],
        json!(["pob-public-run"])
    );
    let mcp_explain = mcp_tool(
        dir.path(),
        &db,
        25,
        "planr_evidence_explain",
        json!({"scope": "plan", "id": "pln-evidence-public"}),
    );
    assert_evidence_envelope(&mcp_explain, "evidence.explain", true);
    assert_eq!(
        mcp_explain["object"]["verdict"],
        http_explain["object"]["verdict"]
    );
    assert_eq!(
        mcp_explain["object"]["explain"]["obligation_ids"],
        http_explain["object"]["explain"]["obligation_ids"]
    );

    let mcp_bad_scope_input = format!(
        "{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "planr_evidence_coverage",
                "arguments": {"scope": "workspace", "id": "pob-public-run"}
            }
        })
    );
    let mcp_bad_scope_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(mcp_bad_scope_input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_bad_scope_response: Value = serde_json::from_slice(&mcp_bad_scope_output).unwrap();
    assert_eq!(mcp_bad_scope_response["result"]["isError"], true);
    let mcp_bad_scope = mcp_text_value(&mcp_bad_scope_response);
    assert_evidence_envelope(&mcp_bad_scope, "evidence.coverage", false);

    let http_bad_scope = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/coverage",
        r#"{"scope":"workspace","id":"pob-public-run"}"#,
    ));
    assert_evidence_envelope(&http_bad_scope, "evidence.coverage", false);

    let forged_run_path = dir.path().join("forged-run.json");
    fs::write(
        &forged_run_path,
        serde_json::to_vec_pretty(&json!({
            "obligation_id": "pob-public-run",
            "capability_instance_id": instance_id,
            "receipt_json": {"id": "forged"},
        }))
        .unwrap(),
    )
    .unwrap();
    let attempts_before_forged_run = evidence_row_count(&db, "evidence_attempts");
    let receipts_before_forged_run = evidence_row_count(&db, "evidence_receipts");
    let forged_run = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "evidence",
            "run",
            "--input",
            forged_run_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let forged_run = single_json_document(&forged_run);
    assert_evidence_envelope(&forged_run, "evidence.run", false);
    assert!(
        forged_run["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot construct trusted receipt field")
    );
    assert_eq!(
        evidence_row_count(&db, "evidence_attempts"),
        attempts_before_forged_run,
        "forged run input must not persist an attempt"
    );
    assert_eq!(
        evidence_row_count(&db, "evidence_receipts"),
        receipts_before_forged_run,
        "forged run input must not persist a receipt"
    );

    let import_error = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/import",
        r#"{"artifact_root":".","input":{"receipt_json":{}}}"#,
    ));
    assert_evidence_envelope(&import_error, "evidence.import", false);
    assert!(
        import_error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot construct trusted receipt field")
    );

    let generic_artifact_bytes = br#"{"status":"ok"}"#;
    fs::write(
        dir.path().join("generic-health.json"),
        generic_artifact_bytes,
    )
    .unwrap();
    let generic_artifact_refs = vec![json!({
        "id": "artifact-generic-health",
        "kind": "stdout-json",
        "digest": sha256_prefixed(generic_artifact_bytes),
        "uri": "file://generic-health.json",
    })];
    let generic_artifact_set_digest =
        generic_import_artifact_set_digest(&generic_artifact_refs, &[generic_artifact_bytes]);
    let mut generic_predicate = json!({
        "kind": "generic_versioned_adapter_predicate",
        "version": "1.0.0",
        "type": "planr.import.validator.generic_predicate",
        "outcome": "passed",
        "predicate": {"status": "ok"},
        "actual": {"status": "ok"},
        "attestation": {
            "kind": "planr_import_validator_attestation",
            "version": "1.0.0",
            "artifact_set_digest": generic_artifact_set_digest,
            "predicate_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "verifier_digest": generic_import_validator_digest.clone(),
            "verifier_instance_digest": sha256_json(&validator_instance["capability"]),
            "probe_execution_id": validator_instance["probe_execution_id"],
            "probe_result_digest": sha256_json(&validator_instance["capability"]["probe_result"]),
            "validator_attempt_id": "pending-validator-attempt",
            "validator_receipt_id": "pending-validator-receipt",
            "validator_receipt_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        }
    });
    let generic_predicate_digest = generic_import_predicate_digest(&generic_predicate);
    generic_predicate["attestation"]["predicate_digest"] = json!(generic_predicate_digest);
    let validator_result = json!({
        "kind": "planr.import.validator.generic_predicate.result",
        "version": "1.0.0",
        "verdict": "passed",
        "artifact_set_digest": generic_artifact_set_digest,
        "predicate_digest": generic_predicate_digest,
        "verifier_digest": generic_import_validator_digest.clone(),
        "verifier_instance_digest": sha256_json(&validator_instance["capability"]),
    });
    let mut validator_obligation = evidence_obligation_for(
        "pob-generic-import-validator",
        policy["object"]["digest"].as_str().unwrap(),
        "planr.import.validator.generic_predicate",
        "validated-artifact-import",
        validator_result.clone(),
        json!({"kind": "process", "uri": "local://generic-validator"}),
        validator_instance["capability"]["environment"].clone(),
        json!({"kind": "process", "id": "runtime-local"}),
        json!([]),
        "sha256:6767676767676767676767676767676767676767676767676767676767676767",
    );
    validator_obligation["plan_id"] = json!("pln-evidence-validator");
    validator_obligation["fixture_policy"]["disclosure_required"] = json!(false);
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('pln-evidence-validator', ?1, 'build', '/tmp/current/evidence-validator.plan.md',
                 'Evidence Validator Fixture', 'evidence-validator-fixture', 'ok',
                 'validator-fixture-hash', datetime('now'), datetime('now'))",
        [&project_id],
    )
    .unwrap();
    add_evidence_obligation_value(
        dir.path(),
        &db,
        "pob-generic-import-validator",
        &validator_obligation,
    );
    let validator_run = mcp_tool(
        dir.path(),
        &db,
        30,
        "planr_evidence_run",
        json!({"input": {
            "obligation_id": "pob-generic-import-validator",
            "capability_instance_id": validator_instance_id,
            "target": {"kind": "process", "uri": "local://generic-validator"},
            "env": {"PLANR_GENERIC_VALIDATOR_RESULT": validator_result.to_string()}
        }}),
    );
    assert_evidence_envelope(&validator_run, "evidence.run", true);
    assert_eq!(validator_run["object"]["verdict"], "passed");
    generic_predicate["attestation"]["validator_attempt_id"] =
        validator_run["object"]["attempt"]["id"].clone();
    generic_predicate["attestation"]["validator_receipt_id"] =
        validator_run["object"]["receipt"]["id"].clone();
    generic_predicate["attestation"]["validator_receipt_digest"] =
        validator_run["object"]["receipt"]["receipt_digest"].clone();
    let valid_generic_import = json!({
        "id": "import-valid-generic-attestation",
        "schema_version": "planr.evidence.import.v1",
        "source_kind": "artifact_import",
        "submitted_at": "2026-07-29T00:00:00Z",
        "format": "planr.generic_adapter_predicate.v1",
        "verifier_identity": {
            "kind": "adapter",
            "id": "verifier-generic-import-validator",
            "name": "verifier-generic-import-validator",
            "version": "1.0.0",
            "digest": generic_import_validator_digest,
        },
        "adapter_predicate": generic_predicate,
        "artifact_refs": generic_artifact_refs,
        "producer_metadata": {"client": "forged-generic-importer"},
    });
    let valid_generic = mcp_tool(
        dir.path(),
        &db,
        31,
        "planr_evidence_import",
        json!({"artifact_root": dir.path().to_str().unwrap(), "input": valid_generic_import}),
    );
    assert_evidence_envelope(&valid_generic, "evidence.import", true);
    assert_eq!(valid_generic["object"]["verdict"], "valid");

    let mut generic_case_id = 32;
    let mut assert_forged_generic =
        |label: &str, expected_message: &str, mutate: fn(&mut Value)| {
            let imports_before = evidence_row_count(&db, "evidence_validated_imports");
            let attempts_before = evidence_row_count(&db, "evidence_attempts");
            let receipts_before = evidence_row_count(&db, "evidence_receipts");
            let mut forged = valid_generic_import.clone();
            forged["id"] = json!(format!("import-forged-generic-{label}"));
            mutate(&mut forged);
            let result = mcp_tool(
                dir.path(),
                &db,
                generic_case_id,
                "planr_evidence_import",
                json!({"artifact_root": dir.path().to_str().unwrap(), "input": forged}),
            );
            generic_case_id += 1;
            assert_evidence_envelope(&result, "evidence.import", false);
            assert_eq!(result["error"]["message"], expected_message, "{result}");
            assert_eq!(
                evidence_row_count(&db, "evidence_validated_imports"),
                imports_before,
                "forged generic import {label} must not persist a validated import"
            );
            assert_eq!(
                evidence_row_count(&db, "evidence_attempts"),
                attempts_before,
                "forged generic import {label} must not persist an attempt"
            );
            assert_eq!(
                evidence_row_count(&db, "evidence_receipts"),
                receipts_before,
                "forged generic import {label} must not persist a receipt"
            );
        };
    assert_forged_generic(
        "artifact-set",
        "trusted receipt has invalid Planr-assigned binding adapter_predicate.attestation",
        |import| {
            import["adapter_predicate"]["attestation"]["artifact_set_digest"] =
                json!("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        },
    );
    assert_forged_generic(
        "predicate-body",
        "trusted receipt has invalid Planr-assigned binding adapter_predicate.attestation",
        |import| {
            import["adapter_predicate"]["actual"]["status"] = json!("mutated");
        },
    );
    assert_forged_generic(
        "predicate-digest",
        "trusted receipt has invalid Planr-assigned binding adapter_predicate.attestation",
        |import| {
            import["adapter_predicate"]["attestation"]["predicate_digest"] =
                json!("sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        },
    );
    assert_forged_generic(
        "verifier-digest",
        "trusted receipt has invalid Planr-assigned binding adapter_predicate.attestation",
        |import| {
            import["adapter_predicate"]["attestation"]["verifier_digest"] =
                json!("sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
        },
    );
    assert_forged_generic(
        "verifier-instance",
        "trusted receipt has invalid Planr-assigned binding adapter_predicate.attestation",
        |import| {
            import["adapter_predicate"]["attestation"]["verifier_instance_digest"] =
                json!("sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd");
        },
    );
    assert_forged_generic(
        "probe-result",
        "trusted receipt has invalid Planr-assigned binding adapter_predicate.attestation",
        |import| {
            import["adapter_predicate"]["attestation"]["probe_result_digest"] =
                json!("sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
        },
    );
    assert_forged_generic(
        "validator-attempt",
        "trusted receipt is missing Planr-assigned binding adapter_predicate.trusted_validator_observation",
        |import| {
            import["adapter_predicate"]["attestation"]["validator_attempt_id"] =
                json!("eatt-forged-validator");
        },
    );
    assert_forged_generic(
        "validator-receipt",
        "trusted receipt is missing Planr-assigned binding adapter_predicate.trusted_validator_observation",
        |import| {
            import["adapter_predicate"]["attestation"]["validator_receipt_id"] =
                json!("erec-forged-validator");
        },
    );
    assert_forged_generic(
        "validator-receipt-digest",
        "trusted receipt has invalid Planr-assigned binding adapter_predicate.trusted_validator_observation",
        |import| {
            import["adapter_predicate"]["attestation"]["validator_receipt_digest"] =
                json!("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
        },
    );

    let doctor_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "doctor"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doctor = single_json_document(&doctor_output);
    assert_eq!(doctor["evidence"]["status"], "ok");
    assert_eq!(doctor["evidence"]["observe_only"], true);
    let conn = Connection::open(&db).unwrap();
    conn.create_scalar_function(
        "planr_evidence_receipt_binding_is_valid",
        2,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let trusted_binding_json: String = ctx.get(0)?;
            let receipt_json: String = ctx.get(1)?;
            let trusted_binding: Value = serde_json::from_str(&trusted_binding_json)
                .map_err(|err| rusqlite::Error::UserFunctionError(Box::new(err)))?;
            let receipt: Value = serde_json::from_str(&receipt_json)
                .map_err(|err| rusqlite::Error::UserFunctionError(Box::new(err)))?;
            Ok(
                if planr::evidence::trusted_receipt_binding_matches_receipt(
                    trusted_binding,
                    receipt,
                )
                .is_ok()
                {
                    1i64
                } else {
                    0i64
                },
            )
        },
    )
    .unwrap();
    conn.create_scalar_function(
        "planr_evidence_artifact_path_is_contained",
        3,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let root_path: String = ctx.get(0)?;
            let root_kind: String = ctx.get(1)?;
            let path: String = ctx.get(2)?;
            let root = match root_kind.as_str() {
                "project" => std::path::PathBuf::from(&root_path),
                "planr" => std::path::PathBuf::from(&root_path).join(".planr"),
                "artifact_store" => std::path::PathBuf::from(&root_path).join(".planr/artifacts"),
                _ => return Ok(0_i64),
            };
            let relative = std::path::Path::new(&path);
            if !relative.is_relative()
                || relative.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            {
                return Ok(0_i64);
            }
            let Ok(root) = root.canonicalize() else {
                return Ok(0_i64);
            };
            let Ok(candidate) = root.join(relative).canonicalize() else {
                return Ok(0_i64);
            };
            Ok(if candidate.starts_with(root) {
                1_i64
            } else {
                0_i64
            })
        },
    )
    .unwrap();
    conn.execute(
        "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
         VALUES ('p-other', 'Other', '.', 'active', datetime('now'), datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('pln-other-evidence-public', 'p-other', 'build', '/tmp/other/evidence-public-fixture.plan.md', 'Other Evidence Public', 'other-evidence-public', 'ok', 'other-fixture-hash', datetime('now'), datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO proof_obligations(
          id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
          binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
          assurance_policy_json, policy_digest, config_digest, created_at
        ) VALUES (
          'pob-other-project', 'p-other', 'pln-other-evidence-public', NULL,
          'crit-other', 1, 'Other project', 1, ?1, ?2, '{}', '{}', ?3, ?4, datetime('now')
        )",
        rusqlite::params![
            serde_json::to_string(&obligation["observations"]).unwrap(),
            serde_json::to_string(&obligation["fixture_policy"]).unwrap(),
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO evidence_attempts(
          id, project_id, obligation_id, capability_instance_id, attempt_status,
          execution_contract_digest, resolved_command_json, environment_digest,
          started_at, completed_at, exit_code, output_bounds_json, attempt_json, created_at
        ) VALUES (
          'attempt-other-project', 'p-other', 'pob-other-project', ?1, 'passed',
          ?2, '{}', ?3, datetime('now'), datetime('now'), 0, '{}', ?4, datetime('now')
        )",
        rusqlite::params![
            instance_id,
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            json!({
                "id": "attempt-other-project",
                "status": "passed",
                "exit": {"exit_code": 0, "signal": null, "error": null}
            })
            .to_string(),
        ],
    )
    .unwrap();
    let (foreign_trusted_binding_json, foreign_receipt_json): (String, String) = conn
        .query_row(
            "SELECT trusted_binding_json, receipt_json FROM evidence_receipts WHERE id = ?1",
            [receipt_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO evidence_receipts(
          id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
          trusted_binding_json, observations_json, provenance_json, receipt_json, created_at
        ) VALUES (
          'receipt-other-project', 'p-other', 'pob-other-project', 'attempt-other-project',
          'trusted', 'sha256:3333333333333333333333333333333333333333333333333333333333333333',
          ?1, '[]', '{}', ?2, datetime('now')
        )",
        rusqlite::params![foreign_trusted_binding_json, foreign_receipt_json],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('pln-other', ?1, 'build', '/tmp/current/other.plan.md', 'Other Current Plan', 'other-current-plan', 'ok', 'other-current-hash', datetime('now'), datetime('now'))",
        [&project_id],
    )
    .unwrap();
    for (id, plan, criterion, binding, supersedes) in [
        (
            "pob-advisory",
            "pln-evidence-public",
            "crit-advisory",
            0,
            None,
        ),
        (
            "pob-superseded",
            "pln-evidence-public",
            "crit-superseded",
            1,
            None,
        ),
        (
            "pob-superseding-other-plan",
            "pln-other",
            "crit-superseding",
            1,
            Some("pob-superseded"),
        ),
    ] {
        conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, policy_digest, config_digest, supersedes_obligation_id, created_at
            ) VALUES (
              ?1, ?2, ?3, NULL, ?4, 1, ?1, ?5, ?6, ?7, '{}', '{}', ?8, ?9, ?10, datetime('now')
            )",
            rusqlite::params![
                id,
                project_id,
                plan,
                criterion,
                binding,
                serde_json::to_string(&obligation["observations"]).unwrap(),
                serde_json::to_string(&obligation["fixture_policy"]).unwrap(),
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                supersedes,
            ],
        )
        .unwrap();
    }
    let public_observation_requirements =
        serde_json::to_string(&obligation["observations"]).unwrap();
    let public_fixture_policy = serde_json::to_string(&obligation["fixture_policy"]).unwrap();
    let public_freshness_policy = serde_json::to_string(&json!({"invalidate_on": []})).unwrap();
    let source_freshness_policy =
        serde_json::to_string(&json!({"invalidate_on": ["source_change"]})).unwrap();
    let config_freshness_policy =
        serde_json::to_string(&json!({"invalidate_on": ["configuration_change"]})).unwrap();
    let evidence_plan_path = dir
        .path()
        .join(".planr/plans/build/evidence-public-fixture.plan.md")
        .to_string_lossy()
        .to_string();
    conn.execute(
        "INSERT INTO items(
          id, project_id, title, description, status, work_type, plan_path, created_at, updated_at
        ) VALUES (
          'item-wrong-binding-contained', ?1, 'Wrong binding contained item',
          'contained fixture item for Evidence binding matrix', 'pending', 'test', ?2,
          datetime('now'), datetime('now')
        )",
        rusqlite::params![project_id, evidence_plan_path],
    )
    .unwrap();
    let insert_binding_obligation = |id: &str,
                                     plan: &str,
                                     item: Option<&str>,
                                     criterion: &str,
                                     version: i64,
                                     config_digest: &str,
                                     source_digest: Option<&str>,
                                     freshness_policy: &str,
                                     supersedes: Option<&str>| {
        conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, policy_digest, config_digest, source_digest,
              supersedes_obligation_id, created_at
            ) VALUES (
              ?1, ?2, ?3, ?4, ?5, ?6, ?1, 1, ?7, ?8, ?9, '{}', ?10, ?11, ?12, ?13, datetime('now')
            )",
            rusqlite::params![
                id,
                project_id,
                plan,
                item,
                criterion,
                version,
                public_observation_requirements,
                public_fixture_policy,
                freshness_policy,
                policy["object"]["digest"].as_str().unwrap(),
                config_digest,
                source_digest,
                supersedes,
            ],
        )
        .unwrap();
    };
    let mut binding_mcp_id = 60;
    let mut run_binding_obligation = |obligation_id: &str| -> (String, String) {
        let run = mcp_tool(
            dir.path(),
            &db,
            binding_mcp_id,
            "planr_evidence_run",
            json!({"input": {
                "obligation_id": obligation_id,
                "capability_instance_id": instance_id,
                "target": {"kind": "process", "uri": "local://health"}
            }}),
        );
        binding_mcp_id += 1;
        assert_evidence_envelope(&run, "evidence.run", true);
        assert_eq!(run["object"]["verdict"], "passed", "{run}");
        (
            run["object"]["attempt"]["id"].as_str().unwrap().to_string(),
            run["object"]["receipt"]["id"].as_str().unwrap().to_string(),
        )
    };

    insert_binding_obligation(
        "pob-binding-positive",
        "pln-evidence-public",
        None,
        "crit-binding-positive",
        1,
        obligation_runtime_config_digest,
        Option::<&str>::None,
        &public_freshness_policy,
        Option::<&str>::None,
    );
    let (_, positive_receipt_id) = run_binding_obligation("pob-binding-positive");
    let positive_coverage = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/coverage",
        r#"{"scope":"criterion","id":"crit-binding-positive"}"#,
    ));
    assert_evidence_envelope(&positive_coverage, "evidence.coverage", true);
    assert_eq!(positive_coverage["object"]["verdict"], "satisfied");
    assert_eq!(
        positive_coverage["object"]["coverage"]["observation_coverage"][0]["covering_receipt_ids"],
        json!([positive_receipt_id])
    );
    assert_persisted_coverage_verdict(&db, &positive_coverage);

    let binding_cases = [
        (
            "plan",
            "pob-binding-plan-base",
            "pob-binding-plan-current",
            "pln-other",
            "pln-evidence-public",
            None,
            None,
            "crit-binding-plan",
            1,
            1,
            obligation_runtime_config_digest,
            obligation_runtime_config_digest,
            Option::<&str>::None,
            Option::<&str>::None,
            &public_freshness_policy,
            "target_mismatch",
            &["target_mismatch"][..],
        ),
        (
            "item",
            "pob-binding-item-base",
            "pob-binding-item-current",
            "pln-evidence-public",
            "pln-evidence-public",
            None,
            Some("item-wrong-binding-contained"),
            "crit-binding-item",
            1,
            1,
            obligation_runtime_config_digest,
            obligation_runtime_config_digest,
            Option::<&str>::None,
            Option::<&str>::None,
            &public_freshness_policy,
            "target_mismatch",
            &["target_mismatch"][..],
        ),
        (
            "criterion",
            "pob-binding-criterion-base",
            "pob-binding-criterion-current",
            "pln-evidence-public",
            "pln-evidence-public",
            None,
            None,
            "crit-binding-criterion-base",
            1,
            1,
            obligation_runtime_config_digest,
            obligation_runtime_config_digest,
            Option::<&str>::None,
            Option::<&str>::None,
            &public_freshness_policy,
            "target_mismatch",
            &["target_mismatch"][..],
        ),
        (
            "source",
            "pob-binding-source-base",
            "pob-binding-source-current",
            "pln-evidence-public",
            "pln-evidence-public",
            None,
            None,
            "crit-binding-source",
            1,
            1,
            obligation_runtime_config_digest,
            obligation_runtime_config_digest,
            Some("sha256:1212121212121212121212121212121212121212121212121212121212121212"),
            Some("sha256:3434343434343434343434343434343434343434343434343434343434343434"),
            &source_freshness_policy,
            "stale_configuration",
            &["stale_configuration"][..],
        ),
        (
            "config",
            "pob-binding-config-base",
            "pob-binding-config-current",
            "pln-evidence-public",
            "pln-evidence-public",
            None,
            None,
            "crit-binding-config",
            1,
            1,
            obligation_runtime_config_digest,
            "sha256:5656565656565656565656565656565656565656565656565656565656565656",
            Option::<&str>::None,
            Option::<&str>::None,
            &config_freshness_policy,
            "stale_configuration",
            &["stale_configuration"][..],
        ),
        (
            "version",
            "pob-binding-version-base",
            "pob-binding-version-current",
            "pln-evidence-public",
            "pln-evidence-public",
            None,
            None,
            "crit-binding-version",
            1,
            2,
            obligation_runtime_config_digest,
            obligation_runtime_config_digest,
            Option::<&str>::None,
            Option::<&str>::None,
            &public_freshness_policy,
            "stale_configuration",
            &["stale_configuration"][..],
        ),
    ];
    for (
        label,
        base_id,
        current_id,
        base_plan,
        current_plan,
        base_item,
        current_item,
        criterion,
        base_version,
        current_version,
        base_config,
        current_config,
        base_source,
        current_source,
        freshness_policy,
        expected_gap,
        expected_gaps,
    ) in binding_cases
    {
        insert_binding_obligation(
            base_id,
            base_plan,
            base_item,
            criterion,
            base_version,
            base_config,
            base_source,
            freshness_policy,
            Option::<&str>::None,
        );
        let (_, candidate_receipt_id) = run_binding_obligation(base_id);
        let current_criterion = if label == "criterion" {
            "crit-binding-criterion-current"
        } else {
            criterion
        };
        insert_binding_obligation(
            current_id,
            current_plan,
            current_item,
            current_criterion,
            current_version,
            current_config,
            current_source,
            freshness_policy,
            Some(base_id),
        );
        let coverage = http_json(&http_request(
            port,
            "POST",
            "/v1/evidence/coverage",
            &json!({"scope": "criterion", "id": current_criterion}).to_string(),
        ));
        assert_evidence_envelope(&coverage, "evidence.coverage", true);
        assert_ne!(
            coverage["object"]["verdict"], "satisfied",
            "{label}: {coverage}"
        );
        assert_eq!(
            coverage["object"]["coverage"]["observation_coverage"][0]["gap_reason"], expected_gap,
            "{label}: {coverage}"
        );
        assert_json_string_set(
            &coverage["object"]["coverage"]["observation_coverage"][0]["gap_reasons"],
            expected_gaps,
        );
        assert_eq!(
            coverage["object"]["coverage"]["observation_coverage"][0]["attempted_receipt_ids"],
            json!([candidate_receipt_id]),
            "{label}: {coverage}"
        );
        assert!(
            coverage["object"]["receipt_digests"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{label}: {coverage}"
        );
        assert_persisted_coverage_verdict(&db, &coverage);
    }

    let insert_populated_artifact_receipt = |case_label: &str,
                                             attempt_id: &str,
                                             base_receipt_id: &str,
                                             populated_receipt_id: &str,
                                             attempt_artifact_id: &str,
                                             receipt_artifact_id: &str,
                                             attempt_kind: &str,
                                             receipt_kind: &str,
                                             attempt_path: &str,
                                             receipt_uri: &str,
                                             attempt_root_kind: &str,
                                             receipt_root_kind: &str,
                                             receipt_digest_override: Option<&str>|
     -> String {
        let artifact_bytes = format!("artifact-binding-{case_label}\n");
        let attempt_digest = sha256_prefixed(artifact_bytes.as_bytes());
        let attempt_root = match attempt_root_kind {
            "project" => dir.path().to_path_buf(),
            "planr" => dir.path().join(".planr"),
            "artifact_store" => dir.path().join(".planr/artifacts"),
            root_kind => panic!("unsupported artifact root in fixture: {root_kind}"),
        };
        std::fs::create_dir_all(&attempt_root).unwrap();
        if let Some(parent) = attempt_root.join(attempt_path).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let attempt_file = attempt_root.join(attempt_path);
        std::fs::write(&attempt_file, artifact_bytes.as_bytes()).unwrap();
        if let Some(receipt_path) = receipt_uri.strip_prefix("file://") {
            let receipt_root = match receipt_root_kind {
                "project" => dir.path().to_path_buf(),
                "planr" => dir.path().join(".planr"),
                "artifact_store" => dir.path().join(".planr/artifacts"),
                root_kind => panic!("unsupported artifact root in fixture: {root_kind}"),
            };
            if let Some(parent) = receipt_root.join(receipt_path).parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(receipt_root.join(receipt_path), artifact_bytes.as_bytes()).unwrap();
        }
        let receipt_artifact_digest = receipt_digest_override.unwrap_or(attempt_digest.as_str());
        conn.execute(
            "INSERT INTO artifacts(
              id, project_id, item_id, name, kind, path, content, mime_type, size_bytes, created_at
            ) VALUES (
              ?1, ?2, NULL, ?3, ?4, ?5, '{}', 'application/json', ?6, datetime('now')
            )",
            rusqlite::params![
                attempt_artifact_id,
                project_id,
                format!("Artifact binding {case_label}"),
                attempt_kind,
                attempt_path,
                artifact_bytes.len() as i64,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO evidence_attempt_artifacts(
              id, attempt_id, artifact_id, logical_name, path, digest, root_kind, created_at
            ) VALUES (
              ?1, ?2, ?3, 'stdout', ?4, ?5, ?6, datetime('now')
            )",
            rusqlite::params![
                format!("artifact-binding-{case_label}"),
                attempt_id,
                attempt_artifact_id,
                attempt_path,
                attempt_digest,
                attempt_root_kind,
            ],
        )
        .unwrap();
        let stored_attempt_artifact: (String, String, String) = conn
            .query_row(
                "SELECT path, digest, root_kind FROM evidence_attempt_artifacts WHERE id = ?1",
                [format!("artifact-binding-{case_label}")],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            stored_attempt_artifact,
            (
                attempt_path.to_string(),
                attempt_digest.clone(),
                attempt_root_kind.to_string()
            )
        );
        let (trusted_binding_json, receipt_json): (String, String) = conn
            .query_row(
                "SELECT trusted_binding_json, receipt_json FROM evidence_receipts WHERE id = ?1",
                [base_receipt_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let mut receipt: Value = serde_json::from_str(&receipt_json).unwrap();
        receipt["id"] = json!(populated_receipt_id);
        receipt["artifacts"] = json!([{
            "id": receipt_artifact_id,
            "kind": receipt_kind,
            "digest": receipt_artifact_digest,
            "uri": receipt_uri,
            "root_kind": receipt_root_kind,
        }]);
        receipt["raw_result"] = json!({
            "kind": "artifact",
            "digest": receipt_artifact_digest,
            "artifact_id": receipt_artifact_id,
        });
        let trusted_receipt_digest = sha256_json_without_field(&receipt, "receipt_digest");
        receipt["receipt_digest"] = json!(trusted_receipt_digest);
        conn.execute(
            "INSERT INTO evidence_receipts(
              id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
              trusted_binding_json, observations_json, provenance_json, receipt_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, 'trusted', ?5, ?6, ?7, ?8, ?9, datetime('now'))",
            rusqlite::params![
                populated_receipt_id,
                project_id,
                receipt["obligation_id"].as_str().unwrap(),
                attempt_id,
                trusted_receipt_digest,
                trusted_binding_json,
                receipt["observations"].to_string(),
                receipt["provenance"].to_string(),
                receipt.to_string(),
            ],
        )
        .unwrap();
        trusted_receipt_digest
    };

    insert_binding_obligation(
        "pob-binding-artifact-positive",
        "pln-evidence-public",
        None,
        "crit-binding-artifact-positive",
        1,
        obligation_runtime_config_digest,
        Option::<&str>::None,
        &public_freshness_policy,
        Option::<&str>::None,
    );
    let (artifact_positive_attempt_id, artifact_positive_base_receipt_id) =
        run_binding_obligation("pob-binding-artifact-positive");
    let artifact_positive_receipt_digest = insert_populated_artifact_receipt(
        "positive",
        &artifact_positive_attempt_id,
        &artifact_positive_base_receipt_id,
        "receipt-binding-artifact-positive-populated",
        "artifact-binding-positive",
        "artifact-binding-positive",
        "stdout",
        "stdout",
        "artifact-positive.json",
        "file://artifact-positive.json",
        "project",
        "project",
        None,
    );
    let artifact_positive_coverage = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/coverage",
        r#"{"scope":"criterion","id":"crit-binding-artifact-positive"}"#,
    ));
    assert_evidence_envelope(&artifact_positive_coverage, "evidence.coverage", true);
    assert_eq!(artifact_positive_coverage["object"]["verdict"], "satisfied");
    assert_json_string_set(
        &artifact_positive_coverage["object"]["coverage"]["observation_coverage"][0]["covering_receipt_ids"],
        &["receipt-binding-artifact-positive-populated"],
    );
    assert_json_string_set(
        &artifact_positive_coverage["object"]["receipt_digests"],
        &[artifact_positive_receipt_digest.as_str()],
    );
    assert_persisted_coverage_verdict(&db, &artifact_positive_coverage);

    for (
        label,
        attempt_artifact_id,
        receipt_artifact_id,
        attempt_kind,
        receipt_kind,
        attempt_path,
        receipt_uri,
        attempt_root_kind,
        receipt_root_kind,
        receipt_digest_override,
    ) in [
        (
            "digest",
            "artifact-binding-digest",
            "artifact-binding-digest",
            "stdout",
            "stdout",
            "artifact-digest.json",
            "file://artifact-digest.json",
            "project",
            "project",
            Some("sha256:3434343434343434343434343434343434343434343434343434343434343434"),
        ),
        (
            "path",
            "artifact-binding-path",
            "artifact-binding-path",
            "stdout",
            "stdout",
            "artifact-path-attempt.json",
            "file://artifact-path-receipt.json",
            "project",
            "project",
            None,
        ),
        (
            "root",
            "artifact-binding-root",
            "artifact-binding-root",
            "stdout",
            "stdout",
            "artifact-root.json",
            "file://artifact-root.json",
            "planr",
            "project",
            None,
        ),
        (
            "id",
            "artifact-binding-id-attempt",
            "artifact-binding-id-receipt",
            "stdout",
            "stdout",
            "artifact-id.json",
            "file://artifact-id.json",
            "project",
            "project",
            None,
        ),
        (
            "kind",
            "artifact-binding-kind",
            "artifact-binding-kind",
            "stderr",
            "stdout",
            "artifact-kind.json",
            "file://artifact-kind.json",
            "project",
            "project",
            None,
        ),
    ] {
        let obligation_id = format!("pob-binding-artifact-{label}");
        let criterion_id = format!("crit-binding-artifact-{label}");
        insert_binding_obligation(
            &obligation_id,
            "pln-evidence-public",
            None,
            &criterion_id,
            1,
            obligation_runtime_config_digest,
            Option::<&str>::None,
            &public_freshness_policy,
            Option::<&str>::None,
        );
        let (artifact_attempt_id, base_receipt_id) = run_binding_obligation(&obligation_id);
        let populated_receipt_id = format!("receipt-binding-artifact-{label}-populated");
        insert_populated_artifact_receipt(
            label,
            &artifact_attempt_id,
            &base_receipt_id,
            &populated_receipt_id,
            attempt_artifact_id,
            receipt_artifact_id,
            attempt_kind,
            receipt_kind,
            attempt_path,
            receipt_uri,
            attempt_root_kind,
            receipt_root_kind,
            receipt_digest_override,
        );
        let artifact_coverage = http_json(&http_request(
            port,
            "POST",
            "/v1/evidence/coverage",
            &json!({"scope": "criterion", "id": criterion_id}).to_string(),
        ));
        assert_evidence_envelope(&artifact_coverage, "evidence.coverage", true);
        assert_eq!(
            artifact_coverage["object"]["coverage"]["observation_coverage"][0]["gap_reason"],
            "stale_source",
            "{label}: {artifact_coverage}"
        );
        assert_json_string_set(
            &artifact_coverage["object"]["coverage"]["observation_coverage"][0]["gap_reasons"],
            &["stale_source"],
        );
        assert_json_string_set(
            &artifact_coverage["object"]["coverage"]["observation_coverage"][0]["attempted_receipt_ids"],
            &[base_receipt_id.as_str(), populated_receipt_id.as_str()],
        );
        assert!(
            artifact_coverage["object"]["receipt_digests"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{label}: {artifact_coverage}"
        );
        assert_persisted_coverage_verdict(&db, &artifact_coverage);
    }

    insert_binding_obligation(
        "pob-binding-project-current",
        "pln-evidence-public",
        None,
        "crit-binding-project",
        1,
        obligation_runtime_config_digest,
        Option::<&str>::None,
        &public_freshness_policy,
        Option::<&str>::None,
    );
    let project_coverage = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/coverage",
        r#"{"scope":"criterion","id":"crit-binding-project"}"#,
    ));
    assert_evidence_envelope(&project_coverage, "evidence.coverage", true);
    assert_eq!(project_coverage["object"]["verdict"], "unsatisfied");
    assert_eq!(
        project_coverage["object"]["coverage"]["observation_coverage"][0]["gap_reason"],
        "missing_observation"
    );
    assert!(
        project_coverage["object"]["coverage"]["observation_coverage"][0]
            .get("attempted_receipt_ids")
            .is_none()
    );
    assert!(
        !http_json(&http_request(port, "GET", "/v1/evidence/receipts", ""))["object"]["receipts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|receipt| receipt["id"] == "receipt-other-project")
    );
    assert_persisted_coverage_verdict(&db, &project_coverage);

    let isolated_explain = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/explain",
        r#"{"scope":"plan","id":"pln-evidence-public"}"#,
    ));
    assert_eq!(
        isolated_explain["object"]["explain"]["obligation_ids"],
        json!([
            "pob-binding-artifact-digest",
            "pob-binding-artifact-id",
            "pob-binding-artifact-kind",
            "pob-binding-artifact-path",
            "pob-binding-artifact-positive",
            "pob-binding-artifact-root",
            "pob-binding-config-current",
            "pob-binding-criterion-current",
            "pob-binding-plan-current",
            "pob-binding-positive",
            "pob-binding-project-current",
            "pob-binding-source-current",
            "pob-binding-version-current",
            "pob-public-run",
            "pob-binding-item-current"
        ])
    );

    let cli_all_attempts = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "attempts",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert!(
        !cli_all_attempts["object"]["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|attempt| attempt["id"] == "attempt-other-project")
    );
    let cli_foreign_attempt = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "attempts",
                "--id",
                "attempt-other-project",
            ])
            .assert()
            .failure()
            .code(1)
            .get_output()
            .stdout,
    );
    assert_evidence_envelope(&cli_foreign_attempt, "evidence.attempts", false);

    let cli_all_receipts = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "receipts",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert!(
        !cli_all_receipts["object"]["receipts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|receipt| receipt["id"] == "receipt-other-project")
    );
    let cli_foreign_receipt = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "receipts",
                "--id",
                "receipt-other-project",
            ])
            .assert()
            .failure()
            .code(1)
            .get_output()
            .stdout,
    );
    assert_evidence_envelope(&cli_foreign_receipt, "evidence.receipts", false);

    let mcp_all_attempts = mcp_tool(dir.path(), &db, 26, "planr_evidence_attempts", json!({}));
    assert!(
        !mcp_all_attempts["object"]["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|attempt| attempt["id"] == "attempt-other-project")
    );
    let mcp_foreign_attempt = mcp_tool(
        dir.path(),
        &db,
        27,
        "planr_evidence_attempts",
        json!({"id": "attempt-other-project"}),
    );
    assert_evidence_envelope(&mcp_foreign_attempt, "evidence.attempts", false);

    let mcp_all_receipts = mcp_tool(dir.path(), &db, 28, "planr_evidence_receipts", json!({}));
    assert!(
        !mcp_all_receipts["object"]["receipts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|receipt| receipt["id"] == "receipt-other-project")
    );
    let mcp_foreign_receipt = mcp_tool(
        dir.path(),
        &db,
        29,
        "planr_evidence_receipts",
        json!({"id": "receipt-other-project"}),
    );
    assert_evidence_envelope(&mcp_foreign_receipt, "evidence.receipts", false);

    let http_all_attempts = http_json(&http_request(port, "GET", "/v1/evidence/attempts", ""));
    assert!(
        !http_all_attempts["object"]["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|attempt| attempt["id"] == "attempt-other-project")
    );
    let http_foreign_attempt = http_json(&http_request(
        port,
        "GET",
        "/v1/evidence/attempts/attempt-other-project",
        "",
    ));
    assert_evidence_envelope(&http_foreign_attempt, "evidence.attempts", false);

    let http_all_receipts = http_json(&http_request(port, "GET", "/v1/evidence/receipts", ""));
    assert!(
        !http_all_receipts["object"]["receipts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|receipt| receipt["id"] == "receipt-other-project")
    );
    let http_foreign_receipt = http_json(&http_request(
        port,
        "GET",
        "/v1/evidence/receipts/receipt-other-project",
        "",
    ));
    assert_evidence_envelope(&http_foreign_receipt, "evidence.receipts", false);

    let observe_after = evidence_observe_snapshot(&db, &observed_item_id);
    let close_preview_after = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "map",
                "preview",
                "--close",
                &observed_item_id,
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(observe_after, observe_before);
    assert_eq!(close_preview_after, close_preview_before);

    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn evidence_process_adapter_semantic_mismatch_does_not_satisfy_coverage() {
    let dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db = db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(dir.path());
    init_evidence_project(dir.path(), &db, "Evidence Semantic Mismatch");
    init_git_repo(dir.path());

    let policy = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "--json", "evidence", "policy"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let policy_digest = policy["object"]["digest"].as_str().unwrap();
    let capabilities = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "capability",
                "list",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let instance = capabilities["object"]["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instance| {
            instance["manifest_id"] == "verifier-generic-adapter"
                && instance["availability_status"] == "available"
        })
        .expect("generic adapter instance");
    let instance_id = instance["id"].as_str().unwrap();
    let environment = instance["capability"]["environment"].clone();
    let obligation = evidence_obligation_for(
        "pob-semantic-mismatch",
        policy_digest,
        "com.example.health.status",
        "semantic mismatch health",
        json!({"status": "missing"}),
        json!({"kind": "process", "uri": "local://health"}),
        environment,
        json!({"kind": "process", "id": "runtime-local"}),
        json!(["target_change", "policy_change", "adapter_schema_change"]),
        "sha256:9999999999999999999999999999999999999999999999999999999999999999",
    );
    let obligation = bind_obligation_to_authored_criterion(
        obligation, "pln-evidence-public", "criterion-evidence-public",
    );
    let plan_path = dir.path()
        .join(".planr/plans/build/evidence-public-fixture.plan.md")
        .to_string_lossy()
        .to_string();
    let project_id: String = Connection::open(&db)
        .unwrap()
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let conn = Connection::open(&db).unwrap();
    for (id, work_type) in [
        ("item-semantic-maker", "code"),
        ("item-semantic-verifier", "verification"),
    ] {
        conn.execute(
            "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at) VALUES (?1, ?2, ?1, 'semantic mismatch fixture', 'ready', ?3, ?4, datetime('now'), datetime('now'))",
            rusqlite::params![id, project_id, work_type, plan_path],
        )
        .unwrap();
    }
    drop(conn);
    let mut obligation = obligation;
    obligation["item_id"] = json!("item-semantic-verifier");
    obligation["observations"][0]["payload_schema"] =
        json!({"schema_ref": "schema://com.example.health.status"});
    add_evidence_obligation_value(dir.path(), &db, "pob-semantic-mismatch", &obligation);
    let maker = single_json_document(&planr().current_dir(dir.path()).env("PLANR_WORKER_ID", "semantic-maker").args(["--db", db.to_str().unwrap(), "--json", "pick", "--plan", "pln-evidence-public", "--work-type", "code"]).assert().success().get_output().stdout);
    assert_eq!(maker["item"]["id"], "item-semantic-maker");
    single_json_document(&planr().current_dir(dir.path()).env("PLANR_WORKER_ID", "semantic-maker").args(["--db", db.to_str().unwrap(), "--json", "done", "item-semantic-maker", "--summary", "semantic fixture", "--cmd", "true", "--next"]).assert().success().get_output().stdout);
    let verifier = single_json_document(&planr().current_dir(dir.path()).env("PLANR_WORKER_ID", "semantic-verifier").args(["--db", db.to_str().unwrap(), "--json", "pick", "--plan", "pln-evidence-public", "--work-type", "verification"]).assert().success().get_output().stdout);
    let run_path = verifier["work_packet"]["sealed_run_index"]["repository_path"].as_str().unwrap();
    let run = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "semantic-verifier")
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "run",
                "--input",
                run_path,
            ])
            .assert()
            .code(2)
            .get_output()
            .stdout,
    );
    assert_evidence_envelope(&run, "evidence.run", true);
    assert_eq!(run["object"]["verdict"], "failed");
    let result = &run["object"]["results"][0];
    assert_eq!(result["attempt"]["status"], "failed");
    assert_eq!(
        result["attempt"]["raw_result"]["ordinary_observation_error"],
        json!(
            "ordinary process actual does not satisfy expected predicate for obs-pob-semantic-mismatch"
        )
    );
    assert_eq!(
        result["attempt"]["raw_result"]["planr_adapter_gap_reasons"],
        json!(["target_mismatch"])
    );
    let semantic_actual: Value = serde_json::from_str(
        result["receipt"]["observations"][0]["actual"]["stdout_excerpt"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(semantic_actual, json!({"status": "ok"}));
    assert_eq!(
        result["receipt"]["observations"][0]["predicate"],
        json!({"status": "missing"})
    );
    assert_eq!(
        result["receipt"]["observations"][0]["outcome"],
        "failed"
    );
    assert_eq!(
        result["receipt"]["proof_gaps"],
        json!(["target_mismatch"])
    );

    let coverage = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "coverage",
                "--scope",
                "obligation",
                "--id",
                "pob-semantic-mismatch",
            ])
            .assert()
            .code(2)
            .get_output()
            .stdout,
    );
    assert_evidence_envelope(&coverage, "evidence.coverage", true);
    assert_eq!(coverage["object"]["verdict"], "unsatisfied");
    assert_eq!(
        coverage["object"]["coverage"]["observation_coverage"][0]["gap_reason"],
        "target_mismatch"
    );
    assert!(
        coverage["object"]["coverage"]["observation_coverage"][0]["covering_receipt_ids"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{coverage}"
    );
}

#[test]
fn evidence_process_adapter_schema_invalid_stdout_is_verifier_failed() {
    let dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db = db_dir.path().join("planr.sqlite");
    write_evidence_policy_fixture(dir.path());
    let manifest_digest = rewrite_evidence_runner_manifest(dir.path(), |manifest| {
        manifest["availability_probe"]["execution"]["args"] =
            json!(["-c", "printf '{\"status\":\"ok\",\"extra\":true}'"]);
        manifest["adapter_digest"] = json!(process_adapter_digest(
            &manifest["availability_probe"]["execution"],
            vec![]
        ));
    });
    rewrite_evidence_policy_fixture(dir.path(), |policy| {
        policy["adapter_registrations"][0]["manifest_digest"] = json!(manifest_digest);
        policy["adapter_registrations"][0]["execution_contract"]["args"] =
            json!(["-c", "printf '{\"status\":\"ok\",\"extra\":true}'"]);
    });
    init_evidence_project(dir.path(), &db, "Evidence Schema Invalid Stdout");
    init_git_repo(dir.path());

    let policy = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "--json", "evidence", "policy"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let policy_digest = policy["object"]["digest"].as_str().unwrap();
    let capabilities = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "capability",
                "list",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let instance = capabilities["object"]["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instance| {
            instance["manifest_id"] == "verifier-generic-adapter"
                && instance["availability_status"] == "available"
        })
        .expect("generic adapter instance");
    let instance_id = instance["id"].as_str().unwrap();
    let environment = instance["capability"]["environment"].clone();
    let obligation = evidence_obligation_for(
        "pob-schema-invalid-stdout",
        policy_digest,
        "com.example.health.status",
        "schema invalid health",
        json!({"status": "ok"}),
        json!({"kind": "process", "uri": "local://health"}),
        environment,
        json!({"kind": "process", "id": "runtime-local"}),
        json!(["target_change", "policy_change", "adapter_schema_change"]),
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    let obligation = bind_obligation_to_authored_criterion(
        obligation, "pln-evidence-public", "criterion-evidence-public",
    );
    let plan_path = dir
        .path()
        .join(".planr/plans/build/evidence-public-fixture.plan.md")
        .to_string_lossy()
        .to_string();
    let project_id: String = Connection::open(&db)
        .unwrap()
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let conn = Connection::open(&db).unwrap();
    for (id, work_type) in [
        ("item-schema-invalid-maker", "code"),
        ("item-schema-invalid-verifier", "verification"),
    ] {
        conn.execute(
            "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at) VALUES (?1, ?2, ?1, 'schema invalid fixture', 'ready', ?3, ?4, datetime('now'), datetime('now'))",
            rusqlite::params![id, project_id, work_type, plan_path],
        )
        .unwrap();
    }
    drop(conn);
    let mut obligation = obligation;
    obligation["item_id"] = json!("item-schema-invalid-verifier");
    obligation["observations"][0]["payload_schema"] =
        json!({"schema_ref": "schema://com.example.health.status"});
    add_evidence_obligation_value(dir.path(), &db, "pob-schema-invalid-stdout", &obligation);
    let maker = single_json_document(&planr().current_dir(dir.path()).env("PLANR_WORKER_ID", "schema-invalid-maker").args(["--db", db.to_str().unwrap(), "--json", "pick", "--plan", "pln-evidence-public", "--work-type", "code"]).assert().success().get_output().stdout);
    assert_eq!(maker["item"]["id"], "item-schema-invalid-maker");
    single_json_document(&planr().current_dir(dir.path()).env("PLANR_WORKER_ID", "schema-invalid-maker").args(["--db", db.to_str().unwrap(), "--json", "done", "item-schema-invalid-maker", "--summary", "schema invalid fixture", "--cmd", "true", "--next"]).assert().success().get_output().stdout);
    let verifier = single_json_document(&planr().current_dir(dir.path()).env("PLANR_WORKER_ID", "schema-invalid-verifier").args(["--db", db.to_str().unwrap(), "--json", "pick", "--plan", "pln-evidence-public", "--work-type", "verification"]).assert().success().get_output().stdout);
    let run_path = verifier["work_packet"]["sealed_run_index"]["repository_path"]
        .as_str()
        .unwrap();
    let run = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "schema-invalid-verifier")
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "run",
                "--input",
                run_path,
            ])
            .assert()
            .code(2)
            .get_output()
            .stdout,
    );
    assert_evidence_envelope(&run, "evidence.run", true);
    assert_eq!(run["object"]["verdict"], "failed");
    let result = &run["object"]["results"][0];
    assert_eq!(result["attempt"]["status"], "failed");
    assert_eq!(
        result["attempt"]["raw_result"]["planr_adapter_gap_reasons"],
        json!(["verifier_failed"])
    );
    assert!(
        result["attempt"]["raw_result"]["ordinary_observation_error"]
            .as_str()
            .unwrap()
            .contains("does not match payload schema"),
        "{run}"
    );
    let schema_invalid_actual: Value = serde_json::from_str(
        result["receipt"]["observations"][0]["actual"]["stdout_excerpt"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        schema_invalid_actual,
        json!({"status": "ok", "extra": true})
    );
    assert_eq!(
        result["receipt"]["observations"][0]["outcome"],
        "failed"
    );
    assert_eq!(
        result["receipt"]["proof_gaps"],
        json!(["verifier_failed"])
    );

    let coverage = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "coverage",
                "--scope",
                "obligation",
                "--id",
                "pob-schema-invalid-stdout",
            ])
            .assert()
            .code(2)
            .get_output()
            .stdout,
    );
    assert_evidence_envelope(&coverage, "evidence.coverage", true);
    assert_eq!(coverage["object"]["verdict"], "unsatisfied");
    assert_eq!(
        coverage["object"]["coverage"]["observation_coverage"][0]["gap_reason"],
        "verifier_failed"
    );
    assert!(
        coverage["object"]["coverage"]["observation_coverage"][0]["covering_receipt_ids"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{coverage}"
    );
}

#[test]
fn project_relocation_previews_validates_and_applies_all_paths_atomically() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let source_root = source.path().canonicalize().unwrap();
    let destination_root = destination.path().canonicalize().unwrap();
    let db = source_root.join(".planr/planr.sqlite");
    planr()
        .current_dir(&source_root)
        .args(["--db", db.to_str().unwrap(), "project", "init", "Relocate"])
        .assert()
        .success();

    let relative_plan = Path::new(".planr/plans/build/relocate.plan.md");
    let source_plan = source_root.join(relative_plan);
    let destination_plan = destination_root.join(relative_plan);
    fs::create_dir_all(source_plan.parent().unwrap()).unwrap();
    fs::create_dir_all(destination_plan.parent().unwrap()).unwrap();
    fs::write(&source_plan, "# Relocate\n").unwrap();
    fs::write(&destination_plan, "# Relocate\n").unwrap();

    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('plan-relocate', ?1, 'build', ?2, 'Relocate', 'relocate', 'ok', 'sha256:relocate', datetime('now'), datetime('now'))",
        rusqlite::params![project_id, source_plan.to_string_lossy()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
         VALUES ('item-relocate', ?1, 'Relocate', 'relocate', 'ready', 'code', ?2, datetime('now'), datetime('now'))",
        rusqlite::params![project_id, source_plan.to_string_lossy()],
    )
    .unwrap();
    drop(conn);

    let output = planr()
        .current_dir(&source_root)
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "project",
            "relocate",
            &project_id,
            "--destination",
            destination_root.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(preview["mode"], "preview");
    assert_eq!(preview["relocation"]["project"]["id"], project_id);
    assert_eq!(
        preview["relocation"]["project"]["from"],
        source_root.to_string_lossy().as_ref()
    );
    assert_eq!(
        preview["relocation"]["project"]["to"],
        destination_root.to_string_lossy().as_ref()
    );
    assert_eq!(preview["relocation"]["plans"][0]["id"], "plan-relocate");
    assert_eq!(
        preview["relocation"]["plans"][0]["to"],
        destination_plan.to_string_lossy().as_ref()
    );
    assert_eq!(preview["relocation"]["items"][0]["id"], "item-relocate");
    assert_eq!(
        preview["relocation"]["items"][0]["to"],
        destination_plan.to_string_lossy().as_ref()
    );

    let conn = Connection::open(&db).unwrap();
    let stored_paths = || -> (String, String, String) {
        (
            conn.query_row(
                "SELECT root_path FROM projects WHERE id = ?1",
                [&project_id],
                |row| row.get(0),
            )
            .unwrap(),
            conn.query_row(
                "SELECT path FROM plans WHERE id = 'plan-relocate'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
            conn.query_row(
                "SELECT plan_path FROM items WHERE id = 'item-relocate'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        )
    };
    assert_eq!(
        stored_paths(),
        (
            source_root.to_string_lossy().to_string(),
            source_plan.to_string_lossy().to_string(),
            source_plan.to_string_lossy().to_string(),
        )
    );
    drop(conn);

    let missing_destination = source_root.join("missing-destination");
    planr()
        .current_dir(&source_root)
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "relocate",
            &project_id,
            "--destination",
            missing_destination.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "project_relocation_destination_invalid",
        ));

    let assert_relocation_rejected_without_mutation =
        |expected_error: &str, stored_plan_path: &str| {
            for apply in [false, true] {
                let mut args = vec![
                    "--db".to_string(),
                    db.to_string_lossy().to_string(),
                    "project".to_string(),
                    "relocate".to_string(),
                    project_id.clone(),
                    "--destination".to_string(),
                    destination_root.to_string_lossy().to_string(),
                ];
                if apply {
                    args.push("--apply".to_string());
                }
                planr()
                    .current_dir(&source_root)
                    .args(args)
                    .assert()
                    .failure()
                    .stderr(predicate::str::contains(expected_error.to_string()));
                let conn = Connection::open(&db).unwrap();
                assert_eq!(
                    conn.query_row(
                        "SELECT root_path FROM projects WHERE id = ?1",
                        [&project_id],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap(),
                    source_root.to_string_lossy()
                );
                assert_eq!(
                    conn.query_row(
                        "SELECT path FROM plans WHERE id = 'plan-relocate'",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap(),
                    stored_plan_path
                );
                assert_eq!(
                    conn.query_row(
                        "SELECT plan_path FROM items WHERE id = 'item-relocate'",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap(),
                    stored_plan_path
                );
            }
        };

    let parent_escape = source_root.join("../parent-escape.plan.md");
    let parent_escape = parent_escape.to_string_lossy().to_string();
    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE plans SET path = ?1 WHERE id = 'plan-relocate'",
        [&parent_escape],
    )
    .unwrap();
    conn.execute(
        "UPDATE items SET plan_path = ?1 WHERE id = 'item-relocate'",
        [&parent_escape],
    )
    .unwrap();
    drop(conn);
    assert_relocation_rejected_without_mutation(
        "project_relocation_relative_path_component_rejected",
        &parent_escape,
    );

    #[cfg(unix)]
    {
        let source_link_plan = source_root.join(".planr/plans/build/symlink-escape.plan.md");
        fs::write(&source_link_plan, "# Symlink escape\n").unwrap();
        let destination_link_plan =
            destination_root.join(".planr/plans/build/symlink-escape.plan.md");
        let outside = tempdir().unwrap();
        let outside_plan = outside.path().join("outside.plan.md");
        fs::write(&outside_plan, "# Outside\n").unwrap();
        symlink(&outside_plan, &destination_link_plan).unwrap();
        let source_link_plan = source_link_plan.to_string_lossy().to_string();
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE plans SET path = ?1 WHERE id = 'plan-relocate'",
            [&source_link_plan],
        )
        .unwrap();
        conn.execute(
            "UPDATE items SET plan_path = ?1 WHERE id = 'item-relocate'",
            [&source_link_plan],
        )
        .unwrap();
        drop(conn);
        assert_relocation_rejected_without_mutation(
            "project_relocation_destination_escape",
            &source_link_plan,
        );
    }

    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE plans SET path = ?1 WHERE id = 'plan-relocate'",
        [source_plan.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute(
        "UPDATE items SET plan_path = ?1 WHERE id = 'item-relocate'",
        [source_plan.to_string_lossy().as_ref()],
    )
    .unwrap();
    drop(conn);

    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
         VALUES ('item-dangling-relocate', ?1, 'Dangling', 'dangling', 'ready', 'code', '/not/a/canonical/plan', datetime('now'), datetime('now'))",
        [&project_id],
    )
    .unwrap();
    drop(conn);
    planr()
        .current_dir(&source_root)
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "relocate",
            &project_id,
            "--destination",
            destination_root.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "project_relocation_dangling_item_plan_path",
        ));
    let conn = Connection::open(&db).unwrap();
    conn.execute("DELETE FROM items WHERE id = 'item-dangling-relocate'", [])
        .unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_project_relocation_item_update
         BEFORE UPDATE OF plan_path ON items
         WHEN OLD.id = 'item-relocate'
         BEGIN SELECT RAISE(ABORT, 'injected relocation rollback'); END;",
    )
    .unwrap();
    drop(conn);

    planr()
        .current_dir(&source_root)
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "relocate",
            &project_id,
            "--destination",
            destination_root.to_str().unwrap(),
            "--apply",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("injected relocation rollback"));
    let conn = Connection::open(&db).unwrap();
    let root_after_failure: String = conn
        .query_row(
            "SELECT root_path FROM projects WHERE id = ?1",
            [&project_id],
            |row| row.get(0),
        )
        .unwrap();
    let plan_after_failure: String = conn
        .query_row(
            "SELECT path FROM plans WHERE id = 'plan-relocate'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(root_after_failure, source_root.to_string_lossy());
    assert_eq!(plan_after_failure, source_plan.to_string_lossy());
    conn.execute_batch("DROP TRIGGER fail_project_relocation_item_update")
        .unwrap();
    drop(conn);

    let output = planr()
        .current_dir(&source_root)
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "project",
            "relocate",
            &project_id,
            "--destination",
            destination_root.to_str().unwrap(),
            "--apply",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let applied: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(applied["mode"], "apply");
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT root_path FROM projects WHERE id = ?1",
            [&project_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        destination_root.to_string_lossy()
    );
    assert_eq!(
        conn.query_row(
            "SELECT path FROM plans WHERE id = 'plan-relocate'",
            [],
            |row| { row.get::<_, String>(0) }
        )
        .unwrap(),
        destination_plan.to_string_lossy()
    );
    assert_eq!(
        conn.query_row(
            "SELECT plan_path FROM items WHERE id = 'item-relocate'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        destination_plan.to_string_lossy()
    );
}

#[test]
fn project_plan_map_pick_log_close_flow() {
    let dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db = db_dir.path().join("planr.sqlite");

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

    for client in ["codex", "claude", "cursor", "grok"] {
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
    assert!(dir.path().join(".grok/config.toml").exists());
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
    let prompt_http = prompt_http["prompt"].as_str().unwrap();
    assert!(prompt_http.contains("/final-product-review"));
    assert!(prompt_http.contains("/review-gates/<gate-id>/findings/resolve"));

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
    for subcommand in [
        "annotate", "ingest", "evidence", "close", "findings", "list", "show",
    ] {
        assert!(review_help.contains(subcommand));
    }
    assert!(!review_help.contains("artifact"));
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

    let legacy_fixture = dir.path().join("legacy-fixture.json");
    let legacy_fixture_bytes = br#"{"legacy":true}"#;
    fs::write(&legacy_fixture, legacy_fixture_bytes).unwrap();
    let legacy_fixture_digest = sha256_prefixed(legacy_fixture_bytes);
    let legacy_runner_manifest = with_legacy_eval_digest(json!({
        "schema_version": "eval.suite.v1",
        "suite_id": "legacy-numeric-suite",
        "suite_version": "v1",
        "fixtures": [{"id": "legacy-fixture", "path": "legacy-fixture.json", "digest": legacy_fixture_digest}],
        "scorers": [{"id": "score", "version": "v1", "kind": "test"}],
        "cases": [{
            "case_id": "legacy-case",
            "fixture_id": "legacy-fixture",
            "fixture_ids": ["legacy-fixture"],
            "scorer_id": "score",
            "scorer_ids": ["score@v1"],
            "subject": {"kind": "binary", "argv": ["/bin/echo", "legacy-ok"]},
            "assertions": [{"kind": "exit_code", "expected": 0}],
            "measures": ["duration_ms"],
            "sampling": {"repetitions": 1, "warmups": 0, "seed": 1, "min_successful_samples": 1},
            "timeout_ms": 1000,
            "output_limit_bytes": 4096
        }],
        "comparison_policy": {"required_case_coverage": 1.0},
        "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
    }));
    let legacy_suite_digest = legacy_runner_manifest["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(
        legacy_suite_digest,
        with_canonical_digest(legacy_runner_manifest.clone())["digest"]
            .as_str()
            .unwrap()
    );
    let legacy_suite_path = dir.path().join("legacy-suite.json");
    fs::write(
        &legacy_suite_path,
        serde_json::to_vec_pretty(&legacy_runner_manifest).unwrap(),
    )
    .unwrap();
    let legacy_suite_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "suite-check",
            "--input",
            legacy_suite_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let legacy_suite_value = single_json_document(&legacy_suite_output);
    assert_eval_envelope(&legacy_suite_value, "eval.suite.check", true);
    assert_eq!(
        legacy_suite_value["object"]["suite"]["digest"]
            .as_str()
            .unwrap(),
        legacy_suite_digest
    );

    let legacy_runner_input = json!({
        "id": "legacy-numeric-runner",
        "suite_digest": legacy_suite_digest.clone(),
        "subject": {"kind": "binary", "revision": "legacy-numeric", "argv": ["/bin/echo", "legacy-ok"]},
        "repo_root": ".",
        "runner_manifest": legacy_runner_manifest
    });
    let legacy_runner_path = dir.path().join("legacy-runner.json");
    fs::write(
        &legacy_runner_path,
        serde_json::to_vec_pretty(&legacy_runner_input).unwrap(),
    )
    .unwrap();
    let legacy_runner_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "run",
            "--input",
            legacy_runner_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let legacy_runner_value = single_json_document(&legacy_runner_output);
    assert_eval_envelope(&legacy_runner_value, "eval.run", true);
    assert_eq!(
        legacy_runner_value["object"]["run"]["suite_digest"]
            .as_str()
            .unwrap(),
        legacy_suite_digest
    );

    let legacy_supplied_attempt = json!({
        "id": "legacy-numeric-supplied-attempt",
        "attempt_index": 0,
        "terminal_status": "verified_success",
        "countable": true,
        "effective_client": "codex",
        "effective_provider": "openai",
        "effective_runtime": "codex-cli",
        "effective_model": "gpt-5.6-terra",
        "effective_effort": "low",
        "effective_profile_id": "eval-terra-low",
        "profile_config_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
        "runner_harness_version": "supplied-evidence-v1",
        "route_observation": verified_eval_route_observation(
            "codex",
            "gpt-5.6-terra",
            "low",
            "eval-terra-low"
        ),
        "outcome": {"status": "verified_success"}
    });
    let legacy_supplied_sample = json!({
        "id": "legacy-numeric-supplied-duration",
        "repetition_index": 0,
        "warmup": false,
        "seed": 1,
        "measure": "duration_ms",
        "value": 1,
        "unit": "ms",
        "source": "process",
        "metering_basis": "actual_trusted",
        "basis_source": "process",
        "basis_confidence": "verified",
        "attempt": legacy_supplied_attempt
    });
    let legacy_supplied_case = json!({
        "case": {
            "case_id": "legacy-case",
            "scorer_id": "score",
            "scorer_version": "v1",
            "fixture_digest": legacy_fixture_digest,
            "status": "pass",
            "repetition_count": 1,
            "warmup_count": 0,
            "assertions": [{"kind": "quality_pass", "status": "pass"}],
            "command": {"runner": "supplied-evidence"},
            "reasons": []
        },
        "samples": [legacy_supplied_sample]
    });
    let legacy_supplied_input = json!({
        "id": "legacy-numeric-supplied",
        "suite_digest": legacy_suite_digest.clone(),
        "subject": {"kind": "local_authenticated_agent", "revision": "legacy-numeric-supplied", "argv": ["maintainer-eval"]},
        "runner_version": "eval-runner-v1",
        "planr_version": env!("CARGO_PKG_VERSION"),
        "testbed_fingerprint": {},
        "source_state": {"revision": "legacy-numeric-supplied"},
        "status": "success",
        "cases": [legacy_supplied_case]
    });
    let legacy_supplied_path = dir.path().join("legacy-supplied.json");
    fs::write(
        &legacy_supplied_path,
        serde_json::to_vec_pretty(&legacy_supplied_input).unwrap(),
    )
    .unwrap();
    let legacy_supplied_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "run",
            "--input",
            legacy_supplied_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let legacy_supplied_value = single_json_document(&legacy_supplied_output);
    assert_eval_envelope(&legacy_supplied_value, "eval.run", true);
    assert_eq!(
        legacy_supplied_value["object"]["run"]["suite_digest"],
        legacy_suite_digest
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

    let canonical_ordering_fixture_a = dir.path().join("canonical-ordering-a.json");
    let canonical_ordering_fixture_b = dir.path().join("canonical-ordering-b.json");
    let canonical_ordering_fixture_a_bytes = br#"{"fixture":"a"}"#;
    let canonical_ordering_fixture_b_bytes = br#"{"fixture":"b"}"#;
    fs::write(
        &canonical_ordering_fixture_a,
        canonical_ordering_fixture_a_bytes,
    )
    .unwrap();
    fs::write(
        &canonical_ordering_fixture_b,
        canonical_ordering_fixture_b_bytes,
    )
    .unwrap();
    let canonical_ordering_manifest = with_authored_order_and_canonical_digest(json!({
        "schema_version": "eval.suite.v1",
        "suite_id": "canonical-ordering-suite",
        "suite_version": "v1",
        "fixtures": [
            {"id": "a-fixture", "path": "canonical-ordering-a.json", "digest": sha256_prefixed(canonical_ordering_fixture_a_bytes)},
            {"id": "b-fixture", "path": "canonical-ordering-b.json", "digest": sha256_prefixed(canonical_ordering_fixture_b_bytes)}
        ],
        "scorers": [
            {"id": "a-score", "version": "v1", "kind": "test"},
            {"id": "b-score", "version": "v1", "kind": "test"}
        ],
        "cases": [
            {
                "case_id": "a-case",
                "fixture_id": "a-fixture",
                "fixture_ids": ["a-fixture", "b-fixture"],
                "scorer_id": "a-score",
                "scorer_version": "v1",
                "scorer_ids": ["a-score@v1", "b-score@v1"],
                "subject": {"kind": "binary", "argv": ["/bin/echo", "ordered-a"]},
                "assertions": [{"kind": "exit_code", "expected": 0}],
                "measures": ["cost_micros", "duration_ms"],
                "sampling": {"repetitions": 1, "warmups": 0, "seed": 1, "min_successful_samples": 1},
                "timeout_ms": 1000,
                "output_limit_bytes": 4096
            },
            {
                "case_id": "b-case",
                "fixture_id": "b-fixture",
                "fixture_ids": ["b-fixture", "a-fixture"],
                "scorer_id": "b-score",
                "scorer_version": "v1",
                "scorer_ids": ["b-score@v1", "a-score@v1"],
                "subject": {"kind": "binary", "argv": ["/bin/echo", "ordered-b"]},
                "assertions": [{"kind": "exit_code", "expected": 0}],
                "measures": ["duration_ms", "cost_micros"],
                "sampling": {"repetitions": 1, "warmups": 0, "seed": 1, "min_successful_samples": 1},
                "timeout_ms": 1000,
                "output_limit_bytes": 4096
            }
        ],
        "comparison_policy": {"required_case_coverage": 1.0},
        "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
    }));
    let canonical_ordering_reordered_manifest = with_authored_order_and_canonical_digest(json!({
        "schema_version": "eval.suite.v1",
        "suite_id": "canonical-ordering-suite",
        "suite_version": "v1",
        "fixtures": [
            {"id": "b-fixture", "path": "canonical-ordering-b.json", "digest": sha256_prefixed(canonical_ordering_fixture_b_bytes)},
            {"id": "a-fixture", "path": "canonical-ordering-a.json", "digest": sha256_prefixed(canonical_ordering_fixture_a_bytes)}
        ],
        "scorers": [
            {"id": "b-score", "version": "v1", "kind": "test"},
            {"id": "a-score", "version": "v1", "kind": "test"}
        ],
        "cases": [
            {
                "case_id": "b-case",
                "fixture_id": "b-fixture",
                "fixture_ids": ["a-fixture", "b-fixture"],
                "scorer_id": "b-score",
                "scorer_version": "v1",
                "scorer_ids": ["a-score@v1", "b-score@v1"],
                "subject": {"kind": "binary", "argv": ["/bin/echo", "ordered-b"]},
                "assertions": [{"kind": "exit_code", "expected": 0}],
                "measures": ["cost_micros", "duration_ms"],
                "sampling": {"repetitions": 1, "warmups": 0, "seed": 1, "min_successful_samples": 1},
                "timeout_ms": 1000,
                "output_limit_bytes": 4096
            },
            {
                "case_id": "a-case",
                "fixture_id": "a-fixture",
                "fixture_ids": ["b-fixture", "a-fixture"],
                "scorer_id": "a-score",
                "scorer_version": "v1",
                "scorer_ids": ["b-score@v1", "a-score@v1"],
                "subject": {"kind": "binary", "argv": ["/bin/echo", "ordered-a"]},
                "assertions": [{"kind": "exit_code", "expected": 0}],
                "measures": ["duration_ms", "cost_micros"],
                "sampling": {"repetitions": 1, "warmups": 0, "seed": 1, "min_successful_samples": 1},
                "timeout_ms": 1000,
                "output_limit_bytes": 4096
            }
        ],
        "comparison_policy": {"required_case_coverage": 1.0},
        "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
    }));
    let canonical_ordering_digest = canonical_ordering_manifest["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        canonical_ordering_reordered_manifest["digest"]
            .as_str()
            .unwrap(),
        canonical_ordering_digest
    );
    let canonical_ordering_suite_path = dir.path().join("canonical-ordering-suite.json");
    fs::write(
        &canonical_ordering_suite_path,
        serde_json::to_vec_pretty(&canonical_ordering_manifest).unwrap(),
    )
    .unwrap();
    let canonical_ordering_reordered_suite_path =
        dir.path().join("canonical-ordering-suite-reordered.json");
    fs::write(
        &canonical_ordering_reordered_suite_path,
        serde_json::to_vec_pretty(&canonical_ordering_reordered_manifest).unwrap(),
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
            canonical_ordering_suite_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "suite-check",
            "--input",
            canonical_ordering_reordered_suite_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    let canonical_ordering_runner_input = json!({
        "id": "canonical-ordering-runner",
        "suite_digest": canonical_ordering_digest.clone(),
        "subject": {"kind": "binary", "revision": "canonical-ordering", "argv": ["/bin/echo", "ordered"]},
        "repo_root": ".",
        "runner_manifest": canonical_ordering_reordered_manifest
    });
    let canonical_ordering_runner_path = dir.path().join("canonical-ordering-runner.json");
    fs::write(
        &canonical_ordering_runner_path,
        serde_json::to_vec_pretty(&canonical_ordering_runner_input).unwrap(),
    )
    .unwrap();
    let canonical_ordering_runner_output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "eval",
            "run",
            "--input",
            canonical_ordering_runner_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let canonical_ordering_runner_value = single_json_document(&canonical_ordering_runner_output);
    assert_eval_envelope(&canonical_ordering_runner_value, "eval.run", true);
    assert_eq!(
        canonical_ordering_runner_value["object"]["run"]["suite_digest"],
        canonical_ordering_digest
    );

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
    assert_eq!(refs.len(), 1);
    assert!(
        refs.iter()
            .any(|reference| reference["target_id"] == regressed_id)
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
    let baseline_run_index = package_json["eval_runs"]
        .as_array()
        .unwrap()
        .iter()
        .position(|run| run["id"] == "baseline-run")
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
    let mut conflicting_run = duplicate_run_package["eval_runs"][baseline_run_index].clone();
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
    missing_suite_package["eval_runs"] = json!(
        package_json["eval_runs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|run| run["suite_digest"] == suite_digest)
            .cloned()
            .collect::<Vec<_>>()
    );
    missing_suite_package["eval_comparisons"] = json!([]);
    missing_suite_package["eval_invalidations"] = json!([]);
    missing_suite_package["eval_evidence_refs"] = json!([]);
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
    mismatched_attempt_package["eval_runs"][baseline_run_index]["cases"][0]["samples"][0]["repetition_index"] =
        json!(99);
    assert_rejected_empty_import(
        "eval-package-sample-attempt-mismatch.json",
        mismatched_attempt_package,
        "identity does not match referenced attempt",
    );
    let mut invalid_confidence_package = package_json.clone();
    invalid_confidence_package["eval_runs"][baseline_run_index]["cases"][0]["samples"][0]["basis_confidence"] =
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
    unavailable_value_package["eval_runs"][baseline_run_index]["cases"][0]["samples"][0]["metering_basis"] =
        json!("unavailable");
    unavailable_value_package["eval_runs"][baseline_run_index]["cases"][0]["samples"][0]["basis_confidence"] =
        json!("unavailable");
    unavailable_value_package["eval_runs"][baseline_run_index]["cases"][0]["samples"][0]["value"] =
        json!(42);
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
    missing_estimate_package["eval_runs"][baseline_run_index]["cases"][0]["samples"][0]["metering_basis"] =
        json!("estimated");
    missing_estimate_package["eval_runs"][baseline_run_index]["cases"][0]["samples"][0]["basis_confidence"] =
        json!("estimated");
    missing_estimate_package["eval_runs"][baseline_run_index]["cases"][0]["samples"][0]
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
    malformed_estimate_package["eval_runs"][baseline_run_index]["cases"][0]["samples"][0]["metering_basis"] =
        json!("estimated");
    malformed_estimate_package["eval_runs"][baseline_run_index]["cases"][0]["samples"][0]["basis_confidence"] =
        json!("estimated");
    malformed_estimate_package["eval_runs"][baseline_run_index]["cases"][0]["samples"][0]["estimate_provenance"] = json!({
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
    invalid_run_status_package["eval_runs"][baseline_run_index]["status"] = json!("not-a-status");
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
        1
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
    assert_eq!(import_apply["imported"]["eval_evidence_refs"], 1);
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

#[test]
fn pi_runtime_marker_is_exact_and_advisory() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Pi Host"])
        .assert()
        .success();
    fs::write(
        dir.path().join(".planr/agents.toml"),
        "[profiles.pi]\nclient = \"pi\"\nmodel = \"provider/model\"\n\n[route_default]\nprofile = \"pi\"\n",
    )
    .unwrap();
    let item = create_test_item(dir.path(), &db, "Pi work", "runtime marker");

    for (value, summary) in [
        ("true", "exact marker"),
        ("TRUE", "wrong case"),
        ("1", "wrong value"),
    ] {
        planr()
            .current_dir(dir.path())
            .env("PI_CODING_AGENT", value)
            .args([
                "--db",
                db.to_str().unwrap(),
                "log",
                "add",
                "--item",
                &item,
                "--summary",
                summary,
                "--cmd",
                "cargo test",
            ])
            .assert()
            .success();
    }
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
    assert_eq!(runs[0]["observed_client"], "pi");
    assert_eq!(runs[0]["client_mismatch"], false);
    assert!(
        !runs[1].as_object().unwrap().contains_key("observed_client"),
        "marker matching is case-sensitive: {trace}"
    );
    assert!(
        !runs[2].as_object().unwrap().contains_key("observed_client"),
        "truthy variants are not accepted: {trace}"
    );
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
fn canonical_verification_task_builds_a_sealed_verifier_packet_without_retagging() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    write_evidence_policy_fixture(dir.path());
    write_materiality_policy(dir.path());
    let policy_path = dir.path().join(".planr/policy.toml");
    let policy = fs::read_to_string(&policy_path)
        .unwrap()
        .lines()
        .filter(|line| {
            !line.starts_with("max_wall_time_seconds =")
                && !line.starts_with("max_tool_calls =")
                && !line.starts_with("max_tokens =")
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(policy_path, policy).unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Canonical Verification"])
        .assert()
        .success();

    let product = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                &db_arg,
                "--json",
                "plan",
                "new",
                "Canonical Verification",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let build = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                &db_arg,
                "--json",
                "plan",
                "split",
                product["plan"]["id"].as_str().unwrap(),
                "--slice",
                "Canonical verifier packet",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let plan_id = build["plan"]["id"].as_str().unwrap().to_string();
    let plan_path = build["plan"]["path"].as_str().unwrap().to_string();
    let content = fs::read_to_string(&plan_path).unwrap();
    let start = content.find("## Phase 1").unwrap();
    let end = content.find("## Out Of Scope").unwrap();
    let tasks = "### TASK-001 (code): Prepare deterministic source\n\nPrepare the source freeze.\n\n### TASK-007 (verification): Verify frozen source\n\nCollect binding Evidence from the sealed run index.\n\n";
    fs::write(
        &plan_path,
        format!("{}{}{}", &content[..start], tasks, &content[end..]),
    )
    .unwrap();
    let map = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db", &db_arg, "--json", "map", "build", "--from", &plan_id,
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let item = |title: &str| {
        map["created"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["title"] == title)
            .unwrap_or_else(|| panic!("item `{title}` missing: {map}"))
            .clone()
    };
    let implementation = item("Prepare deterministic source");
    let verification = item("Verify frozen source");
    assert_eq!(implementation["work_type"], "code");
    assert_eq!(verification["work_type"], "verification");
    let implementation_id = implementation["id"].as_str().unwrap().to_string();
    let verification_id = verification["id"].as_str().unwrap().to_string();

    let policy = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args(["--db", &db_arg, "--json", "evidence", "policy"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    init_git_repo(dir.path());
    let source_freeze_planr = private_planr_binary(dir.path());
    let mut obligation = evidence_obligation(
        "pob-canonical-verifier-packet",
        policy["object"]["digest"].as_str().unwrap(),
        json!({"kind": "local", "id": "canonical", "digest": "sha256:5555555555555555555555555555555555555555555555555555555555555555"}),
    );
    obligation = bind_obligation_to_authored_criterion(
        obligation,
        &plan_id,
        "criterion-canonical-verifier-packet",
    );
    obligation["item_id"] = json!(verification_id);
    obligation["fixture_policy"]["fixtures_allowed"] = json!(true);
    obligation["observations"][0]["payload_schema"] =
        json!({"schema_ref": "schema://com.example.health.status"});
    let mut incomplete_obligation = obligation.clone();
    incomplete_obligation["id"] = json!("pob-canonical-verifier-packet-missing-schema");
    incomplete_obligation["observations"][0]
        .as_object_mut()
        .unwrap()
        .remove("payload_schema");
    add_evidence_obligation_value(
        dir.path(),
        &db,
        "pob-canonical-verifier-packet-missing-schema",
        &incomplete_obligation,
    );
    let picked = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "canonical-maker")
            .args([
                "--db",
                &db_arg,
                "--json",
                "pick",
                "--plan",
                &plan_id,
                "--work-type",
                "code",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(picked["item"]["id"], implementation_id);

    let done = single_json_document(
        &planr_from_binary(&source_freeze_planr)
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "canonical-maker")
            .args([
                "--db",
                &db_arg,
                "--json",
                "done",
                &implementation_id,
                "--summary",
                "canonical source prepared",
                "--cmd",
                "true",
                "--tests",
                "true",
                "--next",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(done["next"]["reason"], "verification_handoff_source_frozen");
    assert_eq!(
        done["next"]["work_packet"]["verification_item_id"],
        verification_id
    );

    let blocked_pick = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "canonical-verifier-incomplete-binding")
            .args([
                "--db",
                &db_arg,
                "--json",
                "pick",
                "--plan",
                &plan_id,
                "--work-type",
                "verification",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout,
    );
    assert_eq!(
        blocked_pick["error"]["message"],
        format!("verification_pick_readiness_blocked:{plan_id}")
    );
    assert_eq!(
        blocked_pick["error"]["details"]["gaps"][0]["code"],
        "missing_payload_schema"
    );
    assert_eq!(
        blocked_pick["error"]["details"]["gaps"][0]["obligation_id"],
        "pob-canonical-verifier-packet-missing-schema"
    );
    let conn = Connection::open(&db).unwrap();
    let rolled_back: (String, Option<String>, String, i64) = conn
        .query_row(
            "SELECT items.status, items.worker_id, feature_runs.phase,
                    (SELECT COUNT(*) FROM feature_run_role_leases
                     WHERE run_id = feature_runs.id AND role = 'verifier' AND released_at IS NULL)
             FROM items JOIN plans ON plans.path = items.plan_path
             JOIN feature_runs ON feature_runs.plan_id = plans.id
             WHERE items.id = ?1",
            [&verification_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        rolled_back,
        ("ready".into(), None, "held".into(), 0)
    );
    drop(conn);

    obligation["supersedes"] = json!("pob-canonical-verifier-packet-missing-schema");
    add_evidence_obligation_value(
        dir.path(),
        &db,
        "pob-canonical-verifier-packet",
        &obligation,
    );
    fs::remove_file(
        dir.path()
            .join("pob-canonical-verifier-packet.migration.json"),
    )
    .unwrap();

    let repair = &blocked_pick["error"]["details"]["repair_request"];
    let run_id = repair["run_id"].as_str().unwrap();
    let freeze_id = repair["freeze_id"].as_str().unwrap();
    let revision = repair["run_revision"].as_u64().unwrap().to_string();
    let repaired = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "canonical-verifier-incomplete-binding")
            .args([
                "--db", &db_arg, "--json", "run", "repair-verification-admission", "--plan",
                &plan_id, "--run", run_id, "--freeze", freeze_id, "--revision", &revision,
                "--reason", "readiness-blocked",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(repaired["repair"]["repaired_run"]["phase"], "implementation");
    let invalidation_id = repaired["repair"]["facts"]["invalidation_id"]
        .as_str()
        .unwrap();
    let repair_packet = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "canonical-maker")
            .args(["--db", &db_arg, "--json", "pick", "--plan", &plan_id, "--work-type", "code"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(repair_packet["work_packet"]["mode"], "verification_admission_repair");
    let refrozen = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "canonical-maker")
            .args([
                "--db", &db_arg, "--json", "run", "settle-repair", "--plan", &plan_id,
                "--invalidation", invalidation_id, "--summary", "settled verifier readiness repair",
                "--files", "tests/e2e.rs", "--cmd", "true", "--tests", "focused repair lifecycle",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(refrozen["reason"], "verification_handoff_source_frozen");

    let packet = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "canonical-verifier")
            .args([
                "--db",
                &db_arg,
                "--json",
                "pick",
                "--plan",
                &plan_id,
                "--work-type",
                "verification",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(packet["work_packet"]["kind"], "verification");
    assert_eq!(packet["work_packet"]["item_id"], verification_id);
    assert!(packet["work_packet"].get("mode").is_none());
    assert!(packet["work_packet"].get("repair_id").is_none());
    assert!(
        packet["work_packet"]
            .get("selective_replay_obligation_ids")
            .is_none()
    );
    assert_eq!(
        packet["work_packet"]["execution_state"]["phase"], "verification",
        "{packet}"
    );
    assert_eq!(
        packet["work_packet"]["verification_lease"]["worker_id"],
        "canonical-verifier"
    );
    assert!(packet["work_packet"]["source_freeze"]["source_digest"].is_string());
    assert_eq!(
        packet["work_packet"]["sealed_run_index"]["schema_version"],
        "planr.evidence.run-index.v2"
    );
    assert_eq!(
        packet["work_packet"]["sealed_run_index"]["runs"][0]["input"]["obligation_id"],
        "pob-canonical-verifier-packet"
    );

    let trace = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args(["--db", &db_arg, "--json", "trace", "item", &verification_id])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(trace["proof"]["active_binding"], true);
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
    fs::create_dir_all(dir.path().join(".codex")).unwrap();
    assert_codex_0145_stop_schema_fixture(dir.path());
    fs::write(
        dir.path().join(".codex/hooks.planr-unsupported.json"),
        json!({
            "Notification": [
                {"hooks": [{"type": "command", "command": "preexisting sidecar notification", "timeout": 1}]}
            ]
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        dir.path().join(".codex/hooks.json"),
        json!({
            "hooks": {
                "Notification": [
                    {"hooks": [{"type": "command", "command": "foreign codex notification", "timeout": 1}]}
                ],
                "UnsupportedObject": {"enabled": true, "command": "foreign codex object"},
                "UserPromptSubmit": [
                    {"hooks": [{"type": "command", "command": "foreign codex prompt", "timeout": 1}]}
                ]
            },
            "SessionStart": [
                {"hooks": [{"type": "command", "command": "foreign codex session", "timeout": 1}]},
                {"hooks": [{"type": "command", "command": "planr prime 2>/dev/null || true", "timeout": 10}]}
            ],
            "PermissionRequest": [
                {"hooks": [{"type": "command", "command": "foreign codex permission", "timeout": 1}]}
            ],
            "PostCompact": [
                {"hooks": [{"type": "command", "command": "foreign codex compact", "timeout": 1}]}
            ],
            "SessionEnd": [
                {"hooks": [{"type": "command", "command": "foreign codex session-end", "timeout": 1}]}
            ],
            "SubagentStart": [
                {"hooks": [{"type": "command", "command": "foreign codex subagent-start", "timeout": 1}]}
            ],
            "Notification": [
                {"hooks": [{"type": "command", "command": "foreign codex root notification", "timeout": 1}]}
            ],
            "CustomEvent": [
                {"hooks": [{"type": "command", "command": "foreign codex custom", "timeout": 1}]}
            ]
        })
        .to_string(),
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "install", "codex", "--no-mcp"])
        .assert()
        .success()
        .stdout(predicate::str::contains("run /hooks"))
        .stdout(predicate::str::contains("unsupported Codex hook event"));
    let codex: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".codex/hooks.json")).unwrap())
            .unwrap();
    assert_codex_hooks_config_schema(&codex);
    assert!(
        codex["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("planr prime"))
    );
    assert!(
        codex["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("planr-codex-stop.sh")
    );
    assert!(codex.get("SessionStart").is_none());
    assert!(codex.get("PermissionRequest").is_none());
    assert!(codex.get("PostCompact").is_none());
    assert!(codex.get("SessionEnd").is_none());
    assert!(codex.get("SubagentStart").is_none());
    assert!(codex.get("CustomEvent").is_none());
    assert!(codex["hooks"].get("Notification").is_none());
    assert!(codex["hooks"].get("UnsupportedObject").is_none());
    let nested_session = codex["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(nested_session.len(), 2, "foreign plus planr session hooks");
    assert!(
        nested_session
            .iter()
            .any(|entry| entry["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("foreign codex session")),
        "flat foreign Codex entry is migrated"
    );
    for (event, command) in [
        ("PermissionRequest", "foreign codex permission"),
        ("PostCompact", "foreign codex compact"),
        ("SessionEnd", "foreign codex session-end"),
        ("SubagentStart", "foreign codex subagent-start"),
        ("UserPromptSubmit", "foreign codex prompt"),
    ] {
        assert!(
            codex["hooks"][event]
                .as_array()
                .unwrap_or_else(|| panic!("{event} missing from active hooks: {codex}"))
                .iter()
                .any(|entry| entry["hooks"][0]["command"]
                    .as_str()
                    .unwrap()
                    .contains(command)),
            "{event} foreign hook preserved in active Codex hooks"
        );
    }
    assert!(
        codex["hooks"].get("CustomEvent").is_none(),
        "unsupported root-level Codex event must not be migrated into active hooks"
    );
    let unsupported: Value = serde_json::from_str(
        &fs::read_to_string(dir.path().join(".codex/hooks.planr-unsupported.json")).unwrap(),
    )
    .unwrap();
    assert_unsupported_codex_sidecar_contains(&unsupported, "CustomEvent", "foreign codex custom");
    assert_unsupported_codex_sidecar_contains(
        &unsupported,
        "Notification",
        "preexisting sidecar notification",
    );
    assert_unsupported_codex_sidecar_contains(
        &unsupported,
        "Notification",
        "foreign codex root notification",
    );
    assert_unsupported_codex_sidecar_contains(
        &unsupported,
        "Notification",
        "foreign codex notification",
    );
    assert_eq!(unsupported["Notification"].as_array().unwrap().len(), 3);
    assert_eq!(
        unsupported["UnsupportedObject"]["command"], "foreign codex object",
        "unsupported nested non-array Codex event is preserved out-of-band"
    );
    let first_codex = fs::read_to_string(dir.path().join(".codex/hooks.json")).unwrap();
    let first_unsupported =
        fs::read_to_string(dir.path().join(".codex/hooks.planr-unsupported.json")).unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "install", "codex", "--no-mcp"])
        .assert()
        .success();
    let second_codex = fs::read_to_string(dir.path().join(".codex/hooks.json")).unwrap();
    let second_unsupported =
        fs::read_to_string(dir.path().join(".codex/hooks.planr-unsupported.json")).unwrap();
    assert_eq!(
        first_codex, second_codex,
        "codex hook install is idempotent"
    );
    assert_eq!(
        first_unsupported, second_unsupported,
        "unsupported Codex hook sidecar is idempotent"
    );
    let codex_stop = dir.path().join(".codex/hooks/planr-codex-stop.sh");
    assert!(codex_stop.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert!(
            fs::metadata(&codex_stop).unwrap().permissions().mode() & 0o111 != 0,
            "codex stop hook must be executable"
        );
    }
    assert!(
        codex["hooks"]["PostCompact"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("foreign codex compact"),
        "foreign supported PostCompact hooks remain active even though planr does not wire one"
    );

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
fn codex_unsupported_sidecar_must_be_mergeable_before_quarantine() {
    for (name, sidecar) in [
        ("malformed", "{not json"),
        ("non-object", "[{\"Notification\": []}]"),
    ] {
        let dir = tempdir().unwrap();
        let db = dir.path().join(".planr/planr.sqlite");
        let db_arg = db.to_str().unwrap().to_string();
        planr()
            .current_dir(dir.path())
            .args(["--db", &db_arg, "project", "init", name])
            .assert()
            .success();
        fs::create_dir_all(dir.path().join(".codex")).unwrap();
        let active = json!({
            "hooks": {
                "Notification": [
                    {"hooks": [{"type": "command", "command": format!("foreign nested {name}"), "timeout": 1}]}
                ],
                "SessionStart": [
                    {"hooks": [{"type": "command", "command": "foreign supported session", "timeout": 1}]}
                ]
            },
            "CustomEvent": [
                {"hooks": [{"type": "command", "command": format!("foreign root {name}"), "timeout": 1}]}
            ]
        })
        .to_string();
        fs::write(dir.path().join(".codex/hooks.json"), &active).unwrap();
        fs::write(
            dir.path().join(".codex/hooks.planr-unsupported.json"),
            sidecar,
        )
        .unwrap();
        let sidecar_before =
            fs::read_to_string(dir.path().join(".codex/hooks.planr-unsupported.json")).unwrap();

        for _ in 0..2 {
            planr()
                .current_dir(dir.path())
                .args(["--db", &db_arg, "install", "codex", "--no-mcp"])
                .assert()
                .success()
                .stdout(predicate::str::contains(
                    "Codex unsupported hooks skipped to avoid data loss",
                ));
            assert_eq!(
                fs::read_to_string(dir.path().join(".codex/hooks.json")).unwrap(),
                active,
                "{name}: active Codex hooks must remain untouched"
            );
            assert_eq!(
                fs::read_to_string(dir.path().join(".codex/hooks.planr-unsupported.json")).unwrap(),
                sidecar_before,
                "{name}: malformed sidecar must remain untouched"
            );
        }
    }
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
fn codex_stop_hook_enforces_active_goal_with_bounded_canonical_gaps() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    write_evidence_policy_fixture(dir.path());
    init_git_repo(dir.path());
    init_evidence_project(dir.path(), &db, "Stop Guard");
    let product = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                &db_arg,
                "--json",
                "plan",
                "new",
                "Stop Guard Product",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let product_id = product["plan"]["id"].as_str().unwrap().to_string();
    let build = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                &db_arg,
                "--json",
                "plan",
                "split",
                &product_id,
                "--slice",
                "Stop Guard Build",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let plan_id = build["plan"]["id"].as_str().unwrap().to_string();
    let build_path = build["plan"]["path"].as_str().unwrap().to_string();
    let text = fs::read_to_string(&build_path).unwrap();
    let frontmatter_end = text.find("\n---\n").unwrap() + 5;
    fs::write(
        &build_path,
        format!(
            "{}\ncriteria:\n  - id: crit-stop-missing\n    title: Stop hook browser evidence\n---\n\n# Build Plan\n\n## Scope Decision\n\nStop guard.\n\n## Verification\n\nEvidence coverage.\n\n## Acceptance Criteria\n\n- Evidence must prove the plan.\n\n## Steps\n\n### Ship\n\nClose implementation.\n",
            &text[..frontmatter_end - 5]
        ),
    )
    .unwrap();
    let map = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db", &db_arg, "--json", "map", "build", "--from", &plan_id,
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let item = map["created"][0]["id"].as_str().unwrap().to_string();
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            r#"
INSERT INTO projects(id, name, root_path, created_at, updated_at)
VALUES ('p-stop-other', 'Other Stop Project', '.', datetime('now'), datetime('now'));
INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, archived, created_at, updated_at)
VALUES ('pln-stop-other', 'p-stop-other', 'build', 'other.md', 'Other Stop Plan', 'other-stop-plan', 'ok', 'hash', 0, datetime('now'), datetime('now'));
"#,
        )
        .unwrap();
    planr()
        .current_dir(dir.path())
        .env("CODEX_THREAD_ID", "thread-cross")
        .args([
            "--db",
            &db_arg,
            "--json",
            "stop",
            "activate",
            "--plan",
            "pln-stop-other",
        ])
        .assert()
        .failure();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "context",
            "add",
            &format!("GOAL CONTRACT {plan_id}: DONE when Evidence coverage passes."),
            "--tag",
            "goal-contract",
        ])
        .assert()
        .success();
    let policy = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args(["--db", &db_arg, "--json", "evidence", "policy"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let probe = policy["object"]["registry"]["probes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|probe| probe["manifest_id"] == "verifier-generic-adapter")
        .unwrap();
    let instance_id = probe["instance_id"].as_str().unwrap();
    let environment = capability_instance_environment(&db, instance_id);
    let mut obligation = evidence_obligation_for(
        "pob-stop-missing",
        policy["object"]["digest"].as_str().unwrap(),
        "com.example.browser.page.visible",
        "stop hook missing browser evidence",
        json!({"visible": true}),
        json!({"kind": "browser", "uri": "http://127.0.0.1:9/stop"}),
        environment,
        json!({"kind": "browser", "id": "browser-session"}),
        json!([]),
        "sha256:abababababababababababababababababababababababababababababababab",
    );
    obligation["plan_id"] = json!(plan_id);
    obligation["item_id"] = Value::Null;
    obligation["criterion_id"] = json!("crit-stop-missing");
    add_evidence_obligation_value(dir.path(), &db, "pob-stop-missing", &obligation);
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "close", &item, "--summary", "item done"])
        .assert()
        .success();

    planr()
        .current_dir(dir.path())
        .env("CODEX_THREAD_ID", "thread-stop")
        .args([
            "--db", &db_arg, "--json", "stop", "activate", "--plan", &plan_id,
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .args([
            "--db", &db_arg, "--json", "stop", "activate", "--plan", &plan_id,
        ])
        .env_remove("CODEX_THREAD_ID")
        .assert()
        .failure();

    let official = |session: &str| -> String { codex_stop_envelope(dir.path(), session, false) };
    let stop = |payload: &str, db_path: &str| -> Value {
        if let Ok(value) = serde_json::from_str::<Value>(payload) {
            assert_codex_stop_input_schema(&value);
        }
        let mut cmd = planr();
        cmd.current_dir(dir.path())
            .args(["--db", db_path, "--json", "stop"])
            .write_stdin(payload)
            .env_remove("PLANR_ITEM_ID")
            .env_remove("PLANR_WORKER_ID")
            .env_remove("PLANR_SESSION_ID")
            .env_remove("CODEX_THREAD_ID");
        let output = cmd.output().unwrap();
        assert!(output.status.success(), "stop must fail open");
        single_json_document(&output.stdout)
    };

    let no_goal = stop(&official("thread-other"), &db_arg);
    assert_codex_stop_output_schema(&no_goal, false);
    assert!(no_goal.as_object().unwrap().is_empty(), "{no_goal}");

    let blocked = stop(&official("thread-stop"), &db_arg);
    assert_codex_stop_output_schema(&blocked, true);
    assert_eq!(blocked["decision"], "block", "{blocked}");
    let reason = blocked["reason"].as_str().unwrap();
    assert!(reason.contains("active goal plan"), "{blocked}");
    assert!(reason.contains("actionable gap"), "{blocked}");
    assert!(reason.contains("crit-stop-missing"), "{blocked}");
    assert!(reason.contains("missing_observation"), "{blocked}");
    assert!(
        reason.contains("collect missing trusted evidence"),
        "{blocked}"
    );
    assert!(reason.len() < 700, "{reason}");

    let same_gap_second = stop(&official("thread-stop"), &db_arg);
    assert_eq!(same_gap_second["decision"], "block", "{same_gap_second}");
    let same_gap_exhausted = stop(&official("thread-stop"), &db_arg);
    assert!(
        same_gap_exhausted.as_object().unwrap().is_empty(),
        "{same_gap_exhausted}"
    );

    let reentrant = stop(
        &codex_stop_envelope(dir.path(), "thread-stop", true),
        &db_arg,
    );
    assert_codex_stop_output_schema(&reentrant, false);
    assert!(reentrant.as_object().unwrap().is_empty(), "{reentrant}");
    planr()
        .current_dir(dir.path())
        .env("CODEX_THREAD_ID", "thread-stop")
        .args([
            "--db", &db_arg, "--json", "stop", "resume", "--plan", &plan_id,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"status\": \"resumed\""));
    let resumed = stop(
        &codex_stop_envelope(dir.path(), "thread-stop", true),
        &db_arg,
    );
    assert_codex_stop_output_schema(&resumed, true);
    assert_eq!(resumed["decision"], "block", "{resumed}");
    let malformed = stop("{", &db_arg);
    assert_codex_stop_output_schema(&malformed, false);
    assert!(malformed.as_object().unwrap().is_empty(), "{malformed}");
    let bad_parent = dir.path().join("not-a-directory");
    fs::write(&bad_parent, "file").unwrap();
    let bad_db = bad_parent.join("planr.sqlite");
    let bad_db = bad_db.to_str().unwrap().to_string();
    let db_fail_open = stop(&official("thread-stop"), &bad_db);
    assert_codex_stop_output_schema(&db_fail_open, false);
    assert!(
        db_fail_open.as_object().unwrap().is_empty(),
        "{db_fail_open}"
    );

    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "install", "codex", "--no-mcp"])
        .assert()
        .success();
    let hooks: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".codex/hooks.json")).unwrap())
            .unwrap();
    let command = hooks["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert_eq!(command, ".codex/hooks/planr-codex-stop.sh");
    let script = dir.path().join(command);
    let bin_dir = assert_cmd::cargo::cargo_bin("planr")
        .parent()
        .unwrap()
        .to_path_buf();
    let path_env = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let tmp_dir = dir.path().join("tmp");
    fs::create_dir_all(&tmp_dir).unwrap();
    let run_script = |payload: &str, cwd: &Path, path: &str| -> std::process::Output {
        let output = StdCommand::new("sh")
            .arg(&script)
            .current_dir(cwd)
            .env("PATH", path)
            .env("TMPDIR", &tmp_dir)
            .env_remove("PLANR_WORKER_ID")
            .env_remove("PLANR_ITEM_ID")
            .env_remove("PLANR_SESSION_ID")
            .env_remove("CODEX_THREAD_ID")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                child
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(payload.as_bytes())?;
                child.wait_with_output()
            })
            .unwrap();
        let leftovers = fs::read_dir(&tmp_dir).unwrap().count();
        assert_eq!(leftovers, 0, "mktemp wrapper must clean temporary files");
        output
    };
    let script_output = run_script(&official("thread-stop"), dir.path(), &path_env);
    assert!(script_output.status.success(), "script must fail open");
    let script_decision = single_json_document(&script_output.stdout);
    assert_codex_stop_output_schema(&script_decision, true);
    assert_eq!(script_decision["decision"], "block", "{script_decision}");

    let script_neutral = run_script(&official("thread-other"), dir.path(), &path_env);
    assert!(script_neutral.status.success());
    let script_neutral = single_json_document(&script_neutral.stdout);
    assert_codex_stop_output_schema(&script_neutral, false);

    let script_malformed = run_script("{", dir.path(), &path_env);
    assert!(script_malformed.status.success());
    let script_malformed = single_json_document(&script_malformed.stdout);
    assert_codex_stop_output_schema(&script_malformed, false);

    let script_bad_dir = tempdir().unwrap();
    fs::write(script_bad_dir.path().join(".planr"), "file").unwrap();
    let script_db_failure = run_script(&official("thread-stop"), script_bad_dir.path(), &path_env);
    assert!(script_db_failure.status.success());
    let script_db_failure = single_json_document(&script_db_failure.stdout);
    assert_codex_stop_output_schema(&script_db_failure, false);

    planr()
        .current_dir(dir.path())
        .env("CODEX_THREAD_ID", "thread-move")
        .args([
            "--db", &db_arg, "--json", "stop", "activate", "--plan", &plan_id,
        ])
        .assert()
        .success();
    for index in 0..6 {
        if index > 0 {
            let mut moved_obligation = obligation.clone();
            moved_obligation["id"] = json!(format!("pob-stop-moved-{index}"));
            moved_obligation["title"] = json!(format!("stop hook moved browser evidence {index}"));
            moved_obligation["supersedes"] = json!(if index == 1 {
                "pob-stop-missing".to_string()
            } else {
                format!("pob-stop-moved-{}", index - 1)
            });
            moved_obligation["observations"][0]["id"] = json!(format!("obs-stop-moved-{index}"));
            add_evidence_obligation_value(
                dir.path(),
                &db,
                &format!("pob-stop-moved-{index}"),
                &moved_obligation,
            );
        }
        let moved = stop(&official("thread-move"), &db_arg);
        assert_eq!(moved["decision"], "block", "iteration {index}: {moved}");
        assert!(
            moved["reason"].as_str().unwrap().contains("total 1/6"),
            "iteration {index}: {moved}"
        );
    }
    let mut exhausted_obligation = obligation.clone();
    exhausted_obligation["id"] = json!("pob-stop-moved-exhausted");
    exhausted_obligation["title"] = json!("stop hook exhausted browser evidence");
    exhausted_obligation["supersedes"] = json!("pob-stop-moved-5");
    exhausted_obligation["observations"][0]["id"] = json!("obs-stop-moved-exhausted");
    add_evidence_obligation_value(
        dir.path(),
        &db,
        "pob-stop-moved-exhausted",
        &exhausted_obligation,
    );
    let total_exhausted = stop(&official("thread-move"), &db_arg);
    assert_codex_stop_output_schema(&total_exhausted, true);
    assert_eq!(total_exhausted["decision"], "block", "{total_exhausted}");

    let missing_planr = StdCommand::new("sh")
        .arg(&script)
        .current_dir(dir.path())
        .env("PATH", "/usr/bin:/bin")
        .env("TMPDIR", &tmp_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Err(error) = child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(official("thread-stop").as_bytes())
                && error.kind() != std::io::ErrorKind::BrokenPipe
            {
                return Err(error);
            }
            child.wait_with_output()
        })
        .unwrap();
    assert!(missing_planr.status.success());
    assert!(missing_planr.stdout.is_empty());
    assert_eq!(fs::read_dir(&tmp_dir).unwrap().count(), 0);

    planr()
        .current_dir(dir.path())
        .env("CODEX_THREAD_ID", "thread-stop")
        .args([
            "--db",
            &db_arg,
            "--json",
            "stop",
            "deactivate",
            "--plan",
            &plan_id,
        ])
        .assert()
        .success();
    let deactivated = stop(&official("thread-stop"), &db_arg);
    assert!(deactivated.as_object().unwrap().is_empty(), "{deactivated}");
}

#[test]
fn codex_stop_hook_enforces_explicit_active_plan_until_final_review_or_archive() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    fs::write(dir.path().join("README.md"), "# Stop Plan\n").unwrap();
    init_git_repo(dir.path());
    init_evidence_project(dir.path(), &db, "Stop Plan");

    let run = |args: &[&str]| -> Value {
        single_json_document(
            &planr()
                .current_dir(dir.path())
                .args(["--db", &db_arg, "--json"])
                .args(args)
                .assert()
                .success()
                .get_output()
                .stdout,
        )
    };
    let product = run(&["plan", "new", "Stop Plan Product"]);
    let product_id = product["plan"]["id"].as_str().unwrap().to_string();
    let build = run(&["plan", "split", &product_id, "--slice", "Stop Plan Build"]);
    let plan_id = build["plan"]["id"].as_str().unwrap().to_string();
    let build_path = build["plan"]["path"].as_str().unwrap().to_string();
    let text = fs::read_to_string(&build_path).unwrap();
    let frontmatter_end = text.find("\n---\n").unwrap() + 5;
    fs::write(
        &build_path,
        format!(
            "{}\n# Build Plan\n\n## Scope Decision\n\nStop plan enforcement.\n\n## Verification\n\nPlan audit holds.\n\n## Acceptance Criteria\n\n- Both steps are closed.\n\n## Steps\n\n### First\n\nClose first.\n\n### Second\n\nClose second.\n",
            &text[..frontmatter_end]
        ),
    )
    .unwrap();
    let map = run(&["map", "build", "--from", &plan_id]);
    let items = map["created"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{map}");
    let first = items[0]["id"].as_str().unwrap().to_string();
    let second = items[1]["id"].as_str().unwrap().to_string();

    planr()
        .current_dir(dir.path())
        .env("CODEX_THREAD_ID", "thread-a")
        .args([
            "--db", &db_arg, "--json", "stop", "activate", "--plan", &plan_id,
        ])
        .assert()
        .success();

    let stop = |session: &str| -> Value {
        let output = planr()
            .current_dir(dir.path())
            .args(["--db", &db_arg, "--json", "stop"])
            .write_stdin(codex_stop_envelope(dir.path(), session, false))
            .env_remove("CODEX_THREAD_ID")
            .env_remove("PLANR_SESSION_ID")
            .env_remove("PLANR_WORKER_ID")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        single_json_document(&output)
    };

    let between_items = stop("thread-a");
    assert_eq!(between_items["decision"], "block", "{between_items}");
    assert!(
        between_items["reason"].as_str().unwrap().contains(&plan_id),
        "{between_items}"
    );
    let isolated = stop("thread-b");
    assert!(isolated.as_object().unwrap().is_empty(), "{isolated}");

    run(&["close", &first, "--summary", "first done"]);
    let pending_plan_item = stop("thread-a");
    assert_eq!(
        pending_plan_item["decision"], "block",
        "{pending_plan_item}"
    );
    assert!(
        pending_plan_item["reason"]
            .as_str()
            .unwrap()
            .contains(&second),
        "{pending_plan_item}"
    );

    planr()
        .current_dir(dir.path())
        .env("CODEX_THREAD_ID", "thread-a")
        .args([
            "--db",
            &db_arg,
            "--json",
            "stop",
            "deactivate",
            "--plan",
            &plan_id,
        ])
        .assert()
        .success();
    let deactivated = stop("thread-a");
    assert!(deactivated.as_object().unwrap().is_empty(), "{deactivated}");

    planr()
        .current_dir(dir.path())
        .env("CODEX_THREAD_ID", "thread-a")
        .args([
            "--db", &db_arg, "--json", "stop", "activate", "--plan", &plan_id,
        ])
        .assert()
        .success();
    run(&["close", &second, "--summary", "second done"]);
    let final_review_required = stop("thread-a");
    assert_eq!(
        final_review_required["decision"], "block",
        "{final_review_required}"
    );
    assert!(
        final_review_required["reason"]
            .as_str()
            .unwrap()
            .contains("final_product_review_complete"),
        "{final_review_required}"
    );

    planr()
        .current_dir(dir.path())
        .env("CODEX_THREAD_ID", "thread-archived")
        .args([
            "--db", &db_arg, "--json", "stop", "activate", "--plan", &plan_id,
        ])
        .assert()
        .success();
    run(&["plan", "archive", &plan_id]);
    let archived = stop("thread-archived");
    assert!(archived.as_object().unwrap().is_empty(), "{archived}");
}

#[test]
fn evidence_migration_explicitly_binds_pre_evidence_plans_without_rewriting_claims() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    write_evidence_policy_fixture(dir.path());
    init_git_repo(dir.path());
    init_evidence_project(dir.path(), &db, "Evidence Migration");
    let run = |args: &[&str]| -> Value {
        single_json_document(
            &planr()
                .current_dir(dir.path())
                .args(["--db", &db_arg, "--json"])
                .args(args)
                .assert()
                .success()
                .get_output()
                .stdout,
        )
    };
    let run_human = |args: &[&str]| -> String {
        String::from_utf8(
            planr()
                .current_dir(dir.path())
                .args(["--db", &db_arg])
                .args(args)
                .assert()
                .success()
                .get_output()
                .stdout
                .clone(),
        )
        .unwrap()
    };
    let run_failure = |args: &[&str]| -> Value {
        single_json_document(
            &planr()
                .current_dir(dir.path())
                .args(["--db", &db_arg, "--json"])
                .args(args)
                .assert()
                .failure()
                .get_output()
                .stdout,
        )
    };
    let product = run(&["plan", "new", "Migration Product"]);
    let product_id = product["plan"]["id"].as_str().unwrap();
    let build = run(&["plan", "split", product_id, "--slice", "Migration Build"]);
    let plan_id = build["plan"]["id"].as_str().unwrap().to_string();
    let build_path = build["plan"]["path"].as_str().unwrap().to_string();
    let text = fs::read_to_string(&build_path).unwrap();
    let frontmatter_end = text.find("\n---\n").unwrap() + 5;
    fs::write(
        &build_path,
        format!(
            "{}\n# Build Plan\n\n## Scope Decision\n\nMigration.\n\n## Verification\n\nEvidence coverage.\n\n## Acceptance Criteria\n\n- Binding migration evidence exists.\n\n## Steps\n\n### Ship\n\nDo it.\n",
            &text[..frontmatter_end]
        ),
    )
    .unwrap();
    author_build_plan_criterion(Path::new(&build_path), "crit-migration-binding");
    let map = run(&["map", "build", "--from", &plan_id]);
    let item_id = map["created"][0]["id"].as_str().unwrap().to_string();
    run(&[
        "log",
        "add",
        "--item",
        &item_id,
        "--kind",
        "verification",
        "--summary",
        "legacy browser claim",
        "--cmd",
        "printf legacy",
    ]);
    let unmigrated_status = run(&["map", "status"]);
    let ready_entry = unmigrated_status["ready"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["item"]["id"] == item_id)
        .unwrap();
    assert_eq!(ready_entry["proof"]["status"], "binding_unsatisfied");
    assert!(
        ready_entry["proof"]["next_action"]
            .as_str()
            .unwrap()
            .contains(&plan_id)
    );
    let held_pick = run(&["pick", "--plan", &plan_id, "--work-type", "code"]);
    assert_eq!(held_pick["work_packet"]["kind"], "hold");
    assert_eq!(
        held_pick["work_packet"]["reason_code"],
        "missing_obligation"
    );
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "close",
            &item_id,
            "--summary",
            "unmigrated item done",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "binding_evidence_obligations_missing",
        ));
    run(&[
        "context",
        "add",
        &format!("GOAL CONTRACT {plan_id}: DONE when live verification logged."),
        "--tag",
        "goal-contract",
    ]);
    let pre_audit = run(&["plan", "audit", &plan_id]);
    let pre_clause = pre_audit["clauses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clause| clause["clause"] == "verification_logged")
        .unwrap();
    assert_eq!(pre_audit["holds"], false, "{pre_audit}");
    assert_eq!(pre_clause["authority"], "evidence_policy");
    assert_eq!(pre_clause["pass"], false);
    assert_eq!(pre_clause["log_authority"], "claim_only");
    assert_eq!(pre_audit["proof"]["status"], "binding_unsatisfied");
    assert!(
        pre_audit["proof"]["next_action"]
            .as_str()
            .unwrap()
            .contains(&plan_id)
    );
    assert!(
        !pre_audit["proof"]["next_action"]
            .as_str()
            .unwrap()
            .contains("<plan-id>")
    );
    let pre_audit_human = run_human(&["plan", "audit", &plan_id]);
    assert!(
        pre_audit_human.contains(&format!(
            "planr evidence migrate --input <migration-file-for-plan-{plan_id}> --apply"
        )),
        "{pre_audit_human}"
    );
    let item_trace = run(&["trace", "item", &item_id]);
    assert_eq!(item_trace["proof"]["status"], "binding_unsatisfied");
    assert!(
        item_trace["proof"]["next_action"]
            .as_str()
            .unwrap()
            .contains(&plan_id)
    );

    let policy = run(&["evidence", "policy"]);
    let environment = capability_instance_environment(
        &db,
        policy["object"]["registry"]["probes"][0]["instance_id"]
            .as_str()
            .unwrap(),
    );
    let mut obligation = evidence_obligation_for(
        "pob-migration-binding",
        policy["object"]["digest"].as_str().unwrap(),
        "com.example.health.status",
        "migration health",
        json!({"status": "ok"}),
        json!({"kind": "process", "uri": "local://health"}),
        environment,
        json!({"kind": "process", "id": "runtime-local"}),
        json!([]),
        "sha256:abababababababababababababababababababababababababababababababab",
    );
    obligation["plan_id"] = json!(plan_id);
    obligation["item_id"] = Value::Null;
    obligation["criterion_id"] = json!("crit-migration-binding");
    let migration = json!({
        "schema_version": "planr.evidence.migration.v1",
        "plan_id": plan_id,
        "obligations": [obligation],
    });
    let migration_path = dir.path().join("migration.json");
    fs::write(
        &migration_path,
        serde_json::to_vec_pretty(&migration).unwrap(),
    )
    .unwrap();

    let preview = run(&[
        "evidence",
        "migrate",
        "--input",
        migration_path.to_str().unwrap(),
    ]);
    assert_evidence_envelope(&preview, "evidence.migrate", true);
    assert_eq!(preview["object"]["dry_run"], true);
    assert_eq!(preview["object"]["summary"]["create"], 1);
    assert_eq!(
        preview["object"]["warnings"][0]["code"],
        "verification_claims_are_not_evidence"
    );
    let mut missing_schema = migration.clone();
    missing_schema
        .as_object_mut()
        .unwrap()
        .remove("schema_version");
    let missing_schema_path = dir.path().join("migration-missing-schema.json");
    fs::write(
        &missing_schema_path,
        serde_json::to_vec_pretty(&missing_schema).unwrap(),
    )
    .unwrap();
    let missing_schema_cli = run_failure(&[
        "evidence",
        "migrate",
        "--input",
        missing_schema_path.to_str().unwrap(),
    ]);
    assert_evidence_error(
        &missing_schema_cli,
        "evidence.migrate",
        "bad_request",
        "evidence migration input requires schema_version",
    );
    let mcp_missing_schema_response = mcp_tool_response(
        dir.path(),
        &db,
        43,
        "planr_evidence_migrate",
        json!({"input": missing_schema.clone()}),
    );
    assert_mcp_evidence_error(
        &mcp_missing_schema_response,
        "evidence.migrate",
        "bad_request",
        "evidence migration input requires schema_version",
    );
    let mut unknown_field = migration.clone();
    unknown_field["unexpected"] = json!(true);
    let unknown_field_path = dir.path().join("migration-unknown-field.json");
    fs::write(
        &unknown_field_path,
        serde_json::to_vec_pretty(&unknown_field).unwrap(),
    )
    .unwrap();
    let unknown_field_cli = run_failure(&[
        "evidence",
        "migrate",
        "--input",
        unknown_field_path.to_str().unwrap(),
    ]);
    assert_evidence_error(
        &unknown_field_cli,
        "evidence.migrate",
        "bad_request",
        "unknown fields",
    );
    let mcp_unknown_field_response = mcp_tool_response(
        dir.path(),
        &db,
        44,
        "planr_evidence_migrate",
        json!({"input": unknown_field.clone()}),
    );
    assert_mcp_evidence_error(
        &mcp_unknown_field_response,
        "evidence.migrate",
        "bad_request",
        "unknown fields",
    );
    let wrong_apply_type_response = mcp_tool_response(
        dir.path(),
        &db,
        45,
        "planr_evidence_migrate",
        json!({"input": migration.clone(), "apply": "true"}),
    );
    assert_mcp_evidence_error(
        &wrong_apply_type_response,
        "evidence.migrate",
        "bad_request",
        "apply must be a boolean",
    );
    let mut duplicate = migration.clone();
    let duplicate_obligation = duplicate["obligations"][0].clone();
    duplicate["obligations"]
        .as_array_mut()
        .unwrap()
        .push(duplicate_obligation);
    let duplicate_path = dir.path().join("migration-duplicate.json");
    fs::write(
        &duplicate_path,
        serde_json::to_vec_pretty(&duplicate).unwrap(),
    )
    .unwrap();
    let duplicate_cli = run_failure(&[
        "evidence",
        "migrate",
        "--input",
        duplicate_path.to_str().unwrap(),
    ]);
    assert_evidence_error(
        &duplicate_cli,
        "evidence.migrate",
        "bad_request",
        "duplicate obligation id: pob-migration-binding",
    );
    let mcp_duplicate_response = mcp_tool_response(
        dir.path(),
        &db,
        46,
        "planr_evidence_migrate",
        json!({"input": duplicate.clone()}),
    );
    assert_mcp_evidence_error(
        &mcp_duplicate_response,
        "evidence.migrate",
        "bad_request",
        "duplicate obligation id: pob-migration-binding",
    );
    let still_unmigrated = run(&["plan", "audit", &plan_id]);
    assert_eq!(still_unmigrated["holds"], false, "{still_unmigrated}");
    assert_eq!(still_unmigrated["proof"]["status"], "binding_unsatisfied");

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
    wait_for_http_server(port);
    assert_http_evidence_error(
        &http_request(
            port,
            "POST",
            "/v1/evidence/migrate",
            &json!({"input": missing_schema.clone()}).to_string(),
        ),
        "400 Bad Request",
        "evidence.migrate",
        "bad_request",
        "evidence migration input requires schema_version",
    );
    assert_http_evidence_error(
        &http_request(
            port,
            "POST",
            "/v1/evidence/migrate",
            &json!({"input": unknown_field.clone()}).to_string(),
        ),
        "400 Bad Request",
        "evidence.migrate",
        "bad_request",
        "unknown fields",
    );
    assert_http_evidence_error(
        &http_request(
            port,
            "POST",
            "/v1/evidence/migrate",
            &json!({"input": migration.clone(), "apply": "true"}).to_string(),
        ),
        "400 Bad Request",
        "evidence.migrate",
        "bad_request",
        "apply must be a boolean",
    );
    assert_http_evidence_error(
        &http_request(
            port,
            "POST",
            "/v1/evidence/migrate",
            &json!({"input": duplicate.clone()}).to_string(),
        ),
        "400 Bad Request",
        "evidence.migrate",
        "bad_request",
        "duplicate obligation id: pob-migration-binding",
    );

    let applied = run(&[
        "evidence",
        "migrate",
        "--input",
        migration_path.to_str().unwrap(),
        "--apply",
    ]);
    assert_evidence_envelope(&applied, "evidence.migrate", true);
    assert_eq!(applied["object"]["status"], "applied");
    assert_eq!(applied["object"]["created"].as_array().unwrap().len(), 1);
    assert_eq!(
        applied["object"]["next_action"],
        format!("run planr evidence coverage --scope plan --id {plan_id}")
    );
    let binding_audit = run(&["plan", "audit", &plan_id]);
    let binding_clause = binding_audit["clauses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clause| clause["clause"] == "verification_logged")
        .unwrap();
    assert_eq!(binding_audit["holds"], false, "{binding_audit}");
    assert_eq!(binding_clause["authority"], "evidence_coverage");
    assert_eq!(binding_clause["log_authority"], "claim_only");
    assert_eq!(
        binding_clause["criteria"][0]["observations"][0]["gap_reason"],
        "missing_observation"
    );

    let reapplied = run(&[
        "evidence",
        "migrate",
        "--input",
        migration_path.to_str().unwrap(),
        "--apply",
    ]);
    assert_eq!(reapplied["object"]["summary"]["unchanged"], 1);
    assert!(
        reapplied["object"]["created"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let obligations_after_first_apply =
        run(&["evidence", "obligation", "list", "--plan", &plan_id]);
    let canonical = obligations_after_first_apply["object"]["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|obligation| obligation["id"] == "pob-migration-binding")
        .unwrap_or_else(|| panic!("{obligations_after_first_apply}"));
    assert_eq!(canonical["obligation_shape"], "semantic_v1");
    for runtime_field in ["policy_digest", "config_digest", "source_digest"] {
        assert!(
            canonical.get(runtime_field).is_none(),
            "semantic obligation leaked runtime field {runtime_field}: {canonical}"
        );
    }
    for runtime_field in ["environment", "runtime_target"] {
        assert!(
            canonical["observations"][0].get(runtime_field).is_none(),
            "semantic observation leaked runtime field {runtime_field}: {canonical}"
        );
    }

    let persisted_lineage_collision = |id: &str, title: &str| {
        let mut persisted_obligation = migration["obligations"][0].clone();
        persisted_obligation["id"] = json!(id);
        persisted_obligation["title"] = json!(title);
        json!({
            "schema_version": "planr.evidence.migration.v1",
            "plan_id": plan_id,
            "obligations": [persisted_obligation],
        })
    };
    let assert_persisted_lineage_preview = |preview: &Value, expected_id: &str| {
        assert_evidence_envelope(preview, "evidence.migrate", true);
        assert_eq!(preview["object"]["summary"]["create"], 0, "{preview}");
        assert_eq!(preview["object"]["summary"]["conflict"], 1, "{preview}");
        assert_eq!(preview["object"]["summary"]["blocked"], 0, "{preview}");
        assert_eq!(preview["object"]["obligations"][0]["id"], expected_id);
        assert_eq!(
            preview["object"]["obligations"][0]["action"], "conflict",
            "{preview}"
        );
        assert_eq!(
            preview["object"]["obligations"][0]["reason"], "lineage_identity_collision",
            "{preview}"
        );
        assert_eq!(
            preview["object"]["obligations"][0]["existing_id"], "pob-migration-binding",
            "{preview}"
        );
    };
    let assert_persisted_lineage_state = |candidate_id: &str| {
        let obligations = run(&["evidence", "obligation", "list", "--plan", &plan_id]);
        assert!(
            obligations["object"]["obligations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|obligation| obligation["id"] == "pob-migration-binding"),
            "{obligations}"
        );
        assert!(
            !obligations["object"]["obligations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|obligation| obligation["id"] == candidate_id),
            "{obligations}"
        );
    };

    let persisted_cli_collision =
        persisted_lineage_collision("pob-migration-lineage-cli", "lineage cli");
    let persisted_cli_collision_path = dir.path().join("migration-persisted-lineage-cli.json");
    fs::write(
        &persisted_cli_collision_path,
        serde_json::to_vec_pretty(&persisted_cli_collision).unwrap(),
    )
    .unwrap();
    let persisted_cli_preview = run(&[
        "evidence",
        "migrate",
        "--input",
        persisted_cli_collision_path.to_str().unwrap(),
    ]);
    assert_persisted_lineage_preview(&persisted_cli_preview, "pob-migration-lineage-cli");
    let persisted_cli_apply = run_failure(&[
        "evidence",
        "migrate",
        "--input",
        persisted_cli_collision_path.to_str().unwrap(),
        "--apply",
    ]);
    assert_evidence_error(
        &persisted_cli_apply,
        "evidence.migrate",
        "conflict",
        "evidence migration has 1 conflict(s)",
    );
    assert_persisted_lineage_state("pob-migration-lineage-cli");

    let persisted_mcp_collision =
        persisted_lineage_collision("pob-migration-lineage-mcp", "lineage mcp");
    let persisted_mcp_preview = mcp_tool(
        dir.path(),
        &db,
        49,
        "planr_evidence_migrate",
        json!({"input": persisted_mcp_collision.clone()}),
    );
    assert_persisted_lineage_preview(&persisted_mcp_preview, "pob-migration-lineage-mcp");
    let persisted_mcp_apply = mcp_tool_response(
        dir.path(),
        &db,
        50,
        "planr_evidence_migrate",
        json!({"input": persisted_mcp_collision.clone(), "apply": true}),
    );
    assert_mcp_evidence_error(
        &persisted_mcp_apply,
        "evidence.migrate",
        "conflict",
        "evidence migration has 1 conflict(s)",
    );
    assert_persisted_lineage_state("pob-migration-lineage-mcp");

    let persisted_http_collision =
        persisted_lineage_collision("pob-migration-lineage-http", "lineage http");
    let persisted_http_preview = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/migrate",
        &json!({"input": persisted_http_collision.clone()}).to_string(),
    ));
    assert_persisted_lineage_preview(&persisted_http_preview, "pob-migration-lineage-http");
    assert_http_evidence_error(
        &http_request(
            port,
            "POST",
            "/v1/evidence/migrate",
            &json!({"input": persisted_http_collision.clone(), "apply": true}).to_string(),
        ),
        "409 Conflict",
        "evidence.migrate",
        "conflict",
        "evidence migration has 1 conflict(s)",
    );
    assert_persisted_lineage_state("pob-migration-lineage-http");

    let mcp_migration = migration.clone();
    let mcp_preview = mcp_tool(
        dir.path(),
        &db,
        40,
        "planr_evidence_migrate",
        json!({"input": mcp_migration.clone()}),
    );
    assert_eq!(mcp_preview["object"]["dry_run"], true);
    assert_eq!(mcp_preview["object"]["summary"]["unchanged"], 1);
    let mcp_apply = mcp_tool(
        dir.path(),
        &db,
        41,
        "planr_evidence_migrate",
        json!({"input": mcp_migration.clone(), "apply": true}),
    );
    assert_eq!(mcp_apply["object"]["status"], "applied");
    assert_eq!(mcp_apply["object"]["summary"]["unchanged"], 1);
    let mcp_reapply = mcp_tool(
        dir.path(),
        &db,
        42,
        "planr_evidence_migrate",
        json!({"input": mcp_migration.clone(), "apply": true}),
    );
    assert_eq!(mcp_reapply["object"]["summary"]["unchanged"], 1);
    let http_migration = migration.clone();
    let http_preview = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/migrate",
        &json!({"input": http_migration.clone()}).to_string(),
    ));
    assert_eq!(http_preview["object"]["dry_run"], true);
    assert_eq!(http_preview["object"]["summary"]["unchanged"], 1);
    let http_apply = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/migrate",
        &json!({"input": http_migration.clone(), "apply": true}).to_string(),
    ));
    assert_eq!(http_apply["object"]["status"], "applied");
    assert_eq!(http_apply["object"]["summary"]["unchanged"], 1);
    let http_reapply = http_json(&http_request(
        port,
        "POST",
        "/v1/evidence/migrate",
        &json!({"input": http_migration.clone(), "apply": true}).to_string(),
    ));
    assert_eq!(http_reapply["object"]["summary"]["unchanged"], 1);
    let mut conflict = migration.clone();
    conflict["obligations"][0]["title"] = json!("Conflicting title");
    let conflict_path = dir.path().join("migration-conflict.json");
    fs::write(
        &conflict_path,
        serde_json::to_vec_pretty(&conflict).unwrap(),
    )
    .unwrap();
    let conflict_preview = run(&[
        "evidence",
        "migrate",
        "--input",
        conflict_path.to_str().unwrap(),
    ]);
    assert_eq!(conflict_preview["object"]["summary"]["conflict"], 1);
    let conflict_cli = run_failure(&[
        "evidence",
        "migrate",
        "--input",
        conflict_path.to_str().unwrap(),
        "--apply",
    ]);
    assert_evidence_error(
        &conflict_cli,
        "evidence.migrate",
        "conflict",
        "evidence migration has 1 conflict(s)",
    );
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
             VALUES ('p-foreign-migration', 'Foreign Migration', '.', 'active', datetime('now'), datetime('now'));
             INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
             VALUES ('pln-foreign-migration', 'p-foreign-migration', 'build', '/tmp/foreign-migration.plan.md', 'Foreign Migration', 'foreign-migration', 'ok', 'hash-foreign', datetime('now'), datetime('now'));
             INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
             VALUES ('i-foreign-migration', 'p-foreign-migration', 'Foreign Migration', 'Foreign migration item', 'closed', 'code', '/tmp/foreign-migration.plan.md', datetime('now'), datetime('now'));",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, policy_digest, config_digest, created_at
            ) VALUES (
              'pob-global-collision', 'p-foreign-migration', 'pln-foreign-migration', NULL,
              'crit-global-collision', 1, 'global collision', 1, '[]', '{}', '{}', '{}', ?1, ?2, datetime('now')
            )",
            rusqlite::params![
                policy["object"]["digest"].as_str().unwrap(),
                "sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"
            ],
        )
        .unwrap();
    }
    let mut global_collision = migration.clone();
    global_collision["obligations"][0]["id"] = json!("pob-global-collision");
    let global_collision_path = dir.path().join("migration-global-collision.json");
    fs::write(
        &global_collision_path,
        serde_json::to_vec_pretty(&global_collision).unwrap(),
    )
    .unwrap();
    let global_collision_preview = run(&[
        "evidence",
        "migrate",
        "--input",
        global_collision_path.to_str().unwrap(),
    ]);
    assert_eq!(
        global_collision_preview["object"]["obligations"][0]["reason"],
        "global_obligation_id_collision"
    );
    let global_collision_cli = run_failure(&[
        "evidence",
        "migrate",
        "--input",
        global_collision_path.to_str().unwrap(),
        "--apply",
    ]);
    assert_evidence_error(
        &global_collision_cli,
        "evidence.migrate",
        "conflict",
        "evidence migration has 1 conflict(s)",
    );
    let mcp_global_collision_response = mcp_tool_response(
        dir.path(),
        &db,
        49,
        "planr_evidence_migrate",
        json!({"input": global_collision.clone(), "apply": true}),
    );
    assert_mcp_evidence_error(
        &mcp_global_collision_response,
        "evidence.migrate",
        "conflict",
        "evidence migration has 1 conflict(s)",
    );
    assert_http_evidence_error(
        &http_request(
            port,
            "POST",
            "/v1/evidence/migrate",
            &json!({"input": global_collision.clone(), "apply": true}).to_string(),
        ),
        "409 Conflict",
        "evidence.migrate",
        "conflict",
        "evidence migration has 1 conflict(s)",
    );
    let mut cross_project_item = migration.clone();
    cross_project_item["obligations"][0]["id"] = json!("pob-cross-project-item");
    cross_project_item["obligations"][0]["item_id"] = json!("i-foreign-migration");
    let cross_project_item_path = dir.path().join("migration-cross-project-item.json");
    fs::write(
        &cross_project_item_path,
        serde_json::to_vec_pretty(&cross_project_item).unwrap(),
    )
    .unwrap();
    let cross_project_item_cli = run_failure(&[
        "evidence",
        "migrate",
        "--input",
        cross_project_item_path.to_str().unwrap(),
    ]);
    assert_evidence_error(
        &cross_project_item_cli,
        "evidence.migrate",
        "bad_request",
        "does not belong to project",
    );
    let mcp_cross_project_item_response = mcp_tool_response(
        dir.path(),
        &db,
        50,
        "planr_evidence_migrate",
        json!({"input": cross_project_item.clone()}),
    );
    assert_mcp_evidence_error(
        &mcp_cross_project_item_response,
        "evidence.migrate",
        "bad_request",
        "does not belong to project",
    );
    assert_http_evidence_error(
        &http_request(
            port,
            "POST",
            "/v1/evidence/migrate",
            &json!({"input": cross_project_item.clone()}).to_string(),
        ),
        "400 Bad Request",
        "evidence.migrate",
        "bad_request",
        "does not belong to project",
    );

    let classifications = run(&["evidence", "classifications"]);
    assert_evidence_envelope(&classifications, "evidence.classifications", true);
    let canonical = classifications["object"]["canonical_gap_reasons"]
        .as_array()
        .unwrap();
    for reason in [
        "product_failed",
        "verifier_failed",
        "missing_capability",
        "permission_denied",
        "sandbox_blocked",
        "environment_unavailable",
        "external_dependency_unavailable",
        "target_mismatch",
        "stale_source",
        "untrusted_provenance",
    ] {
        assert!(
            canonical.iter().any(|value| value.as_str() == Some(reason)),
            "{reason} missing from canonical classifications: {classifications}"
        );
    }
    assert!(
        classifications["object"]["legacy_aliases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|alias| alias["alias"] == "capability_unavailable"
                && alias["canonical"] == "missing_capability")
    );
    assert_eq!(
        classifications["object"]["unknown_legacy_reason"]["canonical"],
        "verifier_failed"
    );
    let host_surfaces = classifications["object"]["host_adapters"]["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|surface| surface["surface"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    for surface in [
        "terminal",
        "local-function",
        "codex-app-server",
        "codex-exec",
        "codex-mcp-server",
        "mcp-browser",
        "built-in-browser",
        "chrome-cdp",
        "codex-hook-events",
        "computer-use",
        "claude-code",
        "cursor-agent",
        "pi-cli",
    ] {
        assert!(
            host_surfaces.contains(surface),
            "{surface} missing from host adapter classifications"
        );
    }
    let host_entries = classifications["object"]["host_adapters"]["surfaces"]
        .as_array()
        .unwrap();
    assert!(
        classifications["object"]["host_adapters"]
            .get("enabled_surfaces")
            .is_none()
            && classifications["object"]["host_adapters"]
                .get("disabled_surfaces")
                .is_none(),
        "host adapter DTO must not expose a second enabled/disabled status vocabulary"
    );
    let canonical_host_entry_keys = BTreeSet::from([
        "artifact_kinds",
        "availability_status",
        "blind_spots",
        "host",
        "host_version",
        "observation_types",
        "permissions",
        "provenance",
        "reason",
        "surface",
        "trusted_adapter_enabled",
    ]);
    for entry in host_entries {
        let actual_keys = entry
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual_keys, canonical_host_entry_keys, "{entry}");
    }
    for (host, surface, availability_status) in [
        ("claude", "claude-code", "unavailable"),
        ("cursor", "cursor-agent", "unavailable"),
        ("pi", "pi-cli", "unavailable"),
    ] {
        let entry = host_entries
            .iter()
            .find(|entry| entry["host"] == host && entry["surface"] == surface)
            .unwrap_or_else(|| panic!("{host}/{surface} missing from {classifications}"));
        assert_eq!(entry["trusted_adapter_enabled"], false, "{entry}");
        assert!(entry.get("status").is_none(), "{entry}");
        assert_eq!(entry["availability_status"], availability_status, "{entry}");
        assert_eq!(
            entry["provenance"]["claim_source"], "mechanical_unavailable_probe",
            "{entry}"
        );
        assert_eq!(
            entry["provenance"]["observation_mode"], "mechanical_invocation",
            "{entry}"
        );
        assert!(
            entry["observation_types"]
                .as_array()
                .unwrap()
                .iter()
                .any(|observation| observation.as_str().unwrap().starts_with("host.")),
            "{entry}"
        );
        assert!(
            entry["artifact_kinds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|kind| kind == "invocation-stdout"),
            "{entry}"
        );
        assert!(
            entry["reason"]
                .as_str()
                .unwrap()
                .contains("non-Codex host capture cannot enable a Codex adapter"),
            "{entry}"
        );
    }
    let mcp_classifications = mcp_tool(
        dir.path(),
        &db,
        43,
        "planr_evidence_classifications",
        json!({}),
    );
    assert_eq!(mcp_classifications["object"], classifications["object"]);

    let http_classifications = http_json(&http_request(
        port,
        "GET",
        "/v1/evidence/classifications",
        "",
    ));
    assert_eq!(http_classifications["object"], classifications["object"]);
    server.kill().unwrap();
    server.wait().unwrap();
}

#[test]
fn complete_binding_plan_criteria_contract_rejects_invalid_identity_sets() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    init_evidence_project(dir.path(), &db, "Complete Binding Plan Criteria");
    let run = |args: &[&str]| -> Value {
        single_json_document(
            &planr()
                .current_dir(dir.path())
                .args(["--db", &db_arg, "--json"])
                .args(args)
                .assert()
                .success()
                .get_output()
                .stdout,
        )
    };
    let product = run(&["plan", "new", "Criteria Product"]);
    let build = run(&[
        "plan",
        "split",
        product["plan"]["id"].as_str().unwrap(),
        "--slice",
        "Closed Criteria",
    ]);
    let plan_id = build["plan"]["id"].as_str().unwrap();
    let build_path = build["plan"]["path"].as_str().unwrap();
    let scaffold = fs::read_to_string(build_path).unwrap();
    let criteria_start = scaffold.find("criteria:\n").unwrap();
    let body = "---\n\n# Closed Criteria\n\n## Scope Decision\n\nUse typed criteria.\n\n## Verification\n\nCheck the closed contract.\n\n## Acceptance Criteria\n\nNarrative only.\n\n## Steps\n\n### TASK-001: Verify criteria\n\nRun the focused check.\n";
    let write_criteria = |yaml: &str| {
        fs::write(
            build_path,
            format!("{}{yaml}{body}", &scaffold[..criteria_start]),
        )
        .unwrap();
    };

    for (yaml, expected) in [
        ("", "must be a non-empty list"),
        ("criteria: []\n", "must not be empty"),
        (
            "criteria:\n  - id: duplicate\n    title: First\n  - id: duplicate\n    title: Second\n",
            "is duplicated",
        ),
        (
            "criteria:\n  - id: criterion-unknown\n    title: Unknown field\n    legacy_key: forbidden\n",
            "unknown field",
        ),
        (
            "criteria:\n  - id: criterion-blank-title\n    title: '   '\n",
            "title` must not be empty",
        ),
    ] {
        write_criteria(yaml);
        let checked = run(&["plan", "check", plan_id]);
        assert_eq!(checked["ok"], false, "{checked}");
        assert!(
            checked["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|warning| warning["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(expected))),
            "expected {expected}: {checked}"
        );
    }

    write_criteria("criteria:\n  - id: criterion-valid\n    title: Valid criterion\n");
    let checked = run(&["plan", "check", plan_id]);
    assert_eq!(checked["ok"], true, "{checked}");
    assert_eq!(checked["criteria"][0]["id"], "criterion-valid");
}

#[test]
fn complete_binding_authority_requires_the_exact_declared_criterion_set() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    write_evidence_policy_fixture(dir.path());
    init_git_repo(dir.path());
    init_evidence_project(dir.path(), &db, "Complete Binding Authority");
    let run = |args: &[&str]| -> Value {
        single_json_document(
            &planr()
                .current_dir(dir.path())
                .args(["--db", &db_arg, "--json"])
                .args(args)
                .assert()
                .success()
                .get_output()
                .stdout,
        )
    };
    let run_failure = |args: &[&str]| -> Value {
        single_json_document(
            &planr()
                .current_dir(dir.path())
                .args(["--db", &db_arg, "--json"])
                .args(args)
                .assert()
                .failure()
                .get_output()
                .stdout,
        )
    };

    let product = run(&["plan", "new", "Authority Product"]);
    let product_id = product["plan"]["id"].as_str().unwrap();
    let build = run(&[
        "plan",
        "split",
        product_id,
        "--slice",
        "Complete Binding Authority Matrix",
    ]);
    let plan_id = build["plan"]["id"].as_str().unwrap().to_string();
    let build_path = build["plan"]["path"].as_str().unwrap();
    let build_text = fs::read_to_string(build_path).unwrap();
    let criteria_start = build_text.find("criteria:\n").unwrap();
    let build_text = format!(
        "{}criteria:\n  - id: criterion-authority-first\n    title: First authority criterion\n  - id: criterion-authority-second\n    title: Second authority criterion\n---\n\n# Complete Binding Authority Matrix\n\n## Scope Decision\n\nVerify exact criterion binding.\n\n## Ownership Target\n\nApplication proof owns completeness.\n\n## Existing Leverage\n\nUse the migration boundary.\n\n## Phase 1\n\n- [ ] Verify the first criterion.\n- [ ] Verify the second criterion.\n\n## Out Of Scope\n\nNo compatibility path.\n\n## Verification\n\nRun the focused no-model contract.\n\n## Acceptance Criteria\n\nBoth typed criteria bind exactly.\n",
        &build_text[..criteria_start],
    );
    fs::write(build_path, build_text).unwrap();
    let checked = run(&["plan", "check", &plan_id]);
    assert!(checked["ok"].as_bool().unwrap_or(false), "{checked}");
    let map = run(&["map", "build", "--from", &plan_id]);
    let item_id = map["created"][0]["id"].as_str().unwrap().to_string();
    let zero = run(&["trace", "item", &item_id]);
    assert_eq!(zero["proof"]["status"], "binding_unsatisfied", "{zero}");
    let zero_readiness =
        run_failure(&["evidence", "readiness", "--scope", "plan", "--id", &plan_id]);
    assert_eq!(
        zero_readiness["object"]["status"], "blocked",
        "{zero_readiness}"
    );
    assert_eq!(
        zero_readiness["object"]["proof"]["status"], "binding_unsatisfied",
        "{zero_readiness}"
    );
    let zero_coverage = run_failure(&["evidence", "coverage", "--scope", "plan", "--id", &plan_id]);
    assert_eq!(
        zero_coverage["object"]["authority"], "binding_unsatisfied",
        "{zero_coverage}"
    );

    let policy = run(&["evidence", "policy"]);
    let environment = capability_instance_environment(
        &db,
        policy["object"]["registry"]["probes"][0]["instance_id"]
            .as_str()
            .unwrap(),
    );
    let obligation = |id: &str, criterion_id: &str| {
        let mut value = evidence_obligation_for(
            id,
            policy["object"]["digest"].as_str().unwrap(),
            "com.example.health.status",
            id,
            json!({"status": "ok"}),
            json!({"kind": "process", "uri": "local://health"}),
            environment.clone(),
            json!({"kind": "process", "id": "runtime-local"}),
            json!([]),
            "sha256:abababababababababababababababababababababababababababababababab",
        );
        value["plan_id"] = json!(plan_id);
        value["item_id"] = Value::Null;
        value["criterion_id"] = json!(criterion_id);
        value
    };
    let migration = |obligations: Vec<Value>| {
        json!({
            "schema_version": "planr.evidence.migration.v1",
            "plan_id": plan_id,
            "obligations": obligations,
        })
    };
    let write_migration = |name: &str, value: &Value| {
        let path = dir.path().join(name);
        fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
        path
    };

    let partial = migration(vec![obligation(
        "pob-authority-partial",
        "criterion-authority-first",
    )]);
    let partial_path = write_migration("partial.json", &partial);
    let partial_result = run_failure(&[
        "evidence",
        "migrate",
        "--input",
        partial_path.to_str().unwrap(),
        "--apply",
    ]);
    assert!(
        partial_result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing_obligation:criterion-authority-second"),
        "{partial_result}"
    );

    let duplicate = migration(vec![
        obligation("pob-authority-duplicate-a", "criterion-authority-first"),
        obligation("pob-authority-duplicate-b", "criterion-authority-first"),
    ]);
    let duplicate_path = write_migration("duplicate.json", &duplicate);
    let duplicate_result = run_failure(&[
        "evidence",
        "migrate",
        "--input",
        duplicate_path.to_str().unwrap(),
        "--apply",
    ]);
    assert!(
        duplicate_result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("duplicate_criterion_binding:criterion-authority-first"),
        "{duplicate_result}"
    );

    let foreign = migration(vec![
        obligation("pob-authority-foreign-a", "criterion-authority-first"),
        obligation("pob-authority-foreign-b", "criterion-authority-foreign"),
    ]);
    let foreign_path = write_migration("foreign.json", &foreign);
    let foreign_result = run_failure(&[
        "evidence",
        "migrate",
        "--input",
        foreign_path.to_str().unwrap(),
        "--apply",
    ]);
    assert!(
        foreign_result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("undeclared_criterion_binding:criterion-authority-foreign"),
        "{foreign_result}"
    );

    let before_exact = run(&["evidence", "obligation", "list", "--plan", &plan_id]);
    assert!(
        before_exact["object"]["obligations"]
            .as_array()
            .unwrap()
            .is_empty(),
        "invalid migrations must be atomic: {before_exact}"
    );

    let exact = migration(vec![
        obligation("pob-authority-first", "criterion-authority-first"),
        obligation("pob-authority-second", "criterion-authority-second"),
    ]);
    let exact_path = write_migration("exact.json", &exact);
    let applied = run(&[
        "evidence",
        "migrate",
        "--input",
        exact_path.to_str().unwrap(),
        "--apply",
    ]);
    assert_eq!(applied["object"]["status"], "applied", "{applied}");
    assert_eq!(
        applied["object"]["created"].as_array().unwrap().len(),
        2,
        "{applied}"
    );
    let audit = run(&["plan", "audit", &plan_id]);
    assert_eq!(audit["proof"]["active_binding"], true, "{audit}");
    assert_eq!(audit["proof"]["status"], "not_proven", "{audit}");
}

fn complete_binding_single_owner_inventory_keeps_adapter_at_boundary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let production_obligation_writers = complete_binding_production_rust_files(&root.join("src"))
        .into_iter()
        .filter_map(|file| {
            let source = fs::read_to_string(&file).unwrap();
            let production = source.split("#[cfg(test)]").next().unwrap_or_default();
            production
                .contains("INSERT INTO proof_obligations")
                .then(|| {
                    file.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .to_string()
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        production_obligation_writers,
        vec!["src/app/evidence.rs"],
        "production ProofObligation writers drifted"
    );

    let migration = fs::read_to_string(root.join("src/app/evidence.rs")).unwrap();
    assert!(
        migration.contains("fn insert_migrated_evidence_obligation"),
        "the sole production obligation writer must be the migration boundary"
    );

    let proof = fs::read_to_string(root.join("src/app/proof.rs")).unwrap();
    assert!(
        proof.contains("authoritative_plan_obligation_binding_identities("),
        "app/proof must consume the typed Evidence identity loader"
    );
    assert!(
        !proof.contains("proof_obligations") && !proof.contains("rusqlite::params"),
        "app/proof must not query proof_obligations directly"
    );

    let coverage = fs::read_to_string(root.join("src/evidence/coverage.rs")).unwrap();
    assert!(
        coverage.contains("pub struct AuthoritativeObligationBindingIdentity")
            && coverage.contains("pub fn authoritative_plan_obligation_binding_identities(")
            && !coverage.contains("pub fn authoritative_plan_obligation_bindings("),
        "Evidence coverage must expose only the identity authority loader for proof completeness"
    );

    for relative in [
        "src/app/audit_evidence.rs",
        "src/app/final_review_admission.rs",
        "src/app/execution_state.rs",
        "src/app/stop.rs",
    ] {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        assert!(
            ![
                "build_plan_criteria",
                "parse_plan_metadata",
                "proof_obligations"
            ]
            .iter()
            .any(|needle| source.contains(needle)),
            "{relative} must not recompute completeness"
        );
        assert!(
            source.contains("plan_evidence_authority") || source.contains("proof_status_for_plan"),
            "{relative} must consume app/proof authority"
        );
    }

    let plan_skill =
        fs::read_to_string(root.join("plugins/planr/skills/planr-plan/SKILL.md")).unwrap();
    let goal_skill =
        fs::read_to_string(root.join("plugins/planr/skills/planr-goal/SKILL.md")).unwrap();
    assert!(
        plan_skill.contains("readable narrative, never an identity source")
            && plan_skill.contains("Do not infer criterion IDs from prose")
            && goal_skill.contains("Never write obligations directly")
            && goal_skill.contains("duplicate `app/proof` completeness rules"),
        "Planr skills must delegate identity and completeness to canonical owners"
    );

    let manifest: Value = serde_json::from_str(&fs::read_to_string(
        root.join(".planr/evidence/adapters/verifier-complete-binding-authority-v1.manifest.json"),
    )
    .unwrap())
    .unwrap();
    let policy: Value =
        serde_json::from_str(&fs::read_to_string(root.join(".planr/evidence.yaml")).unwrap())
            .unwrap();
    let registration = policy["adapter_registrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|registration| registration["manifest_id"] == "verifier-complete-binding-authority-v1")
        .unwrap();
    let obsolete_adapter = ["scripts/verify-complete-binding-authority", ".mjs"].concat();
    for current_contract in [
        &manifest["availability_probe"]["execution"],
        &registration["execution_contract"],
    ] {
        assert_eq!(current_contract["executable"], "rustup");
        let args = current_contract["args"].as_array().unwrap();
        assert!(args.iter().any(|arg| arg == "cargo"));
        assert!(args
            .iter()
            .any(|arg| arg == "planr-complete-binding-authority"));
        assert!(!current_contract.to_string().contains(&obsolete_adapter));
    }
    assert!(
        !root.join(obsolete_adapter).exists(),
        "obsolete JavaScript adapter must be deleted"
    );
}

fn complete_binding_production_rust_files(root: &Path) -> Vec<PathBuf> {
    let mut entries = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    let mut files = Vec::new();
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            files.extend(complete_binding_production_rust_files(&path));
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    files
}

#[test]
fn complete_binding_lifecycle_fails_closed_for_partial_active_rows() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    write_evidence_policy_fixture(dir.path());
    init_git_repo(dir.path());
    init_evidence_project(dir.path(), &db, "Complete Binding Lifecycle");
    let run = |args: &[&str]| -> Value {
        single_json_document(
            &planr()
                .current_dir(dir.path())
                .env("PLANR_WORKER_ID", "maker-complete-binding-lifecycle")
                .args(["--db", &db_arg, "--json"])
                .args(args)
                .assert()
                .success()
                .get_output()
                .stdout,
        )
    };
    let run_failure = |args: &[&str]| -> Value {
        single_json_document(
            &planr()
                .current_dir(dir.path())
                .env("PLANR_WORKER_ID", "maker-complete-binding-lifecycle")
                .args(["--db", &db_arg, "--json"])
                .args(args)
                .assert()
                .failure()
                .get_output()
                .stdout,
        )
    };

    let product = run(&["plan", "new", "Lifecycle Product"]);
    let build = run(&[
        "plan",
        "split",
        product["plan"]["id"].as_str().unwrap(),
        "--slice",
        "Lifecycle Boundaries",
    ]);
    let plan_id = build["plan"]["id"].as_str().unwrap().to_string();
    let build_path = build["plan"]["path"].as_str().unwrap();
    let scaffold = fs::read_to_string(build_path).unwrap();
    let criteria_start = scaffold.find("criteria:\n").unwrap();
    fs::write(
        build_path,
        format!(
            "{}criteria:\n  - id: criterion-lifecycle-first\n    title: First lifecycle criterion\n  - id: criterion-lifecycle-second\n    title: Second lifecycle criterion\n---\n\n# Lifecycle Boundaries\n\n## Scope Decision\n\nFail closed.\n\n## Verification\n\nCheck lifecycle boundaries.\n\n## Acceptance Criteria\n\nComplete bindings only.\n\n## Steps\n\n### TASK-001: Verify lifecycle\n\nRun the focused check.\n",
            &scaffold[..criteria_start]
        ),
    )
    .unwrap();
    assert_eq!(run(&["plan", "check", &plan_id])["ok"], true);
    let map = run(&["map", "build", "--from", &plan_id]);
    let item_id = map["created"][0]["id"].as_str().unwrap().to_string();

    let policy = run(&["evidence", "policy"]);
    let environment = capability_instance_environment(
        &db,
        policy["object"]["registry"]["probes"][0]["instance_id"]
            .as_str()
            .unwrap(),
    );
    let obligation = |id: &str, criterion_id: &str| {
        let mut value = evidence_obligation_for(
            id,
            policy["object"]["digest"].as_str().unwrap(),
            "com.example.health.status",
            id,
            json!({"status": "ok"}),
            json!({"kind": "process", "uri": "local://health"}),
            environment.clone(),
            json!({"kind": "process", "id": "runtime-local"}),
            json!([]),
            "sha256:abababababababababababababababababababababababababababababababab",
        );
        value["plan_id"] = json!(plan_id);
        value["item_id"] = Value::Null;
        value["criterion_id"] = json!(criterion_id);
        value
    };
    let migration = json!({
        "schema_version": "planr.evidence.migration.v1",
        "plan_id": plan_id,
        "obligations": [
            obligation("pob-lifecycle-first", "criterion-lifecycle-first"),
            obligation("pob-lifecycle-second", "criterion-lifecycle-second"),
        ],
    });
    let migration_path = dir.path().join("lifecycle-migration.json");
    fs::write(
        &migration_path,
        serde_json::to_vec_pretty(&migration).unwrap(),
    )
    .unwrap();
    let applied = run(&[
        "evidence",
        "migrate",
        "--input",
        migration_path.to_str().unwrap(),
        "--apply",
    ]);
    assert_eq!(applied["object"]["created"].as_array().unwrap().len(), 2);
    let picked = run(&["pick", "--plan", &plan_id, "--work-type", "code"]);
    assert_eq!(picked["work_packet"]["item_id"], item_id, "{picked}");

    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "INSERT INTO proof_obligations(
               id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
               binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
               assurance_policy_json, policy_digest, config_digest, source_digest,
               supersedes_obligation_id, created_at, retry_aggregation, obligation_shape
             )
             SELECT 'pob-lifecycle-second-advisory', project_id, plan_id, item_id, criterion_id, 2,
                    'test-only corrupt advisory successor', 0, observation_requirements_json,
                    fixture_policy_json, freshness_policy_json, assurance_policy_json,
                    policy_digest, config_digest, source_digest, id, datetime('now'),
                    retry_aggregation, obligation_shape
             FROM proof_obligations WHERE id = 'pob-lifecycle-second';",
        )
        .unwrap();
    let partial_trace = run(&["trace", "item", &item_id]);
    assert_eq!(
        partial_trace["proof"]["status"], "binding_unsatisfied",
        "{partial_trace}"
    );
    assert!(
        partial_trace["proof"]["actionable_gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["code"] == "missing_obligation"
                && gap["scope"]["id"] == "criterion-lifecycle-second")
    );
    let partial_readiness =
        run_failure(&["evidence", "readiness", "--scope", "plan", "--id", &plan_id]);
    assert_eq!(
        partial_readiness["object"]["status"], "blocked",
        "{partial_readiness}"
    );
    let partial_coverage =
        run_failure(&["evidence", "coverage", "--scope", "plan", "--id", &plan_id]);
    assert_eq!(
        partial_coverage["object"]["authority"], "binding_unsatisfied",
        "{partial_coverage}"
    );
    let partial_review = run_failure(&["plan", "final-review", &plan_id]);
    assert!(
        partial_review["error"]["message"]
            .as_str()
            .unwrap()
            .contains("final_product_review_binding_evidence_obligations_missing"),
        "{partial_review}"
    );

    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "INSERT INTO proof_obligations(
               id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
               binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
               assurance_policy_json, policy_digest, config_digest, source_digest,
               supersedes_obligation_id, created_at, retry_aggregation, obligation_shape
             )
             SELECT 'pob-lifecycle-second-restored', project_id, plan_id, item_id, criterion_id, 3,
                    'test-only restored binding successor', 1, observation_requirements_json,
                    fixture_policy_json, freshness_policy_json, assurance_policy_json,
                    policy_digest, config_digest, source_digest, id, datetime('now'),
                    retry_aggregation, obligation_shape
             FROM proof_obligations WHERE id = 'pob-lifecycle-second-advisory';",
        )
        .unwrap();
    let complete_trace = run(&["trace", "item", &item_id]);
    assert_ne!(
        complete_trace["proof"]["status"], "binding_unsatisfied",
        "{complete_trace}"
    );
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
    let plan_path = dir.path().join("peek.plan.md");
    fs::write(&plan_path, "# Peek\n").unwrap();
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('plan-peek-read-only', ?1, 'build', ?2, 'Peek', 'peek', 'ok', 'sha256:peek', datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    conn.execute(
        "UPDATE items SET plan_path = ?1 WHERE id = ?2",
        rusqlite::params![plan_path.to_string_lossy(), item],
    )
    .unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "item", "update", &item, "--work-type", "code"])
        .assert()
        .success();
    let events_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    drop(conn);

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
    assert_eq!(peek["work_packet"]["kind"], "outcome");
    assert!(
        peek["work_packet"].get("execution_state").is_none(),
        "peek may project a planned outcome without creating execution state"
    );

    // No execution state, lease, budget, replacement provenance, or event was
    // written by the read-only projection.
    let conn = Connection::open(&db).unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM feature_runs"), 0);
    assert_eq!(count("SELECT COUNT(*) FROM execution_batches"), 0);
    assert_eq!(count("SELECT COUNT(*) FROM feature_run_role_leases"), 0);
    assert_eq!(
        count("SELECT COUNT(*) FROM feature_run_budget_reservations"),
        0
    );
    assert_eq!(
        count(
            "SELECT COUNT(*) FROM execution_batches WHERE replacement_reason IS NOT NULL OR replaced_maker_worker_id IS NOT NULL OR successor_maker_worker_id IS NOT NULL"
        ),
        0
    );
    assert_eq!(count("SELECT COUNT(*) FROM events"), events_before);
    drop(conn);

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
    assert_eq!(pick["work_packet"]["kind"], "outcome");
    assert!(pick["work_packet"]["execution_state"].is_object());

    let conn = Connection::open(&db).unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM feature_runs"), 1);
    assert_eq!(count("SELECT COUNT(*) FROM execution_batches"), 1);
    assert_eq!(count("SELECT COUNT(*) FROM feature_run_role_leases"), 1);
    assert_eq!(
        count("SELECT COUNT(*) FROM feature_run_budget_reservations"),
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM events WHERE event_type = 'feature_run_started'"),
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM events WHERE event_type = 'item_picked'"),
        1
    );
}

#[test]
fn incompatible_feature_run_restart_is_atomic_and_creates_fresh_v2_run_on_next_pick() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("restart-template.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Restart hard cut"])
        .assert()
        .success();
    let plan = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                &db_arg,
                "--json",
                "plan",
                "new",
                "Restart hard cut plan",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let plan_id = plan["plan"]["id"].as_str().unwrap().to_string();
    let plan_path = plan["plan"]["path"].as_str().unwrap().to_string();
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    for ordinal in 1..=2 {
        conn.execute(
            "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'restart proof', 'ready', 'code', ?4, datetime('now'), datetime('now'))",
            rusqlite::params![
                format!("restart-item-{ordinal}"),
                project_id,
                format!("Restart item {ordinal}"),
                plan_path,
            ],
        )
        .unwrap();
    }
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    conn.execute(
        "INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, active_batch_id, outcomes_settled, batch_outcome_count)
         VALUES ('run-incompatible-old', ?1, ?2, 'active', 'implementation', 'sha256:legacy-policy', 'batch-incompatible-old', 1, 1)",
        rusqlite::params![project_id, plan_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO execution_batches(id, run_id, maker_worker_id, status) VALUES ('batch-incompatible-old', 'run-incompatible-old', 'legacy-maker', 'active')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation) VALUES ('run-incompatible-old', 'maker', 'legacy-maker', 1)",
        [],
    )
    .unwrap();
    conn.execute_batch("COMMIT").unwrap();
    conn.execute(
        "INSERT INTO execution_run_outcomes(id, run_id, batch_id, item_id, ordinal, outcome_json) VALUES ('outcome-old', 'run-incompatible-old', 'batch-incompatible-old', 'historical-item', 1, '{}')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO review_gates(id, run_id, scope_kind, scope_id, kind, status, responsible_maker_id, latest_attempt) VALUES ('gate-old', 'run-incompatible-old', 'outcome', 'historical-item', 'risk_checkpoint', 'accepted', 'legacy-maker', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO review_attempts(id, gate_id, attempt_number, reviewer_worker_id, reviewer_mode, verdict, source_revision, artifacts_json) VALUES ('attempt-old', 'gate-old', 1, 'legacy-reviewer', 'independent', 'accepted', 'sha256:old-source', '[]')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO review_findings(id, run_id, gate_id, attempt_id, severity, target, owner_worker_id, status, invalidated_evidence_ids_json) VALUES ('finding-old', 'run-incompatible-old', 'gate-old', 'attempt-old', 'moderate', 'src/old.rs', 'legacy-maker', 'resolved', '[]')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_budget_reservations(id, run_id, phase, boundary_key, status, started_at_unix_ms, provenance) VALUES ('reservation-old', 'run-incompatible-old', 'implementation', 'implementation:historical-item', 'active', 1700000000000, 'legacy.history')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_budget_observations(id, run_id, phase, metering, wall_seconds, provenance) VALUES ('observation-old', 'run-incompatible-old', 'implementation', 'trusted', 9, 'legacy.history')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO logs(id, project_id, item_id, run_id, kind, summary, created_at) VALUES ('log-old', ?1, 'historical-item', 'run-incompatible-old', 'completion', 'historical log', datetime('now'))",
        [&project_id],
    )
    .unwrap();
    let history_tables = [
        "execution_run_outcomes",
        "review_gates",
        "review_attempts",
        "review_findings",
        "feature_run_budget_reservations",
        "feature_run_budget_observations",
        "logs",
    ];
    let history_before = history_tables
        .iter()
        .map(|table| {
            (
                *table,
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    drop(conn);

    let peek = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                &db_arg,
                "--json",
                "pick",
                "--peek",
                "--work-type",
                "code",
                "--plan",
                &plan_id,
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let pick_hold = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "restart-maker")
            .args([
                "--db",
                &db_arg,
                "--json",
                "pick",
                "--work-type",
                "code",
                "--plan",
                &plan_id,
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(peek["work_packet"], pick_hold["work_packet"]);
    assert_eq!(peek["work_packet"]["kind"], "hold");
    assert_eq!(
        peek["work_packet"]["classification"],
        "incompatible_feature_run_budget_contract"
    );
    assert_eq!(
        peek["work_packet"]["reason_code"],
        "feature_run_budget_contract_missing"
    );
    assert_eq!(
        peek["work_packet"]["next_action"],
        "restart_incompatible_feature_run"
    );
    assert_eq!(
        peek["work_packet"]["execution_state"]["schema_version"],
        "planr.execution_state.v2"
    );
    assert_eq!(
        peek["work_packet"]["execution_state"]["restart"]["status"],
        "required"
    );
    assert_eq!(
        peek["work_packet"]["execution_state"]["restart"]["reason"],
        "incompatible-budget"
    );
    assert_eq!(pick_hold["item"]["status"], "ready");
    assert!(pick_hold["item"]["worker_id"].is_null());
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM events WHERE event_type = 'item_picked' AND item_id LIKE 'restart-item-%'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM feature_run_budget_contracts",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(conn);

    let cli_db = dir.path().join("restart-cli.sqlite");
    let mcp_db = dir.path().join("restart-mcp.sqlite");
    let http_db = dir.path().join("restart-http.sqlite");
    fs::copy(&db, &cli_db).unwrap();
    fs::copy(&db, &mcp_db).unwrap();
    fs::copy(&db, &http_db).unwrap();

    let cli_restart = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                cli_db.to_str().unwrap(),
                "--json",
                "run",
                "restart",
                "--plan",
                &plan_id,
                "--reason",
                "incompatible-budget",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let mcp_output = planr()
        .current_dir(dir.path())
        .args(["--db", mcp_db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_run_restart","arguments":{"plan":plan_id,"reason":"incompatible-budget"}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_envelope: Value = serde_json::from_slice(&mcp_output).unwrap();
    let mcp_restart: Value = serde_json::from_str(
        mcp_envelope["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let port = free_port();
    let bin = assert_cmd::cargo::cargo_bin("planr");
    let mut server = StdCommand::new(&bin)
        .current_dir(dir.path())
        .args([
            "--db",
            http_db.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_http_server(port);
    let http_restart = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/plans/{plan_id}/run/restart"),
        r#"{"reason":"incompatible-budget"}"#,
    ));
    server.kill().unwrap();
    server.wait().unwrap();
    assert_eq!(cli_restart, mcp_restart);
    assert_eq!(cli_restart, http_restart);
    assert_eq!(cli_restart["restart"]["disposition"], "retired");
    assert_eq!(
        cli_restart["restart"]["retired_run"]["id"],
        "run-incompatible-old"
    );
    assert_eq!(
        cli_restart["execution_state"]["restart"]["status"],
        "retired"
    );

    let conn = Connection::open(&cli_db).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM feature_run_budget_contracts",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "restart must not create a contract"
    );
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM feature_runs WHERE id = 'run-incompatible-old' AND status = 'cancelled' AND phase = 'cancelled' AND terminal_reason = 'policy_cancelled'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM execution_batches WHERE id = 'batch-incompatible-old' AND status = 'ended'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM feature_run_role_leases WHERE run_id = 'run-incompatible-old' AND released_at IS NULL", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    for (table, expected) in &history_before {
        assert_eq!(
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            *expected,
            "{table} history preserved"
        );
    }
    drop(conn);

    let repeated = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                cli_db.to_str().unwrap(),
                "--json",
                "run",
                "restart",
                "--plan",
                &plan_id,
                "--reason",
                "incompatible-budget",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(repeated["restart"]["disposition"], "already_retired");
    let conn = Connection::open(&cli_db).unwrap();
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM events WHERE event_type = 'feature_run_incompatible_budget_retired'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    drop(conn);

    let next_pick = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "restart-maker")
            .args([
                "--db",
                cli_db.to_str().unwrap(),
                "--json",
                "pick",
                "--work-type",
                "code",
                "--plan",
                &plan_id,
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(next_pick["work_packet"]["kind"], "outcome");
    let new_run_id = next_pick["work_packet"]["execution_state"]["feature_run"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(new_run_id, "run-incompatible-old");
    assert_eq!(
        next_pick["work_packet"]["execution_state"]["budget"]["status"],
        "available"
    );
    assert!(next_pick["work_packet"]["execution_state"]["restart"].is_null());
    let conn = Connection::open(&cli_db).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM feature_runs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM feature_run_budget_contracts",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM feature_run_budget_contracts WHERE run_id = ?1 AND schema = 'planr.feature_run_budget_contract.v2'", [&new_run_id], |row| row.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM feature_run_budget_contracts WHERE run_id = 'run-incompatible-old'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    let retired_event: i64 = conn
        .query_row(
            "SELECT id FROM events WHERE event_type = 'feature_run_incompatible_budget_retired'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let started_event: i64 = conn.query_row("SELECT id FROM events WHERE event_type = 'feature_run_started' AND json_extract(payload, '$.run_id') = ?1", [&new_run_id], |row| row.get(0)).unwrap();
    assert!(
        retired_event < started_event,
        "hold/restart must precede canonical successor creation"
    );
    drop(conn);

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            cli_db.to_str().unwrap(),
            "run",
            "restart",
            "--plan",
            &plan_id,
            "--reason",
            "incompatible-budget",
        ])
        .assert()
        .failure();
    let conn = Connection::open(&cli_db).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM feature_runs WHERE id = ?1 AND status = 'active'",
            [&new_run_id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "healthy successor remains active"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM feature_run_budget_contracts",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn compatible_budget_hold_resolution_is_typed_concurrent_and_transport_owned() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("budget-hold-template.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args(["--db", &db_arg, "project", "init", "Budget hold resolution"])
        .assert()
        .success();
    let plan = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                &db_arg,
                "--json",
                "plan",
                "new",
                "Budget hold resolution plan",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let plan_id = plan["plan"]["id"].as_str().unwrap().to_string();
    let plan_path = plan["plan"]["path"].as_str().unwrap().to_string();
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let contract = test_unbounded_feature_run_contract("run-budget-held-v2");
    conn.execute(
        "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at)
         VALUES ('item-budget-held-v2', ?1, 'Held work', 'resume without database repair', 'picked', 'code', 'hold-owner', ?2, datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, budget_contract_digest, active_batch_id, held_from_phase, hold_reason)
         VALUES ('run-budget-held-v2', ?1, ?2, 'held', 'held', 'sha256:policy', ?3, NULL, 'implementation', 'budget')",
        rusqlite::params![project_id, plan_id, contract.digest],
    )
    .unwrap();
    insert_test_feature_run_contract(&conn, &contract);
    conn.execute(
        "INSERT INTO execution_batches(id, run_id, maker_worker_id, status) VALUES ('batch-budget-held-v2', 'run-budget-held-v2', 'hold-owner', 'active')",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE feature_runs SET active_batch_id = 'batch-budget-held-v2' WHERE id = 'run-budget-held-v2'",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation) VALUES ('run-budget-held-v2', 'maker', 'hold-owner', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_budget_reservations(id, run_id, contract_digest, phase, boundary_key, owner_role, owner_worker_id, lease_generation, status, started_at_unix_ms, provenance)
         VALUES ('reservation-budget-held-v2', 'run-budget-held-v2', ?1, 'implementation', 'implementation:item-budget-held-v2', 'maker', 'hold-owner', 1, 'active', 1700000000000, 'test.resume')",
        [&contract.digest],
    )
    .unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(conn);

    let cli_db = dir.path().join("budget-hold-cli.sqlite");
    let mcp_db = dir.path().join("budget-hold-mcp.sqlite");
    let http_db = dir.path().join("budget-hold-http.sqlite");
    let concurrent_db = dir.path().join("budget-hold-concurrent.sqlite");
    let rollback_db = dir.path().join("budget-hold-rollback.sqlite");
    for copy in [&cli_db, &mcp_db, &http_db, &concurrent_db, &rollback_db] {
        fs::copy(&db, copy).unwrap();
    }

    let cli = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "hold-owner")
            .args([
                "--db",
                cli_db.to_str().unwrap(),
                "--json",
                "run",
                "resolve-budget-hold",
                "--plan",
                &plan_id,
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let mcp_output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "hold-owner")
        .args(["--db", mcp_db.to_str().unwrap(), "mcp"])
        .write_stdin(format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_run_resolve_budget_hold","arguments":{"plan":plan_id}}})
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_envelope: Value = serde_json::from_slice(&mcp_output).unwrap();
    let mcp: Value = serde_json::from_str(
        mcp_envelope["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let port = free_port();
    let bin = assert_cmd::cargo::cargo_bin("planr");
    let mut server = StdCommand::new(&bin)
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "hold-owner")
        .args([
            "--db",
            http_db.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_http_server(port);
    let http = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/plans/{plan_id}/run/resolve-budget-hold"),
        "{}",
    ));
    server.kill().unwrap();
    server.wait().unwrap();
    assert_eq!(cli["resolution"], mcp["resolution"]);
    assert_eq!(cli["resolution"], http["resolution"]);
    assert_eq!(cli["resolution"]["disposition"], "resumed");
    assert_eq!(
        cli["resolution"]["cause"],
        "active_reservations_revalidated"
    );
    assert_eq!(cli["execution_state"]["phase"], "implementation");

    let repeated = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "hold-owner")
            .args([
                "--db",
                cli_db.to_str().unwrap(),
                "--json",
                "run",
                "resolve-budget-hold",
                "--plan",
                &plan_id,
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(repeated["resolution"]["disposition"], "already_resumed");

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let barrier = barrier.clone();
        let db = concurrent_db.clone();
        let plan_id = plan_id.clone();
        let root = dir.path().to_path_buf();
        threads.push(thread::spawn(move || {
            barrier.wait();
            let output = planr()
                .current_dir(root)
                .env("PLANR_WORKER_ID", "hold-owner")
                .args([
                    "--db",
                    db.to_str().unwrap(),
                    "--json",
                    "run",
                    "resolve-budget-hold",
                    "--plan",
                    &plan_id,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice::<Value>(&output.stdout).unwrap()
        }));
    }
    let mut dispositions = threads
        .into_iter()
        .map(|thread| {
            thread.join().unwrap()["resolution"]["disposition"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();
    dispositions.sort();
    assert_eq!(dispositions, ["already_resumed", "resumed"]);
    let conn = Connection::open(&concurrent_db).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM events WHERE event_type = 'feature_run_budget_hold_resolved'",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        1
    );

    let conn = Connection::open(&rollback_db).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_budget_hold_resolution BEFORE UPDATE ON feature_runs
         WHEN NEW.phase = 'implementation'
         BEGIN SELECT RAISE(ABORT, 'injected resolve rollback'); END;",
    )
    .unwrap();
    drop(conn);
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "hold-owner")
        .args([
            "--db",
            rollback_db.to_str().unwrap(),
            "--json",
            "run",
            "resolve-budget-hold",
            "--plan",
            &plan_id,
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("injected resolve rollback"));
    let conn = Connection::open(&rollback_db).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT phase FROM feature_runs WHERE id = 'run-budget-held-v2'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "held"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM events WHERE event_type = 'feature_run_budget_hold_resolved'",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        0
    );
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
fn structurally_supported_policy_upgrade_is_explicit_strict_and_atomic() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Policy Upgrade",
        ])
        .assert()
        .success();
    let fixture = include_str!("fixtures/policy-v1.10.0-alpha.2.toml")
        .replace("version = \"1.10.0-alpha.2\"", "version = \"1.0.0\"");
    let policy_path = dir.path().join(".planr/policy.toml");
    fs::write(&policy_path, &fixture).unwrap();

    let plan_path = dir.path().join(".planr/plans/build/policy-upgrade.plan.md");
    fs::create_dir_all(plan_path.parent().unwrap()).unwrap();
    fs::write(&plan_path, "# Policy upgrade\n").unwrap();
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('plan-policy-upgrade', ?1, 'build', ?2, 'Policy upgrade', 'policy-upgrade', 'ok', 'sha256:policy-upgrade', datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    for ordinal in 1..=2 {
        conn.execute(
            "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'policy upgrade', 'ready', 'code', ?4, datetime('now'), datetime('now'))",
            rusqlite::params![
                format!("item-policy-upgrade-{ordinal}"),
                project_id,
                format!("Policy upgrade {ordinal}"),
                plan_path.to_string_lossy(),
            ],
        )
        .unwrap();
    }
    drop(conn);

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "policy", "check"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("policy_upgrade_required"));
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-upgrade-maker")
        .args([
            "--db",
            db.to_str().unwrap(),
            "pick",
            "--work-type",
            "code",
            "--plan",
            "plan-policy-upgrade",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("policy_upgrade_required"));
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM feature_runs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "legacy policy must be detected before FeatureRun creation"
    );
    drop(conn);

    let ambiguous = format!(
        "{fixture}\n[usage.phase_reserves]\nverification_percent = 0\nreview_percent = 20\nrepair_percent = 0\n"
    );
    fs::write(&policy_path, &ambiguous).unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "policy", "upgrade"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "ambiguous mixed legacy/current policy shape",
        ));
    assert_eq!(fs::read_to_string(&policy_path).unwrap(), ambiguous);

    let lossy = fixture.replace(
        "review_reserve_percent = 20",
        "review_reserve_percent = 120",
    );
    fs::write(&policy_path, &lossy).unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "policy", "upgrade"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must total at most 100 percent"));
    assert_eq!(fs::read_to_string(&policy_path).unwrap(), lossy);

    fs::write(&policy_path, &fixture).unwrap();
    let output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "policy", "upgrade"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(preview["mode"], "preview");
    assert_eq!(
        preview["upgrade"]["from_shape"],
        "planr.policy.v1@v1.10.0-alpha.2"
    );
    let canonical = preview["upgrade"]["canonical_toml"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(canonical.contains("review_percent = 20"));
    assert!(canonical.contains("verification_percent = 0"));
    assert!(canonical.contains("repair_percent = 0"));
    assert!(canonical.contains("protected_risks = ["));
    assert!(canonical.contains("security_or_auth"));
    assert!(canonical.contains("concurrency_or_transaction"));
    assert!(canonical.contains("require_verification_evidence = true"));
    let canonical_value: toml::Value = toml::from_str(&canonical).unwrap();
    assert_eq!(
        canonical_value["execution"]["roles"]["worker"]["filesystem"]["write_roots"],
        toml::Value::Array(vec![
            toml::Value::String("src".to_string()),
            toml::Value::String("tests".to_string()),
        ])
    );
    assert_eq!(fs::read_to_string(&policy_path).unwrap(), fixture);
    assert!(canonical.contains("version = \"1.0.0\""));

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "policy",
            "upgrade",
            "--apply",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let applied: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(applied["mode"], "apply");
    assert_eq!(applied["upgrade"]["canonical_toml"], canonical);
    assert_eq!(fs::read_to_string(&policy_path).unwrap(), canonical);
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "policy", "check"])
        .assert()
        .success();

    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "policy-upgrade-maker")
        .args([
            "--db",
            db.to_str().unwrap(),
            "pick",
            "--work-type",
            "code",
            "--plan",
            "plan-policy-upgrade",
        ])
        .assert()
        .success();
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM feature_runs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1,
        "canonical policy must permit normal FeatureRun initialization"
    );
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
budget_exhaustion = "downgrade_noncritical"
metering = "trusted"
[usage.phase_reserves]
verification_percent = 10
review_percent = 5
repair_percent = 5

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
protected_risks = ["security_or_auth", "secrets_or_crypto", "schema_or_migration", "infrastructure_or_deploy", "public_api", "billing", "concurrency_or_transaction"]
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
        wait_for_http_server(port);
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
        .env("PLANR_WORKER_ID", "runtime-a")
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
    assert_eq!(
        blocked_close["error"]["code"], "invalid_transition",
        "{blocked_close}"
    );

    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "runtime-a")
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
        .env("PLANR_WORKER_ID", "runtime-a")
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
        .env("PLANR_WORKER_ID", "runtime-a")
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
    wait_for_http_server(port);

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
    wait_for_http_server(port);

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

fn seed_final_review_feature_run(dir: &Path, db: &Path) -> String {
    planr()
        .current_dir(dir)
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Final Gate Seed",
        ])
        .assert()
        .success();
    let plan = planr()
        .current_dir(dir)
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "new",
            "Final Gate Seed Plan",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let plan: Value = serde_json::from_slice(&plan).unwrap();
    let plan_id = plan["plan"]["id"].as_str().unwrap().to_string();
    let plan_path = plan["plan"]["path"].as_str().unwrap().to_string();
    init_git_repo(dir);
    let source = evidence_source_binding(dir);
    let conn = Connection::open(db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let budget_contract = test_unbounded_feature_run_contract("run-final-seed");
    conn.execute(
        "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at, completed_at)
         VALUES ('outcome-final-seed', ?1, 'Outcome', 'settled outcome', 'closed', 'code', 'maker-final', ?2, datetime('now'), datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, source_revision, active_batch_id, outcomes_settled, batch_outcome_count, revision, budget_contract_digest)
         VALUES ('run-final-seed', ?1, ?2, 'active', 'verification', 'sha256:policy', ?3, NULL, 1, 0, 0, ?4)",
        rusqlite::params![project_id, plan_id, source["revision"].as_str().unwrap(), budget_contract.digest],
    )
    .unwrap();
    insert_test_feature_run_contract(&conn, &budget_contract);
    conn.execute(
        "INSERT INTO feature_run_source_freezes(id, run_id, source_revision, source_digest, status) VALUES ('freeze-final-seed', 'run-final-seed', ?1, ?2, 'active')",
        rusqlite::params![source["revision"].as_str().unwrap(), source["tree_digest"].as_str().unwrap()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation, released_at)
         VALUES ('run-final-seed', 'maker', 'maker-final', 1, datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation)
         VALUES ('run-final-seed', 'verifier', 'verifier-final', 1)",
        [],
    )
    .unwrap();
    plan_id
}

#[test]
fn canonical_final_review_cli_mcp_pick_accept_and_audit_use_one_gate_without_map_items() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let plan_id = seed_final_review_feature_run(dir.path(), &db);
    let created = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "final-review",
            &plan_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&created).unwrap();
    let gate_id = created["execution_state"]["review_gate"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let picked = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "reviewer-final")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "--plan",
            &plan_id,
            "--work-type",
            "review",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let picked: Value = serde_json::from_slice(&picked).unwrap();
    assert_eq!(
        picked["work_packet"]["execution_state"]["review_gate"]["id"],
        gate_id
    );
    let close_input = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_review_gate_close","arguments":{"review_gate_id":gate_id,"verdict":"complete","reviewer":"reviewer-final"}}}).to_string() + "\n";
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "reviewer-final")
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(close_input)
        .assert()
        .success()
        .stdout(predicate::str::contains("accepted"));
    let audit = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "audit",
            &plan_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let audit: Value = serde_json::from_slice(&audit).unwrap();
    assert_eq!(
        audit["clauses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|clause| clause["clause"] == "final_product_review_complete")
            .unwrap()["pass"],
        true
    );
}

#[test]
fn execution_state_v2_budget_is_byte_equivalent_across_cli_mcp_http_and_packets() {
    let secret_marker = "sk-planr-budget-provenance-0123456789abcdef0123456789abcdef";
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Final Gate",
        ])
        .assert()
        .success();
    let plan = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "new",
            "Canonical Final Gate",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let plan: Value = serde_json::from_slice(&plan).unwrap();
    let plan_id = plan["plan"]["id"].as_str().unwrap().to_string();
    let plan_path = plan["plan"]["path"].as_str().unwrap().to_string();
    init_git_repo(dir.path());
    let source = evidence_source_binding(dir.path());
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, source_revision, active_batch_id, outcomes_settled, batch_outcome_count, revision, terminal_reason, created_at, updated_at)
         VALUES ('run-historical', ?1, ?2, 'complete', 'complete', 'sha256:old-policy', 'source-historical', NULL, 1, 0, 1, 'completed', '2000-01-01 00:00:00', '2000-01-01 00:00:00')",
        rusqlite::params![project_id, plan_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation, released_at)
         VALUES ('run-historical', 'maker', 'maker-historical', 1, '2000-01-01 00:00:00')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO review_gates(id, run_id, scope_kind, scope_id, kind, status, responsible_maker_id, latest_attempt, source_revision, created_at, updated_at)
         VALUES ('gate-historical', 'run-historical', 'plan', ?1, 'final_product', 'changes_requested', 'maker-historical', 1, 'source-historical', '2000-01-01 00:00:00', '2000-01-01 00:00:00')",
        [&plan_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO review_attempts(id, gate_id, attempt_number, reviewer_worker_id, reviewer_mode, verdict, source_revision, artifacts_json, created_at)
         VALUES ('attempt-historical', 'gate-historical', 1, 'reviewer-historical', 'independent', 'changes_requested', 'source-historical', '[]', '2000-01-01 00:00:00')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO review_findings(id, run_id, gate_id, attempt_id, severity, target, owner_worker_id, status, invalidated_evidence_ids_json, created_at)
         VALUES ('finding-historical', 'run-historical', 'gate-historical', 'attempt-historical', 'high', 'historical-target', 'maker-historical', 'open', '[]', '2000-01-01 00:00:00')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_budget_reservations(id, run_id, phase, boundary_key, status, reserved_wall_seconds, reserved_tool_calls, started_at_unix_ms, provenance)
         VALUES ('reservation-historical', 'run-historical', 'review', 'review:historical', 'active', 97, 13, 1, 'historical-test')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_budget_observations(id, run_id, phase, metering, wall_seconds, tokens, tool_calls, provenance)
         VALUES ('observation-historical', 'run-historical', 'review', 'trusted', 97, 1000, 13, 'historical-test')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at, completed_at)
         VALUES ('outcome-final', ?1, 'Outcome', 'settled outcome', 'closed', 'code', 'maker-final', ?2, datetime('now'), datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path],
    )
    .unwrap();
    let budget_contract = test_unbounded_feature_run_contract("run-final");
    conn.execute(
        "INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, source_revision, active_batch_id, outcomes_settled, batch_outcome_count, revision, budget_contract_digest)
         VALUES ('run-final', ?1, ?2, 'active', 'verification', 'sha256:policy', ?3, NULL, 1, 0, 0, ?4)",
        rusqlite::params![project_id, plan_id, source["revision"].as_str().unwrap(), budget_contract.digest],
    )
    .unwrap();
    insert_test_feature_run_contract(&conn, &budget_contract);
    conn.execute(
        "INSERT INTO feature_run_source_freezes(id, run_id, source_revision, source_digest, status) VALUES ('freeze-final', 'run-final', ?1, ?2, 'active')",
        rusqlite::params![source["revision"].as_str().unwrap(), source["tree_digest"].as_str().unwrap()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation, released_at)
         VALUES ('run-final', 'maker', 'maker-final', 1, datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation)
         VALUES ('run-final', 'verifier', 'verifier-final', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO feature_run_budget_reservations(
            id, run_id, contract_digest, phase, boundary_key, owner_role,
            owner_worker_id, lease_generation, status, started_at_unix_ms, provenance
         ) VALUES (
            'reservation-secret-provenance', 'run-final', ?1, 'verification', ?2, 'verifier',
            'verifier-final', 1, 'released', ?3, ?4
         )",
        rusqlite::params![
            budget_contract.digest,
            format!("verification:{secret_marker}"),
            budget_contract.started_at_unix_ms,
            secret_marker,
        ],
    )
    .unwrap();
    drop(conn);

    let created = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "final-review",
            &plan_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&created).unwrap();
    assert_eq!(created["created"], true);
    assert_eq!(
        created["execution_state"]["schema_version"],
        "planr.execution_state.v2"
    );
    assert_eq!(created["execution_state"]["feature_run"]["id"], "run-final");
    assert_eq!(
        created["execution_state"]["feature_run"]["source_revision"],
        source["revision"]
    );
    assert_eq!(
        created["execution_state"]["review_gate"]["source_revision"],
        source["revision"]
    );
    assert_eq!(
        created["execution_state"]["review_source_binding"]["freeze_id"],
        "freeze-final"
    );
    assert_eq!(
        created["execution_state"]["review_source_binding"]["source_digest"],
        source["tree_digest"]
    );
    assert_eq!(
        created["execution_state"]["owner"]["worker_id"],
        "verifier-final"
    );
    assert_eq!(created["execution_state"]["budget"]["status"], "available");
    let gate_id = created["execution_state"]["review_gate"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mcp_input = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"planr_plan_final_review","arguments":{"id":plan_id}}}).to_string() + "\n";
    let mcp_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(mcp_input)
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
    let mcp_value: Value = serde_json::from_str(
        mcp_response["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(mcp_value["execution_state"], created["execution_state"]);
    assert_eq!(mcp_value["created"], false);

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
    wait_for_http_server(port);
    let projected = http_json(&http_request(
        port,
        "GET",
        &format!("/v1/plans/{plan_id}/final-product-review"),
        "",
    ));
    assert_eq!(projected["current"]["review_gate"]["id"], gate_id);
    assert_eq!(projected["current"]["accepted"], false);

    let picked = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "reviewer-final")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "--plan",
            &plan_id,
            "--work-type",
            "review",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let picked: Value = serde_json::from_slice(&picked).unwrap();
    assert_eq!(picked["work_packet"]["kind"], "review_gate");
    let canonical_budget_bytes =
        serde_json::to_vec(&picked["work_packet"]["execution_state"]["budget"]).unwrap();
    thread::sleep(Duration::from_millis(1_100));

    let cli_status = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap(), "--json", "map", "status"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let mcp_status = mcp_tool(dir.path(), &db, 72, "planr_map_status", json!({}));
    let http_status = http_json(&http_request(port, "GET", "/v1/map/status", ""));
    for (surface, status) in [
        ("CLI", &cli_status),
        ("MCP", &mcp_status),
        ("HTTP", &http_status),
    ] {
        let budget = &status["execution_states"]
            .as_array()
            .unwrap()
            .iter()
            .find(|state| state["feature_run"]["id"] == "run-final")
            .unwrap()["budget"];
        assert_eq!(
            serde_json::to_vec(budget).unwrap(),
            canonical_budget_bytes,
            "{surface} must reuse the byte-equivalent work-packet budget projection"
        );
    }
    assert_eq!(
        serde_json::to_vec(&picked["work_packet"]["execution_state"]["budget"]).unwrap(),
        canonical_budget_bytes,
        "work packets must reuse the byte-equivalent canonical budget projection"
    );
    assert_eq!(
        picked["work_packet"]["execution_state"]["review_gate"]["id"],
        gate_id
    );

    let close_input = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"planr_review_gate_close","arguments":{"review_gate_id":gate_id,"verdict":"complete","reviewer":"reviewer-final"}}}).to_string() + "\n";
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "reviewer-final")
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(close_input)
        .assert()
        .success()
        .stdout(predicate::str::contains("accepted"));

    let projected = http_json(&http_request(
        port,
        "GET",
        &format!("/v1/plans/{plan_id}/final-product-review"),
        "",
    ));
    assert_eq!(projected["current"]["accepted"], true);

    let audit = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "audit",
            &plan_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let audit: Value = serde_json::from_slice(&audit).unwrap();
    let final_clause = audit["clauses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clause| clause["clause"] == "final_product_review_complete")
        .unwrap();
    assert_eq!(final_clause["pass"], true, "{final_clause}");

    let status = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "map", "status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let status: Value = serde_json::from_slice(&status).unwrap();
    let current_status = status["execution_states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["feature_run"]["id"] == "run-final")
        .unwrap()
        .clone();
    let historical_status = status["execution_states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["feature_run"]["id"] == "run-historical")
        .unwrap();
    assert_eq!(current_status["reason_code"], "feature_run_complete");
    assert_eq!(historical_status["review_gate"]["id"], "gate-historical");
    assert_eq!(
        historical_status["review_attempts"][0]["id"],
        "attempt-historical"
    );
    assert_eq!(historical_status["findings"][0]["id"], "finding-historical");
    assert_eq!(historical_status["budget"]["status"], "unavailable");
    assert!(historical_status["owner"].is_null());
    assert_eq!(
        historical_status["feature_run"]["source_revision"],
        "source-historical"
    );

    let http_status = http_json(&http_request(port, "GET", "/v1/map/status", ""));
    let http_current = http_status["execution_states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["feature_run"]["id"] == "run-final")
        .unwrap();
    assert_eq!(http_current, &current_status);
    server.kill().unwrap();
    server.wait().unwrap();

    let status_mcp_input = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"planr_map_status","arguments":{}}}).to_string() + "\n";
    let status_mcp_output = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "mcp"])
        .write_stdin(status_mcp_input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let status_mcp_response: Value = serde_json::from_str(
        String::from_utf8(status_mcp_output)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let status_mcp: Value = serde_json::from_str(
        status_mcp_response["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let status_mcp_current = status_mcp["execution_states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["feature_run"]["id"] == "run-final")
        .unwrap();
    assert_eq!(status_mcp_current, &current_status);
    let trace = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "trace",
            "item",
            "outcome-final",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let trace: Value = serde_json::from_slice(&trace).unwrap();
    assert_eq!(
        trace["execution_state"]["reason_code"],
        "feature_run_complete"
    );
    assert_eq!(trace["execution_state"], current_status);
    assert_eq!(
        trace["recovery"]["execution_state"],
        trace["execution_state"]
    );
    assert_eq!(audit["execution_state"], trace["execution_state"]);
    for legacy_key in ["review_attempts", "review_gate", "final_product_review"] {
        assert!(
            trace.get(legacy_key).is_none(),
            "legacy live trace field {legacy_key}: {trace}"
        );
    }

    let export_path = dir.path().join("canonical-package.json");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "export",
            "--out",
            export_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    let package: Value = serde_json::from_slice(&fs::read(export_path).unwrap()).unwrap();
    let packaged_current = package["execution_states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["feature_run"]["id"] == "run-final")
        .unwrap();
    assert_eq!(packaged_current, &trace["execution_state"]);
    for (surface, value) in [
        ("final-review CLI", &created),
        ("final-review MCP", &mcp_value),
        ("review work packet", &picked),
        ("status CLI", &cli_status),
        ("status MCP", &mcp_status),
        ("status HTTP", &http_status),
        ("audit", &audit),
        ("trace", &trace),
        ("package", &package),
    ] {
        assert!(
            !serde_json::to_string(value)
                .unwrap()
                .contains(secret_marker),
            "{surface} leaked private budget ledger provenance"
        );
    }
    assert_eq!(
        trace["execution_state"]["review_attempts"][0]["source_revision"],
        source["revision"]
    );
    assert!(
        trace["execution_state"]["findings"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    for args in [
        vec!["map", "status"],
        vec!["trace", "item", "outcome-final"],
        vec!["review", "show", gate_id.as_str()],
        vec!["plan", "audit", plan_id.as_str()],
    ] {
        let output = planr()
            .current_dir(dir.path())
            .args(["--db", db.to_str().unwrap()])
            .args(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let human = String::from_utf8(output).unwrap();
        assert!(
            !human.contains(secret_marker),
            "human projection leaked private budget ledger provenance"
        );
        let leading = human.lines().take(5).collect::<Vec<_>>();
        assert!(leading[0].starts_with("phase:"), "{human}");
        assert!(leading[1].starts_with("owner:"), "{human}");
        assert!(leading[2].starts_with("budget:"), "{human}");
        assert!(leading[3].starts_with("unmet gate:"), "{human}");
        assert!(leading[4].starts_with("next action:"), "{human}");
    }
}

#[test]
fn capability_and_budget_holds_keep_distinct_reasons_across_cli_mcp_and_http() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Held Reasons",
        ])
        .assert()
        .success();
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    for (run_id, plan_id, reason) in [
        ("run-capability-held", "plan-capability-held", "capability"),
        ("run-budget-held", "plan-budget-held", "budget"),
    ] {
        let budget_contract = test_unbounded_feature_run_contract(run_id);
        conn.execute(
            "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
             VALUES (?1, ?2, 'build', ?3, ?4, ?1, 'ok', ?1, datetime('now'), datetime('now'))",
            rusqlite::params![plan_id, project_id, format!("/tmp/{plan_id}.plan.md"), plan_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, source_revision, active_batch_id, outcomes_settled, batch_outcome_count, held_from_phase, hold_reason, revision, budget_contract_digest)
             VALUES (?1, ?2, ?3, 'held', 'held', 'sha256:policy', 'source-held', NULL, 1, 0, 'source_frozen', ?4, 0, ?5)",
            rusqlite::params![run_id, project_id, plan_id, reason, budget_contract.digest],
        )
        .unwrap();
        insert_test_feature_run_contract(&conn, &budget_contract);
    }
    drop(conn);

    let assert_reasons = |value: &Value| {
        let states = value["execution_states"].as_array().unwrap();
        let capability = states
            .iter()
            .find(|state| state["feature_run"]["id"] == "run-capability-held")
            .unwrap();
        assert_eq!(capability["reason_code"], "evidence_readiness_blocked");
        assert_eq!(capability["next_action"], "repair_evidence_readiness");
        assert_eq!(capability["feature_run"]["hold_reason"], "capability");
        let budget = states
            .iter()
            .find(|state| state["feature_run"]["id"] == "run-budget-held")
            .unwrap();
        assert_eq!(budget["reason_code"], "feature_run_budget_held");
        assert_eq!(budget["next_action"], "resolve_budget_hold");
        assert_eq!(budget["feature_run"]["hold_reason"], "budget");
    };

    let cli = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "map", "status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_reasons(&serde_json::from_slice(&cli).unwrap());

    let mcp = mcp_tool(dir.path(), &db, 71, "planr_map_status", json!({}));
    assert_reasons(&mcp);

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
    wait_for_http_server(port);
    assert_reasons(&http_json(&http_request(port, "GET", "/v1/map/status", "")));
    server.kill().unwrap();
    server.wait().unwrap();
}

#[test]
fn capped_cli_batch_roll_preserves_same_maker_and_fourth_outcome_continues_cleanly() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Same Maker Roll",
        ])
        .assert()
        .success();
    let plan_path = dir.path().join("same-maker-roll.plan.md");
    fs::write(&plan_path, "# Same Maker Roll\n").unwrap();
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('plan-same-maker-roll', ?1, 'build', ?2, 'Same Maker Roll', 'same-maker-roll', 'ok', 'sha256:same-maker-roll', datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    for ordinal in 1..=6 {
        conn.execute(
            "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'same-maker outcome', 'picked', 'code', 'maker-roll-e2e', ?4, datetime('now'), datetime('now'))",
            rusqlite::params![
                format!("item-roll-{ordinal}"),
                project_id,
                format!("Outcome {ordinal}"),
                plan_path.to_string_lossy(),
            ],
        )
        .unwrap();
    }
    conn.execute(
        "UPDATE items SET status = 'ready', worker_id = NULL WHERE id IN ('item-roll-4', 'item-roll-5', 'item-roll-6')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id, project_id, title, description, status, work_type, priority, plan_path, created_at, updated_at)
         VALUES ('item-roll-generic', ?1, 'Non-code work', 'must not be fused into a maker batch', 'ready', 'generic', 100, ?2, datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    let other_plan_path = dir.path().join("other.plan.md");
    fs::write(&other_plan_path, "# Other Plan\n").unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('plan-other', ?1, 'build', ?2, 'Other Plan', 'other', 'ok', 'sha256:other', datetime('now'), datetime('now'))",
        rusqlite::params![project_id, other_plan_path.to_string_lossy()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id, project_id, title, description, status, work_type, priority, plan_path, created_at, updated_at)
         VALUES ('item-other-plan', ?1, 'Other plan outcome', 'must remain outside the fused continuation', 'ready', 'code', 200, ?2, datetime('now'), datetime('now'))",
        rusqlite::params![project_id, other_plan_path.to_string_lossy()],
    )
    .unwrap();
    drop(conn);
    init_git_repo(dir.path());
    let source_freeze_planr = private_planr_binary(dir.path());

    let settle = |ordinal: u32, next: bool| -> Value {
        let mut command = planr_from_binary(&source_freeze_planr);
        command
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "maker-roll-e2e")
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "done",
                &format!("item-roll-{ordinal}"),
                "--summary",
                &format!("settled outcome {ordinal}"),
                "--cmd",
                "true",
                "--tests",
                "true",
            ]);
        if next {
            command.arg("--next");
        }
        let output = command.assert().success().get_output().stdout.clone();
        serde_json::from_slice(&output).unwrap()
    };
    assert_eq!(
        settle(1, false)["work_packet"]["transition"],
        "continue_batch"
    );
    assert_eq!(
        settle(2, false)["work_packet"]["transition"],
        "continue_batch"
    );
    let third = settle(3, true);
    assert_eq!(third["work_packet"]["transition"], "batch_cap_reached");
    let first_batch = third["work_packet"]["batch_id"]
        .as_str()
        .unwrap()
        .to_string();
    let rolled = &third["work_packet"]["rollover"];
    assert_eq!(rolled["reason"], "same_maker_batch_rolled");
    assert_eq!(rolled["ended_batch"]["id"], first_batch);
    assert_eq!(rolled["ended_batch"]["replacement"], Value::Null);
    assert_eq!(rolled["feature_run"]["batch_outcome_count"], 0);
    assert_eq!(
        rolled["feature_run"]["role_owners"][0]["worker_id"],
        "maker-roll-e2e"
    );
    assert_eq!(
        rolled["feature_run"]["role_owners"][0]["lease_generation"],
        1
    );
    assert_eq!(
        third["work_packet"]["successor_batch_id"],
        rolled["execution_batch"]["id"]
    );
    assert_eq!(third["next"]["item"]["id"], "item-roll-4");
    assert_eq!(third["next"]["item"]["work_type"], "code");
    assert_eq!(
        third["next"]["item"]["plan_path"],
        plan_path.to_string_lossy().as_ref()
    );

    let fourth = settle(4, false);
    assert_eq!(fourth["work_packet"]["transition"], "continue_batch");
    assert_eq!(fourth["work_packet"]["batch_outcome_count"], 1);
    assert_ne!(fourth["work_packet"]["batch_id"], first_batch);
    assert_eq!(
        settle(5, false)["work_packet"]["transition"],
        "continue_batch"
    );
    let sixth = settle(6, true);
    assert_eq!(sixth["work_packet"]["transition"], "batch_cap_reached");
    assert_eq!(sixth["work_packet"]["rollover"], Value::Null);
    assert_eq!(sixth["next"]["item"], Value::Null);
    assert_eq!(
        sixth["next"]["reason"],
        "nonbinding_final_review_handoff_source_frozen"
    );
    assert_eq!(sixth["next"]["work_packet"]["kind"], "final_review_handoff");

    let conn = Connection::open(&db).unwrap();
    let scalar = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap() };
    assert_eq!(scalar("SELECT COUNT(*) FROM execution_batches"), 2);
    assert_eq!(
        scalar(
            "SELECT COUNT(*) FROM execution_batches WHERE status = 'ended' AND replacement_reason IS NULL AND replaced_maker_worker_id IS NULL AND successor_maker_worker_id IS NULL"
        ),
        2
    );
    assert_eq!(
        scalar(
            "SELECT COUNT(*) FROM execution_batches WHERE status = 'active' AND maker_worker_id = 'maker-roll-e2e'"
        ),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT phase FROM feature_runs WHERE plan_id = 'plan-same-maker-roll'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "source_frozen"
    );
    assert_eq!(scalar("SELECT COUNT(*) FROM execution_run_outcomes"), 6);
    assert_eq!(
        scalar(
            "SELECT COUNT(*) FROM items WHERE id IN ('item-roll-generic', 'item-other-plan') AND status = 'ready'"
        ),
        2
    );
    assert_eq!(scalar("SELECT COUNT(*) FROM review_gates"), 0);
    assert_eq!(scalar("SELECT COUNT(*) FROM review_attempts"), 0);
    assert_eq!(
        scalar("SELECT COUNT(*) FROM items WHERE work_type IN ('review','fix')"),
        0
    );
    assert_eq!(
        scalar(
            "SELECT COUNT(*) FROM logs WHERE kind = 'completion' AND item_id LIKE 'item-roll-%'"
        ),
        6
    );
}

#[test]
fn already_settled_outcome_retry_reuses_settlement_and_freezes_source_once() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            &db_arg,
            "project",
            "init",
            "Idempotent outcome retry",
        ])
        .assert()
        .success();
    let invoke = |args: &[&str]| -> Value {
        single_json_document(
            &planr()
                .current_dir(dir.path())
                .env("PLANR_WORKER_ID", "maker-idempotent-retry")
                .args(["--db", &db_arg, "--json"])
                .args(args)
                .assert()
                .success()
                .get_output()
                .stdout,
        )
    };
    let product = invoke(&["plan", "new", "Idempotent retry product"]);
    let build = invoke(&[
        "plan",
        "split",
        product["plan"]["id"].as_str().unwrap(),
        "--slice",
        "Idempotent retry build",
    ]);
    let plan_id = build["plan"]["id"].as_str().unwrap().to_string();
    let plan_path = Path::new(build["plan"]["path"].as_str().unwrap());
    fs::write(plan_path, "---\nname: idempotent-retry-build\ncriteria:\n  - id: criterion-idempotent-retry\n    title: Retry freezes once\n---\n# Idempotent retry build\n\n## Scope Decision\n\nOne ordinary outcome.\n\n## Verification\n\nFocused public CLI proof.\n\n## Acceptance Criteria\n\nRetry reuses settlement.\n\n### TASK-001 (fix): Settle one outcome\n\nProve canonical retry.\n").unwrap();
    assert_eq!(invoke(&["plan", "check", &plan_id])["ok"], true);
    let map = invoke(&["map", "build", "--from", &plan_id]);
    let item_id = map["created"][0]["id"]
        .as_str()
        .expect("map build must create the ordinary outcome")
        .to_string();
    let claimed_file = "retry-materiality.txt";
    fs::write(dir.path().join(claimed_file), "settled\n").unwrap();
    init_git_repo(dir.path());
    let binary = private_planr_binary(dir.path());
    let run = |args: &[&str]| -> Value {
        single_json_document(
            &planr_from_binary(&binary)
                .current_dir(dir.path())
                .env("PLANR_WORKER_ID", "maker-idempotent-retry")
                .args(["--db", &db_arg, "--json"])
                .args(args)
                .assert()
                .success()
                .get_output()
                .stdout,
        )
    };
    let picked = run(&["pick", "--plan", &plan_id, "--work-type", "fix"]);
    assert_eq!(picked["work_packet"]["kind"], "outcome");
    assert_eq!(picked["item"]["id"], item_id);
    assert_eq!(
        picked["work_packet"]["execution_state"]["owner"]["worker_id"],
        "maker-idempotent-retry"
    );
    let done_args = [
        "done",
        item_id.as_str(),
        "--summary",
        "settled idempotent outcome",
        "--files",
        claimed_file,
        "--cmd",
        "true",
        "--tests",
        "true",
    ];
    let first = run(&done_args);
    assert_eq!(first["item"]["status"], "closed");
    assert_eq!(first["work_packet"]["transition"], "continue_batch");
    let log_id = first["log_id"]
        .as_str()
        .expect("first settlement must return a completion log id")
        .to_string();
    let run_id = first["work_packet"]["run_id"]
        .as_str()
        .expect("first settlement must return a run id")
        .to_string();
    let batch_id = first["work_packet"]["batch_id"]
        .as_str()
        .expect("first settlement must return a batch id")
        .to_string();
    let persisted_materiality = first["materiality"].clone();
    let settled_counters = (
        first["work_packet"]["execution_state"]["feature_run"]["outcomes_settled"].clone(),
        first["work_packet"]["batch_outcome_count"].clone(),
    );
    assert_eq!(
        persisted_materiality["change_summary"]["files"],
        json!([claimed_file])
    );
    assert_eq!(persisted_materiality["change_summary"]["changed_lines"], 0);
    assert_eq!(
        first["work_packet"]["execution_state"]["feature_run"]["phase"],
        "implementation"
    );
    let history = |conn: &Connection| -> Value {
        serde_json::from_str(&conn.query_row(
            "SELECT json_object('logs',(SELECT COUNT(*) FROM logs WHERE item_id=?1 AND kind='completion'),'outcomes',(SELECT COUNT(*) FROM execution_run_outcomes WHERE item_id=?1),'log_events',(SELECT COUNT(*) FROM events WHERE item_id=?1 AND event_type='log_created'),'close_events',(SELECT COUNT(*) FROM events WHERE item_id=?1 AND event_type='item_closed'),'materiality_events',(SELECT COUNT(*) FROM events WHERE item_id=?1 AND event_type='materiality_decided'),'budget_reservations',(SELECT COUNT(*) FROM feature_run_budget_reservations WHERE run_id=?2 AND boundary_key='implementation:'||?1 AND status='reconciled'),'budget_observations',(SELECT COUNT(*) FROM feature_run_budget_observations o JOIN feature_run_budget_reservations r ON r.id=o.reservation_id WHERE r.run_id=?2 AND r.boundary_key='implementation:'||?1),'outcome_row',(SELECT id||'|'||run_id||'|'||batch_id||'|'||item_id||'|'||ordinal||'|'||outcome_json FROM execution_run_outcomes WHERE item_id=?1),'log_id',(SELECT id FROM logs WHERE item_id=?1 AND kind='completion'))",
            rusqlite::params![item_id, run_id], |row| row.get::<_, String>(0)).unwrap()).unwrap()
    };
    let conn = Connection::open(&db).unwrap();
    let before = history(&conn);
    assert_eq!(before["logs"], 1);
    assert_eq!(before["outcomes"], 1);
    assert_eq!(before["log_events"], 1);
    assert_eq!(before["close_events"], 1);
    assert_eq!(before["materiality_events"], 1);
    assert_eq!(before["budget_reservations"], 1);
    assert_eq!(before["budget_observations"], 1);
    assert_eq!(before["log_id"], log_id);
    assert_eq!(
        conn.query_row(
            "SELECT phase FROM feature_runs WHERE id=?1",
            [&run_id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "implementation"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM feature_run_source_freezes WHERE run_id=?1",
            [&run_id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    fs::write(
        dir.path().join(claimed_file),
        "settled\nmutable worktree drift one\nmutable worktree drift two\n",
    )
    .unwrap();
    let drift = StdCommand::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["diff", "--numstat", "--", claimed_file])
        .output()
        .unwrap();
    assert!(drift.status.success());
    assert_eq!(
        String::from_utf8(drift.stdout).unwrap(),
        format!("2\t0\t{claimed_file}\n")
    );
    let conflict = single_json_document(
        &planr_from_binary(&binary)
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "maker-idempotent-retry")
            .args([
                "--db",
                &db_arg,
                "--json",
                "done",
                item_id.as_str(),
                "--summary",
                "conflicting retry summary",
                "--files",
                claimed_file,
                "--cmd",
                "true",
                "--tests",
                "true",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout,
    );
    assert_eq!(conflict["error"]["code"], "internal_error");
    assert_eq!(
        conflict["error"]["message"],
        format!("already_settled_outcome_rejected:{run_id}:{item_id}:OutcomeSummaryMismatch")
    );
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        history(&conn),
        before,
        "conflicting retry must roll back before settlement mutation"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM feature_run_source_freezes WHERE run_id=?1",
            [&run_id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    let mut retry_args = done_args.to_vec();
    retry_args.push("--next");
    let retry = run(&retry_args);
    assert_eq!(retry["log_id"], log_id);
    assert_eq!(retry["work_packet"]["transition"], "already_settled");
    assert_eq!(retry["work_packet"]["disposition"], "already_settled");
    assert_eq!(retry["work_packet"]["run_id"], run_id);
    assert_eq!(retry["work_packet"]["batch_id"], batch_id);
    assert_eq!(retry["materiality"], persisted_materiality);
    assert_eq!(
        (
            retry["work_packet"]["execution_state"]["feature_run"]["outcomes_settled"].clone(),
            retry["work_packet"]["batch_outcome_count"].clone()
        ),
        settled_counters
    );
    assert_eq!(retry["next"]["item"], Value::Null);
    assert_eq!(
        retry["next"]["reason"],
        "nonbinding_final_review_handoff_source_frozen"
    );
    assert_eq!(retry["next"]["work_packet"]["kind"], "final_review_handoff");
    let freeze = &retry["next"]["work_packet"]["source_freeze"];
    let freeze_id = freeze["source_freeze"]["id"]
        .as_str()
        .expect("canonical handoff must return a non-null source freeze id");
    assert!(
        freeze["source_freeze"]["source_revision"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        freeze["source_freeze"]["source_digest"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert_eq!(freeze["feature_run"]["phase"], "source_frozen");
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        history(&conn),
        before,
        "retry must not mutate authoritative settlement history"
    );
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM feature_run_source_freezes WHERE run_id=?1 AND id=?2 AND status='active'", rusqlite::params![run_id, freeze_id], |row| row.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(
        conn.query_row(
            "SELECT status||':'||phase||':'||active_batch_id FROM feature_runs WHERE id=?1",
            [&run_id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        format!("active:source_frozen:{batch_id}")
    );
    assert_eq!(
        conn.query_row(
            "SELECT status||':'||maker_worker_id FROM execution_batches WHERE id=?1",
            [&batch_id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "ended:maker-idempotent-retry"
    );
}

#[test]
fn code_to_fix_continuation_leases_fix_without_premature_source_freeze() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Code To Fix Continuation",
        ])
        .assert()
        .success();
    let plan_path = dir.path().join("code-to-fix.plan.md");
    fs::write(&plan_path, "# Code To Fix Continuation\n").unwrap();
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('plan-code-to-fix', ?1, 'build', ?2, 'Code To Fix', 'code-to-fix', 'ok', 'sha256:code-to-fix', datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    for (id, title, work_type) in [
        ("item-code-before-fix", "Code outcome", "code"),
        ("item-ready-fix", "Ready fix outcome", "fix"),
    ] {
        conn.execute(
            "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'ordinary outcome', 'ready', ?4, ?5, datetime('now'), datetime('now'))",
            rusqlite::params![id, project_id, title, work_type, plan_path.to_string_lossy()],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO links(from_item, to_item, kind, condition)
         VALUES ('item-code-before-fix', 'item-ready-fix', 'blocks', 'all')",
        [],
    )
    .unwrap();
    drop(conn);
    init_git_repo(dir.path());

    let picked = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "maker-code-to-fix")
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "pick",
                "--plan",
                "plan-code-to-fix",
                "--work-type",
                "code",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(picked["item"]["id"], "item-code-before-fix");

    let done = single_json_document(
        &planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", "maker-code-to-fix")
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "done",
                "item-code-before-fix",
                "--summary",
                "code outcome settled",
                "--cmd",
                "true",
                "--next",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(done["next"]["item"]["id"], "item-ready-fix");
    assert_eq!(done["next"]["item"]["work_type"], "fix");
    assert_eq!(done["next"]["item"]["status"], "picked");
    assert_eq!(done["next"]["item"]["worker_id"], "maker-code-to-fix");
    let run = &done["work_packet"]["execution_state"]["feature_run"];
    assert_eq!(run["status"], "active");
    assert_eq!(run["phase"], "implementation");
    assert_eq!(run["outcomes_settled"], 1);
    assert_eq!(run["batch_outcome_count"], 1);
    let conn = Connection::open(&db).unwrap();
    let freezes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM feature_run_source_freezes",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(freezes, 0);
}

#[test]
fn itemless_verification_lifecycle_settles_admits_review_and_exhausts() {
    let invoke = |dir: &Path, db: &Path, worker: &str, args: &[&str], code: i32| {
        let mut command = planr();
        command
            .current_dir(dir)
            .env("PLANR_WORKER_ID", worker)
            .args(["--db", db.to_str().unwrap(), "--json"])
            .args(args);
        single_json_document(&command.assert().code(code).get_output().stdout)
    };
    let fixture = |one_shot: bool| {
        let dir = tempdir().unwrap();
        let db = dir.path().join("planr.sqlite");
        fs::write(dir.path().join(".gitignore"), "planr.sqlite*\n").unwrap();
        write_evidence_policy_fixture(dir.path());
        if one_shot {
            let manifest_digest = rewrite_evidence_runner_manifest(dir.path(), |manifest| {
                manifest["repeatability"] = json!("non_repeatable_one_shot");
                manifest["availability_probe"]["execution"]["args"] = json!([
                    "-c",
                    "if [ -z \"$PLANR_EVIDENCE_TARGET_JSON\" ]; then printf '{\"status\":\"ok\"}'; else printf 'not-json'; fi"
                ]);
                let execution = manifest["availability_probe"]["execution"].clone();
                manifest["adapter_digest"] = json!(process_adapter_digest(&execution, vec![]));
            });
            let manifest: Value = serde_json::from_slice(
                &fs::read(
                    dir.path()
                        .join(".planr/evidence/adapters/runner.manifest.json"),
                )
                .unwrap(),
            )
            .unwrap();
            rewrite_evidence_policy_fixture(dir.path(), |policy| {
                policy["adapter_registrations"][0]["manifest_digest"] = json!(manifest_digest);
                policy["adapter_registrations"][0]["execution_contract"] =
                    manifest["availability_probe"]["execution"].clone();
            });
        }
        init_evidence_project(dir.path(), &db, "Itemless Verification Lifecycle");
        let plan_path = dir
            .path()
            .join(".planr/plans/build/evidence-public-fixture.plan.md");
        fs::create_dir_all(plan_path.parent().unwrap()).unwrap();
        fs::write(&plan_path, "---\ncriteria:\n  - id: crit-pob-itemless\n    title: Itemless lifecycle\n---\n# Itemless lifecycle\n").unwrap();
        let mut obligation = evidence_obligation(
            "pob-itemless",
            "unused",
            json!({"kind": "local", "id": "itemless"}),
        );
        obligation["observations"][0]["payload_schema"] =
            json!({"schema_ref": "schema://com.example.health.status"});
        add_evidence_obligation_value(dir.path(), &db, "pob-itemless", &obligation);
        init_git_repo(dir.path());
        let source = evidence_source_binding(dir.path());
        let conn = Connection::open(&db).unwrap();
        let project_id: String = conn
            .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
            .unwrap();
        let contract = test_unbounded_feature_run_contract("run-itemless");
        conn.execute("INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, source_revision, active_batch_id, outcomes_settled, batch_outcome_count, revision, budget_contract_digest) VALUES ('run-itemless', ?1, 'pln-evidence-public', 'active', 'source_frozen', 'sha256:policy', ?2, NULL, 1, 0, 0, ?3)", rusqlite::params![project_id, source["revision"].as_str().unwrap(), contract.digest]).unwrap();
        insert_test_feature_run_contract(&conn, &contract);
        conn.execute("INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, completed_at, created_at, updated_at) VALUES ('item-settled', ?1, 'Settled', 'settled', 'closed', 'code', 'maker-itemless', ?2, datetime('now'), datetime('now'), datetime('now'))", rusqlite::params![project_id, plan_path.to_string_lossy()]).unwrap();
        conn.execute("INSERT INTO feature_run_source_freezes(id, run_id, source_revision, source_digest, status) VALUES ('freeze-itemless', 'run-itemless', ?1, ?2, 'active')", rusqlite::params![source["revision"].as_str().unwrap(), source["tree_digest"].as_str().unwrap()]).unwrap();
        conn.execute("INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation, released_at) VALUES ('run-itemless', 'maker', 'maker-itemless', 1, datetime('now'))", []).unwrap();
        drop(conn);
        (dir, db)
    };
    let (passing_dir, passing_db) = fixture(false);
    let pick = invoke(
        passing_dir.path(),
        &passing_db,
        "verifier-itemless",
        &[
            "pick",
            "--plan",
            "pln-evidence-public",
            "--work-type",
            "verification",
        ],
        0,
    );
    assert_eq!(pick["work_packet"]["item_id"], Value::Null);
    assert_eq!(
        pick["work_packet"]["sealed_run_index"]["schema_version"],
        "planr.evidence.run-index.v2"
    );
    let run_path = pick["work_packet"]["sealed_run_index"]["repository_path"]
        .as_str()
        .unwrap();
    assert_eq!(
        invoke(
            passing_dir.path(),
            &passing_db,
            "verifier-itemless",
            &["evidence", "run", "--input", run_path],
            0
        )["object"]["verdict"],
        "passed"
    );
    let conn = Connection::open(&passing_db).unwrap();
    conn.execute("INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at) SELECT 'item-ready-unleased', project_id, 'Ready verifier', 'projection', 'ready', 'verification', path, datetime('now'), datetime('now') FROM plans WHERE id = 'pln-evidence-public'", []).unwrap();
    drop(conn);
    let before = (
        evidence_row_count(&passing_db, "evidence_attempts"),
        evidence_row_count(&passing_db, "evidence_receipts"),
    );
    let blocked = invoke(
        passing_dir.path(),
        &passing_db,
        "verifier-itemless",
        &[
            "evidence",
            "coverage",
            "--scope",
            "plan",
            "--id",
            "pln-evidence-public",
        ],
        1,
    );
    assert!(
        blocked["error"]["message"]
            .as_str()
            .unwrap()
            .contains("verification_coverage_requires_verification_item_lease")
    );
    assert_eq!(
        before,
        (
            evidence_row_count(&passing_db, "evidence_attempts"),
            evidence_row_count(&passing_db, "evidence_receipts")
        )
    );
    let conn = Connection::open(&passing_db).unwrap();
    let blocked_state: (String, i64, i64) = conn.query_row("SELECT phase, (SELECT COUNT(*) FROM feature_run_role_leases WHERE run_id = feature_runs.id AND role = 'verifier' AND released_at IS NULL), (SELECT COUNT(*) FROM logs WHERE item_id = 'item-ready-unleased') FROM feature_runs WHERE id = 'run-itemless'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    assert_eq!(blocked_state, ("verification".into(), 1, 0));
    conn.execute(
        "UPDATE items SET status = 'cancelled' WHERE id = 'item-ready-unleased'",
        [],
    )
    .unwrap();
    drop(conn);
    let coverage = invoke(
        passing_dir.path(),
        &passing_db,
        "verifier-itemless",
        &[
            "evidence",
            "coverage",
            "--scope",
            "plan",
            "--id",
            "pln-evidence-public",
        ],
        0,
    );
    let settlement = &coverage["object"]["feature_run_verification_settlement"];
    assert_eq!(
        (
            settlement["item_id"].clone(),
            settlement["log_id"].clone(),
            settlement["phase"].clone()
        ),
        (Value::Null, Value::Null, json!("source_frozen"))
    );
    let review = invoke(
        passing_dir.path(),
        &passing_db,
        "reviewer-itemless",
        &["plan", "final-review", "pln-evidence-public"],
        0,
    );
    assert_eq!(
        review["execution_state"]["review_source_binding"]["freeze_id"],
        "freeze-itemless"
    );
    assert_eq!(
        review["execution_state"]["feature_run"]["phase"],
        "source_frozen"
    );
    assert_eq!(review["created"], true);
    let gate_id = review["execution_state"]["review_gate"]["id"]
        .as_str()
        .unwrap();
    assert_eq!(
        invoke(
            passing_dir.path(),
            &passing_db,
            "reviewer-itemless",
            &[
                "pick",
                "--plan",
                "pln-evidence-public",
                "--work-type",
                "review"
            ],
            0
        )["work_packet"]["execution_state"]["review_gate"]["id"],
        gate_id
    );
    invoke(
        passing_dir.path(),
        &passing_db,
        "reviewer-itemless",
        &[
            "review",
            "close",
            gate_id,
            "--verdict",
            "complete",
            "--reviewer",
            "reviewer-itemless",
        ],
        0,
    );
    let shown = invoke(
        passing_dir.path(),
        &passing_db,
        "reviewer-itemless",
        &["plan", "final-review", "pln-evidence-public"],
        0,
    );
    assert_eq!(shown["created"], false);
    assert_eq!(shown["execution_state"]["review_gate"]["id"], gate_id);
    assert_eq!(
        shown["execution_state"]["review_gate"]["status"],
        "accepted"
    );
    assert_eq!(shown["execution_state"]["feature_run"]["phase"], "complete");
    let (exhausted_dir, exhausted_db) = fixture(true);
    let pick = invoke(
        exhausted_dir.path(),
        &exhausted_db,
        "verifier-itemless",
        &[
            "pick",
            "--plan",
            "pln-evidence-public",
            "--work-type",
            "verification",
        ],
        0,
    );
    let run_path = pick["work_packet"]["sealed_run_index"]["repository_path"]
        .as_str()
        .unwrap();
    let exhausted = invoke(
        exhausted_dir.path(),
        &exhausted_db,
        "verifier-itemless",
        &["evidence", "run", "--input", run_path],
        2,
    );
    assert_eq!(
        exhausted["object"]["terminal_exhaustion"]["item"],
        Value::Null
    );
    assert!(
        exhausted["object"]["results"][0]["attempt"]["id"].is_string()
            && exhausted["object"]["results"][0]["receipt"]["id"].is_string()
    );
    assert_eq!(
        (
            evidence_row_count(&exhausted_db, "evidence_attempts"),
            evidence_row_count(&exhausted_db, "evidence_receipts")
        ),
        (1, 1)
    );
    let conn = Connection::open(&exhausted_db).unwrap();
    let exhausted_state: (String, i64, i64) = conn.query_row("SELECT phase, (SELECT COUNT(*) FROM feature_run_role_leases WHERE run_id = feature_runs.id AND released_at IS NULL), (SELECT COUNT(*) FROM feature_run_budget_reservations WHERE run_id = feature_runs.id AND phase = 'verification' AND status = 'active') FROM feature_runs WHERE id = 'run-itemless'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    assert_eq!(exhausted_state, ("cancelled".into(), 0, 0));
}

#[test]
fn done_next_stops_at_review_gate_without_leasing_more_work() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Review stop",
        ])
        .assert()
        .success();
    let plan_path = dir.path().join("review-stop.plan.md");
    fs::write(&plan_path, "# Review Stop\n").unwrap();
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('plan-review-stop', ?1, 'build', ?2, 'Review Stop', 'review-stop', 'ok', 'sha256:review-stop', datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    for (id, status, worker) in [
        ("item-review-stop-1", "picked", Some("maker-review-stop")),
        ("item-review-stop-2", "ready", None),
    ] {
        conn.execute(
            "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at)
             VALUES (?1, ?2, ?1, 'review stop outcome', ?3, 'code', ?4, ?5, datetime('now'), datetime('now'))",
            rusqlite::params![id, project_id, status, worker, plan_path.to_string_lossy()],
        )
        .unwrap();
    }
    drop(conn);

    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-review-stop")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            "item-review-stop-1",
            "--summary",
            "protected behavior changed",
            "--cmd",
            "true",
            "--next",
            "--escalate",
            "protected-risk-discovered",
            "--escalation-ref",
            "risk:test",
            "--escalation-explanation",
            "focused review-gate regression",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(done["work_packet"]["transition"], "review_gate");
    assert_eq!(done["next"]["item"], Value::Null);
    assert_eq!(
        done["next"]["reason"],
        "review_gate_pending_independent_lease"
    );

    let conn = Connection::open(&db).unwrap();
    assert_eq!(item_status(&db, "item-review-stop-2"), "ready");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM review_gates", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM execution_batches", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn done_next_freezes_source_without_authored_verification_item() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Verification Handoff",
        ])
        .assert()
        .success();
    let plan_path = dir.path().join("verification-handoff.plan.md");
    fs::write(
        &plan_path,
        "---\ncriteria:\n  - id: criterion-verification-handoff-without-item\n    title: Verification handoff without item\n---\n\n# Verification Handoff\n",
    )
    .unwrap();
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('plan-verification-handoff', ?1, 'build', ?2, 'Verification Handoff', 'verification-handoff', 'ok', 'sha256:verification-handoff', datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at)
         VALUES ('item-last-code-before-verification', ?1, 'Last code', 'settle implementation', 'picked', 'code', 'maker-verification-handoff', ?2, datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    drop(conn);
    let mut obligation = evidence_obligation(
        "pob-verification-handoff-without-item",
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        json!({"kind": "local", "id": "verification-handoff"}),
    );
    obligation["plan_id"] = json!("plan-verification-handoff");
    obligation["item_id"] = Value::Null;
    obligation["criterion_id"] = json!("criterion-verification-handoff-without-item");
    add_evidence_obligation_value(
        dir.path(),
        &db,
        "pob-verification-handoff-without-item",
        &obligation,
    );
    init_git_repo(dir.path());
    let source_freeze_planr = private_planr_binary(dir.path());

    let hostile_bin = dir.path().join("installed-old-global/bin");
    fs::create_dir_all(&hostile_bin).unwrap();
    let hostile_planr = hostile_bin.join("planr");
    fs::write(
        &hostile_planr,
        "#!/bin/sh\necho 'hostile PATH planr was invoked' >&2\nexit 97\n",
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(&hostile_planr, fs::Permissions::from_mode(0o755)).unwrap();
    let hostile_path = std::env::join_paths(std::iter::once(hostile_bin.clone()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();

    let output = planr_from_binary(&source_freeze_planr)
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-verification-handoff")
        .env("PATH", hostile_path)
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            "item-last-code-before-verification",
            "--summary",
            "settled final implementation before verifier",
            "--cmd",
            "true",
            "--tests",
            "true",
            "--next",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(done["next"]["item"], Value::Null);
    assert_eq!(done["next"]["reason"], "verification_handoff_source_frozen");
    assert_eq!(done["next"]["work_packet"]["kind"], "verification_handoff");
    assert_eq!(
        done["next"]["work_packet"]["verification_item_id"],
        Value::Null
    );
    let packet = &done["next"]["work_packet"];
    let identity = &packet["planr_executable"];
    let executable = Path::new(identity["path"].as_str().unwrap());
    assert!(executable.is_absolute());
    assert_ne!(executable, hostile_planr);
    let executable_bytes = fs::read(executable).unwrap();
    assert_eq!(
        identity["sha256"],
        format!("sha256:{:x}", Sha256::digest(&executable_bytes))
    );
    assert_eq!(identity["size_bytes"], executable_bytes.len() as u64);
    assert_eq!(identity["path_lookup_allowed"], false);
    assert_eq!(
        packet["commands"]["lease_verifier"],
        json!({
            "schema_version": "planr.command.v1",
            "executable": identity["path"],
            "executable_sha256": identity["sha256"],
            "path_lookup_allowed": false,
            "argv": ["pick", "--plan", "plan-verification-handoff", "--work-type", "verification", "--json"]
        })
    );
    assert_eq!(
        packet["commands"]["readiness"],
        json!({
            "schema_version": "planr.command.v1",
            "executable": identity["path"],
            "executable_sha256": identity["sha256"],
            "path_lookup_allowed": false,
            "argv": ["evidence", "readiness", "--scope", "plan", "--id", "plan-verification-handoff", "--json"]
        })
    );
    assert!(!packet["commands"].to_string().contains("planr pick"));
    assert_eq!(
        done["next"]["work_packet"]["source_freeze"]["feature_run"]["phase"],
        "source_frozen"
    );

    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-verification-handoff")
        .args([
            "--db",
            db.to_str().unwrap(),
            "pick",
            "--plan",
            "plan-verification-handoff",
            "--work-type",
            "verification",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "verification_requires_fresh_independent_worker",
        ));
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "verifier-verification-handoff")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "--plan",
            "plan-verification-handoff",
            "--work-type",
            "verification",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            "evidence readiness requires .planr/evidence.yaml",
        ));
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "reviewer-too-early")
        .args([
            "--db",
            db.to_str().unwrap(),
            "plan",
            "final-review",
            "plan-verification-handoff",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "final_product_review_requires_settled_exact_source_coverage:phase=held",
        ));

    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM items WHERE work_type = 'verification'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let phase: String = conn
        .query_row(
            "SELECT phase FROM feature_runs WHERE plan_id = 'plan-verification-handoff'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(phase, "held");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM feature_run_source_freezes",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM execution_batches WHERE status = 'ended'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn binding_policy_without_obligations_holds_before_review() {
    complete_binding_single_owner_inventory_keeps_adapter_at_boundary();
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Binding Evidence Hold",
        ])
        .assert()
        .success();
    write_evidence_policy_fixture(dir.path());
    let plan_path = dir.path().join("binding-evidence-hold.plan.md");
    fs::write(
        &plan_path,
        "---\ncriteria:\n  - id: criterion-binding-evidence-hold\n    title: Binding evidence hold\n---\n\n# Binding Evidence Hold\n",
    )
    .unwrap();
    let conn = Connection::open(&db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('plan-binding-evidence-hold', ?1, 'build', ?2, 'Binding Evidence Hold', 'binding-evidence-hold', 'ok', 'sha256:binding-evidence-hold', datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
         VALUES ('item-binding-evidence-code', ?1, 'Binding evidence code', 'settle implementation', 'ready', 'code', ?2, datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    drop(conn);

    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-binding-evidence")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "pick",
            "--plan",
            "plan-binding-evidence-hold",
            "--work-type",
            "code",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pick: Value = serde_json::from_slice(&output).unwrap();
    let packet = &pick["work_packet"];
    assert_eq!(packet["kind"], "hold");
    assert_eq!(packet["proof"]["status"], "binding_unsatisfied");
    assert_eq!(
        packet["proof"]["actionable_gaps"][0]["code"],
        "missing_obligation"
    );

    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "close",
            "item-binding-evidence-code",
            "--summary",
            "must not close before migration",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "binding_evidence_obligations_missing",
        ));

    let audit = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "audit",
            "plan-binding-evidence-hold",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let audit: Value = serde_json::from_slice(&audit).unwrap();
    assert_eq!(audit["holds"], false);
    assert_eq!(audit["proof"]["status"], "binding_unsatisfied");
    let evidence_clause = audit["clauses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clause| clause["clause"] == "verification_logged")
        .unwrap();
    assert_eq!(evidence_clause["authority"], "evidence_policy");
    assert_eq!(evidence_clause["pass"], false);

    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT status FROM items WHERE id = 'item-binding-evidence-code'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "ready"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM feature_runs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM review_gates WHERE kind = 'final_product'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

fn seed_transport_risk_handoff(root: &Path, db: &Path) -> (String, String) {
    planr()
        .current_dir(root)
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Transport handoff",
        ])
        .assert()
        .success();
    let plan_path = root.join("transport-handoff.plan.md");
    fs::write(
        &plan_path,
        "---\nname: transport-handoff\ncriteria:\n  - id: crit-transport\n    title: Transport verification\n---\n# Transport Handoff\n",
    )
    .unwrap();
    let conn = Connection::open(db).unwrap();
    let project_id: String = conn
        .query_row("SELECT id FROM projects LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
         VALUES ('plan-transport-handoff', ?1, 'build', ?2, 'Transport Handoff', 'transport-handoff', 'ok', 'sha256:transport', datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at)
         VALUES ('item-transport-code', ?1, 'Code', 'protected transport change', 'picked', 'code', 'maker-transport', ?2, datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
         VALUES ('item-transport-verification', ?1, 'Verification', 'independent verification', 'ready', 'verification', ?2, datetime('now'), datetime('now'))",
        rusqlite::params![project_id, plan_path.to_string_lossy()],
    )
    .unwrap();
    drop(conn);
    init_git_repo(root);
    let done = single_json_document(
        &planr()
            .current_dir(root)
            .env("PLANR_WORKER_ID", "maker-transport")
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "done",
                "item-transport-code",
                "--summary",
                "protected handoff transport",
                "--cmd",
                "true",
                "--escalate",
                "user-requested",
                "--escalation-ref",
                "transport:test",
                "--escalation-explanation",
                "transport parity",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let gate = done["work_packet"]["review_gate"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut obligation = bind_obligation_to_authored_criterion(
        evidence_obligation_for(
            "pob-transport-handoff",
            "",
            "com.example.health.status",
            "transport obligation",
            json!({"status": "ok"}),
            json!({"kind": "process", "uri": "local://health"}),
            Value::Null,
            Value::Null,
            json!([]),
            "",
        ),
        "plan-transport-handoff",
        "crit-transport",
    );
    obligation["item_id"] = json!("item-transport-verification");
    add_evidence_obligation_value(root, db, "pob-transport-handoff", &obligation);
    planr()
        .current_dir(root)
        .env("PLANR_WORKER_ID", "reviewer-transport")
        .args([
            "--db",
            db.to_str().unwrap(),
            "pick",
            "--plan",
            "plan-transport-handoff",
            "--work-type",
            "review",
        ])
        .assert()
        .success();
    ("plan-transport-handoff".to_string(), gate)
}

fn assert_typed_handoff_transport(value: &Value, plan_id: &str) {
    let packet = &value["verification_handoff"]["work_packet"];
    assert_eq!(packet["schema_version"], "planr.verification_handoff.v2");
    let identity = &packet["planr_executable"];
    let executable = Path::new(identity["path"].as_str().unwrap());
    assert!(executable.is_absolute());
    assert_eq!(identity["path_lookup_allowed"], false);
    assert_eq!(
        identity["sha256"],
        format!("sha256:{:x}", Sha256::digest(fs::read(executable).unwrap()))
    );
    for command in ["lease_verifier", "readiness"] {
        assert_eq!(packet["commands"][command]["executable"], identity["path"]);
        assert_eq!(
            packet["commands"][command]["executable_sha256"],
            identity["sha256"]
        );
        assert_eq!(packet["commands"][command]["path_lookup_allowed"], false);
    }
    assert_eq!(
        packet["commands"]["lease_verifier"]["argv"],
        json!([
            "pick",
            "--plan",
            plan_id,
            "--work-type",
            "verification",
            "--json"
        ])
    );
    assert!(!packet["commands"].to_string().contains("planr pick"));
}

#[test]
fn canonical_handoff_identity_is_real_and_path_independent_across_cli_mcp_http() {
    let hostile = tempdir().unwrap();
    let fake = hostile.path().join("planr");
    fs::write(&fake, "#!/bin/sh\nexit 97\n").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let hostile_path = std::env::join_paths(std::iter::once(hostile.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();

    let cli_root = tempdir().unwrap();
    let cli_db = cli_root.path().join(".planr/planr.sqlite");
    let (plan_id, cli_gate) = seed_transport_risk_handoff(cli_root.path(), &cli_db);
    let cli_planr = private_planr_binary(cli_root.path());
    let cli = single_json_document(
        &planr_from_binary(&cli_planr)
            .current_dir(cli_root.path())
            .env("PLANR_WORKER_ID", "reviewer-transport")
            .env("PATH", &hostile_path)
            .args([
                "--db",
                cli_db.to_str().unwrap(),
                "--json",
                "review",
                "close",
                &cli_gate,
                "--verdict",
                "complete",
                "--reviewer",
                "reviewer-transport",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_typed_handoff_transport(&cli, &plan_id);

    let mcp_root = tempdir().unwrap();
    let mcp_db = mcp_root.path().join(".planr/planr.sqlite");
    let (_, mcp_gate) = seed_transport_risk_handoff(mcp_root.path(), &mcp_db);
    let mcp_planr = private_planr_binary(mcp_root.path());
    let mcp = mcp_tool_response_with_env_and_binary(
        mcp_root.path(),
        &mcp_db,
        1,
        "planr_review_gate_close",
        json!({"review_gate_id": mcp_gate, "verdict": "complete", "reviewer": "reviewer-transport"}),
        &[
            ("PLANR_WORKER_ID", "reviewer-transport"),
            ("PATH", hostile_path.to_str().unwrap()),
        ],
        Some(&mcp_planr),
    );
    assert_typed_handoff_transport(&mcp_text_value(&mcp), &plan_id);

    let http_root = tempdir().unwrap();
    let http_db = http_root.path().join(".planr/planr.sqlite");
    let (_, http_gate) = seed_transport_risk_handoff(http_root.path(), &http_db);
    let http_planr = private_planr_binary(http_root.path());
    let port = free_port();
    let mut server = std_planr_from_binary(&http_planr)
        .current_dir(http_root.path())
        .env("PLANR_WORKER_ID", "reviewer-transport")
        .env("PATH", &hostile_path)
        .args([
            "--db",
            http_db.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for_http_server(port);
    let http = http_json(&http_request(
        port,
        "POST",
        &format!("/v1/review-gates/{http_gate}/close"),
        &json!({"verdict":"complete", "reviewer":"reviewer-transport"}).to_string(),
    ));
    let _ = server.kill();
    let _ = server.wait();
    assert_typed_handoff_transport(&http, &plan_id);
}

#[test]
fn unplanned_materiality_records_missing_change_facts_without_a_review_gate() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Uncertain"])
        .assert()
        .success();
    write_materiality_policy(dir.path());
    let item = create_test_item(dir.path(), &db, "No files", "missing facts");

    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-uncertain")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &item,
            "--summary",
            "implemented but forgot file evidence",
            "--cmd",
            "cargo test focused",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(done["item"]["status"], "closed");
    assert_eq!(done["work_packet"]["review_gate"], Value::Null);
    assert_eq!(done["materiality"]["effective_review"]["required"], false);
    assert!(
        done["materiality"]["decision"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "missing_changed_files")
    );
}

#[test]
fn done_elevates_uncorroborated_file_facts_under_materiality_policy() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Facts"])
        .assert()
        .success();
    write_materiality_policy(dir.path());

    let no_git_item = create_test_item(dir.path(), &db, "No git", "unknown changed lines");
    let no_git = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-no-git")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &no_git_item,
            "--summary",
            "docs update",
            "--files",
            "docs/README.md",
            "--cmd",
            "true",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let no_git: Value = serde_json::from_slice(&no_git).unwrap();
    assert!(
        no_git["materiality"]["decision"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "changed_lines_unknown:git_error"),
        "{no_git}"
    );

    init_materiality_git(dir.path());
    fs::create_dir_all(dir.path().join("docs")).unwrap();
    fs::create_dir_all(dir.path().join("src/app")).unwrap();
    let missing_item = create_test_item(dir.path(), &db, "Missing file", "missing path");
    let missing = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-missing-file")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &missing_item,
            "--summary",
            "claimed missing file",
            "--files",
            "docs/missing.md",
            "--cmd",
            "true",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let missing: Value = serde_json::from_slice(&missing).unwrap();
    assert!(
        missing["materiality"]["decision"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "missing_claimed_file:docs/missing.md"),
        "{missing}"
    );

    fs::write(dir.path().join("docs/untracked.md"), "new untracked docs\n").unwrap();
    let untracked_item = create_test_item(
        dir.path(),
        &db,
        "Ordinary added file",
        "ordinary untracked path",
    );
    let untracked = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-untracked-file")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &untracked_item,
            "--summary",
            "claimed untracked file",
            "--files",
            "docs/untracked.md",
            "--cmd",
            "true",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let untracked: Value = serde_json::from_slice(&untracked).unwrap();
    assert_eq!(untracked["item"]["status"], "closed", "{untracked}");
    assert_eq!(untracked["work_packet"]["review_gate"], Value::Null);
    assert_eq!(
        untracked["materiality"]["change_summary"]["changed_lines"], 1,
        "{untracked}"
    );
    assert!(
        untracked["materiality"]["decision"]["reasons"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{untracked}"
    );

    fs::write(
        dir.path().join("src/app/http_added.rs"),
        "pub fn route() {}\n",
    )
    .unwrap();
    let protected_item = create_test_item(
        dir.path(),
        &db,
        "Protected added file",
        "protected untracked path",
    );
    let protected = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-protected-untracked-file")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &protected_item,
            "--summary",
            "claimed protected added file",
            "--files",
            "src/app/http_added.rs",
            "--cmd",
            "true",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let protected: Value = serde_json::from_slice(&protected).unwrap();
    assert_eq!(protected["item"]["status"], "closed", "{protected}");
    assert_eq!(protected["work_packet"]["review_gate"], Value::Null);
    assert!(
        protected["materiality"]["decision"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "risk:high"),
        "{protected}"
    );
    assert!(
        protected["materiality"]["change_summary"]["triggers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|trigger| trigger == "public_api"),
        "{protected}"
    );

    let large_docs = "line\n".repeat(1001);
    fs::write(dir.path().join("docs/large-untracked.md"), large_docs).unwrap();
    let large_item = create_test_item(dir.path(), &db, "Large added file", "large untracked path");
    let large = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-large-untracked-file")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &large_item,
            "--summary",
            "claimed large added file",
            "--files",
            "docs/large-untracked.md",
            "--cmd",
            "true",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let large: Value = serde_json::from_slice(&large).unwrap();
    assert_eq!(large["item"]["status"], "closed", "{large}");
    assert_eq!(large["work_packet"]["review_gate"], Value::Null);
    assert_eq!(
        large["materiality"]["change_summary"]["changed_lines"], 1001,
        "{large}"
    );
    assert!(
        large["materiality"]["decision"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "changed_lines_threshold:1001"),
        "{large}"
    );

    let unchanged_item = create_test_item(dir.path(), &db, "Unchanged file", "unchanged path");
    let unchanged = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-unchanged-file")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &unchanged_item,
            "--summary",
            "claimed unchanged file",
            "--files",
            "docs/README.md",
            "--cmd",
            "true",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let unchanged: Value = serde_json::from_slice(&unchanged).unwrap();
    assert!(
        unchanged["materiality"]["decision"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "missing_claimed_file:docs/README.md"),
        "{unchanged}"
    );
}

#[test]
fn unplanned_done_records_missing_materiality_policy_without_a_review_gate() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "No policy"])
        .assert()
        .success();
    let item = create_test_item(dir.path(), &db, "No policy item", "missing policy");

    let output = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-no-policy")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &item,
            "--summary",
            "completed without policy",
            "--files",
            "docs/README.md",
            "--cmd",
            "true",
            "--next",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(done["item"]["status"], "closed");
    assert_eq!(done["work_packet"]["review_gate"], Value::Null);
    assert_eq!(done["next"]["reason"], "all_settled");
    assert_eq!(done["materiality"]["policy"]["reason"], "missing");
    assert_eq!(
        done["materiality"]["effective_review"]["reason"],
        "policy_missing_operational_gap"
    );
    assert_eq!(done["materiality"]["decision"]["review"], "none");
}

#[test]
fn unplanned_done_retry_reuses_completion_log_without_review_rows() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Retry"])
        .assert()
        .success();
    write_materiality_policy(dir.path());
    init_materiality_git(dir.path());
    fs::create_dir_all(dir.path().join("src/app")).unwrap();
    fs::write(
        dir.path().join("src/app/retry.rs"),
        "fn initial() {}\nfn retry_route() {}\n",
    )
    .unwrap();

    let item = create_test_item(dir.path(), &db, "Retry item", "retry gate");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "update",
            &item,
            "--work-type",
            "code",
        ])
        .assert()
        .success();
    let first = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-retry")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &item,
            "--summary",
            "changed API route",
            "--files",
            "src/app/retry.rs",
            "--cmd",
            "cargo test retry",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let first: Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(first["work_packet"]["review_gate"], Value::Null);
    let first_log_id = first["log_id"].as_str().unwrap().to_string();

    let retry = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-retry")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &item,
            "--summary",
            "changed API route",
            "--files",
            "src/app/retry.rs",
            "--cmd",
            "cargo test retry",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let retry: Value = serde_json::from_slice(&retry).unwrap();
    assert_eq!(retry["work_packet"]["review_gate"], Value::Null);
    assert_eq!(retry["work_packet"]["transition"], "already_settled");
    assert_eq!(retry["work_packet"]["disposition"], "already_settled");
    assert_eq!(retry["log_id"], first_log_id);

    let conn = Connection::open(&db).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM review_gates WHERE scope_id = ?1",
            [&item],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    let log_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM logs WHERE item_id = ?1 AND kind = 'completion'",
            [&item],
            |row| row.get(0),
        )
        .unwrap();
    let decision_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE item_id = ?1 AND event_type = 'materiality_decided'",
            [&item],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(log_count, 1);
    assert_eq!(decision_count, 1);
    assert_eq!(item_status(&db, &item), "closed");
    let settled = (count, log_count, decision_count, item_status(&db, &item));

    let conflict = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-retry")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &item,
            "--summary",
            "conflicting API route",
            "--files",
            "src/app/retry.rs",
            "--cmd",
            "cargo test retry",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    assert!(
        String::from_utf8_lossy(&conflict).contains("OutcomeSummaryMismatch"),
        "{}",
        String::from_utf8_lossy(&conflict)
    );
    let after_conflict = (
        conn.query_row(
            "SELECT COUNT(*) FROM review_gates WHERE scope_id = ?1",
            [&item],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        conn.query_row(
            "SELECT COUNT(*) FROM logs WHERE item_id = ?1 AND kind = 'completion'",
            [&item],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        conn.query_row(
            "SELECT COUNT(*) FROM events WHERE item_id = ?1 AND event_type = 'materiality_decided'",
            [&item],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        item_status(&db, &item),
    );
    assert_eq!(after_conflict, settled);
}

#[test]
fn done_persists_supplied_settlement_evidence_after_an_earlier_completion_log() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Settlement evidence",
        ])
        .assert()
        .success();
    write_materiality_policy(dir.path());
    init_materiality_git(dir.path());
    fs::create_dir_all(dir.path().join("src/app")).unwrap();
    fs::write(dir.path().join("src/app/flow.rs"), "fn settlement() {}\n").unwrap();

    let item = create_test_item(dir.path(), &db, "Settlement item", "evidence integrity");
    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-settlement-evidence")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item,
            "--kind",
            "completion",
            "--summary",
            "earlier partial completion claim",
            "--files",
            "docs/old.md",
            "--cmd",
            "false",
        ])
        .assert()
        .success();

    let done = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-settlement-evidence")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &item,
            "--summary",
            "canonical settlement evidence",
            "--files",
            "src/app/flow.rs",
            "--cmd",
            "cargo check",
            "--tests",
            "cargo test settlement",
            "--profile",
            "gpt55-coder",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let done: Value = serde_json::from_slice(&done).unwrap();

    let conn = Connection::open(&db).unwrap();
    let (summary, files, commands, tests, profile): (
        String,
        String,
        String,
        String,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT l.summary, l.files, l.commands, l.tests, r.profile FROM logs l LEFT JOIN runs r ON r.id = l.run_id WHERE l.id = ?1",
            [done["log_id"].as_str().unwrap()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(summary, "canonical settlement evidence");
    assert_eq!(
        serde_json::from_str::<Value>(&files).unwrap(),
        json!(["src/app/flow.rs"])
    );
    assert_eq!(
        serde_json::from_str::<Value>(&commands).unwrap(),
        json!(["cargo check"])
    );
    assert_eq!(
        serde_json::from_str::<Value>(&tests).unwrap(),
        json!(["cargo test settlement"])
    );
    assert_eq!(profile.as_deref(), Some("gpt55-coder"));
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM logs WHERE item_id = ?1 AND kind = 'completion'",
            [&item],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        2,
        "the earlier claim and canonical settlement must remain distinct"
    );
}

#[test]
fn materiality_settlement_rolls_back_after_review_gate_failure() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Rollback"])
        .assert()
        .success();
    write_materiality_policy(dir.path());
    init_materiality_git(dir.path());
    fs::create_dir_all(dir.path().join("src/app")).unwrap();
    fs::write(
        dir.path().join("src/app/concurrent.rs"),
        "fn initial() {}\nfn rollback_route() {}\n",
    )
    .unwrap();
    let item = create_test_item(dir.path(), &db, "Rollback item", "partial failure");

    planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker-rollback")
        .env("PLANR_TEST_FAIL_AFTER_REVIEW_GATE", "1")
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "done",
            &item,
            "--summary",
            "changed API route",
            "--files",
            "src/app/http.rs",
            "--cmd",
            "cargo test rollback",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("injected_failure"));

    let conn = Connection::open(&db).unwrap();
    let log_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM logs WHERE item_id = ?1",
            [&item],
            |row| row.get(0),
        )
        .unwrap();
    let review_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM review_gates WHERE scope_id = ?1",
            [&item],
            |row| row.get(0),
        )
        .unwrap();
    let event_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE item_id = ?1 AND event_type IN ('log_created','review_gate_opened','materiality_decided')",
            [&item],
            |row| row.get(0),
        )
        .unwrap();
    let metadata: Option<String> = conn
        .query_row("SELECT metadata FROM items WHERE id = ?1", [&item], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(log_count, 0);
    assert_eq!(review_count, 0);
    assert_eq!(event_count, 0);
    assert!(metadata.is_none(), "{metadata:?}");
    assert_eq!(item_status(&db, &item), "ready");
}

#[test]
fn concurrent_unplanned_settlement_creates_one_log_and_no_review_rows() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "project",
            "init",
            "Concurrent",
        ])
        .assert()
        .success();
    write_materiality_policy(dir.path());
    init_materiality_git(dir.path());
    fs::create_dir_all(dir.path().join("src/app")).unwrap();
    fs::write(
        dir.path().join("src/app/http.rs"),
        "fn initial() {}\nfn concurrent_route() {}\n",
    )
    .unwrap();
    let item = create_test_item(dir.path(), &db, "Concurrent item", "duplicate gate race");
    planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "item",
            "update",
            &item,
            "--work-type",
            "code",
        ])
        .assert()
        .success();

    let bin = assert_cmd::cargo::cargo_bin("planr");
    let mut children = Vec::new();
    for _ in 0..2 {
        children.push(
            StdCommand::new(&bin)
                .current_dir(dir.path())
                .env("PLANR_WORKER_ID", "maker-concurrent")
                .args([
                    "--db",
                    db.to_str().unwrap(),
                    "--json",
                    "done",
                    &item,
                    "--summary",
                    "changed API route",
                    "--files",
                    "src/app/concurrent.rs",
                    "--cmd",
                    "cargo test concurrent",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    let mut settlements = Vec::new();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "stdout:{}\nstderr:{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        settlements.push(serde_json::from_slice::<Value>(&output.stdout).unwrap());
    }
    assert_eq!(
        settlements
            .iter()
            .filter(|value| value["work_packet"]["transition"] == "legacy_unplanned")
            .count(),
        1
    );
    assert_eq!(
        settlements
            .iter()
            .filter(|value| value["work_packet"]["transition"] == "already_settled"
                && value["work_packet"]["disposition"] == "already_settled")
            .count(),
        1
    );
    let first_log_id = settlements[0]["log_id"].as_str().unwrap();
    assert!(!first_log_id.is_empty());
    assert_eq!(settlements[1]["log_id"].as_str(), Some(first_log_id));

    let conn = Connection::open(&db).unwrap();
    let log_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM logs WHERE item_id = ?1 AND kind = 'completion'",
            [&item],
            |row| row.get(0),
        )
        .unwrap();
    let review_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM review_gates WHERE scope_id = ?1",
            [&item],
            |row| row.get(0),
        )
        .unwrap();
    let decision_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE item_id = ?1 AND event_type = 'materiality_decided'",
            [&item],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(log_count, 1);
    assert_eq!(review_count, 0);
    assert_eq!(decision_count, 1);
    assert_eq!(item_status(&db, &item), "closed");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM feature_runs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM execution_run_outcomes",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
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
    write_materiality_policy(dir.path());
    init_materiality_git(dir.path());
    fs::create_dir_all(dir.path().join("docs")).unwrap();
    fs::write(dir.path().join("docs/README.md"), "child setup facts\n").unwrap();
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
            "log",
            "add",
            "--item",
            &children[0],
            "--summary",
            "child a docs fact",
            "--files",
            "docs/README.md",
            "--cmd",
            "true",
        ])
        .assert()
        .success();
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
            "log",
            "add",
            "--item",
            &children[1],
            "--summary",
            "child b docs fact",
            "--files",
            "docs/README.md",
            "--cmd",
            "true",
        ])
        .assert()
        .success();
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
            "log",
            "add",
            "--item",
            &partial_children[0],
            "--summary",
            "kept child docs fact",
            "--files",
            "docs/README.md",
            "--cmd",
            "true",
        ])
        .assert()
        .success();
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
    wait_for_http_server(port);
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
    wait_for_http_server(port);

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
    let screenshot_id = screenshot["artifact"]["id"].as_str().unwrap();
    assert_eq!(
        screenshot["artifact"]["mime_type"], "image/png",
        "mime must be inferred from the path extension: {screenshot}"
    );
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE artifacts SET created_at = '2026-07-26 12:00:00' WHERE item_id = ?1",
            [item_id],
        )
        .unwrap();

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
    let artifacts = artifacts["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 2, "both created artifacts must be listed");
    assert_eq!(
        artifacts[0]["id"], screenshot_id,
        "the most recently inserted artifact must win a created_at tie"
    );
    assert_eq!(artifacts[1]["id"], artifact_id);

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
    wait_for_http_server(port);
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
    let http_events = http_json(&http_request(port, "GET", "/v1/events", ""));
    let http_event_types = http_events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["event_type"].as_str())
        .collect::<Vec<_>>();
    assert!(http_event_types.contains(&"artifact_created"));
    assert!(http_event_types.contains(&"log_created"));
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
    wait_for_http_server(port);
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
    assert!(
        loop_skill.contains("planr stop activate --plan <plan-id>"),
        "planr-loop must activate Stop enforcement for the active goal plan"
    );
    assert!(
        loop_skill.contains("CODEX_THREAD_ID"),
        "planr-loop must bind Codex Stop enforcement to the host thread id"
    );
    assert!(
        loop_skill.contains("planr stop deactivate --plan <plan-id>"),
        "planr-loop must deactivate Stop enforcement on cancel or handoff"
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
fn grok_host_surfaces_are_explicit_portable_and_opt_in() {
    let all_dir = tempdir().unwrap();
    let all_db = all_dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(all_dir.path())
        .args([
            "--db",
            all_db.to_str().unwrap(),
            "project",
            "init",
            "All Hosts",
            "--client",
            "all",
        ])
        .assert()
        .success();
    assert!(
        !all_dir.path().join(".grok").exists(),
        "the legacy all selection must not opt a project into Grok"
    );
    let all_doctor = planr()
        .current_dir(all_dir.path())
        .args([
            "--db",
            all_db.to_str().unwrap(),
            "--json",
            "doctor",
            "--client",
            "all",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let all_doctor: Value = serde_json::from_slice(&all_doctor).unwrap();
    assert_eq!(all_doctor["clients"].as_array().unwrap().len(), 3);
    assert!(
        all_doctor["clients"]
            .as_array()
            .unwrap()
            .iter()
            .all(|check| check["client"] != "grok")
    );

    let dir = tempdir().unwrap();
    let canonical_dir = dir.path().canonicalize().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let initialized = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "project",
            "init",
            "Grok Host",
            "--client",
            "grok",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let initialized: Value = serde_json::from_slice(&initialized).unwrap();
    assert_eq!(initialized["client"], "grok");
    assert!(dir.path().join(".grok/skills/planr/SKILL.md").exists());
    assert!(dir.path().join(".grok/agents/planr-worker.md").exists());
    assert!(
        !dir.path().join(".grok/config.toml").exists(),
        "project init provisions workflows but does not silently enable MCP"
    );

    let preview = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "install",
            "grok",
            "--dry-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&preview).unwrap();
    assert_eq!(preview["client"], "grok");
    assert_eq!(preview["hooks"]["supported"], false);
    assert_eq!(preview["hooks"]["status"], "unsupported");
    let config = preview["config"].as_str().unwrap();
    assert!(config.contains("args = [\"mcp\"]"));
    assert!(config.contains("PLANR_MCP_CLIENT = \"grok\""));
    assert!(!config.contains(db.to_str().unwrap()));
    assert!(!config.to_ascii_lowercase().contains("xai"));

    fs::write(
        dir.path().join(".grok/config.toml"),
        "# keep this comment\ntheme = \"dark\"\n\n[mcp_servers.foreign]\ncommand = \"foreign\"\n",
    )
    .unwrap();
    let installed = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "install", "grok"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let installed: Value = serde_json::from_slice(&installed).unwrap();
    assert_eq!(installed["hooks"]["supported"], false);
    let preview_paths: std::collections::BTreeSet<&str> = preview["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap())
        .collect();
    let mut written_paths: std::collections::BTreeSet<String> = installed["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| {
            path.as_str()
                .unwrap()
                .strip_prefix(canonical_dir.to_str().unwrap())
                .unwrap()
                .trim_start_matches('/')
                .to_string()
        })
        .collect();
    written_paths.insert(".grok/config.toml".to_string());
    assert_eq!(
        preview_paths,
        written_paths.iter().map(String::as_str).collect(),
        "dry-run paths and non-dry writes must have one source of truth"
    );
    let config = fs::read_to_string(dir.path().join(".grok/config.toml")).unwrap();
    assert!(config.contains("# keep this comment"));
    assert!(config.contains("theme = \"dark\""));
    assert!(config.contains("[mcp_servers.foreign]"));
    let parsed: toml::Value = toml::from_str(&config).unwrap();
    assert_eq!(
        parsed["mcp_servers"]["planr"]["env"]["PLANR_MCP_CLIENT"].as_str(),
        Some("grok")
    );
    assert!(!dir.path().join(".grok/hooks.json").exists());

    let installed_skill = dir.path().join(".grok/skills/planr/SKILL.md");
    fs::write(&installed_skill, "local customization\n").unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "install", "grok"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&installed_skill).unwrap(),
        "local customization\n",
        "normal reconciliation must preserve workflow edits"
    );
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "install", "grok", "--force"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&installed_skill).unwrap(),
        include_str!("../plugins/planr/skills/planr/SKILL.md"),
        "forced reconciliation must restore the packaged workflow"
    );

    let doctor = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "doctor",
            "--client",
            "grok",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doctor: Value = serde_json::from_slice(&doctor).unwrap();
    assert_eq!(doctor["clients"].as_array().unwrap().len(), 1);
    assert_eq!(doctor["clients"][0]["client"], "grok");

    let mcp_prompt = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "prompt",
            "mcp",
            "--client",
            "grok",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_prompt: Value = serde_json::from_slice(&mcp_prompt).unwrap();
    assert!(
        mcp_prompt["prompt"]
            .as_str()
            .unwrap()
            .contains(".grok/config.toml")
    );
    assert!(
        !mcp_prompt["prompt"]
            .as_str()
            .unwrap()
            .contains(db.to_str().unwrap())
    );

    let routing = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "prompt",
            "routing",
            "--client",
            "grok",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let routing: Value = serde_json::from_slice(&routing).unwrap();
    assert!(routing["hosts"]["grok"].is_array());
    assert!(
        routing["prompt"]
            .as_str()
            .unwrap()
            .contains("grok --no-auto-update")
    );
    assert!(routing["process_dispatch"].as_array().unwrap().is_empty());

    fs::write(
        dir.path().join(".planr/agents.toml"),
        "[profiles.grok]\nclient = \"grok\"\nmodel = \"grok-code\"\n\n[route_default]\nprofile = \"grok\"\n",
    )
    .unwrap();
    let item = create_test_item(dir.path(), &db, "Grok observation", "narrow marker");
    planr()
        .current_dir(dir.path())
        .env("PLANR_MCP_CLIENT", "grok")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item,
            "--summary",
            "observed through MCP",
            "--cmd",
            "cargo test",
        ])
        .assert()
        .success();
    planr()
        .current_dir(dir.path())
        .env("GROK_SESSION_ID", "ambient-only")
        .args([
            "--db",
            db.to_str().unwrap(),
            "log",
            "add",
            "--item",
            &item,
            "--summary",
            "ambient variable ignored",
            "--cmd",
            "cargo test",
        ])
        .assert()
        .success();
    let trace = planr()
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
    let trace: Value = serde_json::from_slice(&trace).unwrap();
    let runs = trace["routing"]["runs"].as_array().unwrap();
    assert_eq!(runs[0]["observed_client"], "grok");
    assert!(
        !runs[1].as_object().unwrap().contains_key("observed_client"),
        "ambient GROK_* variables must not be treated as a stable observation"
    );

    let conflict_dir = tempdir().unwrap();
    let conflict_db = conflict_dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(conflict_dir.path())
        .args([
            "--db",
            conflict_db.to_str().unwrap(),
            "project",
            "init",
            "Conflict",
        ])
        .assert()
        .success();
    fs::create_dir_all(conflict_dir.path().join(".grok")).unwrap();
    let conflict = "[mcp_servers.planr]\ncommand = \"custom-planr\"\n";
    fs::write(conflict_dir.path().join(".grok/config.toml"), conflict).unwrap();
    planr()
        .current_dir(conflict_dir.path())
        .args(["--db", conflict_db.to_str().unwrap(), "install", "grok"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("re-run with --force"));
    assert_eq!(
        fs::read_to_string(conflict_dir.path().join(".grok/config.toml")).unwrap(),
        conflict
    );
    assert!(
        !conflict_dir
            .path()
            .join(".grok/skills/planr/SKILL.md")
            .exists(),
        "a config conflict must fail before any workflow asset is written"
    );
}

#[test]
fn pi_host_surfaces_are_native_explicit_and_optional() {
    let all_dir = tempdir().unwrap();
    let all_db = all_dir.path().join(".planr/planr.sqlite");
    planr()
        .current_dir(all_dir.path())
        .args([
            "--db",
            all_db.to_str().unwrap(),
            "project",
            "init",
            "All Hosts",
            "--client",
            "all",
        ])
        .assert()
        .success();
    assert!(
        !all_dir.path().join(".pi").exists(),
        "the legacy all selection must not opt a project into Pi"
    );
    let all_doctor = planr()
        .current_dir(all_dir.path())
        .args([
            "--db",
            all_db.to_str().unwrap(),
            "--json",
            "doctor",
            "--client",
            "all",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let all_doctor: Value = serde_json::from_slice(&all_doctor).unwrap();
    assert_eq!(all_doctor["clients"].as_array().unwrap().len(), 3);
    assert!(
        all_doctor["clients"]
            .as_array()
            .unwrap()
            .iter()
            .all(|check| check["client"] != "pi")
    );

    let dir = tempdir().unwrap();
    let canonical_dir = dir.path().canonicalize().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");
    let initialized = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "project",
            "init",
            "Pi Host",
            "--client",
            "pi",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let initialized: Value = serde_json::from_slice(&initialized).unwrap();
    assert_eq!(initialized["client"], "pi");
    assert!(dir.path().join(".pi/skills/planr/SKILL.md").exists());
    assert!(dir.path().join(".pi/agents/planr-worker.md").exists());
    for forbidden in [
        ".pi/settings.json",
        ".pi/mcp.json",
        ".pi/hooks.json",
        ".pi/extensions",
        ".pi/prompts",
    ] {
        assert!(
            !dir.path().join(forbidden).exists(),
            "project init must not create unsupported Pi artifact {forbidden}"
        );
    }

    let preview = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "install",
            "pi",
            "--dry-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview: Value = serde_json::from_slice(&preview).unwrap();
    assert_eq!(preview["client"], "pi");
    assert_eq!(preview["mcp"]["supported"], false);
    assert_eq!(preview["mcp"]["status"], "unsupported");
    assert_eq!(preview["hooks"]["supported"], false);
    assert_eq!(preview["hooks"]["status"], "unsupported");
    assert_eq!(preview["global_config_edited"], false);
    let preview_paths: std::collections::BTreeSet<&str> = preview["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap())
        .collect();
    assert_eq!(preview_paths.len(), 16);
    assert!(
        preview_paths
            .iter()
            .all(|path| path.starts_with(".pi/skills/") || path.starts_with(".pi/agents/"))
    );

    let parity_preview = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "install",
            "pi",
            "--dry-run",
            "--no-mcp",
            "--no-hooks",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parity_preview: Value = serde_json::from_slice(&parity_preview).unwrap();
    assert_eq!(parity_preview["artifacts"], preview["artifacts"]);

    let installed = planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "--json", "install", "pi"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let installed: Value = serde_json::from_slice(&installed).unwrap();
    assert_eq!(installed["mcp"]["supported"], false);
    assert_eq!(installed["hooks"]["supported"], false);
    assert_eq!(installed["global_config_edited"], false);
    let written_paths: std::collections::BTreeSet<String> = installed["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| {
            path.as_str()
                .unwrap()
                .strip_prefix(canonical_dir.to_str().unwrap())
                .unwrap()
                .trim_start_matches('/')
                .to_string()
        })
        .collect();
    assert_eq!(
        preview_paths,
        written_paths.iter().map(String::as_str).collect(),
        "dry-run paths and non-dry reconciliation must have one source of truth"
    );

    let installed_skill = dir.path().join(".pi/skills/planr/SKILL.md");
    fs::write(&installed_skill, "local customization\n").unwrap();
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "install", "pi"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&installed_skill).unwrap(),
        "local customization\n",
        "normal reconciliation must preserve workflow edits"
    );
    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "install", "pi", "--force"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&installed_skill).unwrap(),
        include_str!("../plugins/planr/skills/planr/SKILL.md"),
        "forced reconciliation must restore the packaged workflow"
    );

    let doctor = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "doctor",
            "--client",
            "pi",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doctor: Value = serde_json::from_slice(&doctor).unwrap();
    assert_eq!(doctor["clients"].as_array().unwrap().len(), 1);
    assert_eq!(doctor["clients"][0]["client"], "pi");

    let mcp_prompt = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "prompt",
            "mcp",
            "--client",
            "pi",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let mcp_prompt: Value = serde_json::from_slice(&mcp_prompt).unwrap();
    assert_eq!(mcp_prompt["global_config_edited"], false);
    assert!(
        mcp_prompt["prompt"]
            .as_str()
            .unwrap()
            .contains("Pi core intentionally ships without MCP")
    );
    assert!(
        mcp_prompt["prompt"]
            .as_str()
            .unwrap()
            .contains("/skill:planr")
    );

    let routing = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "prompt",
            "routing",
            "--client",
            "pi",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let routing: Value = serde_json::from_slice(&routing).unwrap();
    assert!(routing["hosts"]["pi"].is_array());
    assert_eq!(
        routing["process_dispatch"][0],
        "pi --approve --model <provider/model> --thinking <level> -p \"Use /skill:planr-work on item <item-id> as the first item in a compatible same-plan maker run. Keep one worker identity, settle each ordinary outcome with planr done --next, write a compact durable handoff only at a genuine stop, and stop after requesting any material review, blocker, incompatible pick, empty pick, or budget boundary.\""
    );
    let routing_prompt = routing["prompt"].as_str().unwrap();
    for required in [
        "optional `pi-subagents`",
        "separate sequential Pi processes",
        "run `planr prime` manually",
        "never write Pi trust or global settings",
    ] {
        assert!(
            routing_prompt.contains(required),
            "Pi routing guidance is missing {required}"
        );
    }
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
        "src/app/execution_run.rs",
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
        ("src/app/audit.rs", 300),
        ("src/app/commands.rs", 1_200),
        ("src/app/grok.rs", 150),
        ("src/app/flow.rs", 1_150),
        ("src/app/git_review.rs", 350),
        ("src/app/mcp.rs", 950),
        ("src/app/packages.rs", 450),
        ("src/app/prompts.rs", 100),
        ("src/app/http.rs", 900),
        ("src/app/repository.rs", 1_100),
        ("src/app/lease.rs", 325),
        ("src/app/review.rs", 600),
        ("src/app/recovery.rs", 450),
        ("src/app/surfaces.rs", 325),
        ("src/app/inspection.rs", 510),
        ("src/app/application.rs", 200),
        ("src/storage/schema.rs", 300),
        ("src/storage/rows.rs", 150),
        ("src/model.rs", 400),
        ("src/planpack.rs", 320),
        ("src/integrations.rs", 500),
        ("src/integrations/grok.rs", 220),
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

    let execution_run = fs::read_to_string(root.join("src/app/execution_run.rs")).unwrap();
    let production_line_count = execution_run
        .split_once("\n#[cfg(test)]")
        .map(|(production, _)| production)
        .unwrap_or(&execution_run)
        .lines()
        .count();
    assert!(
        production_line_count <= 1_400,
        "src/app/execution_run.rs has {production_line_count} production lines; keep ownership split instead of growing a new hub"
    );

    let docs = fs::read_to_string(root.join("docs/ARCHITECTURE.md")).unwrap();
    for owner in [
        "src/main.rs",
        "src/cli.rs",
        "src/app/mod.rs",
        "src/app/audit.rs",
        "src/app/commands.rs",
        "src/app/grok.rs",
        "src/app/flow.rs",
        "src/app/git_review.rs",
        "src/app/mcp.rs",
        "src/app/packages.rs",
        "src/app/prompts.rs",
        "src/app/http.rs",
        "src/app/repository.rs",
        "src/app/lease.rs",
        "src/app/review.rs",
        "src/app/recovery.rs",
        "src/app/execution_run.rs",
        "src/app/surfaces.rs",
        "src/app/inspection.rs",
        "src/app/application.rs",
        "src/storage/mod.rs",
        "src/storage/schema.rs",
        "src/storage/rows.rs",
        "src/planpack.rs",
        "src/integrations.rs",
        "src/integrations/grok.rs",
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
    write_materiality_policy(dir.path());
    init_materiality_git(dir.path());
    fs::create_dir_all(dir.path().join("docs")).unwrap();
    fs::write(dir.path().join("docs/README.md"), "fix evidence\n").unwrap();

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
fn plan_audit_uses_evidence_coverage_for_binding_criteria_and_logs_are_claims_only() {
    let dir = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let db = db_dir.path().join("planr.sqlite");
    let db_arg = db.to_str().unwrap().to_string();
    write_evidence_policy_fixture(dir.path());
    init_git_repo(dir.path());
    init_evidence_project(dir.path(), &db, "Evidence Audit");

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
    let run_human = |worker: &str, args: &[&str]| -> String {
        let output = planr()
            .current_dir(dir.path())
            .env("PLANR_WORKER_ID", worker)
            .args(["--db", &db_arg])
            .args(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        String::from_utf8(output).unwrap()
    };
    let clause = |audit: &Value, name: &str| -> Value {
        audit["clauses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|clause| clause["clause"] == name)
            .unwrap_or_else(|| panic!("missing clause {name}: {audit}"))
            .clone()
    };
    let create_settled_plan = |name: &str, criterion_ids: &[&str]| -> (String, String) {
        let product = run("prep", &["plan", "new", name]);
        let product_id = product["plan"]["id"].as_str().unwrap().to_string();
        let build = run("prep", &["plan", "split", &product_id, "--slice", "Audit"]);
        let build_id = build["plan"]["id"].as_str().unwrap().to_string();
        let build_path = build["plan"]["path"].as_str().unwrap().to_string();
        let text = fs::read_to_string(&build_path).unwrap();
        let frontmatter_end = text.find("\n---\n").unwrap() + 5;
        fs::write(
            &build_path,
            format!(
                "{}\n# Build Plan\n\n## Scope Decision\n\nAudit slice.\n\n## Verification\n\nEvidence coverage.\n\n## Acceptance Criteria\n\n- Binding evidence passes.\n\n## Steps\n\n### Ship\n\nDo the work.\n",
                &text[..frontmatter_end]
            ),
        )
        .unwrap();
        author_build_plan_criteria(Path::new(&build_path), criterion_ids);
        let map = run("prep", &["map", "build", "--from", &build_id]);
        let item_id = map["created"][0]["id"].as_str().unwrap().to_string();
        Connection::open(&db)
            .unwrap()
            .execute(
                "UPDATE items SET status = 'closed', updated_at = datetime('now') WHERE id = ?1",
                [&item_id],
            )
            .unwrap();
        run(
            "prep",
            &[
                "context",
                "add",
                &format!("GOAL CONTRACT {build_id}: DONE when Evidence coverage passes."),
                "--tag",
                "goal-contract",
            ],
        );
        (build_id, item_id)
    };
    let create_open_plan = |name: &str, criterion_ids: &[&str]| -> (String, String) {
        let product = run("prep", &["plan", "new", name]);
        let product_id = product["plan"]["id"].as_str().unwrap().to_string();
        let build = run("prep", &["plan", "split", &product_id, "--slice", "Audit"]);
        let build_id = build["plan"]["id"].as_str().unwrap().to_string();
        let build_path = build["plan"]["path"].as_str().unwrap().to_string();
        let text = fs::read_to_string(&build_path).unwrap();
        let frontmatter_end = text.find("\n---\n").unwrap() + 5;
        fs::write(
            &build_path,
            format!(
                "{}\n# Build Plan\n\n## Scope Decision\n\nAudit slice.\n\n## Verification\n\nEvidence coverage.\n\n## Acceptance Criteria\n\n- Binding evidence passes.\n\n## Steps\n\n### Ship\n\nDo the work.\n",
                &text[..frontmatter_end]
            ),
        )
        .unwrap();
        author_build_plan_criteria(Path::new(&build_path), criterion_ids);
        let map = run("prep", &["map", "build", "--from", &build_id]);
        let item_id = map["created"][0]["id"].as_str().unwrap().to_string();
        run(
            "prep",
            &[
                "context",
                "add",
                &format!("GOAL CONTRACT {build_id}: DONE when Evidence coverage passes."),
                "--tag",
                "goal-contract",
            ],
        );
        (build_id, item_id)
    };
    let policy = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args(["--db", &db_arg, "--json", "evidence", "policy"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let policy_digest = policy["object"]["digest"].as_str().unwrap().to_string();
    let project_id: String = Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT id FROM projects ORDER BY created_at LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let bind_obligation = |mut obligation: Value, plan_id: &str, criterion_id: &str| -> Value {
        obligation["plan_id"] = json!(plan_id);
        obligation["criterion_id"] = json!(criterion_id);
        obligation
    };
    let raw_coverage = |dir: &Path, db: &Path, criterion_id: &str| -> Value {
        let output = planr()
            .current_dir(dir)
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "coverage",
                "--scope",
                "criterion",
                "--id",
                criterion_id,
            ])
            .output()
            .unwrap();
        assert!(!output.stdout.is_empty(), "coverage stdout must be JSON");
        single_json_document(&output.stdout)
    };
    let assert_persisted_verdict = |db: &Path, coverage: &Value| {
        let conn = Connection::open(db).unwrap();
        let persisted: String = conn
            .query_row(
                "SELECT verdict_json FROM coverage_verdicts WHERE id = ?1",
                [coverage["object"]["coverage_id"].as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        let persisted: Value = serde_json::from_str(&persisted).unwrap();
        assert_eq!(persisted, coverage["object"]["coverage"]);
    };
    let assert_criterion_matches_raw_oracle =
        |criterion: &Value,
         coverage: &Value,
         expected_status: &str,
         expected_actionable: bool,
         expected_suggested: &str,
         expected_next: Value,
         expected_receipt_refs: Value,
         expected_attempt_refs: Value,
         expected_waiver_refs: Value,
         expected_receipt_digests: Value,
         expected_waiver_digests: Value,
         expected_actionable_gaps: Value,
         expected_non_actionable_blockers: Value| {
            assert_evidence_envelope(coverage, "evidence.coverage", true);
            assert_eq!(coverage["object"]["status"], expected_status, "{coverage}");
            let verdict = &coverage["object"]["coverage"];
            let raw_scope = &verdict["scope"];
            let raw_status = &verdict["status"];
            let expected_pass = matches!(expected_status, "satisfied" | "waived");
            assert_eq!(coverage["object"]["coverage_id"], verdict["id"]);
            assert_eq!(criterion["criterion_id"], raw_scope["criterion_id"]);
            assert_eq!(criterion["coverage_id"], verdict["id"]);
            assert_eq!(criterion["scope"], *raw_scope);
            assert_eq!(criterion["status"], *raw_status);
            assert_eq!(criterion["pass"], expected_pass);
            assert_eq!(criterion["actionable_now"], verdict["actionable_now"]);
            assert_eq!(
                criterion["suggested_next_action"],
                verdict["suggested_next_action"]
            );
            assert_eq!(criterion["observations"], verdict["observation_coverage"]);
            assert_eq!(criterion["actionable_gaps"], expected_actionable_gaps);
            assert_eq!(
                criterion["non_actionable_blockers"],
                expected_non_actionable_blockers
            );
            assert_eq!(criterion["receipt_refs"], expected_receipt_refs);
            assert_eq!(criterion["attempt_refs"], expected_attempt_refs);
            assert_eq!(criterion["waiver_refs"], expected_waiver_refs);
            assert_eq!(criterion["receipt_digests"], expected_receipt_digests);
            assert_eq!(criterion["waiver_digests"], expected_waiver_digests);
            assert_eq!(
                coverage["object"]["receipt_digests"],
                criterion["receipt_digests"]
            );
            assert_eq!(
                coverage["object"]["waiver_digests"],
                criterion["waiver_digests"]
            );
            assert_eq!(criterion["status"], expected_status);
            assert_eq!(criterion["actionable_now"], expected_actionable);
            assert_eq!(criterion["suggested_next_action"], expected_suggested);
            assert_eq!(criterion["next_action"], expected_next);

            let projection = &coverage["object"]["canonical_projection"];
            assert_eq!(projection["criterion_id"], criterion["criterion_id"]);
            assert_eq!(projection["coverage_id"], criterion["coverage_id"]);
            assert_eq!(projection["scope"], criterion["scope"]);
            assert_eq!(projection["status"], criterion["status"]);
            assert_eq!(projection["pass"], criterion["pass"]);
        };
    let criterion_from = |proof: &Value, criterion_id: &str| -> Value {
        proof["criteria"]
            .as_array()
            .unwrap()
            .iter()
            .find(|criterion| criterion["criterion_id"] == criterion_id)
            .unwrap_or_else(|| panic!("missing criterion {criterion_id}: {proof}"))
            .clone()
    };
    let assert_proof_criterion_matches_raw_oracle =
        |proof: &Value,
         coverage: &Value,
         criterion_id: &str,
         expected_status: &str,
         expected_actionable: bool,
         expected_suggested: &str,
         expected_next: Value,
         expected_receipt_refs: Value,
         expected_attempt_refs: Value,
         expected_waiver_refs: Value,
         expected_receipt_digests: Value,
         expected_waiver_digests: Value,
         expected_actionable_gaps: Value,
         expected_non_actionable_blockers: Value| {
            let criterion = criterion_from(proof, criterion_id);
            assert_criterion_matches_raw_oracle(
                &criterion,
                coverage,
                expected_status,
                expected_actionable,
                expected_suggested,
                expected_next,
                expected_receipt_refs,
                expected_attempt_refs,
                expected_waiver_refs,
                expected_receipt_digests,
                expected_waiver_digests,
                expected_actionable_gaps,
                expected_non_actionable_blockers,
            );
        };
    let expected_gap = |criterion_id: &str,
                        requirement_id: &str,
                        status: &str,
                        reason: &str,
                        attempt_refs: Value| {
        json!({
            "criterion_id": criterion_id,
            "requirement_id": requirement_id,
            "status": status,
            "reason": reason,
            "receipt_refs": [],
            "attempt_refs": attempt_refs,
            "waiver_ref": null,
        })
    };
    let assert_missing_observation_proof =
        |proof: &Value, coverage: &Value, criterion_id: &str, requirement_id: &str| {
            assert_proof_criterion_matches_raw_oracle(
                proof,
                coverage,
                criterion_id,
                "unsatisfied",
                true,
                "collect missing trusted evidence",
                json!("collect missing trusted evidence"),
                json!([]),
                json!([]),
                json!([]),
                json!([]),
                json!([]),
                json!([expected_gap(
                    criterion_id,
                    requirement_id,
                    "missing",
                    "missing_observation",
                    json!([])
                )]),
                json!([]),
            );
        };

    let (workflow_plan, workflow_item) =
        create_open_plan("Workflow proof propagation audit", &["crit-audit-workflow", "crit-audit-planless"]);
    let mut workflow_obligation = bind_obligation(
        evidence_obligation_for(
            "pob-audit-workflow",
            &policy_digest,
            "com.example.browser.page.visible",
            "workflow proof propagation",
            json!({"visible": true}),
            json!({"kind": "browser", "uri": "http://127.0.0.1:9/workflow"}),
            Value::Null,
            json!({"kind": "browser", "id": "browser-session"}),
            json!([]),
            "sha256:9999999999999999999999999999999999999999999999999999999999999999",
        ),
        &workflow_plan,
        "crit-audit-workflow",
    );
    workflow_obligation["item_id"] = json!(workflow_item.clone());
    let planless_item = "i-audit-planless-bound";
    {
        let conn = Connection::open(&db).unwrap();
        let workflow_plan_path: String = conn
            .query_row(
                "SELECT path FROM plans WHERE id = ?1",
                [&workflow_plan],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO items(id, project_id, title, description, status, work_type, priority, plan_path, created_at, updated_at)
             VALUES (?1, ?2, 'Plan-bound proof', 'Active item binding follows plan containment', 'ready', 'code', 0, ?3, datetime('now'), datetime('now'))",
            rusqlite::params![planless_item, project_id, workflow_plan_path],
        )
        .unwrap();
    }
    let mut planless_obligation = bind_obligation(
        evidence_obligation_for(
            "pob-audit-planless",
            &policy_digest,
            "com.example.browser.page.visible",
            "planless item still proof gated",
            json!({"visible": true}),
            json!({"kind": "browser", "uri": "http://127.0.0.1:9/planless"}),
            Value::Null,
            json!({"kind": "browser", "id": "browser-session"}),
            json!([]),
            "sha256:1010101010101010101010101010101010101010101010101010101010101010",
        ),
        &workflow_plan,
        "crit-audit-planless",
    );
    planless_obligation["item_id"] = json!(planless_item);
    let workflow_migration = json!({
        "schema_version": "planr.evidence.migration.v1",
        "plan_id": workflow_plan,
        "obligations": [workflow_obligation, planless_obligation],
    });
    let workflow_migration_path = dir.path().join("pob-audit-workflow.migration.json");
    fs::write(
        &workflow_migration_path,
        serde_json::to_vec_pretty(&workflow_migration).unwrap(),
    )
    .unwrap();
    single_json_document(
        &planr()
            .current_dir(dir.path())
            .args([
                "--db",
                db.to_str().unwrap(),
                "--json",
                "evidence",
                "migrate",
                "--input",
                workflow_migration_path.to_str().unwrap(),
                "--apply",
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let workflow_preview = run("prep", &["map", "preview", "--close", &workflow_item]);
    assert_eq!(workflow_preview["can_close"], false, "{workflow_preview}");
    assert_eq!(workflow_preview["proof_blocks_close"], true);
    assert_eq!(workflow_preview["proof"]["status"], "not_proven");
    assert_eq!(
        workflow_preview["proof"]["scope"]["kind"], "item",
        "{workflow_preview}"
    );
    assert_eq!(
        workflow_preview["proof"]["scope"]["id"], workflow_item,
        "{workflow_preview}"
    );
    assert_eq!(
        workflow_preview["proof"]["scope"]["binding_ids"],
        json!(["pob-audit-workflow"]),
        "{workflow_preview}"
    );
    assert!(
        workflow_preview["proof"]["actionable_gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["reason"] == "missing_observation"),
        "{workflow_preview}"
    );
    let workflow_trace = run("prep", &["trace", "item", &workflow_item]);
    assert_eq!(
        workflow_trace["proof"]["coverage_status"],
        Value::Null,
        "proof DTO should not expose ad hoc status fields: {workflow_trace}"
    );
    assert_eq!(workflow_trace["proof"]["status"], "not_proven");
    assert_eq!(
        workflow_trace["proof"]["next_action"],
        "collect missing trusted evidence"
    );
    assert_eq!(workflow_trace["proof"]["actionable_now"], true);
    assert_eq!(
        workflow_trace["proof"]["criteria"][0]["suggested_next_action"],
        "collect missing trusted evidence"
    );
    let workflow_raw = raw_coverage(dir.path(), &db, "crit-audit-workflow");
    assert_persisted_verdict(&db, &workflow_raw);
    for proof in [&workflow_preview["proof"], &workflow_trace["proof"]] {
        assert_missing_observation_proof(
            proof,
            &workflow_raw,
            "crit-audit-workflow",
            "obs-pob-audit-workflow",
        );
    }
    let approval = run(
        "prep",
        &[
            "approval",
            "request",
            &workflow_item,
            "--reason",
            "human signoff cannot replace proof",
        ],
    );
    assert_eq!(approval["proof"]["status"], "not_proven");
    assert_eq!(approval["proof"], workflow_trace["proof"]);
    assert_missing_observation_proof(
        &approval["proof"],
        &workflow_raw,
        "crit-audit-workflow",
        "obs-pob-audit-workflow",
    );
    let approved = run(
        "prep",
        &[
            "approval",
            "approve",
            &workflow_item,
            "--by",
            "qa",
            "--comment",
            "approved but still not proof",
        ],
    );
    assert_eq!(approved["approval"]["status"], "approved");
    assert_eq!(
        approved["proof"]["completion_language"],
        "not proven by canonical Evidence coverage"
    );
    assert_eq!(approved["proof"], workflow_trace["proof"]);
    assert_missing_observation_proof(
        &approved["proof"],
        &workflow_raw,
        "crit-audit-workflow",
        "obs-pob-audit-workflow",
    );
    let review_evidence = run("checker", &["review", "evidence", &workflow_item]);
    assert_eq!(
        review_evidence["evidence"]["proof"],
        workflow_trace["proof"]
    );
    assert_missing_observation_proof(
        &review_evidence["evidence"]["proof"],
        &workflow_raw,
        "crit-audit-workflow",
        "obs-pob-audit-workflow",
    );
    run(
        "maker",
        &[
            "log",
            "add",
            "--item",
            &workflow_item,
            "--kind",
            "completion",
            "--summary",
            "implementation complete but proof missing",
        ],
    );
    run("maker", &["pick", "release", &workflow_item, "--force"]);
    let workflow_status = run("prep", &["map", "status"]);
    let ready_workflow = workflow_status["ready"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["item"]["id"] == workflow_item)
        .unwrap();
    assert_eq!(ready_workflow["item"]["status"], "ready");
    assert_eq!(ready_workflow["proof"], workflow_trace["proof"]);
    assert_missing_observation_proof(
        &ready_workflow["proof"],
        &workflow_raw,
        "crit-audit-workflow",
        "obs-pob-audit-workflow",
    );
    let workflow_pick = run("maker", &["pick", "--peek", "--plan", &workflow_plan]);
    assert_eq!(workflow_pick["item"]["id"], workflow_item);
    assert_eq!(workflow_pick["proof"], workflow_trace["proof"]);
    assert_missing_observation_proof(
        &workflow_pick["proof"],
        &workflow_raw,
        "crit-audit-workflow",
        "obs-pob-audit-workflow",
    );
    let workflow_picked = run("maker", &["pick", "--plan", &workflow_plan]);
    assert_eq!(workflow_picked["proof"], workflow_trace["proof"]);
    assert_missing_observation_proof(
        &workflow_picked["proof"],
        &workflow_raw,
        "crit-audit-workflow",
        "obs-pob-audit-workflow",
    );
    let workflow_picked_status = run("prep", &["map", "status"]);
    let picked_workflow = workflow_picked_status["picked"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["item"]["id"] == workflow_item)
        .unwrap();
    assert_eq!(picked_workflow["proof"], workflow_trace["proof"]);
    assert_missing_observation_proof(
        &picked_workflow["proof"],
        &workflow_raw,
        "crit-audit-workflow",
        "obs-pob-audit-workflow",
    );
    {
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE items SET picked_at = datetime('now', '-1 hour'), last_heartbeat_at = datetime('now', '-1 hour') WHERE id = ?1",
            [&workflow_item],
        )
        .unwrap();
    }
    let recovery = run("prep", &["recover", "sweep", "--older-than-seconds", "1"]);
    assert!(
        recovery["stale"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["item"]["id"] == workflow_item
                && entry["proof"]["status"] == "not_proven"),
        "{recovery}"
    );
    let recovered_workflow = recovery["stale"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["item"]["id"] == workflow_item)
        .unwrap();
    assert_missing_observation_proof(
        &recovered_workflow["proof"],
        &workflow_raw,
        "crit-audit-workflow",
        "obs-pob-audit-workflow",
    );

    let planless_preview = run("prep", &["map", "preview", "--close", planless_item]);
    assert_eq!(planless_preview["proof"]["active_binding"], true);
    assert_eq!(planless_preview["proof"]["status"], "not_proven");
    assert_eq!(planless_preview["proof_blocks_close"], true);

    let broken_item = "i-audit-broken-proof";
    {
        let conn = Connection::open(&db).unwrap();
        let workflow_plan_path: String = conn
            .query_row(
                "SELECT path FROM plans WHERE id = ?1",
                [&workflow_plan],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO items(id, project_id, title, description, status, work_type, priority, plan_path, created_at, updated_at)
             VALUES (?1, ?2, 'Broken proof', 'Malformed active binding fails closed', 'ready', 'code', 0, ?3, datetime('now'), datetime('now'))",
            rusqlite::params![broken_item, project_id, workflow_plan_path],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, policy_digest, config_digest, supersedes_obligation_id, created_at
            ) VALUES (
              'pob-audit-broken', ?1, ?2, ?3, 'crit-audit-planless', 2, 'broken proof',
              1, 'not-json', '{}', '{}', '{}', ?4, ?5, 'pob-audit-planless', datetime('now')
            )",
            rusqlite::params![
                project_id,
                workflow_plan,
                broken_item,
                policy_digest,
                "sha256:2020202020202020202020202020202020202020202020202020202020202020",
            ],
        )
        .unwrap();
    }
    let broken_preview = run("prep", &["map", "preview", "--close", broken_item]);
    assert_eq!(
        broken_preview["proof"]["status"], "error",
        "{broken_preview}"
    );
    assert_eq!(broken_preview["proof"]["active_binding"], true);
    assert_eq!(broken_preview["proof_blocks_close"], true);


    let (browser_plan, browser_item) =
        create_settled_plan("Browser evidence audit", &["crit-audit-browser"]);
    let browser_obligation = bind_obligation(
        evidence_obligation_for(
            "pob-audit-browser",
            &policy_digest,
            "com.example.browser.page.visible",
            "browser rendering is visible",
            json!({"visible": true}),
            json!({"kind": "browser", "uri": "http://127.0.0.1:9/page"}),
            Value::Null,
            json!({"kind": "browser", "id": "browser-session"}),
            json!([]),
            "sha256:aaaaaaaabbbbbbbbccccccccddddddddeeeeeeeeffffffff0000000011111111",
        ),
        &browser_plan,
        "crit-audit-browser",
    );
    add_evidence_obligation_value(dir.path(), &db, "pob-audit-browser", &browser_obligation);
    run(
        "maker",
        &[
            "log",
            "add",
            "--item",
            &browser_item,
            "--kind",
            "verification",
            "--summary",
            "verified page in browser",
            "--cmd",
            "curl http://127.0.0.1:9/page",
        ],
    );
    let forged = mcp_tool(
        dir.path(),
        &db,
        201,
        "planr_evidence_import",
        json!({"artifact_root": dir.path().to_str().unwrap(), "input": {"receipt_json": {"provenance": {"assigned_by": "agent"}}}}),
    );
    assert_evidence_envelope(&forged, "evidence.import", false);
    let browser_audit = run("prep", &["plan", "audit", &browser_plan]);
    assert_eq!(browser_audit["holds"], false, "{browser_audit}");
    let browser_clause = clause(&browser_audit, "verification_logged");
    assert_eq!(browser_clause["authority"], "evidence_coverage");
    assert_eq!(browser_clause["log_authority"], "claim_only");
    assert_eq!(browser_clause["logs"].as_array().unwrap().len(), 1);
    assert_eq!(browser_clause["pass"], false);
    assert_eq!(
        browser_clause["criteria"][0]["criterion_id"],
        "crit-audit-browser"
    );
    assert_eq!(browser_clause["criteria"][0]["status"], "unsatisfied");
    assert_eq!(
        browser_clause["criteria"][0]["observations"][0]["gap_reason"],
        "missing_observation"
    );
    let browser_human = run_human("prep", &["plan", "audit", &browser_plan]);
    assert!(
        browser_human.contains("claim-only untrusted verification log:"),
        "{browser_human}"
    );
    assert!(browser_human.contains("item="), "{browser_human}");
    assert!(
        browser_human.contains("summary=verified page in browser"),
        "{browser_human}"
    );
    assert_eq!(
        run("prep", &["plan", "audit", &browser_plan])["holds"],
        false,
        "claim rendering must not affect binding closure"
    );

    let (browser_close_plan, browser_close_item) =
        create_open_plan("Browser curl cannot close binding plan", &["crit-audit-browser-close"]);
    let mut browser_close_obligation = bind_obligation(
        evidence_obligation_for(
            "pob-audit-browser-close",
            &policy_digest,
            "com.example.browser.page.visible",
            "browser-rendered obligation cannot be replaced by curl",
            json!({"visible": true}),
            json!({"kind": "browser", "uri": "http://127.0.0.1:9/close-page"}),
            Value::Null,
            json!({"kind": "browser", "id": "browser-session"}),
            json!([]),
            "sha256:abababababababababababababababababababababababababababababababab",
        ),
        &browser_close_plan,
        "crit-audit-browser-close",
    );
    browser_close_obligation["item_id"] = json!(browser_close_item.clone());
    add_evidence_obligation_value(
        dir.path(),
        &db,
        "pob-audit-browser-close",
        &browser_close_obligation,
    );
    run(
        "maker",
        &[
            "log",
            "add",
            "--item",
            &browser_close_item,
            "--kind",
            "completion",
            "--summary",
            "production ready: verified page in browser with curl against local fixture",
            "--cmd",
            "curl http://127.0.0.1:9/close-page",
        ],
    );
    let browser_close_preview = run("prep", &["map", "preview", "--close", &browser_close_item]);
    assert_eq!(
        browser_close_preview["can_close"], false,
        "{browser_close_preview}"
    );
    assert_eq!(browser_close_preview["proof_blocks_close"], true);
    assert_eq!(
        browser_close_preview["proof"]["completion_language"],
        "not proven by canonical Evidence coverage",
        "{browser_close_preview}"
    );
    assert_eq!(
        browser_close_preview["proof"]["next_action"], "collect missing trusted evidence",
        "{browser_close_preview}"
    );
    assert!(
        !browser_close_preview["proof"]["completion_language"]
            .as_str()
            .unwrap()
            .contains("production ready"),
        "{browser_close_preview}"
    );
    let browser_close_raw = raw_coverage(dir.path(), &db, "crit-audit-browser-close");
    assert_persisted_verdict(&db, &browser_close_raw);
    assert_missing_observation_proof(
        &browser_close_preview["proof"],
        &browser_close_raw,
        "crit-audit-browser-close",
        "obs-pob-audit-browser-close",
    );
    let browser_close = planr()
        .current_dir(dir.path())
        .env("PLANR_WORKER_ID", "maker")
        .args([
            "--db",
            &db_arg,
            "--json",
            "close",
            &browser_close_item,
            "--summary",
            "production ready after browser check",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let browser_close: Value = serde_json::from_slice(&browser_close).unwrap();
    assert!(
        browser_close["error"]["message"]
            .as_str()
            .unwrap()
            .contains("binding Evidence coverage is not proven"),
        "{browser_close}"
    );
    planr()
        .current_dir(dir.path())
        .env("CODEX_THREAD_ID", "thread-browser-close")
        .args([
            "--db",
            &db_arg,
            "--json",
            "stop",
            "activate",
            "--plan",
            &browser_close_plan,
        ])
        .assert()
        .success();
    let browser_stop = single_json_document(
        &planr()
            .current_dir(dir.path())
            .args(["--db", &db_arg, "--json", "stop"])
            .write_stdin(codex_stop_envelope(
                dir.path(),
                "thread-browser-close",
                false,
            ))
            .env_remove("CODEX_THREAD_ID")
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_codex_stop_output_schema(&browser_stop, true);
    assert_eq!(browser_stop["decision"], "block", "{browser_stop}");
    assert!(
        browser_stop["reason"]
            .as_str()
            .unwrap()
            .contains("crit-audit-browser-close"),
        "{browser_stop}"
    );

    let (policy_missing_obligation_plan, policy_missing_obligation_item) =
        create_settled_plan("Binding policy missing obligation audit", &["crit-audit-policy-missing"]);
    run(
        "maker",
        &[
            "log",
            "add",
            "--item",
            &policy_missing_obligation_item,
            "--kind",
            "verification",
            "--summary",
            "diagnostic claim",
            "--cmd",
            "printf diagnostic",
        ],
    );
    let missing_obligation_audit = run("prep", &["plan", "audit", &policy_missing_obligation_plan]);
    let missing_obligation_clause = clause(&missing_obligation_audit, "verification_logged");
    assert_eq!(
        missing_obligation_audit["holds"], false,
        "{missing_obligation_audit}"
    );
    assert_eq!(missing_obligation_clause["pass"], false);
    assert_eq!(
        clause(&missing_obligation_audit, "final_product_review_complete")["pass"],
        false
    );
    assert_eq!(missing_obligation_clause["authority"], "evidence_policy");
    assert_eq!(missing_obligation_clause["log_authority"], "claim_only");
    assert_eq!(
        missing_obligation_audit["proof"]["status"],
        "binding_unsatisfied"
    );
    let missing_obligation_human =
        run_human("prep", &["plan", "audit", &policy_missing_obligation_plan]);
    assert!(
        missing_obligation_human.contains("claim-only untrusted verification log:"),
        "{missing_obligation_human}"
    );
    assert!(
        missing_obligation_human.contains("summary=diagnostic claim"),
        "{missing_obligation_human}"
    );
    assert_eq!(
        run("prep", &["plan", "audit", &browser_plan])["holds"],
        false,
        "a missing obligation on one plan must not weaken a separate binding plan"
    );

}
