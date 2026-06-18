//! Deterministic verification commands and strict audit extensions.

use super::App;
use crate::util::{collect_rows, command_exists, detect_client, short_id, worker_id};
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command as StdCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: usize = 12_000;

#[derive(Default)]
struct AuditFlags {
    strict: bool,
    autonomous: bool,
    require_points: bool,
    git_policy: String,
}

struct RunSpec {
    item_id: String,
    command: String,
    kind: String,
    point_id: Option<String>,
    cwd: Option<String>,
    expect_exit: i32,
    stdout_contains: Vec<String>,
    stderr_contains: Vec<String>,
    file_exists: Vec<String>,
    timeout_seconds: Option<u64>,
    allow_fail: bool,
    replay_of: Option<String>,
}

struct Capture {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    spawn_error: Option<String>,
    duration_ms: i64,
}

impl App {
    pub(crate) fn try_raw_dispatch(&self, raw_args: &[String]) -> Result<bool> {
        let args = strip_global_args(raw_args);
        if args.is_empty() {
            return Ok(false);
        }
        if args[0] == "verify" {
            self.verify_cli(&args[1..])?;
            return Ok(true);
        }
        if args[0] == "plan" && args.get(1).map(String::as_str) == Some("audit") {
            self.audit_cli(&args[2..])?;
            return Ok(true);
        }
        Ok(false)
    }

    fn verify_cli(&self, args: &[String]) -> Result<()> {
        match args.first().map(String::as_str) {
            Some("run") => self.verify_run(parse_run_spec(&args[1..])?),
            Some("replay") => self.verify_replay(&args[1..]),
            Some("point") => self.verify_point_cli(&args[1..]),
            Some("evidence") => self.verify_evidence_cli(&args[1..]),
            Some("--help") | Some("-h") | None => self.emit(json!({"usage": verify_usage()}), verify_usage()),
            Some(other) => bail!("unknown verify command: {other}"),
        }
    }

    fn verify_point_cli(&self, args: &[String]) -> Result<()> {
        match args.first().map(String::as_str) {
            Some("add") => self.verify_point_add(&args[1..]),
            Some("list") => {
                let (item, plan) = parse_scope(&args[1..])?;
                let points = self.list_verification_points(item.as_deref(), plan.as_deref())?;
                self.emit(json!({"points": points}), format!("{} verification point(s)", points.len()))
            }
            Some(other) => bail!("unknown verify point command: {other}"),
            None => bail!("verify point requires add or list"),
        }
    }

    fn verify_evidence_cli(&self, args: &[String]) -> Result<()> {
        match args.first().map(String::as_str) {
            Some("list") => {
                let (item, plan) = parse_scope(&args[1..])?;
                let evidence = self.list_verification_evidence(item.as_deref(), plan.as_deref())?;
                self.emit(json!({"evidence": evidence}), format!("{} verification evidence record(s)", evidence.len()))
            }
            Some(other) => bail!("unknown verify evidence command: {other}"),
            None => bail!("verify evidence requires list"),
        }
    }

