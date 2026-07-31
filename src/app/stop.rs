use super::App;
use crate::cli::{StopArgs, StopCommand};
use crate::storage::{ensure_schema, open_db};
use crate::util::print_json;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::env;
use std::io::Read;
use std::path::PathBuf;

const TOTAL_CONTINUATION_LIMIT: i64 = 6;
const SAME_GAP_CONTINUATION_LIMIT: i64 = 2;
const NON_ACTIONABLE_SAME_GAP_LIMIT: i64 = 1;
const CONTINUATION_REASON_BYTE_LIMIT: usize = 700;
const CONTINUATION_FIELD_BYTE_LIMIT: usize = 160;

pub(crate) fn run_stop_cli(
    root: PathBuf,
    db_path: PathBuf,
    _json_mode: bool,
    args: StopArgs,
) -> Result<()> {
    if let Some(command) = args.command {
        let conn = open_db(&db_path).and_then(|conn| {
            ensure_schema(&conn)?;
            Ok(conn)
        })?;
        let app = App::new(conn, root, db_path, true, false);
        return app.stop_command(command);
    }
    let envelope = match read_stop_envelope(args.input) {
        Ok(value) => value,
        Err(err) => {
            let _ = err;
            return emit_stop_decision(stop_allow());
        }
    };
    let conn = match open_db(&db_path).and_then(|conn| {
        ensure_schema(&conn)?;
        Ok(conn)
    }) {
        Ok(conn) => conn,
        Err(err) => {
            let _ = err;
            return emit_stop_decision(stop_allow());
        }
    };
    let app = App::new(conn, root, db_path, true, false);
    let decision = match app.stop_decision(&envelope) {
        Ok(decision) => decision,
        Err(_err) => stop_allow(),
    };
    emit_stop_decision(decision)
}

fn emit_stop_decision(value: Value) -> Result<()> {
    print_json(&value)?;
    Ok(())
}

fn read_stop_envelope(input: Option<PathBuf>) -> Result<Value> {
    let mut text = String::new();
    match input {
        Some(path) => text = std::fs::read_to_string(path)?,
        None => {
            std::io::stdin().read_to_string(&mut text)?;
        }
    }
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    Ok(serde_json::from_str(&text)?)
}

fn stop_allow() -> Value {
    json!({})
}

fn stop_block(reason: String) -> Value {
    json!({
        "decision": "block",
        "reason": reason,
    })
}

impl App {
    pub(crate) fn stop(&self, args: StopArgs) -> Result<()> {
        if let Some(command) = args.command {
            return self.stop_command(command);
        }
        let envelope = match read_stop_envelope(args.input) {
            Ok(value) => value,
            Err(err) => {
                let _ = err;
                return emit_stop_decision(stop_allow());
            }
        };
        let decision = match self.stop_decision(&envelope) {
            Ok(decision) => decision,
            Err(_err) => stop_allow(),
        };
        emit_stop_decision(decision)
    }

    pub(crate) fn stop_command(&self, command: StopCommand) -> Result<()> {
        match command {
            StopCommand::Activate(args) => {
                let session_id = stop_session_identity(args.session)?;
                self.record_active_stop_binding_for_plan(&args.plan, &session_id)?;
                emit_stop_decision(json!({
                    "status": "activated",
                    "scope": {"kind": "plan", "id": args.plan},
                    "session_id": session_id
                }))
            }
            StopCommand::Deactivate(args) => {
                let session_id = stop_session_identity(args.session)?;
                self.deactivate_stop_binding(args.plan.as_deref(), &session_id)?;
                emit_stop_decision(json!({
                    "status": "deactivated",
                    "session_id": session_id
                }))
            }
        }
    }

