use super::App;
use super::repository::execution_run::{
    ExecutionRunRepository, FindingRecord, ReviewAttemptRecord, ReviewGateKind, ReviewGateRecord,
    ReviewGateStatus,
};
use crate::execution_run::{
    ExecutionBatch, FeatureRun, FeatureRunHoldReason, FeatureRunPhase, FeatureRunStatus, RoleOwner,
    RunRole,
};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CanonicalExecutionBudgetDto {
    pub(crate) status: &'static str,
    pub(crate) active_reservations: u64,
    pub(crate) observation_count: u64,
    pub(crate) reserved_wall_seconds: u64,
    pub(crate) reserved_tool_calls: u64,
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
    pub(crate) feature_run: FeatureRun,
    pub(crate) execution_batch: Option<ExecutionBatch>,
    pub(crate) review_gate: Option<ReviewGateRecord>,
    pub(crate) review_attempts: Vec<ReviewAttemptRecord>,
    pub(crate) findings: Vec<FindingRecord>,
}

impl App {
    pub(crate) fn canonical_execution_state_value(
        &self,
        run_id: &str,
        preferred_gate_id: Option<&str>,
    ) -> Result<Value> {
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository.feature_run(run_id)?;
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
        let (active_reservations, reserved_wall_seconds, reserved_tool_calls): (u64, u64, u64) =
            self.conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(reserved_wall_seconds), 0), COALESCE(SUM(reserved_tool_calls), 0) FROM feature_run_budget_reservations WHERE run_id = ?1 AND status = 'active'",
                [run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        let observation_count: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM feature_run_budget_observations WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )?;
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
        let (reason_code, next_action) = if let Some(gate) = unmet_gate.as_ref() {
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
        serde_json::to_value(CanonicalExecutionStateDto {
            schema_version: "planr.execution_state.v1",
            reason_code,
            phase: persisted.run.phase,
            owner,
            budget: CanonicalExecutionBudgetDto {
                status: if persisted.run.status == FeatureRunStatus::Held {
                    "held"
                } else {
                    "available"
                },
                active_reservations,
                observation_count,
                reserved_wall_seconds,
                reserved_tool_calls,
            },
            unmet_gate,
            next_action,
            feature_run: persisted.run,
            execution_batch: batch,
            review_gate: gate,
            review_attempts: attempts,
            findings,
        })
        .map_err(Into::into)
    }

    pub(crate) fn canonical_execution_state_for_plan_value(
        &self,
        plan_id: &str,
    ) -> Result<Option<Value>> {
        self.canonical_execution_run_id_for_plan(plan_id)?
            .map(|run_id| self.canonical_execution_state_value(&run_id, None))
            .transpose()
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
        format!(
            "phase: {}\nowner: {} ({})\nbudget: {} ({} active reservation(s))\nunmet gate: {}\nnext action: {}",
            state["phase"].as_str().unwrap_or("unknown"),
            owner,
            owner_role,
            state["budget"]["status"].as_str().unwrap_or("unknown"),
            state["budget"]["active_reservations"].as_u64().unwrap_or(0),
            unmet,
            state["next_action"].as_str().unwrap_or("none"),
        )
    }
}
