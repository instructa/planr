use super::App;
use super::repository::execution_run::{
    BudgetReservationStatus, ExecutionRunRepository, FindingRecord, PersistedBudgetReservation,
    ReviewAttemptRecord, ReviewGateKind, ReviewGateRecord, ReviewGateStatus,
};
use crate::execution_run::{
    ExecutionBatch, FeatureRun, FeatureRunBudgetContractCompatibility, FeatureRunHoldReason,
    FeatureRunPhase, FeatureRunRestartDisposition, FeatureRunRestartReason, FeatureRunStatus,
    RoleOwner, RunRole,
};
use crate::usage_policy::{
    BudgetAmounts, BudgetProvenance, BudgetSnapshot, FeatureRunBudgetMode, FeatureRunBudgetPhase,
};
use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CanonicalExecutionBudgetDto {
    pub(crate) status: &'static str,
    pub(crate) mode: Option<FeatureRunBudgetMode>,
    pub(crate) consumed: Option<BudgetAmounts>,
    pub(crate) reserved: Option<BudgetAmounts>,
    pub(crate) remaining: Option<BudgetAmounts>,
    pub(crate) protected: Option<BudgetAmounts>,
    pub(crate) available: Option<BudgetAmounts>,
    pub(crate) released_through: Option<FeatureRunBudgetPhase>,
    pub(crate) provenance: Option<BudgetProvenance>,
    pub(crate) contract_digest: Option<String>,
    pub(crate) task_deadline_unix_ms: Option<u64>,
    pub(crate) unavailable_reason: Option<String>,
}