    pub(crate) fn stop_decision(&self, envelope: &Value) -> Result<Value> {
        let Some(active) = self.active_stop_scope(envelope)? else {
            return Ok(stop_allow());
        };
        if active.cancelled {
            self.clear_stop_state(&active.key)?;
            return Ok(stop_allow());
        }
        let evaluation = self.stop_evaluation(&active)?;
        if evaluation.allows {
            self.clear_stop_state(&active.key)?;
            return Ok(stop_allow());
        }
        let fingerprint = stop_gap_fingerprint(&evaluation.proof, evaluation.audit.as_ref());
        let has_actionable = evaluation.actionable;
        let state = self.bump_stop_state(&active, &fingerprint)?;
        let same_limit = same_gap_limit(has_actionable);
        if let Some(exhaustion) = continuation_exhaustion(has_actionable, &state) {
            self.clear_stop_state(&active.key)?;
            let _ = exhaustion;
            return Ok(stop_allow());
        }
        Ok(stop_block(compact_continuation_reason(
            "plan",
            &active.scope_id,
            &evaluation.proof,
            evaluation.audit.as_ref(),
            state.total_count,
            state.same_count,
            same_limit,
        )))
    }

    fn stop_evaluation(&self, active: &ActiveStopScope) -> Result<StopEvaluation> {
        let plan = self.get_plan(&active.scope_id)?;
        if plan.archived {
            return Ok(StopEvaluation {
                allows: true,
                proof: json!({}),
                audit: None,
                actionable: false,
            });
        }
        let audit = self.plan_audit_value(&plan.id)?;
        if audit["holds"].as_bool() == Some(true) {
            return Ok(StopEvaluation {
                allows: true,
                proof: audit["proof"].clone(),
                audit: Some(audit),
                actionable: false,
            });
        }
        let proof = audit["proof"].clone();
        let actionable = plan_audit_actionable(&audit, &proof);
        Ok(StopEvaluation {
            allows: false,
            proof,
            audit: Some(audit),
            actionable,
        })
    }

    pub(crate) fn record_active_stop_binding_for_plan(
        &self,
        plan_id: &str,
        session_id: &str,
    ) -> Result<()> {
        let project_id = self.default_project()?.id;
        let plan = self.get_plan(plan_id)?;
        if plan.project_id != project_id {
            anyhow::bail!("stop activation plan must belong to the default project");
        }
        let key = format!("session:{project_id}:{session_id}");
        self.conn.execute(
            "INSERT INTO stop_active_bindings(key, project_id, session_id, plan_id, status, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'active', datetime('now'))
             ON CONFLICT(project_id, session_id)
             DO UPDATE SET plan_id = excluded.plan_id, status = 'active', updated_at = datetime('now')",
            params![key, project_id, session_id, plan_id],
        )?;
        Ok(())
    }

    fn deactivate_stop_binding(&self, plan_id: Option<&str>, session_id: &str) -> Result<()> {
        let project_id = self.default_project()?.id;
        self.conn.execute(
            "UPDATE stop_active_bindings
             SET status = 'cancelled', updated_at = datetime('now')
             WHERE project_id = ?1
               AND session_id = ?2
               AND (?3 IS NULL OR plan_id = ?3)",
            params![project_id, session_id, plan_id],
        )?;
        self.conn.execute(
            "DELETE FROM stop_enforcement_state
             WHERE project_id = ?1
               AND session_id = ?2
               AND (?3 IS NULL OR scope_id = ?3)",
            params![project_id, session_id, plan_id],
        )?;
        Ok(())
    }