    fn verify_point_add(&self, args: &[String]) -> Result<()> {
        let item_id = args.first().filter(|v| !v.starts_with('-')).ok_or_else(|| anyhow!("verify point add requires an item id"))?.to_string();
        let item = self.get_item(&item_id)?;
        let mut text = None;
        let mut kind = "custom".to_string();
        let mut source_type = "manual".to_string();
        let mut source_id: Option<String> = None;
        let mut required = true;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--text" => text = Some(take(args, &mut i, "--text")?),
                "--kind" => kind = take(args, &mut i, "--kind")?,
                "--source" => {
                    let source = take(args, &mut i, "--source")?;
                    if let Some((left, right)) = source.split_once(':') {
                        source_type = left.to_string();
                        source_id = Some(right.to_string());
                    } else {
                        source_id = Some(source);
                    }
                }
                "--optional" => {
                    required = false;
                    i += 1;
                }
                other => bail!("unknown verify point add argument: {other}"),
            }
        }
        let text = text.ok_or_else(|| anyhow!("verify point add requires --text"))?;
        let id = short_id("vpt");
        let plan_id = item.plan_path.as_deref().and_then(|p| self.plan_id_for_path(p).ok().flatten());
        self.conn.execute(
            "INSERT INTO verification_points(id, project_id, item_id, plan_id, source_type, source_id, kind, text, required, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', datetime('now'), datetime('now'))",
            params![id, item.project_id, item.id, plan_id, source_type, source_id, kind, text, if required { 1 } else { 0 }],
        )?;
        self.record_event("verification_point_created", Some(&item_id), json!({"point_id": id.clone(), "kind": kind, "required": required}))?;
        self.emit(json!({"point": self.get_verification_point(&id)?}), format!("verification point {id} added"))
    }

    fn verify_replay(&self, args: &[String]) -> Result<()> {
        let id = args.first().filter(|v| !v.starts_with('-')).ok_or_else(|| anyhow!("verify replay requires an evidence id"))?;
        let row = self.conn.query_row(
            "SELECT item_id, point_id, kind, command, cwd FROM verification_evidence WHERE id = ?1",
            params![id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?)),
        ).optional()?.ok_or_else(|| anyhow!("verification evidence not found: {id}"))?;
        let command = row.3.ok_or_else(|| anyhow!("verification evidence {id} has no replayable command"))?;
        self.verify_run(RunSpec {
            item_id: row.0,
            point_id: row.1,
            kind: row.2,
            command,
            cwd: row.4,
            expect_exit: 0,
            stdout_contains: Vec::new(),
            stderr_contains: Vec::new(),
            file_exists: Vec::new(),
            timeout_seconds: None,
            allow_fail: false,
            replay_of: Some(id.to_string()),
        })
    }

    fn verify_run(&self, spec: RunSpec) -> Result<()> {
        let item = self.get_item(&spec.item_id)?;
        if let Some(point_id) = spec.point_id.as_deref() {
            self.get_verification_point(point_id)?;
        }
        let project = self.default_project()?;
        let cwd = resolve_cwd(&self.root, spec.cwd.as_deref());
        let capture = run_command(&spec.command, &cwd, spec.timeout_seconds);
        let (status, assertions) = evaluate(&spec, &capture, &cwd);
        let passed = status == "pass";
        let run_id = short_id("run");
        let evidence_id = short_id("vev");
        let log_id = short_id("log");
        let stdout = truncate(&capture.stdout);
        let stderr = truncate(&capture.stderr);
        let cwd_display = cwd.to_string_lossy().to_string();
        let capability_state = json!({
            "client": detect_client(),
            "executor": "planr",
            "shell": shell_name(),
            "timed_out": capture.timed_out,
            "spawn_error": capture.spawn_error.clone(),
        });
        self.conn.execute(
            "INSERT INTO runs(id, project_id, item_id, worker_id, client, command, cwd, status, started_at, ended_at, exit_code, metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'), datetime('now'), ?9, ?10)",
            params![run_id, project.id, item.id, worker_id(), detect_client(), spec.command, cwd_display, status, capture.exit_code, json!({"verification_evidence_id": evidence_id.clone(), "kind": spec.kind.clone()}).to_string()],
        )?;
        self.conn.execute(
            "INSERT INTO verification_evidence(id, project_id, item_id, point_id, run_id, executor, kind, command, cwd, exit_code, status, stdout_summary, stderr_summary, assertions, artifacts, capability_state, duration_ms, replay_of, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'planr', ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, datetime('now'))",
            params![evidence_id, project.id, item.id, spec.point_id, run_id, spec.kind, spec.command, cwd_display, capture.exit_code, status, stdout.value, stderr.value, assertions.to_string(), json!([]).to_string(), capability_state.to_string(), capture.duration_ms, spec.replay_of],
        )?;
        if let Some(point_id) = spec.point_id.as_deref() {
            self.conn.execute("UPDATE verification_points SET status = ?1, evidence_id = ?2, updated_at = datetime('now') WHERE id = ?3", params![status, evidence_id, point_id])?;
        }
        let tests = if matches!(spec.kind.as_str(), "test" | "unit" | "integration" | "e2e") {
            serde_json::to_string(&vec![spec.command.clone()])?
        } else {
            serde_json::to_string(&Vec::<String>::new())?
        };
        let summary = format!("verification {status}: {} (exit {:?})", first_line(&spec.command), capture.exit_code);
        let blocked = if status == "pass" { None } else { Some(assertions.to_string()) };
        self.conn.execute(
            "INSERT INTO logs(id, project_id, item_id, run_id, kind, summary, commands, tests, blocked_or_unverified, created_at) VALUES (?1, ?2, ?3, ?4, 'verification', ?5, ?6, ?7, ?8, datetime('now'))",
            params![log_id, project.id, item.id, run_id, summary, serde_json::to_string(&vec![spec.command.clone()])?, tests, blocked],
        )?;
        self.conn.execute("UPDATE items SET last_heartbeat_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1", params![item.id])?;
        self.record_event("verification_executed", Some(&spec.item_id), json!({"evidence_id": evidence_id.clone(), "log_id": log_id.clone(), "run_id": run_id.clone(), "status": status, "kind": spec.kind.clone(), "point_id": spec.point_id.clone()}))?;
        let value = json!({
            "evidence": self.get_verification_evidence(&evidence_id)?,
            "run_id": run_id,
            "log_id": log_id,
            "status": status,
            "assertions": assertions,
            "stdout_truncated": stdout.truncated,
            "stderr_truncated": stderr.truncated,
        });
        self.emit(value, format!("verification {status}: {evidence_id} (exit {:?}, {} ms)", capture.exit_code, capture.duration_ms))?;
        if !passed && !spec.allow_fail {
            bail!("verification failed: {evidence_id} ({status})");
        }
        Ok(())
    }

    fn audit_cli(&self, args: &[String]) -> Result<()> {
        if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
            return self.emit(json!({"usage": audit_usage()}), audit_usage());
        }
        let plan_id = args[0].clone();
        let flags = parse_audit_flags(&args[1..])?;
        let value = self.plan_audit_ext(&plan_id, &flags)?;
        self.emit(value.clone(), Self::audit_human_ext(&value))
    }

    fn plan_audit_ext(&self, plan_id: &str, flags: &AuditFlags) -> Result<Value> {
        let mut value = self.plan_audit_value(plan_id)?;
        let points = self.list_verification_points(None, Some(plan_id))?;
        let evidence = self.list_verification_evidence(None, Some(plan_id))?;
        let agent_logs = self.agent_verification_logs(plan_id)?;
        let required: Vec<Value> = points.iter().filter(|p| p["required"].as_bool().unwrap_or(true)).cloned().collect();
        let passed_required = required.iter().filter(|p| p["status"].as_str() == Some("pass")).count();
        let strong: Vec<Value> = evidence.iter().filter(|e| e["executor"].as_str() == Some("planr") && e["status"].as_str() == Some("pass")).cloned().collect();
        let strict = flags.strict || flags.autonomous;
        let git = self.git_policy_value(&flags.git_policy)?;
        let git_required = matches!(flags.git_policy.as_str(), "require-clean" | "require-scoped");
        let mut clauses = value["clauses"].as_array().cloned().unwrap_or_default();
        clauses.push(json!({
            "clause": "verification_points",
            "pass": !flags.require_points || (!required.is_empty() && passed_required == required.len()),
            "required": flags.require_points,
            "total": points.len(),
            "required_points": required.len(),
            "passed_required_points": passed_required,
            "open": required.iter().filter(|p| p["status"].as_str() != Some("pass")).cloned().collect::<Vec<_>>(),
        }));
        clauses.push(json!({
            "clause": "planr_executed_verification",
            "pass": !strong.is_empty(),
            "required": strict,
            "evidence": strong,
            "agent_authored_logs": agent_logs,
            "detail": if strict { "strict/autonomous audit requires Planr-executed pass evidence" } else { "Planr-executed evidence is preferred; default mode remains backwards compatible" },
        }));
        clauses.push(json!({
            "clause": "git_policy",
            "pass": git["pass"].as_bool().unwrap_or(false),
            "required": git_required,
            "policy": flags.git_policy.clone(),
            "evidence": git,
        }));
        let holds = clauses.iter().all(|c| c["pass"].as_bool().unwrap_or(false) || !c["required"].as_bool().unwrap_or(true));
        value["clauses"] = Value::Array(clauses);
        value["holds"] = json!(holds);
        value["mode"] = json!(if flags.autonomous { "autonomous" } else if flags.strict { "strict" } else { "default" });
        value["git_policy"] = json!(flags.git_policy.clone());
        value["verification"] = json!({"points": points, "evidence": evidence, "strong_passes": strong.len(), "agent_authored_logs": agent_logs});
        if !holds {
            value["next"] = json!(audit_next(&value, flags));
        }
        Ok(value)
    }

    fn audit_human_ext(value: &Value) -> String {
        let mut out = String::new();
        for clause in value["clauses"].as_array().into_iter().flatten() {
            let pass = clause["pass"].as_bool().unwrap_or(false);
            let required = clause["required"].as_bool().unwrap_or(true);
            let verdict = if pass { "PASS" } else if required { "FAIL" } else { "SKIP" };
            out.push_str(&format!("{verdict} {}", clause["clause"].as_str().unwrap_or("clause")));
            if let Some(detail) = clause["detail"].as_str() {
                out.push_str(&format!(" - {detail}"));
            }
            for open in clause["open"].as_array().into_iter().flatten() {
                out.push_str(&format!("\n  open: {} [{}]", open["id"].as_str().unwrap_or_default(), open["status"].as_str().or(open["approval_status"].as_str()).unwrap_or_default()));
            }
            out.push('\n');
        }
        if value["holds"].as_bool().unwrap_or(false) {
            out.push_str("contract holds");
        } else {
            out.push_str("contract open");
            if let Some(next) = value["next"].as_str() {
                out.push_str(&format!("\nnext: {next}"));
            }
        }
        out
    }

    fn get_verification_point(&self, id: &str) -> Result<Value> {
        self.conn.query_row("SELECT id, item_id, plan_id, source_type, source_id, kind, text, required, status, evidence_id, created_at, updated_at FROM verification_points WHERE id = ?1", params![id], point_row).optional()?.ok_or_else(|| anyhow!("verification point not found: {id}"))
    }

    fn list_verification_points(&self, item_id: Option<&str>, plan_id: Option<&str>) -> Result<Vec<Value>> {
        if let Some(plan_id) = plan_id {
            let plan = self.get_plan(plan_id)?;
            let mut stmt = self.conn.prepare("SELECT vp.id, vp.item_id, vp.plan_id, vp.source_type, vp.source_id, vp.kind, vp.text, vp.required, vp.status, vp.evidence_id, vp.created_at, vp.updated_at FROM verification_points vp LEFT JOIN items i ON i.id = vp.item_id WHERE vp.plan_id = ?1 OR i.plan_path = ?2 ORDER BY vp.created_at")?;
            return collect_rows(stmt.query_map(params![plan_id, plan.path], point_row)?);
        }
        if let Some(item_id) = item_id {
            let mut stmt = self.conn.prepare("SELECT id, item_id, plan_id, source_type, source_id, kind, text, required, status, evidence_id, created_at, updated_at FROM verification_points WHERE item_id = ?1 ORDER BY created_at")?;
            return collect_rows(stmt.query_map(params![item_id], point_row)?);
        }
        let mut stmt = self.conn.prepare("SELECT id, item_id, plan_id, source_type, source_id, kind, text, required, status, evidence_id, created_at, updated_at FROM verification_points ORDER BY created_at")?;
        collect_rows(stmt.query_map([], point_row)?)
    }

    fn get_verification_evidence(&self, id: &str) -> Result<Value> {
        self.conn.query_row("SELECT id, item_id, point_id, run_id, executor, kind, command, cwd, exit_code, status, stdout_summary, stderr_summary, assertions, artifacts, capability_state, duration_ms, replay_of, created_at FROM verification_evidence WHERE id = ?1", params![id], evidence_row).optional()?.ok_or_else(|| anyhow!("verification evidence not found: {id}"))
    }

    fn list_verification_evidence(&self, item_id: Option<&str>, plan_id: Option<&str>) -> Result<Vec<Value>> {
        if let Some(plan_id) = plan_id {
            let plan = self.get_plan(plan_id)?;
            let mut stmt = self.conn.prepare("SELECT ve.id, ve.item_id, ve.point_id, ve.run_id, ve.executor, ve.kind, ve.command, ve.cwd, ve.exit_code, ve.status, ve.stdout_summary, ve.stderr_summary, ve.assertions, ve.artifacts, ve.capability_state, ve.duration_ms, ve.replay_of, ve.created_at FROM verification_evidence ve JOIN items i ON i.id = ve.item_id WHERE i.plan_path = ?1 ORDER BY ve.created_at")?;
            return collect_rows(stmt.query_map(params![plan.path], evidence_row)?);
        }
        if let Some(item_id) = item_id {
            let mut stmt = self.conn.prepare("SELECT id, item_id, point_id, run_id, executor, kind, command, cwd, exit_code, status, stdout_summary, stderr_summary, assertions, artifacts, capability_state, duration_ms, replay_of, created_at FROM verification_evidence WHERE item_id = ?1 ORDER BY created_at")?;
            return collect_rows(stmt.query_map(params![item_id], evidence_row)?);
        }
        let mut stmt = self.conn.prepare("SELECT id, item_id, point_id, run_id, executor, kind, command, cwd, exit_code, status, stdout_summary, stderr_summary, assertions, artifacts, capability_state, duration_ms, replay_of, created_at FROM verification_evidence ORDER BY created_at")?;
        collect_rows(stmt.query_map([], evidence_row)?)
    }

    fn agent_verification_logs(&self, plan_id: &str) -> Result<Vec<Value>> {
        let plan = self.get_plan(plan_id)?;
        let mut stmt = self.conn.prepare("SELECT l.id, l.item_id, l.summary, l.commands, l.created_at FROM logs l JOIN items i ON i.id = l.item_id WHERE i.plan_path = ?1 AND l.kind = 'verification' AND (l.run_id IS NULL OR l.run_id NOT IN (SELECT run_id FROM verification_evidence WHERE run_id IS NOT NULL)) ORDER BY l.created_at")?;
        collect_rows(stmt.query_map(params![plan.path], |row| Ok(json!({"id": row.get::<_, String>(0)?, "item_id": row.get::<_, String>(1)?, "summary": row.get::<_, String>(2)?, "commands": parse_json_array(row.get::<_, Option<String>>(3)?), "created_at": row.get::<_, String>(4)?})))?)
    }

    fn git_policy_value(&self, policy: &str) -> Result<Value> {
        if policy == "off" {
            return Ok(json!({"policy": policy, "pass": true, "status": "skipped"}));
        }
        if !self.root.join(".git").exists() {
            return Ok(json!({"policy": policy, "pass": policy == "auto", "status": if policy == "auto" { "not_available" } else { "blocked" }, "detail": "Git metadata is not present; Git evidence is adaptive, not a global requirement"}));
        }
        if !command_exists("git") {
            return Ok(json!({"policy": policy, "pass": policy == "auto", "status": if policy == "auto" { "not_available" } else { "blocked" }, "detail": "git executable was not found"}));
        }
        let output = match StdCommand::new("git").arg("-C").arg(&self.root).args(["status", "--porcelain"]).output() {
            Ok(output) => output,
            Err(error) => return Ok(json!({"policy": policy, "pass": policy == "auto", "status": if policy == "auto" { "not_available" } else { "blocked" }, "detail": error.to_string()})),
        };
        let dirty = String::from_utf8_lossy(&output.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(ToOwned::to_owned).collect::<Vec<_>>();
        let clean = output.status.success() && dirty.is_empty();
        let pass = match policy {
            "auto" => true,
            "require-clean" | "require-scoped" => clean,
            other => bail!("unsupported git policy: {other}"),
        };
        Ok(json!({"policy": policy, "pass": pass, "status": if clean { "clean" } else { "dirty" }, "dirty_files": dirty, "detail": if policy == "require-scoped" && !clean { "require-scoped currently requires a clean working tree unless a higher-level loop supplies scoped file policy" } else { "" }}))
    }
}