impl CanonicalExecutionBudgetDto {
    fn available(snapshot: BudgetSnapshot, held: bool) -> Self {
        Self {
            status: if held { "held" } else { "available" },
            mode: Some(snapshot.mode),
            consumed: Some(snapshot.consumed),
            reserved: Some(snapshot.reserved),
            remaining: snapshot.remaining,
            protected: Some(snapshot.protected),
            available: snapshot.available,
            released_through: Some(snapshot.released_through),
            provenance: Some(snapshot.metering),
            contract_digest: Some(snapshot.contract_digest),
            task_deadline_unix_ms: snapshot.task_deadline_unix_ms,
            unavailable_reason: None,
        }
    }

    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: "unavailable",
            mode: None,
            consumed: None,
            reserved: None,
            remaining: None,
            protected: None,
            available: None,
            released_through: None,
            provenance: None,
            contract_digest: None,
            task_deadline_unix_ms: None,
            unavailable_reason: Some(reason.into()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CanonicalUnmetGateDto {
    pub(crate) id: String,
    pub(crate) kind: ReviewGateKind,
    pub(crate) status: ReviewGateStatus,
    pub(crate) reason_code: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CanonicalExecutionStateDto {
    pub(crate) schema_version: &'static str,
    pub(crate) reason_code: &'static str,
    pub(crate) phase: FeatureRunPhase,
    pub(crate) owner: Option<RoleOwner>,
    pub(crate) budget: CanonicalExecutionBudgetDto,
    pub(crate) unmet_gate: Option<CanonicalUnmetGateDto>,
    pub(crate) next_action: &'static str,
    pub(crate) restart: Option<CanonicalFeatureRunRestartDto>,
    pub(crate) feature_run: FeatureRun,
    pub(crate) execution_batch: Option<ExecutionBatch>,
    pub(crate) review_gate: Option<ReviewGateRecord>,
    pub(crate) review_attempts: Vec<ReviewAttemptRecord>,
    pub(crate) findings: Vec<FindingRecord>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CanonicalFeatureRunRestartDto {
    pub(crate) status: &'static str,
    pub(crate) reason: FeatureRunRestartReason,
    pub(crate) incompatibility: FeatureRunBudgetContractCompatibility,
    pub(crate) disposition: Option<FeatureRunRestartDisposition>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CanonicalPlanrExecutableIdentity {
    pub(crate) schema_version: &'static str,
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
    pub(crate) size_bytes: u64,
    pub(crate) version: &'static str,
    pub(crate) path_lookup_allowed: bool,
    #[serde(skip)]
    modified: SystemTime,
}

static CURRENT_PLANR_EXECUTABLE_IDENTITY: OnceLock<
    std::result::Result<CanonicalPlanrExecutableIdentity, String>,
> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
struct CanonicalPlanrCommand {
    schema_version: &'static str,
    executable: PathBuf,
    executable_sha256: String,
    path_lookup_allowed: bool,
    argv: Vec<String>,
}

fn digest_file(path: &Path) -> Result<(String, u64)> {
    let mut file = File::open(path).map_err(|error| {
        anyhow::anyhow!(
            "planr_executable_identity_unavailable:path={}:error={error}",
            path.display()
        )
    })?;
    let mut digest = Sha256::new();
    let mut size_bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        size_bytes = size_bytes.saturating_add(read as u64);
    }
    Ok((format!("sha256:{:x}", digest.finalize()), size_bytes))
}

fn observe_planr_executable_identity(path: &Path) -> Result<CanonicalPlanrExecutableIdentity> {
    let path = path.canonicalize().map_err(|error| {
        anyhow::anyhow!(
            "planr_executable_identity_unavailable:path={}:error={error}",
            path.display()
        )
    })?;
    if !path.is_absolute() {
        bail!(
            "planr_executable_identity_path_not_absolute:path={}",
            path.display()
        );
    }
    let before = path.metadata()?;
    if !before.is_file() {
        bail!(
            "planr_executable_identity_not_regular_file:path={}",
            path.display()
        );
    }
    let (sha256, size_bytes) = digest_file(&path)?;
    let after = path.metadata()?;
    if before.len() != after.len()
        || size_bytes != after.len()
        || before.modified()? != after.modified()?
    {
        bail!(
            "planr_executable_identity_mutated_during_observation:path={}",
            path.display()
        );
    }
    Ok(CanonicalPlanrExecutableIdentity {
        schema_version: "planr.executable_identity.v1",
        path,
        sha256,
        size_bytes,
        version: env!("CARGO_PKG_VERSION"),
        path_lookup_allowed: false,
        modified: after.modified()?,
    })
}

fn current_planr_executable_identity() -> Result<CanonicalPlanrExecutableIdentity> {
    let identity = CURRENT_PLANR_EXECUTABLE_IDENTITY.get_or_init(|| {
        std::env::current_exe()
            .map_err(|error| format!("planr_executable_identity_unavailable:error={error}"))
            .and_then(|path| {
                observe_planr_executable_identity(&path).map_err(|error| error.to_string())
            })
    });
    identity.clone().map_err(anyhow::Error::msg)
}

pub(crate) fn initialize_planr_executable_identity() {
    // Capture the running executable before any command transaction begins. Handoff generation
    // then performs only a metadata recheck while holding workflow locks; consumers re-hash the
    // emitted digest immediately before executing the structured command.
    let _ = current_planr_executable_identity();
}

fn validate_cached_planr_executable_identity(
    identity: &CanonicalPlanrExecutableIdentity,
) -> Result<()> {
    if identity.path_lookup_allowed {
        bail!("planr_executable_identity_path_lookup_forbidden");
    }
    let canonical = identity.path.canonicalize().map_err(|error| {
        anyhow::anyhow!(
            "planr_executable_identity_unavailable:path={}:error={error}",
            identity.path.display()
        )
    })?;
    let metadata = canonical.metadata()?;
    if canonical != identity.path
        || !metadata.is_file()
        || metadata.len() != identity.size_bytes
        || metadata.modified()? != identity.modified
    {
        bail!(
            "planr_executable_identity_mismatch:expected_path={}:expected_sha256={}",
            identity.path.display(),
            identity.sha256
        );
    }
    Ok(())
}

#[cfg(test)]
fn validate_planr_executable_identity(identity: &CanonicalPlanrExecutableIdentity) -> Result<()> {
    if identity.path_lookup_allowed {
        bail!("planr_executable_identity_path_lookup_forbidden");
    }
    let observed = observe_planr_executable_identity(&identity.path)?;
    if observed.path != identity.path
        || observed.sha256 != identity.sha256
        || observed.size_bytes != identity.size_bytes
        || observed.version != identity.version
    {
        bail!(
            "planr_executable_identity_mismatch:expected_path={}:expected_sha256={}:observed_path={}:observed_sha256={}",
            identity.path.display(),
            identity.sha256,
            observed.path.display(),
            observed.sha256
        );
    }
    Ok(())
}

fn canonical_planr_command(
    identity: &CanonicalPlanrExecutableIdentity,
    argv: Vec<String>,
) -> CanonicalPlanrCommand {
    CanonicalPlanrCommand {
        schema_version: "planr.command.v1",
        executable: identity.path.clone(),
        executable_sha256: identity.sha256.clone(),
        path_lookup_allowed: false,
        argv,
    }
}

impl App {
    pub(crate) fn canonical_execution_state_value(
        &self,
        run_id: &str,
        preferred_gate_id: Option<&str>,
    ) -> Result<Value> {
        let persisted = ExecutionRunRepository::new(&self.conn).feature_run(run_id)?;
        self.canonical_execution_state_value_at(
            run_id,
            preferred_gate_id,
            persisted.budget_projection_at_unix_ms,
        )
    }

    fn canonical_execution_state_value_at(
        &self,
        run_id: &str,
        preferred_gate_id: Option<&str>,
        now_unix_ms: u64,
    ) -> Result<Value> {
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository.feature_run(run_id)?;
        let compatibility = repository.budget_contract_compatibility(run_id)?;
        let restart = if compatibility.is_incompatible()
            && matches!(
                persisted.run.status,
                FeatureRunStatus::Active | FeatureRunStatus::Held
            ) {
            Some(CanonicalFeatureRunRestartDto {
                status: "required",
                reason: FeatureRunRestartReason::IncompatibleBudget,
                incompatibility: compatibility,
                disposition: None,
            })
        } else if persisted.run.status == FeatureRunStatus::Cancelled {
            repository
                .latest_incompatible_feature_run_restart(
                    &persisted.project_id,
                    &persisted.run.plan_id,
                )?
                .filter(|transition| transition.retired_run.id == persisted.run.id)
                .map(|transition| CanonicalFeatureRunRestartDto {
                    status: "retired",
                    reason: transition.request.reason,
                    incompatibility: transition.incompatibility,
                    disposition: Some(transition.disposition),
                })
        } else {
            None
        };
        let batch = persisted
            .run
            .active_batch_id
            .as_deref()
            .map(|id| repository.batch(id).map(|value| value.batch))
            .transpose()?
            .filter(|batch| batch.run_id == persisted.run.id);
        let gates = repository.review_gates_for_run(&persisted.run.id, false)?;
        let gate = preferred_gate_id
            .and_then(|id| gates.iter().find(|gate| gate.id == id))
            .cloned()
            .or_else(|| {
                gates
                    .iter()
                    .find(|gate| {
                        !matches!(
                            gate.status,
                            ReviewGateStatus::Accepted | ReviewGateStatus::Cancelled
                        )
                    })
                    .cloned()
            })
            .or_else(|| gates.last().cloned());
        let attempts = gate
            .as_ref()
            .map(|gate| repository.review_attempts(&gate.id))
            .transpose()?
            .unwrap_or_default();
        let findings = gate
            .as_ref()
            .map(|gate| repository.findings(&gate.id))
            .transpose()?
            .unwrap_or_default();
        let budget = (|| -> Result<BudgetSnapshot> {
            let reservations = repository.budget_reservations(run_id)?;
            let released_through = canonical_released_budget_phase(&persisted.run, &reservations)?;
            self.persisted_budget_snapshot(&persisted, released_through, now_unix_ms)
                .map(|(_, snapshot)| snapshot)
        })()
        .map_or_else(
            |error| CanonicalExecutionBudgetDto::unavailable(error.to_string()),
            |snapshot| {
                CanonicalExecutionBudgetDto::available(
                    snapshot,
                    persisted.run.status == FeatureRunStatus::Held,
                )
            },
        );
        let owner_role = match persisted.run.phase {
            FeatureRunPhase::RiskReview | FeatureRunPhase::FinalReview => RunRole::Reviewer,
            FeatureRunPhase::Verification | FeatureRunPhase::SourceFrozen => RunRole::Verifier,
            FeatureRunPhase::Implementation => RunRole::Maker,
            FeatureRunPhase::Held => match persisted.run.held_from_phase {
                Some(FeatureRunPhase::RiskReview | FeatureRunPhase::FinalReview) => {
                    RunRole::Reviewer
                }
                Some(FeatureRunPhase::Verification | FeatureRunPhase::SourceFrozen) => {
                    RunRole::Verifier
                }
                _ => RunRole::Maker,
            },
            FeatureRunPhase::Complete | FeatureRunPhase::Cancelled => RunRole::Maker,
        };
        let owner = persisted
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == owner_role)
            .cloned();
        let unmet_gate = gate.as_ref().and_then(|gate| {
            let reason_code = match gate.status {
                ReviewGateStatus::Pending => "review_gate_pending",
                ReviewGateStatus::Leased => "review_gate_leased",
                ReviewGateStatus::ChangesRequested => "review_changes_requested",
                ReviewGateStatus::Accepted | ReviewGateStatus::Cancelled => return None,
            };
            Some(CanonicalUnmetGateDto {
                id: gate.id.clone(),
                kind: gate.kind,
                status: gate.status,
                reason_code,
            })
        });
        let (reason_code, next_action) = if restart
            .as_ref()
            .is_some_and(|restart| restart.status == "required")
        {
            (
                match compatibility {
                    FeatureRunBudgetContractCompatibility::Missing => {
                        "feature_run_budget_contract_missing"
                    }
                    FeatureRunBudgetContractCompatibility::Invalid => {
                        "feature_run_budget_contract_invalid"
                    }
                    FeatureRunBudgetContractCompatibility::DigestMismatch => {
                        "feature_run_budget_contract_digest_mismatch"
                    }
                    FeatureRunBudgetContractCompatibility::Compatible => unreachable!(),
                },
                "restart_incompatible_feature_run",
            )
        } else if let Some(gate) = unmet_gate.as_ref() {
            (
                gate.reason_code,
                match gate.status {
                    ReviewGateStatus::Pending => "lease_review_gate",
                    ReviewGateStatus::Leased => "complete_review_gate",
                    ReviewGateStatus::ChangesRequested => "resolve_review_findings",
                    ReviewGateStatus::Accepted | ReviewGateStatus::Cancelled => "none",
                },
            )
        } else {
            match persisted.run.phase {
                FeatureRunPhase::Implementation => {
                    ("implementation_in_progress", "settle_next_outcome")
                }
                FeatureRunPhase::RiskReview => ("risk_review_in_progress", "complete_review_gate"),
                FeatureRunPhase::SourceFrozen => ("source_frozen", "lease_verification"),
                FeatureRunPhase::Verification => {
                    ("verification_in_progress", "complete_binding_evidence")
                }
                FeatureRunPhase::FinalReview => {
                    ("final_review_in_progress", "complete_final_review_gate")
                }
                FeatureRunPhase::Held => match persisted.run.hold_reason {
                    Some(FeatureRunHoldReason::Capability) => {
                        ("evidence_readiness_blocked", "repair_evidence_readiness")
                    }
                    Some(FeatureRunHoldReason::Budget) | None => {
                        ("feature_run_budget_held", "resolve_budget_hold")
                    }
                },
                FeatureRunPhase::Complete => ("feature_run_complete", "none"),
                FeatureRunPhase::Cancelled => ("feature_run_cancelled", "none"),
            }
        };
        let review_source_binding = gate
            .as_ref()
            .map(|gate| repository.review_source_binding(&gate.id))
            .transpose()?
            .flatten();
        let mut value = serde_json::to_value(CanonicalExecutionStateDto {
            schema_version: "planr.execution_state.v2",
            reason_code,
            phase: persisted.run.phase,
            owner,
            budget,
            unmet_gate,
            next_action,
            restart,
            feature_run: persisted.run,
            execution_batch: batch,
            review_gate: gate,
            review_attempts: attempts,
            findings,
        })
        .map_err(anyhow::Error::from)?;
        value["review_source_binding"] = serde_json::to_value(review_source_binding)?;
        Ok(value)
    }

    pub(crate) fn canonical_execution_state_for_plan_value(
        &self,
        plan_id: &str,
    ) -> Result<Option<Value>> {
        self.canonical_execution_run_id_for_plan(plan_id)?
            .map(|run_id| self.canonical_execution_state_value(&run_id, None))
            .transpose()
    }

    pub(crate) fn canonical_verification_handoff_value(
        &self,
        plan_id: &str,
        verification_item_id: Option<String>,
        source_freeze: Value,
    ) -> Result<Value> {
        let planr_executable = current_planr_executable_identity()?;
        validate_cached_planr_executable_identity(&planr_executable)?;
        let execution_state = self
            .canonical_execution_state_for_plan_value(plan_id)?
            .ok_or_else(|| {
                anyhow::anyhow!("verification_handoff_execution_state_missing:{plan_id}")
            })?;
        let lease_verifier = canonical_planr_command(
            &planr_executable,
            vec![
                "pick".to_string(),
                "--plan".to_string(),
                plan_id.to_string(),
                "--work-type".to_string(),
                "verification".to_string(),
                "--json".to_string(),
            ],
        );
        let readiness = canonical_planr_command(
            &planr_executable,
            vec![
                "evidence".to_string(),
                "readiness".to_string(),
                "--scope".to_string(),
                "plan".to_string(),
                "--id".to_string(),
                plan_id.to_string(),
                "--json".to_string(),
            ],
        );
        Ok(json!({
            "item": null,
            "reason": "verification_handoff_source_frozen",
            "work_packet": {
                "schema_version": "planr.verification_handoff.v2",
                "kind": "verification_handoff",
                "plan_id": plan_id,
                "verification_item_id": verification_item_id,
                "execution_state": execution_state,
                "source_freeze": source_freeze,
                "planr_executable": planr_executable,
                "commands": {
                    "lease_verifier": lease_verifier,
                    "readiness": readiness,
                },
                "next_action": "lease_verifier_then_run_readiness",
            }
        }))
    }

    pub(crate) fn canonical_execution_run_id_for_plan(
        &self,
        plan_id: &str,
    ) -> Result<Option<String>> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        Ok(repository
            .active_feature_run_for_plan(&project.id, plan_id)?
            .map(|value| value.run.id)
            .or_else(|| {
                self.conn
                    .query_row(
                        "SELECT id FROM feature_runs WHERE project_id = ?1 AND plan_id = ?2 ORDER BY created_at DESC, id DESC LIMIT 1",
                        rusqlite::params![project.id, plan_id],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
            }))
    }

    pub(crate) fn canonical_execution_states_for_project_value(&self) -> Result<Vec<Value>> {
        let project = self.default_project()?;
        let run_ids = {
            let mut stmt = self.conn.prepare(
                "SELECT id FROM feature_runs WHERE project_id = ?1 ORDER BY created_at, id",
            )?;
            stmt.query_map([project.id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        run_ids
            .into_iter()
            .map(|run_id| self.canonical_execution_state_value(&run_id, None))
            .collect()
    }

    pub(crate) fn canonical_execution_state_human(state: &Value) -> String {
        let owner = state["owner"]["worker_id"].as_str().unwrap_or("unassigned");
        let owner_role = state["owner"]["role"].as_str().unwrap_or("none");
        let unmet = state["unmet_gate"]["reason_code"]
            .as_str()
            .unwrap_or("none");
        let budget = &state["budget"];
        let available = budget["available"].as_object().map_or_else(
            || budget["mode"].as_str().unwrap_or("unknown").to_string(),
            |amounts| {
                format!(
                    "{}s/{} tools/{} tokens available",
                    amounts["wall_seconds"].as_u64().unwrap_or(0),
                    amounts["tool_calls"].as_u64().unwrap_or(0),
                    amounts["tokens"].as_u64().unwrap_or(0),
                )
            },
        );
        let deadline = budget["task_deadline_unix_ms"]
            .as_u64()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string());
        format!(
            "phase: {}\nowner: {} ({})\nbudget: {} ({available}; deadline {deadline})\nunmet gate: {}\nnext action: {}",
            state["phase"].as_str().unwrap_or("unknown"),
            owner,
            owner_role,
            budget["status"].as_str().unwrap_or("unknown"),
            unmet,
            state["next_action"].as_str().unwrap_or("none"),
        )
    }
}

fn canonical_released_budget_phase(
    run: &FeatureRun,
    reservations: &[PersistedBudgetReservation],
) -> Result<FeatureRunBudgetPhase> {
    let mut active_phase = None;
    for reservation in reservations
        .iter()
        .filter(|reservation| reservation.status == BudgetReservationStatus::Active)
    {
        let phase = match reservation.reservation.phase {
            crate::usage_policy::BudgetPhase::Implementation => FeatureRunBudgetPhase::Maker,
            crate::usage_policy::BudgetPhase::Verification => FeatureRunBudgetPhase::Verification,
            crate::usage_policy::BudgetPhase::Review => FeatureRunBudgetPhase::Review,
            crate::usage_policy::BudgetPhase::Repair => FeatureRunBudgetPhase::Repair,
        };
        if active_phase.is_some_and(|current| current != phase) {
            bail!("feature_run_active_budget_phase_mismatch:{}", run.id);
        }
        active_phase = Some(phase);
    }
    if let Some(phase) = active_phase {
        return Ok(phase);
    }

    let phase = if run.phase == FeatureRunPhase::Held {
        run.held_from_phase
            .unwrap_or(FeatureRunPhase::Implementation)
    } else {
        run.phase
    };
    Ok(match phase {
        FeatureRunPhase::Implementation => FeatureRunBudgetPhase::Maker,
        FeatureRunPhase::RiskReview | FeatureRunPhase::FinalReview => FeatureRunBudgetPhase::Review,
        FeatureRunPhase::SourceFrozen | FeatureRunPhase::Verification => {
            FeatureRunBudgetPhase::Verification
        }
        FeatureRunPhase::Complete | FeatureRunPhase::Cancelled => FeatureRunBudgetPhase::Release,
        FeatureRunPhase::Held => unreachable!("held phase was resolved above"),
    })
}

#[cfg(test)]
mod tests {
    use super::super::repository::execution_run::{
        BudgetObservationRecord, BudgetReservationRecord,
    };
    use super::*;
    use crate::execution_run::{
        ExecutionBatchStatus, PhaseTransition, PhaseTransitionCause, apply_phase_transition,
    };
    use crate::storage::ensure_schema;
    use crate::usage_policy::{
        BudgetPhase, ExecutionBudget, FeatureRunBudgetContract, FeatureRunPhaseReserves,
        MeteringMode, MeteringProvenance,
    };
    use rusqlite::Connection;
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    const STARTED_AT_UNIX_MS: u64 = 1_700_000_000_000;
    const PROJECTED_AT_UNIX_MS: u64 = STARTED_AT_UNIX_MS + 12_000;

    fn test_app() -> App {
        let conn = Connection::open_in_memory().expect("database");
        ensure_schema(&conn).expect("schema");
        conn.execute(
            "INSERT INTO projects(id, name, root_path, status, created_at, updated_at) VALUES ('project-a', 'Project', '.', 'active', datetime('now'), datetime('now'))",
            [],
        )
        .expect("project");
        conn.execute(
            "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at) VALUES ('plan-a', 'project-a', 'build', 'plan-a.md', 'Plan', 'plan', 'ok', 'sha256:plan', datetime('now'), datetime('now'))",
            [],
        )
        .expect("plan");
        App::new(
            conn,
            PathBuf::from("."),
            PathBuf::from("planr.sqlite"),
            true,
            false,
        )
    }

    fn bounded_contract(run_id: &str) -> FeatureRunBudgetContract {
        FeatureRunBudgetContract::bounded(
            run_id,
            STARTED_AT_UNIX_MS,
            BudgetAmounts {
                wall_seconds: 100,
                tool_calls: 100,
                tokens: 1_000,
            },
            FeatureRunPhaseReserves {
                maker: BudgetAmounts {
                    wall_seconds: 60,
                    tool_calls: 60,
                    tokens: 600,
                },
                verification: BudgetAmounts {
                    wall_seconds: 20,
                    tool_calls: 20,
                    tokens: 200,
                },
                review: BudgetAmounts {
                    wall_seconds: 10,
                    tool_calls: 10,
                    tokens: 100,
                },
                repair: BudgetAmounts {
                    wall_seconds: 10,
                    tool_calls: 10,
                    tokens: 100,
                },
                release: BudgetAmounts::ZERO,
            },
            BudgetProvenance {
                wall_seconds: MeteringProvenance::Trusted,
                tool_calls: MeteringProvenance::Estimated,
                tokens: MeteringProvenance::Unavailable,
            },
        )
        .expect("bounded contract")
    }

    fn implementation_projection_fixture() -> (App, FeatureRunBudgetContract) {
        let app = test_app();
        let run_id = "run-projection";
        let contract = bounded_contract(run_id);
        let batch = ExecutionBatch {
            id: "batch-projection".to_string(),
            run_id: run_id.to_string(),
            maker_worker_id: "maker-a".to_string(),
            status: ExecutionBatchStatus::Active,
            settled_outcome_ids: Vec::new(),
            replacement: None,
        };
        let run = FeatureRun {
            id: run_id.to_string(),
            plan_id: "plan-a".to_string(),
            status: FeatureRunStatus::Active,
            phase: FeatureRunPhase::Implementation,
            policy_digest: "sha256:policy".to_string(),
            source_revision: None,
            active_batch_id: Some(batch.id.clone()),
            role_owners: vec![RoleOwner {
                role: RunRole::Maker,
                worker_id: "maker-a".to_string(),
                lease_generation: 1,
            }],
            outcomes_settled: 0,
            batch_outcome_count: 0,
            held_from_phase: None,
            hold_reason: None,
            terminal_reason: None,
        };
        let repository = ExecutionRunRepository::new(&app.conn);
        repository
            .create_feature_run("project-a", &run, &contract, Some(&batch))
            .expect("feature run");

        let reconciled_budget = ExecutionBudget::new(
            STARTED_AT_UNIX_MS + 1_000,
            BudgetAmounts {
                wall_seconds: 5,
                tool_calls: 5,
                tokens: 50,
            },
        )
        .expect("reconciled task budget");
        let reconciled = BudgetReservationRecord {
            id: "reservation-reconciled".to_string(),
            run_id: run_id.to_string(),
            phase: BudgetPhase::Implementation,
            boundary_key: "implementation:reconciled".to_string(),
            owner_role: RunRole::Maker,
            owner_worker_id: "maker-a".to_string(),
            lease_generation: 1,
            execution_budget: Some(reconciled_budget),
            started_at_unix_ms: STARTED_AT_UNIX_MS + 1_000,
            provenance: "test.reconciled".to_string(),
        };
        repository
            .create_budget_reservation(&reconciled)
            .expect("reconciled reservation");
        repository
            .record_budget_observation(&BudgetObservationRecord {
                id: "observation-reconciled".to_string(),
                run_id: run_id.to_string(),
                reservation_id: Some(reconciled.id.clone()),
                sequence: Some(1),
                phase: BudgetPhase::Implementation,
                metering: MeteringMode::Unavailable,
                wall_metering: Some(MeteringMode::Trusted),
                tool_calls_metering: Some(MeteringMode::Estimated),
                tokens_metering: Some(MeteringMode::Unavailable),
                wall_seconds: Some(1),
                tokens: None,
                tool_calls: Some(3),
                credits_micros: None,
                provenance: "test.adapter".to_string(),
                adapter_id: Some("adapter-a".to_string()),
                observed_at_unix_ms: Some(STARTED_AT_UNIX_MS + 6_000),
            })
            .expect("budget observation");
        repository
            .reconcile_budget_reservation(&reconciled.id, run_id)
            .expect("reconcile reservation");

        let active = BudgetReservationRecord {
            id: "reservation-active".to_string(),
            run_id: run_id.to_string(),
            phase: BudgetPhase::Implementation,
            boundary_key: "implementation:active".to_string(),
            owner_role: RunRole::Maker,
            owner_worker_id: "maker-a".to_string(),
            lease_generation: 1,
            execution_budget: Some(
                ExecutionBudget::new(
                    STARTED_AT_UNIX_MS + 10_000,
                    BudgetAmounts {
                        wall_seconds: 5,
                        tool_calls: 4,
                        tokens: 40,
                    },
                )
                .expect("active task budget"),
            ),
            started_at_unix_ms: STARTED_AT_UNIX_MS + 10_000,
            provenance: "test.active".to_string(),
        };
        repository
            .create_budget_reservation(&active)
            .expect("active reservation");
        (app, contract)
    }

    #[test]
    fn deterministic_v2_projection_exposes_persisted_budget_truth_and_absolute_deadline() {
        let (app, contract) = implementation_projection_fixture();
        let state = app
            .canonical_execution_state_value_at("run-projection", None, PROJECTED_AT_UNIX_MS)
            .expect("canonical execution state");

        assert_eq!(state["schema_version"], "planr.execution_state.v2");
        assert_eq!(
            state["budget"],
            json!({
                "status": "available",
                "mode": "bounded",
                "consumed": {"wall_seconds": 12, "tool_calls": 5, "tokens": 50},
                "reserved": {"wall_seconds": 5, "tool_calls": 4, "tokens": 40},
                "remaining": {"wall_seconds": 88, "tool_calls": 95, "tokens": 950},
                "protected": {"wall_seconds": 40, "tool_calls": 40, "tokens": 400},
                "available": {"wall_seconds": 43, "tool_calls": 51, "tokens": 510},
                "released_through": "maker",
                "provenance": {
                    "wall_seconds": "trusted",
                    "tool_calls": "estimated",
                    "tokens": "unavailable"
                },
                "contract_digest": contract.digest,
                "task_deadline_unix_ms": STARTED_AT_UNIX_MS + 15_000,
                "unavailable_reason": null
            })
        );
    }

    #[test]
    fn public_budget_shape_is_one_opaque_byte_equivalent_projection() {
        let (app, _) = implementation_projection_fixture();
        let state = app
            .canonical_execution_state_value_at("run-projection", None, PROJECTED_AT_UNIX_MS)
            .expect("canonical execution state");
        let keys = state["budget"]
            .as_object()
            .expect("budget object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            keys,
            BTreeSet::from([
                "available".to_string(),
                "consumed".to_string(),
                "contract_digest".to_string(),
                "mode".to_string(),
                "protected".to_string(),
                "provenance".to_string(),
                "released_through".to_string(),
                "remaining".to_string(),
                "reserved".to_string(),
                "status".to_string(),
                "task_deadline_unix_ms".to_string(),
                "unavailable_reason".to_string(),
            ])
        );
        let canonical = serde_json::to_vec(&state["budget"]).expect("canonical budget bytes");
        let envelopes = [
            json!({"work_packet": {"execution_state": state.clone()}}),
            json!({"cli": {"execution_state": state.clone()}}),
            json!({"mcp": {"execution_state": state.clone()}}),
            json!({"http": {"execution_state": state.clone()}}),
            json!({"trace": {"execution_state": state.clone()}}),
            json!({"status": {"execution_state": state}}),
        ];
        for envelope in envelopes {
            let projected = envelope
                .pointer("/work_packet/execution_state/budget")
                .or_else(|| envelope.pointer("/cli/execution_state/budget"))
                .or_else(|| envelope.pointer("/mcp/execution_state/budget"))
                .or_else(|| envelope.pointer("/http/execution_state/budget"))
                .or_else(|| envelope.pointer("/trace/execution_state/budget"))
                .or_else(|| envelope.pointer("/status/execution_state/budget"))
                .expect("public budget projection");
            assert_eq!(serde_json::to_vec(projected).unwrap(), canonical);
        }
    }

    #[test]
    fn capability_hold_preserves_budget_projection_without_adapter_arithmetic() {
        let app = test_app();
        let run_id = "run-capability";
        let contract = bounded_contract(run_id);
        let run = FeatureRun {
            id: run_id.to_string(),
            plan_id: "plan-a".to_string(),
            status: FeatureRunStatus::Active,
            phase: FeatureRunPhase::SourceFrozen,
            policy_digest: "sha256:policy".to_string(),
            source_revision: Some("source-a".to_string()),
            active_batch_id: None,
            role_owners: Vec::new(),
            outcomes_settled: 1,
            batch_outcome_count: 0,
            held_from_phase: None,
            hold_reason: None,
            terminal_reason: None,
        };
        let repository = ExecutionRunRepository::new(&app.conn);
        repository
            .create_feature_run("project-a", &run, &contract, None)
            .expect("source-frozen run");
        let before = app
            .canonical_execution_state_value_at(run_id, None, PROJECTED_AT_UNIX_MS)
            .expect("projection before hold");
        let persisted = repository.feature_run(run_id).expect("persisted run");
        let held = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::Held,
                cause: PhaseTransitionCause::CapabilityHold,
                reference: "adapter:missing-deadline-enforcement".to_string(),
                owner: None,
            },
        )
        .expect("capability hold");
        repository
            .save_feature_run(&held, persisted.revision)
            .expect("save capability hold");
        let after = app
            .canonical_execution_state_value_at(run_id, None, PROJECTED_AT_UNIX_MS)
            .expect("projection after hold");

        assert_eq!(after["reason_code"], "evidence_readiness_blocked");
        assert_eq!(after["next_action"], "repair_evidence_readiness");
        assert_eq!(after["budget"]["status"], "held");
        for field in [
            "mode",
            "consumed",
            "reserved",
            "remaining",
            "protected",
            "available",
            "released_through",
            "provenance",
            "contract_digest",
            "task_deadline_unix_ms",
        ] {
            assert_eq!(after["budget"][field], before["budget"][field], "{field}");
        }
    }

    #[test]
    fn generated_roles_and_planr_skills_consume_v2_without_recomputing_policy() {
        for (path, content) in [
            (
                "plugins/planr/agents/planr-worker.md",
                include_str!("../../plugins/planr/agents/planr-worker.md"),
            ),
            (
                "plugins/planr/agents/planr-reviewer.md",
                include_str!("../../plugins/planr/agents/planr-reviewer.md"),
            ),
            (
                "plugins/planr/agents/pi/planr-worker.md",
                include_str!("../../plugins/planr/agents/pi/planr-worker.md"),
            ),
            (
                "plugins/planr/agents/pi/planr-reviewer.md",
                include_str!("../../plugins/planr/agents/pi/planr-reviewer.md"),
            ),
            (
                "plugins/planr/skills/planr-work/SKILL.md",
                include_str!("../../plugins/planr/skills/planr-work/SKILL.md"),
            ),
            (
                "plugins/planr/skills/planr-review/SKILL.md",
                include_str!("../../plugins/planr/skills/planr-review/SKILL.md"),
            ),
            (
                "plugins/planr/skills/planr-task-graph/SKILL.md",
                include_str!("../../plugins/planr/skills/planr-task-graph/SKILL.md"),
            ),
            (
                "plugins/planr/skills/planr-verify-web/SKILL.md",
                include_str!("../../plugins/planr/skills/planr-verify-web/SKILL.md"),
            ),
        ] {
            assert!(content.contains("planr.execution_state.v2"), "{path}");
            assert!(
                content.contains("must not recompute budget policy"),
                "{path}"
            );
        }
    }

    #[test]
    fn budget_repository_and_storage_remain_mechanics_only() {
        let repository = include_str!("repository/execution_run.rs");
        let storage = include_str!("../storage/execution_run_schema.rs");
        for (owner, source) in [("repository", repository), ("storage", storage)] {
            for forbidden in [
                "load_policy(",
                "admit_budget_task(",
                "budget_snapshot(",
                "HUMAN_PHASE_WALL_ALLOWANCE_SECONDS",
                "max_over_cumulative",
                "planr.execution_state.v1",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{owner} must not own budget policy path {forbidden}"
                );
            }
        }

        let admission = include_str!("../execution_policy.rs");
        let runtime = include_str!("feature_run_evidence.rs");
        assert_eq!(admission.matches("pub fn admit_budget_task(").count(), 1);
        assert_eq!(runtime.matches("admit_budget_task(").count(), 1);
    }

    #[test]
    fn executable_identity_rejects_missing_and_digest_drift_without_path_lookup() {
        let dir = tempfile::tempdir().expect("temporary executable root");
        let executable = dir.path().join("planr-reviewed");
        std::fs::write(&executable, b"reviewed-planr-v1").expect("test executable");
        let identity = observe_planr_executable_identity(&executable).expect("identity");

        assert!(identity.path.is_absolute());
        assert!(!identity.path_lookup_allowed);
        assert!(identity.sha256.starts_with("sha256:"));
        validate_planr_executable_identity(&identity).expect("unchanged identity");

        let mut wrong_digest = identity.clone();
        wrong_digest.sha256 = format!("sha256:{}", "0".repeat(64));
        assert!(
            validate_planr_executable_identity(&wrong_digest)
                .unwrap_err()
                .to_string()
                .contains("planr_executable_identity_mismatch")
        );

        std::fs::write(&executable, b"installed-old-global-planr").expect("replace executable");
        let error = validate_planr_executable_identity(&identity).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("planr_executable_identity_mismatch")
        );

        let missing = dir.path().join("missing-planr");
        let error = observe_planr_executable_identity(&missing).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("planr_executable_identity_unavailable")
        );
    }

