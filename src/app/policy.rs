//! CLI/MCP projection for the provider-neutral usage policy. Both surfaces
//! reuse `policy_show_value` and `policy_check_value` so response shapes do
//! not drift.

use super::App;
use crate::cli::PolicyCommand;
use crate::execution_policy::{
    ActiveExecution, ConcurrencySnapshot, ExecutionAdmission, ExecutionAdmissionRequest,
    IsolationMode, admit_execution,
};
use crate::usage_policy::{POLICY_RELATIVE_PATH, PolicyLoad, load_policy};
use crate::util::worker_id;
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

impl App {
    pub(crate) fn policy(&self, command: PolicyCommand) -> Result<()> {
        match command {
            PolicyCommand::Show(_) => {
                let (value, human) = self.policy_show_value();
                self.emit(value, human)
            }
            PolicyCommand::Check => {
                let (value, human) = self.policy_check_value()?;
                self.emit(value, human)
            }
            PolicyCommand::Admit(args) => {
                let path = Path::new(&args.id);
                let path = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    self.root.join(path)
                };
                let request = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
                let value = self.policy_admit_value(request)?;
                let human = format!("execution admission: {}", value["admission"]["status"]);
                self.emit(value, human)
            }
        }
    }

    pub(crate) fn policy_admit_value(&self, value: Value) -> Result<Value> {
        let PolicyLoad::Loaded(policy) = load_policy(&self.root) else {
            bail!("usage policy parse/validation failed: no valid policy is available")
        };
        let request: ExecutionAdmissionRequest = serde_json::from_value(value)
            .map_err(|error| anyhow::anyhow!("execution admission parse failed: {error}"))?;
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let current_worker = worker_id();
        let candidate = transaction
            .query_row(
                "SELECT project_id, status, worker_id, pick_token FROM items WHERE id = ?1",
                params![request.item_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((project_id, status, owner, current_pick_token)) = candidate else {
            bail!("item not found: {}", request.item_id)
        };
        if !matches!(status.as_str(), "picked" | "running") {
            bail!(
                "invalid_transition: execution admission requires item {} to be picked or running; current status is {}",
                request.item_id,
                status
            );
        }
        if owner.as_deref() != Some(current_worker.as_str()) {
            bail!(
                "invalid_transition: execution admission requires current lease owner {}; current worker is {}",
                owner.as_deref().unwrap_or("<none>"),
                current_worker
            );
        }
        if current_pick_token.as_deref() != Some(request.pick_token.as_str()) {
            bail!(
                "invalid_transition: execution admission pick token does not match the current lease for item {}",
                request.item_id
            );
        }
        let active = authoritative_active_executions(&transaction, &project_id, &request.item_id)?;
        let concurrency = concurrency_snapshot(&active);
        let admission = admit_execution(
            &policy.execution,
            &policy.usage,
            &request,
            concurrency,
            &active,
        );
        let event_type = match &admission {
            ExecutionAdmission::Allowed { .. } => "execution_admitted",
            ExecutionAdmission::Rejected { .. } => "execution_rejected",
        };
        let payload = json!({
            "policy_id": &policy.id,
            "policy_version": &policy.version,
            "role": &request.role,
            "lease": {
                "worker_id": &current_worker,
                "pick_token": &request.pick_token,
            },
            "admission": &admission,
            "execution": {
                "read_scope": &request.contract.read_scope,
                "write_scope": &request.contract.write_scope,
                "isolation": &request.isolation,
            }
        });
        transaction.execute(
            "INSERT INTO events(project_id, item_id, worker_id, event_type, payload, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
            params![project_id, request.item_id, current_worker, event_type, payload.to_string()],
        )?;
        transaction.commit()?;
        Ok(json!({
            "policy_id": &policy.id,
            "policy_version": &policy.version,
            "concurrency": concurrency,
            "admission": admission,
        }))
    }

    pub(crate) fn policy_show_value(&self) -> (Value, String) {
        match load_policy(&self.root) {
            PolicyLoad::Missing => (
                json!({
                    "policy": null,
                    "reason": "missing",
                    "path": POLICY_RELATIVE_PATH,
                    "enforcement": "unavailable"
                }),
                format!(
                    "no policy at {POLICY_RELATIVE_PATH}; existing advisory routing behavior is preserved"
                ),
            ),
            PolicyLoad::Invalid(diagnostics) => (
                json!({
                    "policy": null,
                    "reason": "degraded",
                    "path": POLICY_RELATIVE_PATH,
                    "enforcement": "unavailable",
                    "diagnostics": diagnostics.diagnostics
                }),
                format!("usage policy is invalid: {diagnostics}"),
            ),
            PolicyLoad::Loaded(policy) => (
                json!({
                    "policy": policy,
                    "path": POLICY_RELATIVE_PATH,
                    "enforcement": "available"
                }),
                format!(
                    "usage policy {}@{} is valid (schema v{})",
                    policy.id, policy.version, policy.schema_version
                ),
            ),
        }
    }

    pub(crate) fn policy_check_value(&self) -> Result<(Value, String)> {
        match load_policy(&self.root) {
            PolicyLoad::Missing => Ok((
                json!({
                    "ok": true,
                    "reason": "missing",
                    "path": POLICY_RELATIVE_PATH,
                    "enforcement": "unavailable"
                }),
                format!(
                    "no policy at {POLICY_RELATIVE_PATH}; existing advisory routing behavior is preserved"
                ),
            )),
            PolicyLoad::Invalid(diagnostics) => {
                bail!("usage policy parse/validation failed: {diagnostics}")
            }
            PolicyLoad::Loaded(policy) => Ok((
                json!({
                    "ok": true,
                    "path": POLICY_RELATIVE_PATH,
                    "schema_version": policy.schema_version,
                    "policy_id": policy.id,
                    "policy_version": policy.version,
                    "enforcement": "available"
                }),
                format!("usage policy {}@{} check passed", policy.id, policy.version),
            )),
        }
    }
}

fn authoritative_active_executions(
    transaction: &Transaction<'_>,
    project_id: &str,
    candidate_id: &str,
) -> Result<Vec<ActiveExecution>> {
    let mut statement = transaction.prepare(
        "SELECT id, worker_id, pick_token FROM items WHERE project_id = ?1 AND status IN ('picked', 'running') AND id <> ?2 ORDER BY id",
    )?;
    let ids = statement
        .query_map(params![project_id, candidate_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    let mut active = Vec::with_capacity(ids.len());
    for (item_id, owner, pick_token) in ids {
        let payload = match (owner.as_deref(), pick_token.as_deref()) {
            (Some(owner), Some(pick_token)) => {
                let mut events = transaction.prepare(
                    "SELECT payload FROM events WHERE item_id = ?1 AND worker_id = ?2 AND event_type = 'execution_admitted' ORDER BY id DESC",
                )?;
                let payloads = events
                    .query_map(params![item_id, owner], |row| {
                        row.get::<_, Option<String>>(0)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                payloads.into_iter().flatten().find_map(|payload| {
                    let payload = serde_json::from_str::<Value>(&payload).ok()?;
                    let lease = payload.get("lease")?;
                    (lease.get("worker_id").and_then(Value::as_str) == Some(owner)
                        && lease.get("pick_token").and_then(Value::as_str) == Some(pick_token))
                    .then_some(payload)
                })
            }
            _ => None,
        };
        let execution = payload
            .as_ref()
            .and_then(|payload| payload.get("execution"))
            .cloned()
            .and_then(|value| serde_json::from_value::<RecordedExecution>(value).ok());
        let (read_scope, write_scope, isolation, scope_known) = match execution {
            Some(execution) => (
                execution.read_scope,
                execution.write_scope,
                execution.isolation,
                true,
            ),
            None => (Vec::new(), Vec::new(), IsolationMode::Shared, false),
        };
        active.push(ActiveExecution {
            item_id,
            read_scope,
            write_scope,
            isolation,
            scope_known,
        });
    }
    Ok(active)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedExecution {
    read_scope: Vec<String>,
    write_scope: Vec<String>,
    isolation: IsolationMode,
}

fn concurrency_snapshot(active: &[ActiveExecution]) -> ConcurrencySnapshot {
    ConcurrencySnapshot {
        active_agents: active.len() as u32,
        parallel_readers: active
            .iter()
            .filter(|item| item.write_scope.is_empty() && !item.read_scope.is_empty())
            .count() as u32,
        parallel_writers: active
            .iter()
            .filter(|item| !item.write_scope.is_empty())
            .count() as u32,
    }
}