fn point_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({"id": row.get::<_, String>(0)?, "item_id": row.get::<_, Option<String>>(1)?, "plan_id": row.get::<_, Option<String>>(2)?, "source_type": row.get::<_, Option<String>>(3)?, "source_id": row.get::<_, Option<String>>(4)?, "kind": row.get::<_, String>(5)?, "text": row.get::<_, String>(6)?, "required": row.get::<_, i64>(7)? != 0, "status": row.get::<_, String>(8)?, "evidence_id": row.get::<_, Option<String>>(9)?, "created_at": row.get::<_, String>(10)?, "updated_at": row.get::<_, String>(11)?}))
}

fn evidence_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({"id": row.get::<_, String>(0)?, "item_id": row.get::<_, String>(1)?, "point_id": row.get::<_, Option<String>>(2)?, "run_id": row.get::<_, Option<String>>(3)?, "executor": row.get::<_, String>(4)?, "kind": row.get::<_, String>(5)?, "command": row.get::<_, Option<String>>(6)?, "cwd": row.get::<_, Option<String>>(7)?, "exit_code": row.get::<_, Option<i64>>(8)?, "status": row.get::<_, String>(9)?, "stdout_summary": row.get::<_, Option<String>>(10)?, "stderr_summary": row.get::<_, Option<String>>(11)?, "assertions": parse_json_array(row.get::<_, Option<String>>(12)?), "artifacts": parse_json_array(row.get::<_, Option<String>>(13)?), "capability_state": parse_json_or_null(row.get::<_, Option<String>>(14)?), "duration_ms": row.get::<_, Option<i64>>(15)?, "replay_of": row.get::<_, Option<String>>(16)?, "created_at": row.get::<_, String>(17)?}))
}