    #[test]
    fn executable_identity_rejects_path_lookup_and_command_serialization_is_structured() {
        let current = std::env::current_exe().expect("current executable");
        let mut identity = observe_planr_executable_identity(&current).expect("identity");
        identity.path_lookup_allowed = true;
        assert_eq!(
            validate_planr_executable_identity(&identity)
                .unwrap_err()
                .to_string(),
            "planr_executable_identity_path_lookup_forbidden"
        );

        identity.path_lookup_allowed = false;
        let command = serde_json::to_value(canonical_planr_command(
            &identity,
            vec!["pick".into(), "--json".into()],
        ))
        .expect("command JSON");
        assert_eq!(command["schema_version"], "planr.command.v1");
        assert_eq!(command["executable"], json!(identity.path));
        assert_eq!(command["executable_sha256"], identity.sha256);
        assert_eq!(command["path_lookup_allowed"], false);
        assert_eq!(command["argv"], json!(["pick", "--json"]));
        assert!(!command.to_string().contains("\"planr pick"));

        let serialized = serde_json::to_vec(&command).expect("canonical command bytes");
        for projection in [
            json!({"cli": command.clone()}),
            json!({"mcp": command.clone()}),
            json!({"http": command.clone()}),
        ] {
            let projected = projection
                .get("cli")
                .or_else(|| projection.get("mcp"))
                .or_else(|| projection.get("http"))
                .expect("transport projection");
            assert_eq!(serde_json::to_vec(projected).unwrap(), serialized);
        }
    }
}
