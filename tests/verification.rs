use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::tempdir;

fn planr() -> Command {
    Command::cargo_bin("planr").expect("planr binary")
}

#[test]
fn verify_run_executes_and_stores_deterministic_evidence() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Verify"])
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
            "Verify Rust Toolchain",
            "--description",
            "record deterministic command evidence",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let created: Value = serde_json::from_slice(&created).unwrap();
    let item_id = created["item"]["id"].as_str().unwrap();

    let point = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "verify",
            "point",
            "add",
            item_id,
            "--text",
            "rustc reports its version",
            "--kind",
            "test",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let point: Value = serde_json::from_slice(&point).unwrap();
    let point_id = point["point"]["id"].as_str().unwrap();

    let evidence = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "verify",
            "run",
            item_id,
            "--kind",
            "test",
            "--point",
            point_id,
            "--cmd",
            "rustc --version",
            "--assert-stdout-contains",
            "rustc",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let evidence: Value = serde_json::from_slice(&evidence).unwrap();
    assert_eq!(evidence["status"], "pass");
    assert_eq!(evidence["evidence"]["executor"], "planr");
    assert_eq!(evidence["evidence"]["status"], "pass");

    let conn = Connection::open(&db).unwrap();
    let evidence_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM verification_evidence WHERE item_id = ?1 AND status = 'pass'",
            [item_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(evidence_count, 1);
    let log_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM logs WHERE item_id = ?1 AND kind = 'verification'",
            [item_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(log_count, 1);
    let point_status: String = conn
        .query_row(
            "SELECT status FROM verification_points WHERE id = ?1",
            [point_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(point_status, "pass");
}

#[test]
fn strict_plan_audit_requires_planr_executed_evidence() {
    let dir = tempdir().unwrap();
    let db = dir.path().join(".planr/planr.sqlite");

    planr()
        .current_dir(dir.path())
        .args(["--db", db.to_str().unwrap(), "project", "init", "Audit"])
        .assert()
        .success();

    let output = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "new",
            "Audit Target",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let plan: Value = serde_json::from_slice(&output).unwrap();
    let plan_id = plan["plan"]["id"].as_str().unwrap();

    let audit = planr()
        .current_dir(dir.path())
        .args([
            "--db",
            db.to_str().unwrap(),
            "--json",
            "plan",
            "audit",
            plan_id,
            "--strict",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let audit: Value = serde_json::from_slice(&audit).unwrap();
    assert_eq!(audit["mode"], "strict");
    assert_eq!(audit["holds"], false);
    let clauses = audit["clauses"].as_array().unwrap();
    let planr_clause = clauses
        .iter()
        .find(|clause| clause["clause"] == "planr_executed_verification")
        .unwrap();
    assert_eq!(planr_clause["required"], true);
    assert_eq!(planr_clause["pass"], false);
}