fn parse_run_spec(args: &[String]) -> Result<RunSpec> {
    let item_id = args.first().filter(|v| !v.starts_with('-')).ok_or_else(|| anyhow!("verify run requires an item id"))?.to_string();
    let mut spec = RunSpec { item_id, command: String::new(), kind: "custom".to_string(), point_id: None, cwd: None, expect_exit: 0, stdout_contains: Vec::new(), stderr_contains: Vec::new(), file_exists: Vec::new(), timeout_seconds: None, allow_fail: false, replay_of: None };
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--cmd" => spec.command = take(args, &mut i, "--cmd")?,
            "--kind" => spec.kind = take(args, &mut i, "--kind")?,
            "--point" => spec.point_id = Some(take(args, &mut i, "--point")?),
            "--cwd" => spec.cwd = Some(take(args, &mut i, "--cwd")?),
            "--expect-exit" => spec.expect_exit = take(args, &mut i, "--expect-exit")?.parse()?,
            "--assert-stdout-contains" => spec.stdout_contains.push(take(args, &mut i, "--assert-stdout-contains")?),
            "--assert-stderr-contains" => spec.stderr_contains.push(take(args, &mut i, "--assert-stderr-contains")?),
            "--assert-file-exists" => spec.file_exists.push(take(args, &mut i, "--assert-file-exists")?),
            "--timeout-seconds" => spec.timeout_seconds = Some(take(args, &mut i, "--timeout-seconds")?.parse()?),
            "--allow-fail" => { spec.allow_fail = true; i += 1; }
            other => bail!("unknown verify run argument: {other}"),
        }
    }
    if spec.command.trim().is_empty() {
        bail!("verify run requires --cmd");
    }
    Ok(spec)
}