    fn active_stop_scope(&self, envelope: &Value) -> Result<Option<ActiveStopScope>> {
        let Some(session_id) = envelope["session_id"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                env::var("CODEX_THREAD_ID")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
        else {
            return Ok(None);
        };
        let project_id = self.default_project()?.id;
        let (scope_id, binding_status) = match self.active_stop_binding(&project_id, &session_id)? {
            Some(binding) => binding,
            None => return Ok(None),
        };
        let key = format!("{project_id}:{session_id}:plan:{scope_id}");
        Ok(Some(ActiveStopScope {
            key,
            scope_id,
            session_id,
            cancelled: binding_status == "cancelled",
        }))
    }

    fn active_stop_binding(
        &self,
        project_id: &str,
        session_id: &str,
    ) -> Result<Option<(String, String)>> {
        self.conn
            .query_row(
                "SELECT plan_id, status
                 FROM stop_active_bindings
                 WHERE project_id = ?1
                   AND status IN ('active','cancelled')
                   AND session_id = ?2
                 LIMIT 1",
                params![project_id, session_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(Into::into)
    }

    fn clear_stop_state(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM stop_enforcement_state WHERE key = ?1", [key])?;
        Ok(())
    }

    fn bump_stop_state(&self, active: &ActiveStopScope, fingerprint: &str) -> Result<StopState> {
        let existing = self
            .conn
            .query_row(
                "SELECT fingerprint, total_count, same_count FROM stop_enforcement_state WHERE key = ?1",
                [&active.key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let state = next_stop_counts(
            existing
                .as_ref()
                .map(|(old, total, same)| (old.as_str(), *total, *same)),
            fingerprint,
        );
        let project_id = self.default_project()?.id;
        self.conn.execute(
            "INSERT INTO stop_enforcement_state(key, project_id, session_id, scope_kind, scope_id, plan_id, fingerprint, total_count, same_count, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET fingerprint = excluded.fingerprint, total_count = excluded.total_count, same_count = excluded.same_count, updated_at = datetime('now')",
            params![
                active.key,
                project_id,
                active.session_id,
                "plan",
                active.scope_id,
                active.scope_id,
                fingerprint,
                state.total_count,
                state.same_count
            ],
        )?;
        Ok(state)
    }
}

struct ActiveStopScope {
    key: String,
    scope_id: String,
    session_id: String,
    cancelled: bool,
}

struct StopEvaluation {
    allows: bool,
    proof: Value,
    audit: Option<Value>,
    actionable: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct StopState {
    total_count: i64,
    same_count: i64,
}

fn next_stop_counts(existing: Option<(&str, i64, i64)>, fingerprint: &str) -> StopState {
    match existing {
        Some((old, total, same)) if old == fingerprint => StopState {
            total_count: total + 1,
            same_count: same + 1,
        },
        Some((_old, total, _same)) => StopState {
            total_count: total + 1,
            same_count: 1,
        },
        None => StopState {
            total_count: 1,
            same_count: 1,
        },
    }
}

fn same_gap_limit(has_actionable: bool) -> i64 {
    if has_actionable {
        SAME_GAP_CONTINUATION_LIMIT
    } else {
        NON_ACTIONABLE_SAME_GAP_LIMIT
    }
}

fn continuation_exhaustion(has_actionable: bool, state: &StopState) -> Option<&'static str> {
    if state.total_count > TOTAL_CONTINUATION_LIMIT {
        Some("total")
    } else if state.same_count > same_gap_limit(has_actionable) {
        Some("same")
    } else {
        None
    }
}

fn stop_session_identity(explicit: Option<String>) -> Result<String> {
    explicit
        .or_else(|| env::var("CODEX_THREAD_ID").ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("stop activation requires --session or CODEX_THREAD_ID"))
}

fn stop_gap_fingerprint(proof: &Value, audit: Option<&Value>) -> String {
    let value = json!({
        "status": proof["status"],
        "actionable_gaps": proof["actionable_gaps"],
        "non_actionable_blockers": proof["non_actionable_blockers"],
        "suggested_next_action": proof["suggested_next_action"],
        "next_action": proof["next_action"],
        "audit_holds": audit.map(|value| value["holds"].clone()),
        "audit_clauses": audit.map(|value| value["clauses"].clone()),
        "audit_next": audit.map(|value| value["next"].clone()),
    });
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&value).unwrap_or_default())
    )
}

fn compact_continuation_reason(
    scope_kind: &str,
    scope_id: &str,
    proof: &Value,
    audit: Option<&Value>,
    total_count: i64,
    same_count: i64,
    same_limit: i64,
) -> String {
    let mut parts = vec![format!(
        "planr: active goal {scope_kind} {} is not proven by canonical Planr audit",
        bounded_field("scope", scope_id)
    )];
    let actionable = proof["actionable_gaps"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let blockers = proof["non_actionable_blockers"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if !actionable.is_empty() {
        parts.push(format!(
            "actionable gap: {}",
            format_gap_bounded(actionable.first().unwrap())
        ));
    } else if !blockers.is_empty() {
        parts.push(format!(
            "non-actionable blocker: {}",
            format_gap_bounded(blockers.first().unwrap())
        ));
    } else if let Some(clause) = first_failed_audit_clause(audit) {
        let name = clause["clause"].as_str().unwrap_or("unknown_clause");
        let detail = clause["detail"]
            .as_str()
            .unwrap_or("required clause failed");
        if let Some(open) = clause["open"].as_array().and_then(|open| open.first()) {
            let open_id = bounded_field("open", open["id"].as_str().unwrap_or("unknown"));
            let status = bounded_field(
                "status",
                open["status"]
                    .as_str()
                    .or(open["approval_status"].as_str())
                    .unwrap_or("open"),
            );
            parts.push(format!(
                "audit gap: {} open {open_id} {status}",
                bounded_field("clause", name)
            ));
        } else {
            parts.push(format!(
                "audit gap: {} {}",
                bounded_field("clause", name),
                bounded_field("detail", detail)
            ));
        }
    } else {
        parts.push(format!(
            "proof status: {}",
            bounded_field("status", proof["status"].as_str().unwrap_or("unknown"))
        ));
    }
    let next = proof["next_action"]
        .as_str()
        .or_else(|| proof["suggested_next_action"].as_str())
        .or_else(|| audit.and_then(|value| value["next"].as_str()))
        .unwrap_or("none");
    if next != "none" {
        parts.push(format!("next: {}", bounded_field("next", next)));
    }
    parts.push(format!(
        "budget: total {total_count}/{TOTAL_CONTINUATION_LIMIT}, same-gap {same_count}/{same_limit}"
    ));
    bounded_reason(parts)
}

fn first_failed_audit_clause(audit: Option<&Value>) -> Option<&Value> {
    audit?["clauses"].as_array()?.iter().find(|clause| {
        clause["pass"].as_bool() != Some(true) && clause["required"].as_bool().unwrap_or(true)
    })
}

fn plan_audit_actionable(audit: &Value, proof: &Value) -> bool {
    let Some(clause) = first_failed_audit_clause(Some(audit)) else {
        return false;
    };
    if clause["clause"] != "verification_logged" && clause["clause"] != "evidence_coverage" {
        return true;
    }
    if !proof["actionable_gaps"]
        .as_array()
        .map(|gaps| gaps.is_empty())
        .unwrap_or(true)
    {
        return true;
    }
    clause["criteria"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|criterion| criterion["actionable_now"].as_bool() == Some(true))
}

fn format_gap_bounded(gap: &Value) -> String {
    let criterion = gap["criterion_id"].as_str().unwrap_or("unknown-criterion");
    let requirement = gap["requirement_id"]
        .as_str()
        .unwrap_or("unknown-requirement");
    let status = gap["status"].as_str().unwrap_or("unknown-status");
    let reason = gap["reason"].as_str().unwrap_or("unknown-reason");
    [
        bounded_field("criterion", criterion),
        "/".to_string(),
        bounded_field("requirement", requirement),
        " ".to_string(),
        bounded_field("status", status),
        " ".to_string(),
        bounded_field("reason", reason),
    ]
    .join("")
}

fn bounded_field(label: &str, value: &str) -> String {
    if value.len() <= CONTINUATION_FIELD_BYTE_LIMIT {
        return value.to_string();
    }
    let digest = short_digest(value);
    let marker = format!("... [truncated {label} sha256:{digest}]");
    let keep = CONTINUATION_FIELD_BYTE_LIMIT.saturating_sub(marker.len());
    format!("{}{}", utf8_prefix(value, keep), marker)
}

fn bounded_reason(parts: Vec<String>) -> String {
    let joined = parts.join("; ");
    if joined.len() <= CONTINUATION_REASON_BYTE_LIMIT {
        return joined;
    }
    let digest = short_digest(&joined);
    let marker = format!("; [truncated reason sha256:{digest}]");
    let keep = CONTINUATION_REASON_BYTE_LIMIT.saturating_sub(marker.len());
    format!("{}{}", utf8_prefix(&joined, keep), marker)
}

fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = 0;
    for (idx, ch) in value.char_indices() {
        let next = idx + ch.len_utf8();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    &value[..end]
}

fn short_digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
        .chars()
        .take(16)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        CONTINUATION_REASON_BYTE_LIMIT, StopState, compact_continuation_reason,
        continuation_exhaustion, next_stop_counts, plan_audit_actionable, stop_gap_fingerprint,
    };
    use serde_json::json;

    #[test]
    fn gap_fingerprint_moves_when_canonical_gap_moves() {
        let first = stop_gap_fingerprint(
            &json!({
                "status": "not_proven",
                "actionable_gaps": [{"criterion_id":"a","reason":"missing_observation"}],
                "non_actionable_blockers": [],
                "suggested_next_action": "collect missing trusted evidence",
                "next_action": "collect missing trusted evidence"
            }),
            None,
        );
        let second = stop_gap_fingerprint(
            &json!({
                "status": "not_proven",
                "actionable_gaps": [{"criterion_id":"a","reason":"stale_target"}],
                "non_actionable_blockers": [],
                "suggested_next_action": "refresh stale evidence",
                "next_action": "refresh stale evidence"
            }),
            None,
        );
        assert_ne!(first, second);
    }

    #[test]
    fn same_gap_count_resets_only_when_gap_fingerprint_moves() {
        assert_eq!(
            next_stop_counts(Some(("fp-a", 2, 2)), "fp-a"),
            StopState {
                total_count: 3,
                same_count: 3
            }
        );
        assert_eq!(
            next_stop_counts(Some(("fp-a", 2, 2)), "fp-b"),
            StopState {
                total_count: 3,
                same_count: 1
            }
        );
    }

    #[test]
    fn continuation_budgets_cover_total_and_non_actionable_limits() {
        assert_eq!(
            continuation_exhaustion(
                true,
                &StopState {
                    total_count: 7,
                    same_count: 1
                }
            ),
            Some("total")
        );
        assert_eq!(
            continuation_exhaustion(
                false,
                &StopState {
                    total_count: 1,
                    same_count: 2
                }
            ),
            Some("same")
        );
        assert_eq!(
            continuation_exhaustion(
                true,
                &StopState {
                    total_count: 2,
                    same_count: 2
                }
            ),
            None
        );
    }

    #[test]
    fn plan_audit_generic_next_does_not_make_non_actionable_evidence_actionable() {
        let audit = json!({
            "holds": false,
            "next": "planr evidence explain --scope criterion --id crit-sandbox",
            "clauses": [{
                "clause": "verification_logged",
                "pass": false,
                "required": true,
                "criteria": [{"criterion_id": "crit-sandbox", "actionable_now": false}]
            }]
        });
        let proof = json!({
            "actionable_gaps": [],
            "non_actionable_blockers": [{
                "criterion_id": "crit-sandbox",
                "requirement_id": "req-sandbox",
                "status": "blocked",
                "reason": "missing_capability"
            }]
        });
        assert!(!plan_audit_actionable(&audit, &proof));
    }

    #[test]
    fn continuation_reason_has_hard_byte_bound_and_stable_truncation_marker() {
        let long_id = format!("crit-{}-ü", "x".repeat(2_000));
        let proof = json!({
            "status": "not_proven",
            "actionable_gaps": [{
                "criterion_id": long_id,
                "requirement_id": format!("req-{}", "y".repeat(2_000)),
                "status": "missing",
                "reason": "missing_observation"
            }],
            "non_actionable_blockers": [],
            "next_action": format!("collect {}", "z".repeat(2_000)),
        });
        let reason = compact_continuation_reason(
            "plan",
            &format!("pln-{}", "s".repeat(2_000)),
            &proof,
            None,
            1,
            1,
            2,
        );
        assert!(reason.len() <= CONTINUATION_REASON_BYTE_LIMIT, "{reason}");
        assert!(std::str::from_utf8(reason.as_bytes()).is_ok());
        assert!(reason.contains("[truncated"), "{reason}");
        assert!(reason.contains("sha256:"), "{reason}");
    }

    #[test]
    fn continuation_reason_keeps_exact_gap_identity_when_within_bound() {
        let proof = json!({
            "status": "not_proven",
            "actionable_gaps": [{
                "criterion_id": "crit-stop-missing",
                "requirement_id": "req-stop-visible",
                "status": "missing",
                "reason": "missing_observation"
            }],
            "non_actionable_blockers": [],
            "next_action": "collect missing trusted evidence",
        });
        let reason = compact_continuation_reason("plan", "pln-stop", &proof, None, 1, 1, 2);
        assert!(reason.len() <= CONTINUATION_REASON_BYTE_LIMIT, "{reason}");
        assert!(
            reason.contains("crit-stop-missing/req-stop-visible"),
            "{reason}"
        );
        assert!(!reason.contains("[truncated"), "{reason}");
    }
}