fn parse_audit_flags(args: &[String]) -> Result<AuditFlags> {
    let mut flags = AuditFlags { git_policy: "auto".to_string(), ..AuditFlags::default() };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--strict" => { flags.strict = true; i += 1; }
            "--autonomous" => { flags.autonomous = true; flags.strict = true; i += 1; }
            "--require-verification-points" => { flags.require_points = true; i += 1; }
            "--git-policy" => flags.git_policy = take(args, &mut i, "--git-policy")?,
            other => bail!("unknown plan audit argument: {other}"),
        }
    }
    if !matches!(flags.git_policy.as_str(), "off" | "auto" | "require-clean" | "require-scoped") {
        bail!("unsupported git policy: {}", flags.git_policy);
    }
    Ok(flags)
}

fn parse_scope(args: &[String]) -> Result<(Option<String>, Option<String>)> {
    let mut item = None;
    let mut plan = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--plan" => plan = Some(take(args, &mut i, "--plan")?),
            value if value.starts_with('-') => bail!("unknown scope argument: {value}"),
            value => { item = Some(value.to_string()); i += 1; }
        }
    }
    Ok((item, plan))
}

fn take(args: &[String], index: &mut usize, flag: &str) -> Result<String> {
    let value = args.get(*index + 1).ok_or_else(|| anyhow!("{flag} requires a value"))?;
    *index += 2;
    Ok(value.to_string())
}

fn strip_global_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" | "--no-color" => i += 1,
            "--db" => i += 2,
            value if value.starts_with("--db=") => i += 1,
            value => { out.push(value.to_string()); i += 1; }
        }
    }
    out
}

fn resolve_cwd(root: &PathBuf, cwd: Option<&str>) -> PathBuf {
    match cwd {
        Some(cwd) => {
            let path = PathBuf::from(cwd);
            if path.is_absolute() { path } else { root.join(path) }
        }
        None => root.clone(),
    }
}

fn run_command(command: &str, cwd: &PathBuf, timeout_seconds: Option<u64>) -> Capture {
    let start = Instant::now();
    let mut child_cmd = shell_command(command);
    child_cmd.current_dir(cwd).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match child_cmd.spawn() {
        Ok(child) => child,
        Err(error) => return Capture { exit_code: None, stdout: String::new(), stderr: String::new(), timed_out: false, spawn_error: Some(error.to_string()), duration_ms: start.elapsed().as_millis() as i64 },
    };
    let deadline = timeout_seconds.map(|seconds| Instant::now() + Duration::from_secs(seconds));
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if deadline.map(|d| Instant::now() >= d).unwrap_or(false) {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().ok();
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => break None,
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() { let _ = pipe.read_to_string(&mut stdout); }
    if let Some(mut pipe) = child.stderr.take() { let _ = pipe.read_to_string(&mut stderr); }
    Capture { exit_code: status.and_then(|s| s.code()), stdout, stderr, timed_out, spawn_error: None, duration_ms: start.elapsed().as_millis() as i64 }
}

fn shell_command(command: &str) -> StdCommand {
    #[cfg(windows)]
    { let mut cmd = StdCommand::new("cmd"); cmd.args(["/C", command]); cmd }
    #[cfg(not(windows))]
    { let mut cmd = StdCommand::new("sh"); cmd.args(["-lc", command]); cmd }
}

fn shell_name() -> &'static str {
    #[cfg(windows)]
    { "cmd" }
    #[cfg(not(windows))]
    { "sh" }
}

fn evaluate(spec: &RunSpec, capture: &Capture, cwd: &PathBuf) -> (&'static str, Value) {
    let mut ok = true;
    let mut assertions = Vec::new();
    if let Some(error) = &capture.spawn_error { ok = false; assertions.push(json!({"type": "spawn", "pass": false, "error": error})); }
    if capture.timed_out { ok = false; assertions.push(json!({"type": "timeout", "pass": false, "timeout_seconds": spec.timeout_seconds})); }
    let exit_ok = capture.exit_code == Some(spec.expect_exit);
    if !exit_ok { ok = false; }
    assertions.push(json!({"type": "exit_code", "expected": spec.expect_exit, "actual": capture.exit_code, "pass": exit_ok}));
    for needle in &spec.stdout_contains { let pass = capture.stdout.contains(needle); if !pass { ok = false; } assertions.push(json!({"type": "stdout_contains", "needle": needle, "pass": pass})); }
    for needle in &spec.stderr_contains { let pass = capture.stderr.contains(needle); if !pass { ok = false; } assertions.push(json!({"type": "stderr_contains", "needle": needle, "pass": pass})); }
    for file in &spec.file_exists {
        let path = PathBuf::from(file);
        let path = if path.is_absolute() { path } else { cwd.join(path) };
        let pass = fs::metadata(&path).is_ok();
        if !pass { ok = false; }
        assertions.push(json!({"type": "file_exists", "path": path.to_string_lossy(), "pass": pass}));
    }
    let status = if ok { "pass" } else if capture.timed_out || capture.spawn_error.is_some() { "blocked" } else { "fail" };
    (status, Value::Array(assertions))
}

struct Trunc { value: String, truncated: bool }

fn truncate(value: &str) -> Trunc {
    if value.len() <= OUTPUT_LIMIT { return Trunc { value: value.to_string(), truncated: false }; }
    let mut end = OUTPUT_LIMIT;
    while !value.is_char_boundary(end) { end -= 1; }
    Trunc { value: format!("{}\n[truncated after {} bytes]", &value[..end], OUTPUT_LIMIT), truncated: true }
}

fn parse_json_array(raw: Option<String>) -> Value {
    raw.and_then(|text| serde_json::from_str(&text).ok()).filter(Value::is_array).unwrap_or_else(|| json!([]))
}

fn parse_json_or_null(raw: Option<String>) -> Value {
    raw.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null)
}

fn first_line(value: &str) -> String {
    value.lines().next().unwrap_or(value).chars().take(80).collect()
}

fn audit_next(value: &Value, flags: &AuditFlags) -> String {
    if flags.require_points && value["verification"]["points"].as_array().map_or(0, Vec::len) == 0 {
        return "planr verify point add <item-id> --text \"<acceptance-specific assertion>\"".to_string();
    }
    if value["verification"]["strong_passes"].as_i64().unwrap_or(0) == 0 {
        return "planr verify run <item-id> --cmd \"<replayable command>\" --assert-stdout-contains \"<observable result>\"".to_string();
    }
    "planr plan audit <plan-id> --strict".to_string()
}

fn verify_usage() -> String {
    "usage:\n  planr verify run <item-id> --cmd <command> [--kind test|browser|lint|typecheck|custom] [--point <point-id>] [--assert-stdout-contains <text>] [--assert-stderr-contains <text>] [--assert-file-exists <path>] [--expect-exit <code>] [--timeout-seconds <n>] [--allow-fail]\n  planr verify point add <item-id> --text <assertion> [--kind <kind>] [--source <type:id>] [--optional]\n  planr verify point list [item-id] [--plan <plan-id>]\n  planr verify evidence list [item-id] [--plan <plan-id>]\n  planr verify replay <evidence-id>".to_string()
}

fn audit_usage() -> String {
    "usage:\n  planr plan audit <plan-id> [--strict] [--autonomous] [--require-verification-points] [--git-policy off|auto|require-clean|require-scoped]".to_string()
}
