//! FeatureRun coordination at the Evidence and Usage Policy boundaries.

use super::App;
use super::evidence::CurrentPlanCoverageForSourceFreeze;
use super::proof::PlanEvidenceAuthority;
use super::repository::execution_run::{
    BudgetObservationRecord, BudgetReservationRecord, BudgetReservationStatus,
    CurrentVerificationSnapshot, EvidenceInvalidationRecord, ExecutionRunRepository, FindingStatus,
    PersistedFeatureRun, ProductRepairSettlementRecord, ReviewGateKind, ReviewGateRecord,
    ReviewGateStatus, SourceFreezeRecord, SourceFreezeStatus, VerificationAdmissionRecord,
    VerificationAdmissionRepairSettlementRecord, VerificationReadinessDiagnosticRecord,
};
use crate::cli::EvidenceCoverageScope;
use crate::evidence::policy::capture_repository_snapshot;
use crate::execution_policy::{BudgetTaskAdmission, BudgetTaskHoldReason, admit_budget_task};
use crate::execution_run::{
    CurrentVerificationDiagnosis, CurrentVerificationInvariantStatus,
    CurrentVerificationItemLeaseStatus, EvidenceInvalidationKind, ExecutionBatch,
    ExecutionBatchStatus, FeatureRunBudgetContractCompatibility,
    FeatureRunBudgetHoldResolutionCause, FeatureRunBudgetHoldResolutionDisposition,
    FeatureRunBudgetHoldResolutionRequest, FeatureRunBudgetHoldResolutionTransition,
    FeatureRunHoldReason, FeatureRunPhase, FeatureRunRestartDisposition, FeatureRunRestartReason,
    FeatureRunRestartRequest, InconsistentVerificationBatchFacts,
    InconsistentVerificationRetirementFacts, PhaseTransition, PhaseTransitionCause, RoleOwner,
    RunRole, VerificationAdmissionRepairFacts, VerificationAdmissionRepairReason,
    VerificationAdmissionRepairRequest, apply_phase_transition, classify_current_verification,
    owner_for_role, repair_verification_admission, resolve_budget_held_feature_run,
    resolve_evidence_invalidation_kind, retire_inconsistent_verification_feature_run,
};
use crate::usage_policy::{
    BudgetAmounts, BudgetPhase, BudgetProvenance, BudgetSnapshot, ExecutionBudget,
    FeatureRunBudgetContract, FeatureRunBudgetMode, FeatureRunBudgetPhase, MeteringMode,
    MeteringProvenance, budget_snapshot,
};
use crate::util::{short_id, worker_id};
use anyhow::{Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub(crate) struct VerificationPickReadinessError {
    plan_id: String,
    gaps: Value,
    repair_request: Option<VerificationAdmissionRepairRequest>,
    execution_state: Option<Value>,
}

impl VerificationPickReadinessError {
    fn from_readiness(plan_id: &str, readiness: &Value) -> Self {
        Self {
            plan_id: plan_id.to_string(),
            gaps: readiness["gaps"].clone(),
            repair_request: None,
            execution_state: None,
        }
    }

    fn from_durable_diagnostic(
        plan_id: &str,
        diagnostic: Value,
        repair_request: VerificationAdmissionRepairRequest,
        execution_state: Value,
    ) -> Self {
        Self {
            plan_id: plan_id.to_string(),
            gaps: diagnostic,
            repair_request: Some(repair_request),
            execution_state: Some(execution_state),
        }
    }

    pub(crate) fn details(&self) -> Value {
        json!({
            "plan_id": self.plan_id,
            "gaps": self.gaps,
            "repair_request": self.repair_request,
            "execution_state": self.execution_state,
        })
    }
}

impl std::fmt::Display for VerificationPickReadinessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "verification_pick_readiness_blocked:{}",
            self.plan_id
        )
    }
}

impl std::error::Error for VerificationPickReadinessError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct CanonicalFeatureRunEvidenceLease {
    pub(crate) project_id: String,
    pub(crate) plan_id: String,
    pub(crate) run_id: String,
    pub(crate) freeze_id: String,
    pub(crate) verifier_worker_id: String,
    pub(crate) lease_generation: u64,
    pub(crate) source_revision: String,
    pub(crate) source_digest: String,
}

pub(crate) struct VerificationAttemptExhaustion<'a> {
    pub(crate) obligation_id: &'a str,
    pub(crate) attempt_id: &'a str,
    pub(crate) attempt_index: u32,
    pub(crate) max_attempts: u32,
    pub(crate) repeatability: &'a str,
}

pub(crate) type FeatureRunBudgetReservation = BudgetReservationRecord;

#[derive(Clone, Debug)]
pub(crate) struct CurrentVerificationDiagnosisSnapshot {
    pub(crate) diagnosis: CurrentVerificationDiagnosis,
    pub(crate) admission: Option<VerificationAdmissionRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RepairSettlementDispatchMode {
    ProductFinding,
    VerificationAdmission,
}

fn add_selective_replay_metadata(
    packet: &mut Value,
    settlement: Option<&ProductRepairSettlementRecord>,
) {
    let Some(settlement) = settlement else {
        return;
    };
    packet["mode"] = json!("selective_replay");
    packet["repair_id"] = json!(settlement.invalidation_id);
    packet["responsible_maker_id"] = json!(settlement.responsible_maker_id);
    packet["selective_replay_obligation_ids"] = json!(settlement.selective_obligation_ids);
}

impl App {
    fn repair_settlement_dispatch(
        &self,
        invalidation: &EvidenceInvalidationRecord,
    ) -> Result<Option<RepairSettlementDispatchMode>> {
        resolve_evidence_invalidation_kind(
            &invalidation.reason,
            invalidation.finding_id.as_deref(),
            &invalidation.affected_evidence_ids,
        )
        .map(|kind| {
            kind.map(|kind| match kind {
                EvidenceInvalidationKind::ProductFinding => {
                    RepairSettlementDispatchMode::ProductFinding
                }
                EvidenceInvalidationKind::VerificationAdmission => {
                    RepairSettlementDispatchMode::VerificationAdmission
                }
            })
        })
        .map_err(|violation| {
            anyhow!(
                "repair_invalidation_kind_rejected:{}:{violation:?}",
                invalidation.id
            )
        })
    }

    pub(crate) fn pending_repair_settlement(
        &self,
        run_id: &str,
    ) -> Result<Option<(EvidenceInvalidationRecord, RepairSettlementDispatchMode)>> {
        let repository = ExecutionRunRepository::new(&self.conn);
        for invalidation in repository.invalidations(run_id)?.into_iter().rev() {
            let Some(dispatch) = self.repair_settlement_dispatch(&invalidation)? else {
                continue;
            };
            let settled = match dispatch {
                RepairSettlementDispatchMode::ProductFinding => repository
                    .product_repair_settlement(&invalidation.id)?
                    .is_some(),
                RepairSettlementDispatchMode::VerificationAdmission => repository
                    .verification_admission_repair_settlement(&invalidation.id)?
                    .is_some(),
            };
            if !settled
                && self
                    .historical_invalidation_reconciliation_payload(run_id, &invalidation.id)?
                    .is_none()
            {
                return Ok(Some((invalidation, dispatch)));
            }
        }
        Ok(None)
    }

    pub(crate) fn current_verification_diagnosis(
        &self,
        persisted: &PersistedFeatureRun,
    ) -> Result<Option<CurrentVerificationDiagnosisSnapshot>> {
        if persisted.run.status != crate::execution_run::FeatureRunStatus::Active
            || persisted.run.phase != FeatureRunPhase::Verification
        {
            return Ok(None);
        }
        let CurrentVerificationSnapshot { facts, admission } =
            ExecutionRunRepository::new(&self.conn).current_verification_snapshot(persisted)?;
        if facts.verification_item.as_ref().is_some_and(|item| {
            !matches!(
                item.status,
                CurrentVerificationItemLeaseStatus::Picked
                    | CurrentVerificationItemLeaseStatus::Running
            ) || item.worker_id.as_deref() != Some(facts.verifier_worker_id.as_str())
        }) {
            bail!(
                "current_verification_item_ownership_conflict:{}",
                facts.run_id
            );
        }
        Ok(Some(CurrentVerificationDiagnosisSnapshot {
            diagnosis: classify_current_verification(&facts),
            admission,
        }))
    }

    pub(crate) fn restart_inconsistent_verification_feature_run_value(
        &self,
        plan_id: &str,
    ) -> Result<Value> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let Some(persisted) = repository.active_feature_run_for_plan(&project.id, plan_id)? else {
            let mut previous = repository
                .latest_inconsistent_verification_feature_run_restart(&project.id, plan_id)?
                .ok_or_else(|| anyhow!("feature_run_not_found_for_plan:{plan_id}"))?;
            previous.disposition = FeatureRunRestartDisposition::AlreadyRetired;
            let execution_state =
                self.canonical_execution_state_value(&previous.retired_run.id, None)?;
            return Ok(json!({"schema_version": "planr.feature_run_restart.v1",
                "restart": previous, "execution_state": execution_state}));
        };
        let diagnosis = self
            .current_verification_diagnosis(&persisted)?
            .ok_or_else(|| {
                anyhow!("feature_run_inconsistent_verification_restart_ineligible:{plan_id}")
            })?
            .diagnosis;
        if diagnosis.status != CurrentVerificationInvariantStatus::Inconsistent {
            bail!("feature_run_inconsistent_verification_restart_not_required:{plan_id}");
        }
        let batch = persisted
            .run
            .active_batch_id
            .as_deref()
            .map(|id| repository.batch(id))
            .transpose()?
            .map(|persisted| InconsistentVerificationBatchFacts {
                id: persisted.batch.id,
                status: persisted.batch.status,
            });
        let facts = InconsistentVerificationRetirementFacts {
            diagnosis,
            batch,
            active_verification_reservation_ids: repository
                .active_verification_reservation_ids(&persisted.run.id)?,
            preserved_history: repository.inconsistent_verification_preserved_history(
                &persisted.project_id,
                &persisted.run.plan_id,
                &persisted.run.id,
            )?,
            invalidation_id: short_id("invalidation"),
        };
        let request = FeatureRunRestartRequest {
            plan_id: plan_id.to_string(),
            reason: FeatureRunRestartReason::InconsistentVerification,
        };
        let transition =
            retire_inconsistent_verification_feature_run(&persisted.run, &request, &facts)
                .map_err(|violation| anyhow!("feature_run_restart_rejected:{violation:?}"))?;
        repository.retire_inconsistent_verification_feature_run(&transition, &worker_id())?;
        Ok(json!({"schema_version": "planr.feature_run_restart.v1",
            "restart": transition,
            "execution_state": self.canonical_execution_state_value(&persisted.run.id, None)?}))
    }

    fn inconsistent_verification_restart_hold_for_run(
        &self,
        persisted: &PersistedFeatureRun,
        diagnosis: &CurrentVerificationDiagnosis,
    ) -> Result<Option<Value>> {
        if diagnosis.status != CurrentVerificationInvariantStatus::Inconsistent {
            return Ok(None);
        }
        let command = format!(
            "planr --json run restart --plan {} --reason inconsistent-verification",
            persisted.run.plan_id
        );
        Ok(Some(json!({
            "item": null,
            "reason": "current_verification_inconsistent",
            "repair": [command],
            "work_packet": {"kind": "hold", "classification": "current_verification_inconsistent",
                "reason_code": "current_verification_inconsistent", "next_action": command,
                "current_verification": diagnosis,
                "execution_state": self.canonical_execution_state_value(&persisted.run.id, None)?},
            "remaining": self.progress_value()?,
        })))
    }

    pub(crate) fn validate_review_source_binding(
        &self,
        repository: &ExecutionRunRepository<'_>,
        gate: &ReviewGateRecord,
    ) -> Result<()> {
        let Some(stored) = repository.review_source_binding(&gate.id)? else {
            if gate.kind == ReviewGateKind::FinalProduct {
                bail!("final_product_review_source_binding_missing:{}", gate.id);
            }
            return Ok(());
        };
        let freeze = repository
            .active_source_freeze(&gate.run_id)?
            .ok_or_else(|| anyhow!("review_source_binding_missing_active_freeze:{}", gate.id))?;
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("review_source_binding_capture_failed:{error}"))?;
        if freeze.id != stored.freeze_id
            || freeze.source_revision != stored.source_revision
            || freeze.source_digest != stored.source_digest
            || snapshot.source.revision != stored.source_revision
            || snapshot.source.tree_digest.as_str() != stored.source_digest
            || gate.source_revision.as_deref() != Some(stored.source_revision.as_str())
        {
            bail!("review_source_binding_source_freeze_stale:{}", gate.id);
        }
        Ok(())
    }

    /// Release an active verification pick and its FeatureRun lease as one
    /// application-owned transition. Generic item release cannot safely own
    /// the SourceFrozen boundary because the item row and verifier role lease
    /// must never diverge.
    pub(crate) fn release_verification_pick_value(
        &self,
        item_id: &str,
        force: bool,
    ) -> Result<Option<Value>> {
        let item = self.get_item(item_id)?;
        if item.work_type.as_str() != "verification" {
            return Ok(None);
        }
        let current_worker = worker_id();
        let plan_id = self.conn.query_row(
            "SELECT plans.id FROM plans JOIN items ON items.plan_path = plans.path WHERE items.id = ?1",
            [item_id],
            |row| row.get::<_, String>(0),
        )?;
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository
            .active_feature_run_for_plan(&project.id, &plan_id)?
            .ok_or_else(|| anyhow!("verification_release_run_missing:{plan_id}"))?;

        if item.status == "ready" && persisted.run.phase == FeatureRunPhase::SourceFrozen {
            return Ok(Some(json!({
                "released": item_id,
                "item": item,
                "feature_run": persisted.run,
                "disposition": "already_released",
            })));
        }
        if !force && item.worker_id.as_deref() != Some(current_worker.as_str()) {
            bail!(
                "item is owned by {:?}; use --force to release",
                item.worker_id
            );
        }
        if !matches!(item.status.as_str(), "picked" | "running") {
            bail!(
                "verification_release_item_not_active:{item_id}:{}",
                item.status
            );
        }
        if persisted.run.phase != FeatureRunPhase::Verification {
            bail!(
                "verification_release_wrong_phase:{}:{:?}",
                persisted.run.id,
                persisted.run.phase
            );
        }
        let verifier = owner_for_role(&persisted.run, RunRole::Verifier)
            .ok_or_else(|| anyhow!("verification_release_missing_owner:{}", persisted.run.id))?;
        if item.worker_id.as_deref() != Some(verifier.worker_id.as_str()) {
            bail!("verification_release_stale_item_owner:{item_id}");
        }
        let released = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::SourceFrozen,
                cause: PhaseTransitionCause::VerificationReleased,
                reference: format!("verification_item:{item_id}"),
                owner: None,
            },
        )
        .map_err(|violation| anyhow!("verification_release_transition:{violation:?}"))?;

        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT release_verification_pick")?;
        let result = (|| -> Result<()> {
            let current = repository.feature_run(&persisted.run.id)?;
            if current.revision != persisted.revision
                || current.run.phase != FeatureRunPhase::Verification
            {
                bail!("verification_release_stale_run:{}", persisted.run.id);
            }
            self.reconcile_active_phase_wall(&persisted.run.id, BudgetPhase::Verification)?;
            repository.save_feature_run(&released, persisted.revision)?;
            let changed = self.conn.execute(
                "UPDATE items SET status = 'ready', worker_id = NULL, pick_token = NULL,
                     picked_at = NULL, last_heartbeat_at = NULL, paused_at = NULL,
                     updated_at = datetime('now')
                 WHERE id = ?1 AND work_type = 'verification'
                   AND status IN ('picked','running') AND worker_id = ?2",
                params![item_id, verifier.worker_id],
            )?;
            if changed != 1 {
                bail!("verification_release_stale_item:{item_id}");
            }
            self.record_event(
                "verification_pick_released",
                Some(item_id),
                json!({"force": force, "run_id": persisted.run.id,
                    "lease_generation": verifier.lease_generation}),
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => self
                .conn
                .execute_batch("RELEASE release_verification_pick; COMMIT")?,
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO release_verification_pick; RELEASE release_verification_pick; ROLLBACK",
                );
                return Err(error);
            }
        }
        Ok(Some(json!({
            "released": item_id,
            "item": self.get_item(item_id)?,
            "feature_run": released,
            "disposition": "released",
        })))
    }

    fn persist_verification_pick_readiness_failure(
        &self,
        plan_id: &str,
        freeze_id: &str,
        verifier_worker_id: &str,
        reason: VerificationAdmissionRepairReason,
        diagnostic: Value,
    ) -> Result<VerificationPickReadinessError> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository
            .active_feature_run_for_plan(&project.id, plan_id)?
            .ok_or_else(|| anyhow!("verification_readiness_hold_run_missing:{plan_id}"))?;
        if persisted.run.phase != FeatureRunPhase::SourceFrozen {
            bail!(
                "verification_readiness_hold_wrong_phase:{}:{:?}",
                persisted.run.id,
                persisted.run.phase
            );
        }
        let freeze = repository
            .active_source_freeze(&persisted.run.id)?
            .ok_or_else(|| {
                anyhow!(
                    "verification_readiness_hold_freeze_missing:{}",
                    persisted.run.id
                )
            })?;
        if freeze.id != freeze_id {
            bail!(
                "verification_readiness_hold_freeze_changed:{}",
                persisted.run.id
            );
        }
        let held = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::Held,
                cause: PhaseTransitionCause::CapabilityHold,
                reference: format!("verification_readiness:{}", freeze.id),
                owner: None,
            },
        )
        .map_err(|violation| anyhow!("verification_readiness_hold_transition:{violation:?}"))?;
        let repair_request = VerificationAdmissionRepairRequest {
            plan_id: plan_id.to_string(),
            run_id: persisted.run.id.clone(),
            freeze_id: freeze.id.clone(),
            run_revision: persisted.revision.checked_add(1).ok_or_else(|| {
                anyhow!(
                    "verification_readiness_hold_revision_overflow:{}",
                    persisted.run.id
                )
            })?,
            reason,
            run_index_digest: None,
        };
        repository.persist_verification_readiness_hold(
            &held,
            persisted.revision,
            &freeze.id,
            &VerificationReadinessDiagnosticRecord {
                repair_request: repair_request.clone(),
                verifier_worker_id: verifier_worker_id.to_string(),
                diagnostic: diagnostic.clone(),
            },
        )?;
        Ok(VerificationPickReadinessError::from_durable_diagnostic(
            plan_id,
            diagnostic,
            repair_request,
            self.canonical_execution_state_value(&persisted.run.id, None)?,
        ))
    }

    pub(crate) fn repair_verification_admission_value(
        &self,
        request: VerificationAdmissionRepairRequest,
    ) -> Result<Value> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        if let Some(previous) = repository.latest_verification_admission_repair(&request)? {
            repository.prove_verification_admission_repair_idempotent(&previous)?;
            return Ok(json!({
                "schema_version": "planr.feature_run_verification_admission_repair.v1",
                "repair": previous,
                "verification_item_id": previous.facts.verification_item_id,
                "execution_state": self.canonical_execution_state_value(&previous.request.run_id, None)?,
            }));
        }
        let persisted = repository.feature_run(&request.run_id)?;
        if persisted.project_id != project.id
            || persisted.run.plan_id != request.plan_id
            || persisted.revision != request.run_revision
        {
            bail!(
                "verification_admission_repair_stale_identity:{}",
                request.run_id
            );
        }
        let freeze = repository
            .active_source_freeze(&request.run_id)?
            .ok_or_else(|| {
                anyhow!(
                    "verification_admission_repair_missing_freeze:{}",
                    request.run_id
                )
            })?;
        if freeze.id != request.freeze_id {
            bail!(
                "verification_admission_repair_stale_freeze:{}",
                request.freeze_id
            );
        }
        let requester_worker_id = worker_id();
        let admitted_run_index_digest = if request.reason.requires_run_index_digest() {
            let admission = repository
                .latest_verification_admission(&request.run_id, &request.freeze_id)?
                .ok_or_else(|| {
                    anyhow!(
                        "verification_admission_repair_seal_missing:{}",
                        request.run_id
                    )
                })?;
            if admission.run_revision != request.run_revision
                || admission.verifier_worker_id != requester_worker_id
                || Some(admission.run_index_digest.as_str()) != request.run_index_digest.as_deref()
            {
                bail!(
                    "verification_admission_repair_seal_mismatch:{}",
                    request.run_id
                );
            }
            Some(admission.run_index_digest)
        } else {
            let diagnostic = repository
                .latest_verification_readiness_diagnostic(&request.run_id, &request.freeze_id)?
                .ok_or_else(|| {
                    anyhow!(
                        "verification_admission_repair_diagnostic_missing:{}",
                        request.run_id
                    )
                })?;
            if diagnostic.repair_request != request
                || diagnostic.verifier_worker_id != requester_worker_id
            {
                bail!(
                    "verification_admission_repair_wrong_owner:{}",
                    request.run_id
                );
            }
            None
        };
        let (historical_maker_worker_id, next_maker_lease_generation) =
            repository.historical_maker_identity(&request.run_id)?;
        let verification_item_id = repository
            .verification_item_projection(&request.plan_id)?
            .map(|item| item.id);
        let facts = VerificationAdmissionRepairFacts {
            active_freeze_id: freeze.id,
            requester_worker_id,
            historical_maker_worker_id,
            next_maker_lease_generation,
            verification_item_id,
            admitted_run_index_digest,
            invalidation_id: short_id("invalidation"),
            repair_batch_id: short_id("batch"),
        };
        let transition =
            repair_verification_admission(&persisted.run, persisted.revision, &request, &facts)
                .map_err(|violation| {
                    anyhow!("verification_admission_repair_rejected:{violation:?}")
                })?;
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT repair_verification_admission")?;
        let result = (|| -> Result<()> {
            if persisted.run.phase == FeatureRunPhase::Verification {
                self.reconcile_active_phase_wall(&persisted.run.id, BudgetPhase::Verification)?;
            }
            repository.persist_verification_admission_repair(&transition, &worker_id())
        })();
        match result {
            Ok(()) => self
                .conn
                .execute_batch("RELEASE repair_verification_admission; COMMIT")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK TO repair_verification_admission; RELEASE repair_verification_admission; ROLLBACK");
                return Err(error);
            }
        }
        Ok(json!({
            "schema_version": "planr.feature_run_verification_admission_repair.v1",
            "repair": transition,
            "verification_item_id": transition.facts.verification_item_id,
            "execution_state": self.canonical_execution_state_value(&request.run_id, None)?,
        }))
    }

    fn add_review_finding_reverification_metadata(
        &self,
        packet: &mut Value,
        run_id: &str,
        plan_id: &str,
    ) -> Result<()> {
        if !packet["item_id"].is_null() {
            return Ok(());
        }
        let repository = ExecutionRunRepository::new(&self.conn);
        let Some(gate) = repository
            .review_gates_for_run(run_id, false)?
            .into_iter()
            .find(|gate| {
                gate.kind == ReviewGateKind::FinalProduct
                    && gate.status == ReviewGateStatus::ChangesRequested
            })
        else {
            return Ok(());
        };
        let findings = repository.findings(&gate.id)?;
        if findings.is_empty()
            || findings
                .iter()
                .any(|finding| finding.status != FindingStatus::Resolved)
        {
            return Ok(());
        }
        let attempts = repository.review_attempts(&gate.id)?;
        let attempt_id = attempts
            .last()
            .map(|attempt| attempt.id.clone())
            .ok_or_else(|| anyhow!("review_reverification_attempt_missing:{}", gate.id))?;
        let mut stmt = self.conn.prepare(
            "SELECT id FROM proof_obligations WHERE plan_id = ?1 AND binding = 1
             AND NOT EXISTS (SELECT 1 FROM proof_obligations successor WHERE successor.supersedes_obligation_id = proof_obligations.id)
             ORDER BY id",
        )?;
        let obligation_ids = stmt
            .query_map([plan_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if obligation_ids.is_empty() {
            bail!("review_reverification_obligations_missing:{}", gate.id);
        }
        packet["mode"] = json!("review_finding_reverification");
        packet["repair_id"] = json!(format!("review:{}:{attempt_id}", gate.id));
        packet["review_gate_id"] = json!(gate.id);
        packet["review_attempt_id"] = json!(attempt_id);
        packet["review_finding_ids"] = json!(
            findings
                .into_iter()
                .map(|finding| finding.id)
                .collect::<Vec<_>>()
        );
        packet["responsible_maker_id"] = json!(gate.responsible_maker_id);
        packet["selective_replay_obligation_ids"] = json!(obligation_ids);
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BudgetUsageReport {
    pub(crate) tool_calls: Option<u64>,
    pub(crate) tool_calls_metering: MeteringMode,
    pub(crate) tokens: Option<u64>,
    pub(crate) tokens_metering: MeteringMode,
    pub(crate) adapter_id: String,
}

impl BudgetUsageReport {
    pub(crate) fn application(tool_calls: Option<u64>) -> Self {
        Self {
            tool_calls,
            tool_calls_metering: if tool_calls.is_some() {
                MeteringMode::Estimated
            } else {
                MeteringMode::Unavailable
            },
            tokens: None,
            tokens_metering: MeteringMode::Unavailable,
            adapter_id: "planr.application_reconciliation".to_string(),
        }
    }
}

pub(crate) enum FeatureRunBudgetAdmission {
    Reserved(FeatureRunBudgetReservation),
    Held(Value),
}

impl App {
    pub(crate) fn resolve_feature_run_evidence_lease(
        &self,
        project_id: &str,
        plan_id: &str,
    ) -> Result<Option<CanonicalFeatureRunEvidenceLease>> {
        let repository = ExecutionRunRepository::new(&self.conn);
        let Some(persisted) = repository.active_feature_run_for_plan(project_id, plan_id)? else {
            return Ok(None);
        };
        if persisted.run.phase == FeatureRunPhase::SourceFrozen {
            bail!(
                "evidence_readiness_requires_verification_lease:phase=source_frozen:owner=unleased: run `planr pick --plan {plan_id} --work-type verification --json`"
            );
        }
        if persisted.run.phase != FeatureRunPhase::Verification {
            bail!(
                "binding_evidence_requires_verification:{}",
                persisted.run.id
            );
        }
        let verifier = persisted
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Verifier)
            .ok_or_else(|| anyhow!("verification_run_missing_verifier:{}", persisted.run.id))?;
        let actual_worker_id = worker_id();
        if verifier.worker_id != actual_worker_id {
            bail!("verification_lease_owned_by:{}", verifier.worker_id);
        }
        let freeze = repository
            .active_source_freeze(&persisted.run.id)?
            .ok_or_else(|| {
                anyhow!(
                    "verification_run_missing_active_freeze:{}",
                    persisted.run.id
                )
            })?;
        let lease = CanonicalFeatureRunEvidenceLease {
            project_id: project_id.to_string(),
            plan_id: plan_id.to_string(),
            run_id: persisted.run.id,
            freeze_id: freeze.id,
            verifier_worker_id: actual_worker_id,
            lease_generation: verifier.lease_generation,
            source_revision: freeze.source_revision,
            source_digest: freeze.source_digest,
        };
        self.validate_feature_run_evidence_lease(&self.conn, &lease)?;
        Ok(Some(lease))
    }

    pub(crate) fn validate_feature_run_evidence_lease(
        &self,
        conn: &rusqlite::Connection,
        lease: &CanonicalFeatureRunEvidenceLease,
    ) -> Result<()> {
        if worker_id() != lease.verifier_worker_id {
            bail!("verification_worker_changed:{}", lease.run_id);
        }
        let repository = ExecutionRunRepository::new(conn);
        let persisted = repository
            .active_feature_run_for_plan(&lease.project_id, &lease.plan_id)?
            .ok_or_else(|| anyhow!("feature_run_not_active_for_plan:{}", lease.plan_id))?;
        if persisted.run.id != lease.run_id || persisted.run.phase != FeatureRunPhase::Verification
        {
            bail!("stale_verification_run:{}", lease.run_id);
        }
        let verifier = persisted
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Verifier)
            .ok_or_else(|| anyhow!("verification_run_missing_verifier:{}", lease.run_id))?;
        if verifier.worker_id != lease.verifier_worker_id
            || verifier.lease_generation != lease.lease_generation
        {
            bail!("stale_verification_lease:{}", lease.run_id);
        }
        let freeze = repository
            .active_source_freeze(&lease.run_id)?
            .ok_or_else(|| anyhow!("verification_run_missing_active_freeze:{}", lease.run_id))?;
        if freeze.id != lease.freeze_id
            || freeze.source_revision != lease.source_revision
            || freeze.source_digest != lease.source_digest
        {
            bail!("stale_source_freeze:{}", lease.freeze_id);
        }
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("checking feature run source freeze: {error}"))?;
        if snapshot.source.revision != lease.source_revision
            || snapshot.source.tree_digest.as_str() != lease.source_digest
        {
            bail!("stale_source_freeze:{}", lease.freeze_id);
        }
        Ok(())
    }

    pub(crate) fn settle_exhausted_verification_attempt_in_transaction(
        &self,
        conn: &rusqlite::Connection,
        lease: &CanonicalFeatureRunEvidenceLease,
        exhaustion: VerificationAttemptExhaustion<'_>,
    ) -> Result<(Option<String>, String)> {
        let VerificationAttemptExhaustion {
            obligation_id,
            attempt_id,
            attempt_index,
            max_attempts,
            repeatability,
        } = exhaustion;
        if conn.is_autocommit() {
            bail!("verification_exhaustion_requires_active_transaction");
        }
        if obligation_id.trim().is_empty() || attempt_id.trim().is_empty() {
            bail!("verification_exhaustion_identity_required");
        }
        if repeatability != "non_repeatable_one_shot" {
            bail!("verification_exhaustion_requires_explicit_one_shot");
        }
        if max_attempts == 0 || attempt_index.checked_add(1) != Some(max_attempts) {
            bail!("verification_exhaustion_requires_final_declared_attempt");
        }
        self.validate_feature_run_evidence_lease(conn, lease)?;
        let repository = ExecutionRunRepository::new(conn);
        let persisted = repository.feature_run(&lease.run_id)?;
        let verifier = owner_for_role(&persisted.run, RunRole::Verifier)
            .ok_or_else(|| anyhow!("verification_exhaustion_missing_verifier:{}", lease.run_id))?;
        if verifier.worker_id != lease.verifier_worker_id
            || verifier.lease_generation != lease.lease_generation
        {
            bail!("verification_exhaustion_stale_lease:{}", lease.run_id);
        }
        let verification_items = conn
            .prepare(
                "SELECT items.id, items.status, items.worker_id
                 FROM items JOIN plans ON plans.path = items.plan_path
                 WHERE plans.id = ?1 AND items.work_type = 'verification'
                   AND items.status IN ('ready','picked','running')
                 ORDER BY items.created_at, items.id",
            )?
            .query_map([&lease.plan_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if verification_items
            .iter()
            .any(|(_, status, _)| status == "ready")
        {
            bail!(
                "verification_exhaustion_requires_verification_item_lease:{}",
                lease.plan_id
            );
        }
        let active_items = verification_items
            .into_iter()
            .filter(|(_, status, _)| matches!(status.as_str(), "picked" | "running"))
            .collect::<Vec<_>>();
        if active_items.len() > 1 {
            bail!(
                "verification_exhaustion_ambiguous_active_items:{}:{}",
                lease.plan_id,
                active_items.len()
            );
        }
        let item_id = active_items
            .first()
            .map(|(item_id, _, item_worker)| {
                if item_worker.as_deref() != Some(lease.verifier_worker_id.as_str()) {
                    bail!("verification_exhaustion_stale_item_owner:{item_id}");
                }
                Ok(item_id.clone())
            })
            .transpose()?;
        let terminal = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::Cancelled,
                cause: PhaseTransitionCause::VerificationAttemptsExhausted,
                reference: format!("evidence_attempt:{attempt_id}"),
                owner: None,
            },
        )
        .map_err(|violation| anyhow!("verification_exhaustion_transition:{violation:?}"))?;
        self.reconcile_active_phase_wall(&lease.run_id, BudgetPhase::Verification)?;
        repository.save_feature_run(&terminal, persisted.revision)?;
        let log_id = if let Some(item_id) = item_id.as_deref() {
            let summary = format!(
                "non-repeatable verification attempt {attempt_id} exhausted {max_attempts} allowed attempt(s)"
            );
            let log_id = self.add_log_entry(super::flow::LogInput {
                item_id,
                kind: "failure",
                summary: &summary,
                files: &[],
                commands: &[],
                tests: &[],
                source: Some("evidence.run"),
                profile: None,
                route_observation: None,
            })?;
            let changed = conn.execute(
                "UPDATE items
                 SET status = 'failed', error = ?2, worker_id = NULL, pick_token = NULL,
                     last_heartbeat_at = NULL, paused_at = NULL, updated_at = datetime('now')
                 WHERE id = ?1 AND work_type = 'verification'
                   AND status IN ('picked','running') AND worker_id = ?3",
                params![item_id, summary, lease.verifier_worker_id],
            )?;
            if changed != 1 {
                bail!("verification_exhaustion_stale_item:{item_id}");
            }
            Some(log_id)
        } else {
            None
        };
        self.record_event(
            "verification_attempts_exhausted",
            item_id.as_deref(),
            json!({
                "run_id": lease.run_id,
                "freeze_id": lease.freeze_id,
                "item_id": item_id.clone(),
                "log_id": log_id.clone(),
                "obligation_id": obligation_id,
                "attempt_id": attempt_id,
                "attempt_index": attempt_index,
                "max_attempts": max_attempts,
                "repeatability": repeatability,
            }),
        )?;
        Ok((item_id, terminal.id))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn claim_non_repeatable_one_shot_in_transaction(
        &self,
        conn: &rusqlite::Connection,
        lease: &CanonicalFeatureRunEvidenceLease,
        obligation_id: &str,
        capability_instance_id: &str,
        retry_of: Option<&str>,
        attempt_index: u32,
        max_attempts: u32,
        repeatability: &str,
    ) -> Result<()> {
        if repeatability != "non_repeatable_one_shot" {
            return Ok(());
        }
        if retry_of.is_some() || attempt_index != 0 || max_attempts != 1 {
            bail!(
                "non_repeatable_one_shot permits exactly one fresh initial attempt per FeatureRun source freeze"
            );
        }
        if !conn.is_autocommit() {
            bail!("non_repeatable_one_shot claim requires an autocommit boundary");
        }
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            self.validate_feature_run_evidence_lease(conn, lease)?;
            let existing = conn
                .query_row(
                    "SELECT obligation_id, capability_instance_id
                     FROM feature_run_one_shot_claims
                     WHERE freeze_id = ?1",
                    [&lease.freeze_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            if let Some((claimed_obligation, claimed_capability)) = existing {
                bail!(
                    "non_repeatable_one_shot allowance already consumed for FeatureRun {} source freeze {} by obligation {} capability {}",
                    lease.run_id,
                    lease.freeze_id,
                    claimed_obligation,
                    claimed_capability,
                );
            }
            conn.execute(
                "INSERT INTO feature_run_one_shot_claims(
                   freeze_id, run_id, obligation_id, capability_instance_id,
                   verifier_worker_id, lease_generation, attempt_index, max_attempts
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 1)",
                params![
                    lease.freeze_id,
                    lease.run_id,
                    obligation_id,
                    capability_instance_id,
                    lease.verifier_worker_id,
                    lease.lease_generation,
                ],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(error);
            }
        }
        Ok(())
    }

    // Admission and reservation share one immediate transaction. Every bounded reservation stores
    // the exact numeric maxima selected from the persisted snapshot; no caller projection is ever
    // converted into an observation or metering claim.
    pub(crate) fn admit_feature_run_budget(
        &self,
        persisted: &PersistedFeatureRun,
        phase: BudgetPhase,
        boundary_key: &str,
        actor_worker_id: &str,
        provenance: &str,
    ) -> Result<FeatureRunBudgetAdmission> {
        if boundary_key.trim().is_empty()
            || actor_worker_id.trim().is_empty()
            || provenance.trim().is_empty()
        {
            bail!("budget reservation boundary, actor, and provenance must be non-empty");
        }
        let owns_transaction = self.conn.is_autocommit();
        self.conn.execute_batch(if owns_transaction {
            "BEGIN IMMEDIATE"
        } else {
            "SAVEPOINT admit_feature_run_budget"
        })?;
        let result = (|| -> Result<FeatureRunBudgetAdmission> {
            let repository = ExecutionRunRepository::new(&self.conn);
            let current = repository.feature_run(&persisted.run.id)?;
            let now_unix_ms = unix_time_ms()?;
            let existing = match repository.active_budget_reservation(&current.run.id, boundary_key)
            {
                Ok(value) => value,
                Err(error) => {
                    let hold = self.persist_budget_hold(
                        &current,
                        BudgetTaskHoldReason::InvalidBudgetState,
                        format!("cannot load persisted budget reservation: {error}"),
                    )?;
                    return Ok(FeatureRunBudgetAdmission::Held(hold));
                }
            };
            if let Some(existing) = existing {
                if existing.phase != phase || existing.owner_worker_id != actor_worker_id {
                    let hold = self.persist_budget_hold(
                        &current,
                        BudgetTaskHoldReason::InvalidBudgetState,
                        "active budget reservation phase or owner does not match its dispatch boundary",
                    )?;
                    return Ok(FeatureRunBudgetAdmission::Held(hold));
                }
                if existing
                    .execution_budget
                    .is_some_and(|budget| now_unix_ms >= budget.deadline_unix_ms)
                {
                    let hold = self.persist_budget_hold(
                        &current,
                        BudgetTaskHoldReason::TaskDeadlineExceeded,
                        "the admitted task deadline has elapsed",
                    )?;
                    return Ok(FeatureRunBudgetAdmission::Held(hold));
                }
                return Ok(FeatureRunBudgetAdmission::Reserved(existing));
            }

            let (mut contract, mut snapshot) = match self.persisted_budget_snapshot(
                &current,
                feature_run_budget_phase(phase),
                now_unix_ms,
            ) {
                Ok(value) => value,
                Err(error) => {
                    let hold = self.persist_budget_hold(
                        &current,
                        BudgetTaskHoldReason::InvalidBudgetState,
                        format!("cannot derive persisted budget snapshot: {error}"),
                    )?;
                    return Ok(FeatureRunBudgetAdmission::Held(hold));
                }
            };
            let mut declared_maxima = match contract.mode {
                FeatureRunBudgetMode::Unbounded => None,
                FeatureRunBudgetMode::Bounded => snapshot.available,
            };
            if contract.mode == FeatureRunBudgetMode::Bounded
                && declared_maxima.is_none_or(|maxima| !budget_amounts_are_positive(maxima))
            {
                let authoritative = repository.feature_run(&current.run.id)?;
                (contract, snapshot) = self.persisted_budget_snapshot(
                    &authoritative,
                    feature_run_budget_phase(phase),
                    unix_time_ms()?,
                )?;
                declared_maxima = match contract.mode {
                    FeatureRunBudgetMode::Unbounded => None,
                    FeatureRunBudgetMode::Bounded => snapshot.available,
                };
                if contract.mode == FeatureRunBudgetMode::Bounded
                    && declared_maxima.is_none_or(|maxima| !budget_amounts_are_positive(maxima))
                {
                    let hold = self.persist_budget_hold(
                        &authoritative,
                        BudgetTaskHoldReason::BudgetExhausted,
                        "no complete bounded task maxima remain after authoritative reservation reconciliation",
                    )?;
                    return Ok(FeatureRunBudgetAdmission::Held(hold));
                }
            }
            let execution_budget =
                match admit_budget_task(&contract, &snapshot, declared_maxima, now_unix_ms) {
                    BudgetTaskAdmission::Admitted { execution_budget } => execution_budget,
                    BudgetTaskAdmission::Held { reason, message } => {
                        let hold = self.persist_budget_hold(&current, reason, message)?;
                        return Ok(FeatureRunBudgetAdmission::Held(hold));
                    }
                };
            let owner_role = budget_phase_owner_role(phase);
            let owner = owner_for_role(&current.run, owner_role).ok_or_else(|| {
                anyhow!(
                    "budget_reservation_missing_{owner_role:?}_owner:{}",
                    current.run.id
                )
            })?;
            if owner.worker_id != actor_worker_id {
                let hold = self.persist_budget_hold(
                    &current,
                    BudgetTaskHoldReason::InvalidBudgetState,
                    format!(
                        "budget reservation owner mismatch: active {:?} owner is {}",
                        owner_role, owner.worker_id
                    ),
                )?;
                return Ok(FeatureRunBudgetAdmission::Held(hold));
            }
            let reservation = BudgetReservationRecord {
                id: short_id("budget-reservation"),
                run_id: current.run.id.clone(),
                phase,
                boundary_key: boundary_key.to_string(),
                owner_role,
                owner_worker_id: owner.worker_id.clone(),
                lease_generation: owner.lease_generation,
                execution_budget,
                started_at_unix_ms: now_unix_ms,
                provenance: provenance.to_string(),
            };
            repository.create_budget_reservation(&reservation)?;
            Ok(FeatureRunBudgetAdmission::Reserved(reservation))
        })();
        match result {
            Ok(value) => {
                self.conn.execute_batch(if owns_transaction {
                    "COMMIT"
                } else {
                    "RELEASE admit_feature_run_budget"
                })?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(if owns_transaction {
                    "ROLLBACK"
                } else {
                    "ROLLBACK TO admit_feature_run_budget; RELEASE admit_feature_run_budget"
                });
                Err(error)
            }
        }
    }

    pub(crate) fn reconcile_feature_run_budget(
        &self,
        reservation: &FeatureRunBudgetReservation,
        usage: &BudgetUsageReport,
    ) -> Result<()> {
        validate_usage_report(usage)?;
        let owns_transaction = self.conn.is_autocommit();
        self.conn.execute_batch(if owns_transaction {
            "BEGIN IMMEDIATE"
        } else {
            "SAVEPOINT reconcile_feature_run_budget"
        })?;
        let result = (|| -> Result<()> {
            let repository = ExecutionRunRepository::new(&self.conn);
            let persisted = repository
                .budget_reservations(&reservation.run_id)?
                .into_iter()
                .find(|value| value.reservation.id == reservation.id)
                .ok_or_else(|| anyhow!("budget_reservation_not_found:{}", reservation.id))?;
            if persisted.status == BudgetReservationStatus::Reconciled {
                return Ok(());
            }
            if persisted.status != BudgetReservationStatus::Active
                || persisted.reservation != *reservation
            {
                bail!("budget_reservation_not_active:{}", reservation.id);
            }
            let now_unix_ms = unix_time_ms()?;
            let elapsed_ms = now_unix_ms
                .checked_sub(reservation.started_at_unix_ms)
                .ok_or_else(|| anyhow!("budget_reconciliation_clock_before_admission"))?;
            let actual_wall_seconds = elapsed_ms
                .saturating_add(999)
                .checked_div(1_000)
                .unwrap_or(0)
                .max(1);
            let observations = repository.budget_observations(&reservation.run_id)?;
            let ledger_sequence = observations
                .iter()
                .filter(|observation| {
                    observation.reservation_id.as_deref() == Some(&reservation.id)
                })
                .filter_map(|observation| observation.sequence)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| {
                    anyhow!("budget_observation_sequence_overflow:{}", reservation.id)
                })?;
            let aggregate_metering = MeteringMode::Trusted
                .min(usage.tool_calls_metering)
                .min(usage.tokens_metering);
            repository.record_budget_observation(&BudgetObservationRecord {
                id: short_id("budget"),
                run_id: reservation.run_id.clone(),
                reservation_id: Some(reservation.id.clone()),
                sequence: Some(ledger_sequence),
                phase: reservation.phase,
                metering: aggregate_metering,
                wall_metering: Some(MeteringMode::Trusted),
                tool_calls_metering: Some(usage.tool_calls_metering),
                tokens_metering: Some(usage.tokens_metering),
                wall_seconds: Some(actual_wall_seconds),
                tokens: usage.tokens,
                tool_calls: usage.tool_calls,
                credits_micros: None,
                provenance: format!(
                    "wall_seconds=planr.utc_clock:trusted;tool_calls={}:{:?};tokens={}:{:?}",
                    usage.adapter_id,
                    usage.tool_calls_metering,
                    usage.adapter_id,
                    usage.tokens_metering,
                )
                .to_ascii_lowercase(),
                adapter_id: Some(usage.adapter_id.clone()),
                observed_at_unix_ms: Some(now_unix_ms),
            })?;
            repository.reconcile_budget_reservation(&reservation.id, &reservation.run_id)?;
            Ok(())
        })();
        match result {
            Ok(()) => self
                .conn
                .execute_batch(if owns_transaction {
                    "COMMIT"
                } else {
                    "RELEASE reconcile_feature_run_budget"
                })
                .map_err(Into::into),
            Err(error) => {
                let _ = self.conn.execute_batch(if owns_transaction {
                    "ROLLBACK"
                } else {
                    "ROLLBACK TO reconcile_feature_run_budget; RELEASE reconcile_feature_run_budget"
                });
                Err(error)
            }
        }
    }

    pub(crate) fn release_feature_run_budget(
        &self,
        reservation: &FeatureRunBudgetReservation,
    ) -> Result<()> {
        ExecutionRunRepository::new(&self.conn)
            .release_budget_reservation(&reservation.id, &reservation.run_id)
    }

    pub(crate) fn load_active_budget_reservation(
        &self,
        run_id: &str,
        boundary_key: &str,
    ) -> Result<Option<FeatureRunBudgetReservation>> {
        ExecutionRunRepository::new(&self.conn).active_budget_reservation(run_id, boundary_key)
    }

    pub(crate) fn reconcile_active_phase_wall(
        &self,
        run_id: &str,
        phase: BudgetPhase,
    ) -> Result<()> {
        let reservations = ExecutionRunRepository::new(&self.conn)
            .budget_reservations(run_id)?
            .into_iter()
            .filter(|value| {
                value.status == BudgetReservationStatus::Active && value.reservation.phase == phase
            })
            .map(|value| value.reservation)
            .collect::<Vec<_>>();
        for reservation in reservations {
            self.reconcile_feature_run_budget(
                &reservation,
                &BudgetUsageReport::application(Some(1)),
            )?;
        }
        Ok(())
    }

    pub(crate) fn persisted_budget_snapshot(
        &self,
        persisted: &PersistedFeatureRun,
        released_through: FeatureRunBudgetPhase,
        now_unix_ms: u64,
    ) -> Result<(FeatureRunBudgetContract, BudgetSnapshot)> {
        let repository = ExecutionRunRepository::new(&self.conn);
        let contract = repository.budget_contract(&persisted.run.id)?;
        let reservations = repository.budget_reservations(&persisted.run.id)?;
        let observations = repository.budget_observations(&persisted.run.id)?;
        if observations
            .iter()
            .any(|observation| observation.reservation_id.is_none())
        {
            bail!(
                "feature_run_budget_contains_unbound_observation:{}",
                persisted.run.id
            );
        }
        let elapsed_ms = now_unix_ms
            .checked_sub(contract.started_at_unix_ms)
            .ok_or_else(|| anyhow!("feature_run_clock_before_persisted_run_start"))?;
        let wall_seconds = elapsed_ms.saturating_add(999) / 1_000;
        let mut reserved = BudgetAmounts::ZERO;
        let mut consumed_tool_calls = 0_u64;
        let mut consumed_tokens = 0_u64;
        let mut tool_calls_metering = MeteringMode::Unavailable;
        let mut tokens_metering = MeteringMode::Unavailable;
        let mut observed_terminal_reservation = false;
        let mut active_execution_budget: Option<ExecutionBudget> = None;

        for persisted_reservation in &reservations {
            let reservation = &persisted_reservation.reservation;
            match (contract.mode, reservation.execution_budget) {
                (FeatureRunBudgetMode::Bounded, None)
                | (FeatureRunBudgetMode::Unbounded, Some(_)) => {
                    bail!("budget_reservation_mode_mismatch:{}", reservation.id)
                }
                _ => {}
            }
            match persisted_reservation.status {
                BudgetReservationStatus::Active => {
                    if let Some(execution_budget) = reservation.execution_budget {
                        reserved = reserved.checked_add(execution_budget.maxima())?;
                        if active_execution_budget.is_none_or(|current| {
                            execution_budget.deadline_unix_ms < current.deadline_unix_ms
                        }) {
                            active_execution_budget = Some(execution_budget);
                        }
                    }
                }
                BudgetReservationStatus::Released => {
                    if observations.iter().any(|observation| {
                        observation.reservation_id.as_deref() == Some(&reservation.id)
                    }) {
                        bail!(
                            "released_budget_reservation_has_observation:{}",
                            reservation.id
                        );
                    }
                }
                BudgetReservationStatus::Reconciled => {
                    let task_observations = observations
                        .iter()
                        .filter(|observation| {
                            observation.reservation_id.as_deref() == Some(&reservation.id)
                        })
                        .collect::<Vec<_>>();
                    if task_observations.is_empty() {
                        bail!(
                            "reconciled_budget_reservation_missing_observation:{}",
                            reservation.id
                        );
                    }
                    let (tool_calls, task_tool_metering) = reconciled_dimension(
                        &task_observations,
                        |observation| observation.tool_calls,
                        |observation| observation.tool_calls_metering,
                        reservation
                            .execution_budget
                            .map(|budget| budget.max_tool_calls),
                    )?;
                    let (tokens, task_token_metering) = reconciled_dimension(
                        &task_observations,
                        |observation| observation.tokens,
                        |observation| observation.tokens_metering,
                        reservation.execution_budget.map(|budget| budget.max_tokens),
                    )?;
                    consumed_tool_calls = consumed_tool_calls
                        .checked_add(tool_calls)
                        .ok_or_else(|| anyhow!("consumed tool-call budget overflow"))?;
                    consumed_tokens = consumed_tokens
                        .checked_add(tokens)
                        .ok_or_else(|| anyhow!("consumed token budget overflow"))?;
                    tool_calls_metering = if observed_terminal_reservation {
                        tool_calls_metering.min(task_tool_metering)
                    } else {
                        task_tool_metering
                    };
                    tokens_metering = if observed_terminal_reservation {
                        tokens_metering.min(task_token_metering)
                    } else {
                        task_token_metering
                    };
                    observed_terminal_reservation = true;
                }
            }
        }
        if !observed_terminal_reservation {
            tool_calls_metering = MeteringMode::Unavailable;
            tokens_metering = MeteringMode::Unavailable;
        }
        let snapshot = budget_snapshot(
            &contract,
            released_through,
            BudgetAmounts {
                wall_seconds,
                tool_calls: consumed_tool_calls,
                tokens: consumed_tokens,
            },
            reserved,
            BudgetProvenance {
                wall_seconds: MeteringProvenance::Trusted,
                tool_calls: MeteringProvenance::from(tool_calls_metering),
                tokens: MeteringProvenance::from(tokens_metering),
            },
            active_execution_budget,
        )?;
        Ok((contract, snapshot))
    }

    fn persist_budget_hold(
        &self,
        persisted: &PersistedFeatureRun,
        reason: BudgetTaskHoldReason,
        message: impl Into<String>,
    ) -> Result<Value> {
        let repository = ExecutionRunRepository::new(&self.conn);
        let held = if persisted.run.phase == FeatureRunPhase::Held {
            persisted.run.clone()
        } else {
            let held = apply_phase_transition(
                &persisted.run,
                &PhaseTransition {
                    to: FeatureRunPhase::Held,
                    cause: PhaseTransitionCause::BudgetHold,
                    reference: format!("budget:{reason:?}").to_ascii_lowercase(),
                    owner: None,
                },
            )
            .map_err(|violation| anyhow!("budget_hold_transition:{violation:?}"))?;
            repository.save_feature_run(&held, persisted.revision)?;
            held
        };
        let reason = serde_json::to_value(reason)?;
        Ok(json!({
            "work_packet": {
                "kind": "hold",
                "classification": "budget",
                "reason": reason,
                "message": message.into(),
                "execution_state": self.canonical_execution_state_value(&held.id, None)?,
            },
            "remaining": self.progress_value()?,
        }))
    }

    pub(crate) fn resolve_feature_run_budget_hold_value(&self, plan_id: &str) -> Result<Value> {
        let project = self.default_project()?;
        let operator = worker_id();
        let owns_transaction = self.conn.is_autocommit();
        self.conn.execute_batch(if owns_transaction {
            "BEGIN IMMEDIATE"
        } else {
            "SAVEPOINT resolve_feature_run_budget_hold"
        })?;
        let result = (|| -> Result<FeatureRunBudgetHoldResolutionTransition> {
            let repository = ExecutionRunRepository::new(&self.conn);
            let persisted = repository
                .active_feature_run_for_plan(&project.id, plan_id)?
                .ok_or_else(|| anyhow!("feature_run_not_found_for_plan:{plan_id}"))?;
            if persisted.run.phase != FeatureRunPhase::Held {
                let previous = self
                    .latest_feature_run_budget_hold_resolution(&project.id, plan_id)?
                    .filter(|transition| transition.resumed_run == persisted.run)
                    .ok_or_else(|| {
                        anyhow!("feature_run_budget_hold_resolution_rejected:not_budget_held")
                    })?;
                return Ok(FeatureRunBudgetHoldResolutionTransition {
                    disposition: FeatureRunBudgetHoldResolutionDisposition::AlreadyResumed,
                    ..previous
                });
            }
            if persisted.run.hold_reason != Some(FeatureRunHoldReason::Budget) {
                bail!("feature_run_budget_hold_resolution_rejected:capability_hold");
            }
            let compatibility = repository.budget_contract_compatibility(&persisted.run.id)?;
            if compatibility != FeatureRunBudgetContractCompatibility::Compatible {
                bail!(
                    "feature_run_budget_hold_requires_restart:{compatibility:?}:restart_incompatible_feature_run"
                );
            }

            let reservations = repository.budget_reservations(&persisted.run.id).map_err(
                |error| {
                    anyhow!(
                        "feature_run_budget_hold_resolution_rejected:corrupt_reservation:{error}"
                    )
                },
            )?;
            let active = reservations
                .iter()
                .filter(|reservation| reservation.status == BudgetReservationStatus::Active)
                .map(|reservation| &reservation.reservation)
                .collect::<Vec<_>>();
            let (_phase, cause) = if active.is_empty() {
                let phase = held_feature_run_budget_phase(&persisted.run)?;
                if !reservations.iter().any(|reservation| {
                    reservation.status == BudgetReservationStatus::Reconciled
                        && reservation.reservation.phase == phase
                }) {
                    bail!(
                        "feature_run_budget_hold_resolution_rejected:unrepaired_ceiling_or_missing_reservation"
                    );
                }
                let owner_role = budget_phase_owner_role(phase);
                let owner = owner_for_role(&persisted.run, owner_role).ok_or_else(|| {
                    anyhow!("feature_run_budget_hold_resolution_rejected:owner_missing")
                })?;
                if owner.worker_id != operator {
                    bail!("feature_run_budget_hold_resolution_rejected:owner_mismatch");
                }
                let (_, snapshot) = self
                    .persisted_budget_snapshot(
                        &persisted,
                        feature_run_budget_phase(phase),
                        unix_time_ms()?,
                    )
                    .map_err(|error| {
                        anyhow!(
                            "feature_run_budget_hold_resolution_rejected:corrupt_budget:{error}"
                        )
                    })?;
                if snapshot.mode == FeatureRunBudgetMode::Bounded
                    && snapshot
                        .available
                        .is_none_or(|available| !budget_amounts_are_positive(available))
                {
                    bail!("feature_run_budget_hold_resolution_rejected:unrepaired_ceiling");
                }
                (
                    phase,
                    FeatureRunBudgetHoldResolutionCause::TransientContentionCleared,
                )
            } else {
                let phase = active[0].phase;
                if !active.iter().all(|reservation| reservation.phase == phase)
                    || !budget_phase_matches_held_origin(phase, persisted.run.held_from_phase)
                {
                    bail!("feature_run_budget_hold_resolution_rejected:phase_mismatch");
                }
                let owner_role = budget_phase_owner_role(phase);
                let owner = owner_for_role(&persisted.run, owner_role).ok_or_else(|| {
                    anyhow!("feature_run_budget_hold_resolution_rejected:owner_missing")
                })?;
                if owner.worker_id != operator
                    || active.iter().any(|reservation| {
                        reservation.owner_role != owner_role
                            || reservation.owner_worker_id != owner.worker_id
                            || reservation.lease_generation != owner.lease_generation
                    })
                {
                    bail!("feature_run_budget_hold_resolution_rejected:owner_mismatch");
                }
                let now_unix_ms = unix_time_ms()?;
                if active.iter().any(|reservation| {
                    reservation
                        .execution_budget
                        .is_some_and(|budget| now_unix_ms >= budget.deadline_unix_ms)
                }) {
                    bail!("feature_run_budget_hold_resolution_rejected:task_deadline_exceeded");
                }
                self.persisted_budget_snapshot(
                    &persisted,
                    feature_run_budget_phase(phase),
                    now_unix_ms,
                )
                .map_err(|error| {
                    anyhow!("feature_run_budget_hold_resolution_rejected:corrupt_budget:{error}")
                })?;
                (
                    phase,
                    FeatureRunBudgetHoldResolutionCause::ActiveReservationsRevalidated,
                )
            };

            let request = FeatureRunBudgetHoldResolutionRequest {
                plan_id: plan_id.to_string(),
            };
            let resumed_run = resolve_budget_held_feature_run(&persisted.run, &request, &operator)
                .map_err(|violation| {
                    anyhow!("feature_run_budget_hold_resolution_rejected:{violation:?}")
                })?;
            repository.save_feature_run(&resumed_run, persisted.revision)?;
            let transition = FeatureRunBudgetHoldResolutionTransition {
                request,
                disposition: FeatureRunBudgetHoldResolutionDisposition::Resumed,
                cause,
                previous_phase: persisted
                    .run
                    .held_from_phase
                    .expect("held origin validated"),
                active_reservation_ids: active
                    .iter()
                    .map(|reservation| reservation.id.clone())
                    .collect(),
                resumed_run,
            };
            self.conn.execute(
                "INSERT INTO events(project_id, item_id, worker_id, event_type, payload, timestamp) VALUES (?1, NULL, ?2, 'feature_run_budget_hold_resolved', ?3, datetime('now'))",
                params![project.id, operator, serde_json::to_string(&transition)?],
            )?;
            Ok(transition)
        })();
        let transition = match result {
            Ok(value) => {
                self.conn.execute_batch(if owns_transaction {
                    "COMMIT"
                } else {
                    "RELEASE resolve_feature_run_budget_hold"
                })?;
                value
            }
            Err(error) => {
                let _ = self.conn.execute_batch(if owns_transaction {
                    "ROLLBACK"
                } else {
                    "ROLLBACK TO resolve_feature_run_budget_hold; RELEASE resolve_feature_run_budget_hold"
                });
                return Err(error);
            }
        };
        let run_id = transition.resumed_run.id.clone();
        Ok(json!({
            "schema_version": "planr.feature_run_budget_hold_resolution.v2",
            "resolution": transition,
            "execution_state": self.canonical_execution_state_value(&run_id, None)?,
        }))
    }

    fn latest_feature_run_budget_hold_resolution(
        &self,
        project_id: &str,
        plan_id: &str,
    ) -> Result<Option<FeatureRunBudgetHoldResolutionTransition>> {
        self.conn
            .query_row(
                "SELECT payload FROM events WHERE project_id = ?1 AND event_type = 'feature_run_budget_hold_resolved' AND json_extract(payload, '$.request.plan_id') = ?2 ORDER BY id DESC LIMIT 1",
                params![project_id, plan_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn continue_review_budget(
        &self,
        run: &PersistedFeatureRun,
        gate_id: &str,
        reviewer_worker_id: &str,
    ) -> Result<Option<Value>> {
        match self.admit_feature_run_budget(
            run,
            BudgetPhase::Review,
            &format!("review:{gate_id}"),
            reviewer_worker_id,
            "review.completion",
        )? {
            FeatureRunBudgetAdmission::Held(hold) => Ok(Some(hold)),
            FeatureRunBudgetAdmission::Reserved(_) => Ok(None),
        }
    }

    pub(crate) fn repair_work_packet_value(&self, plan_id: &str) -> Result<Option<Value>> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        if let Some(run) = repository.active_feature_run_for_plan(&project.id, plan_id)?
            && let Some(hold) = self.premature_source_freeze_restart_hold_for_run(&run)?
        {
            return Ok(Some(hold));
        }
        if let Some(gate) = repository.repair_review_gate_for_plan(&project.id, plan_id)? {
            if gate.responsible_maker_id != worker_id() {
                return Ok(None);
            }
            let persisted = repository.feature_run(&gate.run_id)?;
            match self.admit_feature_run_budget(
                &persisted,
                BudgetPhase::Repair,
                &format!("repair:{}", persisted.run.id),
                &worker_id(),
                "repair.dispatch",
            )? {
                FeatureRunBudgetAdmission::Held(hold) => return Ok(Some(hold)),
                FeatureRunBudgetAdmission::Reserved(_) => {}
            }
            return Ok(Some(json!({
                "work_packet": {"kind": "outcome", "mode": "finding_repair", "gate_id": gate.id,
                    "finding_ids": repository.findings(&gate.id)?.into_iter().filter(|finding| finding.status == FindingStatus::Open).map(|finding| finding.id).collect::<Vec<_>>(),
                    "responsible_maker_id": gate.responsible_maker_id,
                    "execution_state": self.canonical_execution_state_value(&persisted.run.id, Some(&gate.id))?},
                "remaining": self.progress_value()?
            })));
        }
        if let Some(run) = repository.active_feature_run_for_plan(&project.id, plan_id)?
            && run.run.phase == FeatureRunPhase::Implementation
        {
            if let Some((latest, dispatch)) = self.pending_repair_settlement(&run.run.id)? {
                let maker_worker_id = run
                    .run
                    .role_owners
                    .iter()
                    .find(|owner| owner.role == RunRole::Maker)
                    .map(|owner| owner.worker_id.clone())
                    .ok_or_else(|| anyhow!("feature_run_missing_maker:{}", run.run.id))?;
                if maker_worker_id != worker_id() {
                    return Ok(None);
                }
                let replay_obligation_ids = match dispatch {
                    RepairSettlementDispatchMode::ProductFinding => {
                        Some(self.product_repair_obligation_ids(&latest.affected_evidence_ids)?)
                    }
                    RepairSettlementDispatchMode::VerificationAdmission => None,
                };
                match self.admit_feature_run_budget(
                    &run,
                    BudgetPhase::Repair,
                    &format!("repair:{}", run.run.id),
                    &worker_id(),
                    "repair.dispatch",
                )? {
                    FeatureRunBudgetAdmission::Held(hold) => return Ok(Some(hold)),
                    FeatureRunBudgetAdmission::Reserved(_) => {}
                }
                let plan = self.get_plan(plan_id)?;
                let verification_item_id =
                    self.ready_verification_item_for_plan_path(Some(plan.path.as_str()))?;
                let mut packet = json!({
                    "kind": "outcome",
                    "mode": match dispatch {
                        RepairSettlementDispatchMode::ProductFinding => "product_finding_repair",
                        RepairSettlementDispatchMode::VerificationAdmission => "verification_admission_repair",
                    },
                    "execution_state": self.canonical_execution_state_value(&run.run.id, None)?,
                    "repair_id": latest.id,
                    "responsible_maker_id": maker_worker_id,
                    "verification_item_id": verification_item_id,
                    "invalidation": latest,
                });
                if let Some(replay_obligation_ids) = replay_obligation_ids {
                    packet["selective_replay_obligation_ids"] = json!(replay_obligation_ids);
                }
                return Ok(Some(json!({"work_packet": packet,
                    "remaining": self.progress_value()?})));
            }
        }
        Ok(None)
    }

    fn product_repair_obligation_ids(&self, affected_ids: &[String]) -> Result<Vec<String>> {
        let mut obligations = std::collections::BTreeSet::new();
        for id in affected_ids {
            let mut current = id.clone();
            let mut visited = std::collections::BTreeSet::new();
            loop {
                if !visited.insert(current.clone()) {
                    bail!("product_finding_repair_obligation_cycle:{current}");
                }
                let mut statement = self.conn.prepare(
                    "SELECT id FROM proof_obligations WHERE supersedes_obligation_id = ?1 ORDER BY created_at, id",
                )?;
                let successors = statement
                    .query_map([&current], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                match successors.as_slice() {
                    [] => break,
                    [successor] => current = successor.clone(),
                    _ => bail!("product_finding_repair_ambiguous_successor:{current}"),
                }
            }
            let active = self
                .conn
                .query_row(
                    "SELECT binding FROM proof_obligations WHERE id = ?1",
                    [&current],
                    |row| row.get::<_, bool>(0),
                )
                .optional()?;
            if active == Some(true) {
                obligations.insert(current);
            }
        }
        if obligations.is_empty() {
            bail!("product_finding_repair_missing_selective_obligations");
        }
        Ok(obligations.into_iter().collect())
    }

    pub(crate) fn settle_repair_value(
        &self,
        plan_id: &str,
        invalidation_id: &str,
        summary: &str,
        files: &[String],
        commands: &[String],
        tests: &[String],
    ) -> Result<Value> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository
            .active_feature_run_for_plan(&project.id, plan_id)?
            .ok_or_else(|| anyhow!("repair_settlement_run_missing:{plan_id}"))?;
        let invalidation = repository
            .invalidations(&persisted.run.id)?
            .into_iter()
            .find(|invalidation| invalidation.id == invalidation_id)
            .ok_or_else(|| anyhow!("repair_settlement_invalidation_missing:{invalidation_id}"))?;
        match self
            .repair_settlement_dispatch(&invalidation)?
            .ok_or_else(|| {
                anyhow!("repair_settlement_invalidation_unsupported:{invalidation_id}")
            })? {
            RepairSettlementDispatchMode::ProductFinding => self
                .settle_product_finding_repair_value(
                    plan_id,
                    invalidation_id,
                    summary,
                    files,
                    commands,
                    tests,
                ),
            RepairSettlementDispatchMode::VerificationAdmission => self
                .settle_verification_admission_repair_value(
                    plan_id,
                    &invalidation,
                    summary,
                    files,
                    commands,
                    tests,
                ),
        }
    }

    fn settle_verification_admission_repair_value(
        &self,
        plan_id: &str,
        invalidation: &EvidenceInvalidationRecord,
        summary: &str,
        files: &[String],
        commands: &[String],
        tests: &[String],
    ) -> Result<Value> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository
            .active_feature_run_for_plan(&project.id, plan_id)?
            .ok_or_else(|| {
                anyhow!("verification_admission_repair_settlement_run_missing:{plan_id}")
            })?;
        let plan = self.get_plan(plan_id)?;
        if let Some(existing) =
            repository.verification_admission_repair_settlement(&invalidation.id)?
        {
            repository.prove_verification_admission_repair_settlement(
                invalidation,
                &existing,
                &worker_id(),
            )?;
            let source_freeze = repository.source_freeze(&existing.source_freeze_id)?;
            let verification_item_id =
                self.ready_verification_item_for_plan_path(Some(plan.path.as_str()))?;
            let mut handoff = self.canonical_verification_handoff_value(
                plan_id,
                verification_item_id,
                serde_json::to_value(source_freeze)?,
            )?;
            handoff["settlement"] = serde_json::to_value(existing)?;
            handoff["created"] = json!(false);
            return Ok(handoff);
        }
        if persisted.run.phase != FeatureRunPhase::Implementation
            || persisted.run.status != crate::execution_run::FeatureRunStatus::Active
            || persisted.run.role_owners.len() != 1
        {
            bail!(
                "verification_admission_repair_settlement_requires_implementation:{}",
                invalidation.id
            );
        }
        let maker = owner_for_role(&persisted.run, RunRole::Maker)
            .ok_or_else(|| anyhow!("feature_run_missing_maker:{}", persisted.run.id))?;
        if maker.worker_id != worker_id() {
            bail!(
                "verification_admission_repair_settlement_maker_mismatch:{}",
                invalidation.id
            );
        }
        let repair_batch_id = persisted.run.active_batch_id.clone().ok_or_else(|| {
            anyhow!(
                "verification_admission_repair_batch_missing:{}",
                persisted.run.id
            )
        })?;
        let verification_item_id =
            self.ready_verification_item_for_plan_path(Some(plan.path.as_str()))?;
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("capturing repaired canonical source freeze: {error}"))?;
        let freeze = SourceFreezeRecord {
            id: short_id("freeze"),
            run_id: persisted.run.id.clone(),
            source_revision: snapshot.source.revision.clone(),
            source_digest: snapshot.source.tree_digest.as_str().to_string(),
            status: SourceFreezeStatus::Active,
        };
        let frozen = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::SourceFrozen,
                cause: PhaseTransitionCause::ImplementationSettled,
                reference: freeze.source_revision.clone(),
                owner: None,
            },
        )
        .map_err(|violation| {
            anyhow!("verification_admission_repair_settlement_refreeze:{violation:?}")
        })?;
        let settlement = VerificationAdmissionRepairSettlementRecord {
            invalidation_id: invalidation.id.clone(),
            run_id: persisted.run.id.clone(),
            repair_batch_id,
            responsible_maker_id: worker_id(),
            ended_revision: persisted.revision.checked_add(1).ok_or_else(|| {
                anyhow!(
                    "verification_admission_repair_settlement_revision_overflow:{}",
                    persisted.run.id
                )
            })?,
            settlement: json!({
                "summary": summary,
                "files": files,
                "commands": commands,
                "tests": tests,
            }),
            source_freeze_id: freeze.id.clone(),
            source_revision: freeze.source_revision.clone(),
            source_digest: freeze.source_digest.clone(),
        };
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT settle_verification_admission_repair")?;
        let result = (|| -> Result<()> {
            self.reconcile_active_phase_wall(&persisted.run.id, BudgetPhase::Repair)?;
            repository.persist_verification_admission_repair_settlement(
                invalidation,
                persisted.revision,
                verification_item_id.as_deref(),
                &settlement,
                &freeze,
                &frozen,
                &worker_id(),
            )
        })();
        match result {
            Ok(()) => self
                .conn
                .execute_batch("RELEASE settle_verification_admission_repair; COMMIT")?,
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO settle_verification_admission_repair; RELEASE settle_verification_admission_repair; ROLLBACK",
                );
                return Err(error);
            }
        }
        let mut handoff = self.canonical_verification_handoff_value(
            plan_id,
            verification_item_id,
            serde_json::to_value(&freeze)?,
        )?;
        handoff["settlement"] = serde_json::to_value(settlement)?;
        handoff["created"] = json!(true);
        Ok(handoff)
    }

    pub(crate) fn settle_product_finding_repair_value(
        &self,
        plan_id: &str,
        invalidation_id: &str,
        summary: &str,
        files: &[String],
        commands: &[String],
        tests: &[String],
    ) -> Result<Value> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository
            .active_feature_run_for_plan(&project.id, plan_id)?
            .ok_or_else(|| anyhow!("product_finding_repair_run_missing:{plan_id}"))?;
        let plan = self.get_plan(plan_id)?;
        if let Some(existing) = repository.product_repair_settlement(invalidation_id)? {
            if existing.run_id != persisted.run.id || existing.responsible_maker_id != worker_id() {
                bail!("product_finding_repair_owner_or_run_mismatch:{invalidation_id}");
            }
            if let Some(gate) = repository
                .review_gates_for_run(&persisted.run.id, false)?
                .into_iter()
                .rev()
                .find(|gate| {
                    gate.kind == ReviewGateKind::RiskCheckpoint
                        && gate.status == ReviewGateStatus::Pending
                })
                && repository
                    .review_source_binding(&gate.id)?
                    .is_some_and(|binding| binding.freeze_id == existing.source_freeze_id)
            {
                return Ok(json!({
                    "created": false,
                    "work_packet": {
                        "kind": "review_gate",
                        "gate_id": gate.id,
                        "repair_id": invalidation_id,
                        "responsible_maker_id": worker_id(),
                        "execution_state": self.canonical_execution_state_value(
                            &persisted.run.id,
                            Some(&gate.id),
                        )?,
                    },
                }));
            }
            let source_freeze = repository
                .active_source_freeze(&existing.run_id)?
                .unwrap_or(repository.source_freeze(&existing.source_freeze_id)?);
            let verifier_worker_id = owner_for_role(&persisted.run, RunRole::Verifier)
                .map(|owner| owner.worker_id.as_str());
            let verification_item_id =
                self.verification_item_for_plan_path(Some(plan.path.as_str()), verifier_worker_id)?;
            let mut handoff = self.canonical_verification_handoff_value(
                plan_id,
                verification_item_id,
                serde_json::to_value(source_freeze)?,
            )?;
            handoff["work_packet"]["mode"] = json!("selective_replay");
            handoff["work_packet"]["repair_id"] = json!(invalidation_id);
            handoff["work_packet"]["responsible_maker_id"] = json!(worker_id());
            handoff["work_packet"]["selective_replay_obligation_ids"] =
                json!(existing.selective_obligation_ids);
            handoff["created"] = json!(false);
            return Ok(handoff);
        }
        if persisted.run.phase != FeatureRunPhase::Implementation {
            bail!("product_finding_repair_requires_implementation:{invalidation_id}");
        }
        let maker = owner_for_role(&persisted.run, RunRole::Maker)
            .ok_or_else(|| anyhow!("feature_run_missing_maker:{}", persisted.run.id))?;
        if maker.worker_id != worker_id() {
            bail!("product_finding_repair_maker_mismatch:{invalidation_id}");
        }
        let invalidation = repository
            .invalidations(&persisted.run.id)?
            .into_iter()
            .find(|value| value.id == invalidation_id)
            .ok_or_else(|| {
                anyhow!("product_finding_repair_invalidation_missing:{invalidation_id}")
            })?;
        let verification_item_id =
            self.ready_verification_item_for_plan_path(Some(plan.path.as_str()))?;
        let selective_obligation_ids =
            self.product_repair_obligation_ids(&invalidation.affected_evidence_ids)?;
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("capturing repaired canonical source freeze: {error}"))?;
        let freeze = SourceFreezeRecord {
            id: short_id("freeze"),
            run_id: persisted.run.id.clone(),
            source_revision: snapshot.source.revision.clone(),
            source_digest: snapshot.source.tree_digest.as_str().to_string(),
            status: SourceFreezeStatus::Active,
        };
        let settlement_value = json!({
            "summary": summary,
            "files": files,
            "commands": commands,
            "tests": tests,
        });
        let accepted_material_gate = repository
            .review_gates_for_run(&persisted.run.id, false)?
            .into_iter()
            .rev()
            .find(|gate| {
                gate.kind == ReviewGateKind::RiskCheckpoint
                    && gate.status == ReviewGateStatus::Accepted
            });
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT settle_product_finding_repair")?;
        let result = (|| -> Result<()> {
            if repository
                .product_repair_settlement(invalidation_id)?
                .is_some()
            {
                bail!("product_finding_repair_concurrent_settlement:{invalidation_id}");
            }
            self.reconcile_active_phase_wall(&persisted.run.id, BudgetPhase::Repair)?;
            if let Some(batch_id) = persisted.run.active_batch_id.as_deref() {
                let batch = repository.batch(batch_id)?;
                let mut ended = batch.batch;
                ended.status = ExecutionBatchStatus::Ended;
                repository.save_batch(&ended, batch.revision)?;
            }
            let frozen = apply_phase_transition(
                &persisted.run,
                &PhaseTransition {
                    to: FeatureRunPhase::SourceFrozen,
                    cause: PhaseTransitionCause::ImplementationSettled,
                    reference: format!("product_repair:{invalidation_id}"),
                    owner: None,
                },
            )
            .map_err(|violation| anyhow!("product_finding_repair_refreeze:{violation:?}"))?;
            repository.save_feature_run(&frozen, persisted.revision)?;
            repository.freeze_source(&freeze)?;
            if let Some(verification_item_id) = verification_item_id.as_deref() {
                let released = self.conn.execute(
                    "UPDATE items SET status = 'ready', worker_id = NULL, pick_token = NULL,
                         picked_at = NULL, last_heartbeat_at = NULL, updated_at = datetime('now')
                     WHERE id = ?1 AND work_type = 'verification'
                       AND status IN ('ready','picked','running')",
                    [verification_item_id],
                )?;
                if released != 1 {
                    bail!("product_finding_repair_verifier_release_failed:{verification_item_id}");
                }
            }
            repository.record_product_repair_settlement(&ProductRepairSettlementRecord {
                invalidation_id: invalidation_id.to_string(),
                run_id: persisted.run.id.clone(),
                responsible_maker_id: worker_id(),
                selective_obligation_ids: selective_obligation_ids.clone(),
                settlement: settlement_value.clone(),
                source_freeze_id: freeze.id.clone(),
            })?;
            if let Some(gate) = accepted_material_gate.as_ref() {
                let binding = super::repository::execution_run::ReviewSourceBindingRecord {
                    gate_id: gate.id.clone(),
                    freeze_id: freeze.id.clone(),
                    source_revision: freeze.source_revision.clone(),
                    source_digest: freeze.source_digest.clone(),
                    receipt_lineage: json!({
                        "kind": "product_repair",
                        "repair_id": invalidation_id,
                        "selective_obligation_ids": selective_obligation_ids,
                        "settlement": settlement_value,
                    }),
                };
                repository.reopen_review_gate_with_source_binding(&binding)?;
                let maker_generation = self.conn.query_row(
                    "SELECT COALESCE(MAX(lease_generation), 0) + 1 FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'maker'",
                    [&persisted.run.id],
                    |row| row.get::<_, u64>(0),
                )?;
                let mut review_pending = apply_phase_transition(
                    &frozen,
                    &PhaseTransition {
                        to: FeatureRunPhase::Implementation,
                        cause: PhaseTransitionCause::SourceInvalidated,
                        reference: format!("review_gate:{}", gate.id),
                        owner: Some(RoleOwner {
                            role: RunRole::Maker,
                            worker_id: worker_id(),
                            lease_generation: maker_generation,
                        }),
                    },
                )
                .map_err(|violation| anyhow!("product_repair_review_reopen:{violation:?}"))?;
                let review_batch = ExecutionBatch {
                    id: short_id("batch"),
                    run_id: persisted.run.id.clone(),
                    maker_worker_id: worker_id(),
                    status: ExecutionBatchStatus::Active,
                    settled_outcome_ids: Vec::new(),
                    replacement: None,
                };
                review_pending.active_batch_id = Some(review_batch.id.clone());
                review_pending.batch_outcome_count = 0;
                repository.save_feature_run_with_new_batch(
                    &review_pending,
                    persisted.revision + 1,
                    &review_batch,
                )?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => self
                .conn
                .execute_batch("RELEASE settle_product_finding_repair; COMMIT")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK TO settle_product_finding_repair; RELEASE settle_product_finding_repair; ROLLBACK");
                if error
                    .to_string()
                    .starts_with("product_finding_repair_concurrent_settlement:")
                {
                    return self.settle_product_finding_repair_value(
                        plan_id,
                        invalidation_id,
                        summary,
                        files,
                        commands,
                        tests,
                    );
                }
                return Err(error);
            }
        }
        if let Some(gate) = accepted_material_gate {
            return Ok(json!({
                "created": true,
                "work_packet": {
                    "kind": "review_gate",
                    "gate_id": gate.id,
                    "repair_id": invalidation_id,
                    "responsible_maker_id": worker_id(),
                    "execution_state": self.canonical_execution_state_value(
                        &persisted.run.id,
                        Some(&gate.id),
                    )?,
                },
            }));
        }
        let mut handoff = self.canonical_verification_handoff_value(
            plan_id,
            verification_item_id,
            serde_json::to_value(&freeze)?,
        )?;
        handoff["work_packet"]["mode"] = json!("selective_replay");
        handoff["work_packet"]["repair_id"] = json!(invalidation_id);
        handoff["work_packet"]["responsible_maker_id"] = json!(worker_id());
        handoff["work_packet"]["selective_replay_obligation_ids"] = json!(selective_obligation_ids);
        handoff["created"] = json!(true);
        Ok(handoff)
    }

    pub(crate) fn verification_work_packet_value(
        &self,
        plan_id: &str,
        peek: bool,
    ) -> Result<Option<Value>> {
        let project = self.default_project()?;
        let plan = self.get_plan(plan_id)?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let Some(run) = repository.active_feature_run_for_plan(&project.id, plan_id)? else {
            return Ok(None);
        };
        if let Some(hold) = self.premature_source_freeze_restart_hold_for_run(&run)? {
            return Ok(Some(hold));
        }
        let current_verification = self.current_verification_diagnosis(&run)?;
        if let Some(snapshot) = current_verification.as_ref()
            && let Some(hold) =
                self.inconsistent_verification_restart_hold_for_run(&run, &snapshot.diagnosis)?
        {
            return Ok(Some(hold));
        }
        let verification_item_id = match current_verification.as_ref() {
            Some(snapshot) => snapshot
                .diagnosis
                .facts
                .verification_item
                .as_ref()
                .map(|item| item.id.clone()),
            None => self.ready_or_owned_verification_item_for_plan_path(
                Some(plan.path.as_str()),
                worker_id().as_str(),
            )?,
        };
        if run.run.phase == FeatureRunPhase::SourceFrozen {
            let freeze = repository
                .active_source_freeze(&run.run.id)?
                .ok_or_else(|| anyhow!("source_frozen_run_missing_freeze:{}", run.run.id))?;
            let snapshot = capture_repository_snapshot(&self.root)
                .map_err(|error| anyhow!("checking canonical source freeze: {error}"))?;
            if snapshot.source.revision != freeze.source_revision
                || snapshot.source.tree_digest.as_str() != freeze.source_digest
            {
                bail!("source_freeze_stale:{}", freeze.id);
            }
            if self.plan_evidence_authority(plan_id)? == PlanEvidenceAuthority::BindingActive
                && matches!(
                    self.current_plan_coverage_for_source_freeze(&project.id, plan_id, &freeze,)?,
                    CurrentPlanCoverageForSourceFreeze::Satisfied(_)
                )
            {
                return Ok(None);
            }
            let repair =
                repository.product_repair_settlement_for_source_freeze(&run.run.id, &freeze.id)?;
            let verifier_worker_id = worker_id();
            let maker_worker_id = self.conn.query_row(
                "SELECT worker_id FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'maker' ORDER BY lease_generation DESC LIMIT 1",
                [&run.run.id], |row| row.get::<_, String>(0))?;
            if !peek && verifier_worker_id == maker_worker_id {
                bail!(
                    "verification_requires_fresh_independent_worker:{}",
                    run.run.id
                );
            }
            let lease_generation = self.conn.query_row(
                "SELECT COALESCE(MAX(lease_generation), 0) + 1 FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'verifier'",
                [&run.run.id], |row| row.get::<_, u64>(0))?;
            if peek {
                let mut packet = json!({"kind": "verification", "execution_state": self.canonical_execution_state_value(&run.run.id, None)?,
                    "item_id": verification_item_id, "source_freeze": freeze, "verification_lease": {"worker_id": verifier_worker_id, "generation": lease_generation}});
                add_selective_replay_metadata(&mut packet, repair.as_ref());
                self.add_review_finding_reverification_metadata(&mut packet, &run.run.id, plan_id)?;
                return Ok(Some(json!({"work_packet": packet,
                    "peek": true, "remaining": self.progress_value()?})));
            }
            let verification = apply_phase_transition(
                &run.run,
                &PhaseTransition {
                    to: FeatureRunPhase::Verification,
                    cause: PhaseTransitionCause::VerificationStarted,
                    reference: format!("source_freeze:{}", freeze.id),
                    owner: Some(RoleOwner {
                        role: RunRole::Verifier,
                        worker_id: verifier_worker_id.clone(),
                        lease_generation,
                    }),
                },
            )
            .map_err(|violation| anyhow!("verification_lease_transition:{violation:?}"))?;
            self.conn
                .execute_batch("BEGIN IMMEDIATE; SAVEPOINT verification_pick")?;
            let mut sealed_run_index = None;
            let mut budget_hold = None;
            let mut preseal_failure = None;
            let pick_result = (|| -> Result<()> {
                repository.save_feature_run(&verification, run.revision)?;
                let verification = repository.feature_run(&run.run.id)?;
                match self.admit_feature_run_budget(
                    &verification,
                    BudgetPhase::Verification,
                    &format!("verification:{}", run.run.id),
                    &verifier_worker_id,
                    "verification.dispatch",
                )? {
                    FeatureRunBudgetAdmission::Held(hold) => {
                        budget_hold = Some(hold);
                        return Ok(());
                    }
                    FeatureRunBudgetAdmission::Reserved(_) => {}
                }
                if let Some(item_id) = verification_item_id.as_deref() {
                    self.lease_verification_item(item_id, &verifier_worker_id)?;
                }
                let readiness =
                    match self.evidence_readiness_value(EvidenceCoverageScope::Plan, plan_id) {
                        Ok(readiness) => readiness,
                        Err(error) => {
                            preseal_failure = Some((
                                VerificationAdmissionRepairReason::RunIndexSealFailed,
                                json!({"message": error.to_string()}),
                            ));
                            return Err(error);
                        }
                    };
                if readiness["status"] != "passed" {
                    preseal_failure = Some((
                        VerificationAdmissionRepairReason::ReadinessBlocked,
                        readiness["gaps"].clone(),
                    ));
                    return Err(VerificationPickReadinessError::from_readiness(
                        plan_id, &readiness,
                    )
                    .into());
                }
                sealed_run_index = readiness.get("run_index").cloned();
                let Some(sealed) = sealed_run_index.as_ref() else {
                    let error = anyhow!("verification_pick_missing_sealed_run_index:{plan_id}");
                    preseal_failure = Some((
                        VerificationAdmissionRepairReason::RunIndexSealFailed,
                        json!({"message": error.to_string()}),
                    ));
                    return Err(error);
                };
                let Some(run_index_digest) = sealed["run_index_digest"].as_str() else {
                    let error = anyhow!("verification_pick_missing_run_index_digest:{plan_id}");
                    preseal_failure = Some((
                        VerificationAdmissionRepairReason::RunIndexSealFailed,
                        json!({"message": error.to_string()}),
                    ));
                    return Err(error);
                };
                let admitted = repository.feature_run(&run.run.id)?;
                repository.record_verification_admission(&VerificationAdmissionRecord {
                    plan_id: plan_id.to_string(),
                    run_id: run.run.id.clone(),
                    freeze_id: freeze.id.clone(),
                    run_revision: admitted.revision,
                    verifier_worker_id: verifier_worker_id.clone(),
                    verifier_lease_generation: lease_generation,
                    verification_item_id: verification_item_id.clone(),
                    run_index_digest: run_index_digest.to_string(),
                    sealed_run_index: sealed.clone(),
                })?;
                Ok(())
            })();
            if let Err(error) = pick_result {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO verification_pick; RELEASE verification_pick; ROLLBACK",
                );
                if let Some((reason, diagnostic)) = preseal_failure {
                    return Err(self
                        .persist_verification_pick_readiness_failure(
                            plan_id,
                            &freeze.id,
                            &verifier_worker_id,
                            reason,
                            diagnostic,
                        )?
                        .into());
                }
                return Err(error);
            }
            self.conn
                .execute_batch("RELEASE verification_pick; COMMIT")?;
            if let Some(hold) = budget_hold {
                return Ok(Some(hold));
            }
            let admitted = repository
                .latest_verification_admission(&run.run.id, &freeze.id)?
                .ok_or_else(|| {
                    anyhow!("verification_pick_admission_record_missing:{}", run.run.id)
                })?;
            let mut packet = json!({"kind": "verification", "execution_state": self.canonical_execution_state_value(&verification.id, None)?,
                "item_id": verification_item_id, "source_freeze": freeze, "verification_lease": {"worker_id": verifier_worker_id, "generation": lease_generation}, "sealed_run_index": sealed_run_index});
            packet["verification_admission"] = serde_json::to_value(admitted)?;
            add_selective_replay_metadata(&mut packet, repair.as_ref());
            self.add_review_finding_reverification_metadata(&mut packet, &run.run.id, plan_id)?;
            return Ok(Some(json!({"work_packet": packet,
                "peek": false, "remaining": self.progress_value()?})));
        }
        if run.run.phase != FeatureRunPhase::Verification {
            return Ok(None);
        }
        let verifier = run
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Verifier)
            .ok_or_else(|| anyhow!("verification_run_missing_verifier:{}", run.run.id))?;
        if !peek && verifier.worker_id != worker_id() {
            bail!("verification_lease_owned_by:{}", verifier.worker_id);
        }
        if !peek {
            match self.admit_feature_run_budget(
                &run,
                BudgetPhase::Verification,
                &format!("verification:{}", run.run.id),
                &worker_id(),
                "verification.dispatch",
            )? {
                FeatureRunBudgetAdmission::Held(hold) => return Ok(Some(hold)),
                FeatureRunBudgetAdmission::Reserved(_) => {}
            }
        }
        let source_freeze = repository
            .active_source_freeze(&run.run.id)?
            .ok_or_else(|| anyhow!("verification_run_missing_freeze:{}", run.run.id))?;
        let admission = current_verification
            .as_ref()
            .and_then(|snapshot| snapshot.admission.clone())
            .ok_or_else(|| {
                anyhow!(
                    "current_verification_diagnosis_missing_admission:{}",
                    run.run.id
                )
            })?;
        let sealed_run_index = admission.sealed_run_index.clone();
        let repair = repository
            .product_repair_settlement_for_source_freeze(&run.run.id, &source_freeze.id)?;
        let mut packet = json!({"kind": "verification", "execution_state": self.canonical_execution_state_value(&run.run.id, None)?,
            "item_id": verification_item_id, "verifier_worker_id": verifier.worker_id, "verification_lease": {"worker_id": verifier.worker_id, "generation": verifier.lease_generation},
            "source_freeze": source_freeze, "sealed_run_index": sealed_run_index,
            "verification_admission": admission});
        add_selective_replay_metadata(&mut packet, repair.as_ref());
        self.add_review_finding_reverification_metadata(&mut packet, &run.run.id, plan_id)?;
        Ok(Some(
            json!({"work_packet": packet, "peek": peek, "remaining": self.progress_value()?}),
        ))
    }

    pub(crate) fn ready_verification_item_for_plan_path(
        &self,
        plan_path: Option<&str>,
    ) -> Result<Option<String>> {
        self.verification_item_for_plan_path(plan_path, None)
    }

    fn ready_or_owned_verification_item_for_plan_path(
        &self,
        plan_path: Option<&str>,
        worker_id: &str,
    ) -> Result<Option<String>> {
        self.verification_item_for_plan_path(plan_path, Some(worker_id))
    }

    fn verification_item_for_plan_path(
        &self,
        plan_path: Option<&str>,
        worker_id: Option<&str>,
    ) -> Result<Option<String>> {
        let Some(plan_path) = plan_path else {
            return Ok(None);
        };
        self.conn
            .query_row(
                "SELECT id FROM items
                 WHERE plan_path = ?1
                   AND work_type = 'verification'
                   AND (
                     status = 'ready'
                     OR (?2 IS NOT NULL AND status IN ('picked','running') AND worker_id = ?2)
                   )
                 ORDER BY CASE WHEN ?2 IS NOT NULL AND worker_id = ?2 THEN 0 ELSE 1 END, priority DESC, created_at
                 LIMIT 1",
                params![plan_path, worker_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    fn lease_verification_item(&self, item_id: &str, verifier_worker_id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE items
             SET status = 'picked', worker_id = ?2, picked_at = datetime('now'),
                 last_heartbeat_at = datetime('now'), updated_at = datetime('now')
             WHERE id = ?1
               AND work_type = 'verification'
               AND (
                 status = 'ready'
                 OR (status IN ('picked','running') AND worker_id = ?2)
               )",
            params![item_id, verifier_worker_id],
        )?;
        if changed != 1 {
            bail!("verification_item_not_leasable:{item_id}");
        }
        Ok(())
    }

    pub(crate) fn refresh_nonbinding_final_review_source_freeze(
        &self,
        plan_id: &str,
        run_id: &str,
    ) -> Result<bool> {
        let owns_transaction = self.conn.is_autocommit();
        if owns_transaction {
            self.conn.execute_batch(
                "BEGIN IMMEDIATE; SAVEPOINT refresh_nonbinding_final_review_source_freeze",
            )?;
        }
        let result = (|| -> Result<Option<(EvidenceInvalidationRecord, SourceFreezeRecord)>> {
            if self.plan_evidence_authority(plan_id)?
                != super::proof::PlanEvidenceAuthority::NonBinding
            {
                bail!("nonbinding_final_review_refresh_evidence_authority_changed:{plan_id}");
            }
            let snapshot = capture_repository_snapshot(&self.root)
                .map_err(|error| anyhow!("capturing nonbinding final-review source: {error}"))?;
            let repository = ExecutionRunRepository::new(&self.conn);
            let active = repository.active_source_freeze(run_id)?.ok_or_else(|| {
                anyhow!("nonbinding_final_review_source_freeze_missing:{plan_id}")
            })?;
            if active.source_revision == snapshot.source.revision
                && active.source_digest == snapshot.source.tree_digest.as_str()
            {
                return Ok(None);
            }
            let invalidation = EvidenceInvalidationRecord {
                id: short_id("invalidation"),
                run_id: run_id.to_string(),
                freeze_id: active.id,
                finding_id: None,
                reason: "nonbinding_final_review_source_changed".to_string(),
                affected_evidence_ids: Vec::new(),
            };
            let replacement = SourceFreezeRecord {
                id: short_id("freeze"),
                run_id: run_id.to_string(),
                source_revision: snapshot.source.revision.clone(),
                source_digest: snapshot.source.tree_digest.as_str().to_string(),
                status: SourceFreezeStatus::Active,
            };
            repository.invalidate_source(&invalidation)?;
            repository.freeze_source(&replacement)?;
            let persisted = repository.feature_run(run_id)?;
            let mut refreshed = persisted.run;
            refreshed.source_revision = Some(replacement.source_revision.clone());
            repository.save_feature_run(&refreshed, persisted.revision)?;
            self.record_event(
                "nonbinding_final_review_source_refrozen",
                None,
                json!({
                    "plan_id": plan_id,
                    "run_id": run_id,
                    "invalidation_id": invalidation.id,
                    "invalidated_source_freeze_id": invalidation.freeze_id,
                    "source_freeze_id": replacement.id,
                }),
            )?;
            Ok(Some((invalidation, replacement)))
        })();
        if !owns_transaction {
            return result.map(|value| value.is_some());
        }
        match result {
            Ok(value) => {
                self.conn.execute_batch(
                    "RELEASE refresh_nonbinding_final_review_source_freeze; COMMIT",
                )?;
                Ok(value.is_some())
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO refresh_nonbinding_final_review_source_freeze; RELEASE refresh_nonbinding_final_review_source_freeze; ROLLBACK",
                );
                Err(error)
            }
        }
    }

    pub(crate) fn freeze_feature_run_source_value(&self, plan_id: &str) -> Result<Option<Value>> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let Some(persisted) = repository.active_feature_run_for_plan(&project.id, plan_id)? else {
            return Ok(None);
        };
        if persisted.run.phase == FeatureRunPhase::Held
            && persisted.run.held_from_phase == Some(FeatureRunPhase::SourceFrozen)
        {
            let active = repository
                .active_source_freeze(&persisted.run.id)?
                .ok_or_else(|| {
                    anyhow!("held_source_frozen_run_missing_freeze:{}", persisted.run.id)
                })?;
            let snapshot = capture_repository_snapshot(&self.root)
                .map_err(|error| anyhow!("capturing repaired canonical source freeze: {error}"))?;
            if active.source_digest == snapshot.source.tree_digest.as_str() {
                return Ok(Some(json!({
                    "feature_run": persisted.run,
                    "source_freeze": active,
                    "created": false,
                })));
            }
            let replacement = SourceFreezeRecord {
                id: short_id("freeze"),
                run_id: persisted.run.id.clone(),
                source_revision: snapshot.source.revision.clone(),
                source_digest: snapshot.source.tree_digest.as_str().to_string(),
                status: SourceFreezeStatus::Active,
            };
            let invalidation = EvidenceInvalidationRecord {
                id: short_id("invalidation"),
                run_id: persisted.run.id.clone(),
                freeze_id: active.id.clone(),
                finding_id: None,
                reason: "source_changed_during_evidence_readiness_repair".to_string(),
                affected_evidence_ids: Vec::new(),
            };
            self.conn
                .execute_batch("BEGIN IMMEDIATE; SAVEPOINT refreeze_repaired_feature_run_source")?;
            let result = (|| -> Result<Value> {
                repository.invalidate_source(&invalidation)?;
                repository.freeze_source(&replacement)?;
                Ok(json!({
                    "feature_run": persisted.run,
                    "source_freeze": replacement,
                    "invalidated_source_freeze": active,
                    "invalidation": invalidation,
                    "created": true,
                }))
            })();
            match result {
                Ok(value) => {
                    self.conn
                        .execute_batch("RELEASE refreeze_repaired_feature_run_source; COMMIT")?;
                    return Ok(Some(value));
                }
                Err(error) => {
                    let _ = self.conn.execute_batch(
                        "ROLLBACK TO refreeze_repaired_feature_run_source; RELEASE refreeze_repaired_feature_run_source; ROLLBACK",
                    );
                    return Err(error);
                }
            }
        }
        if persisted.run.phase == FeatureRunPhase::SourceFrozen {
            let freeze = repository
                .active_source_freeze(&persisted.run.id)?
                .ok_or_else(|| anyhow!("source_frozen_run_missing_freeze:{}", persisted.run.id))?;
            return Ok(Some(json!({
                "feature_run": persisted.run,
                "source_freeze": freeze,
                "created": false,
            })));
        }
        if persisted.run.phase != FeatureRunPhase::Implementation {
            return Ok(None);
        }
        let open_ordinary_outcome_ids =
            repository.open_ordinary_outcome_ids(&persisted.run.plan_id)?;
        if !open_ordinary_outcome_ids.is_empty() {
            bail!(
                "feature_run_source_freeze_open_ordinary_outcomes:{}:{}",
                plan_id,
                open_ordinary_outcome_ids.join(",")
            );
        }
        let repair_refreeze = !repository.invalidations(&persisted.run.id)?.is_empty();
        if !repair_refreeze
            && let Some(hold) =
                self.feature_run_budget_hold(&persisted, BudgetPhase::Implementation)?
        {
            return Ok(Some(hold));
        }
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("capturing canonical source freeze: {error}"))?;
        let freeze = SourceFreezeRecord {
            id: short_id("freeze"),
            run_id: persisted.run.id.clone(),
            source_revision: snapshot.source.revision.clone(),
            source_digest: snapshot.source.tree_digest.as_str().to_string(),
            status: SourceFreezeStatus::Active,
        };
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT freeze_feature_run_source")?;
        let result = (|| -> Result<Value> {
            self.reconcile_active_phase_wall(&persisted.run.id, BudgetPhase::Implementation)?;
            if let Some(batch_id) = persisted.run.active_batch_id.as_deref() {
                let batch = repository.batch(batch_id)?;
                let mut ended = batch.batch;
                ended.status = ExecutionBatchStatus::Ended;
                repository.save_batch(&ended, batch.revision)?;
            }
            let frozen = apply_phase_transition(
                &persisted.run,
                &PhaseTransition {
                    to: FeatureRunPhase::SourceFrozen,
                    cause: PhaseTransitionCause::ImplementationSettled,
                    reference: freeze.source_revision.clone(),
                    owner: None,
                },
            )
            .map_err(|violation| anyhow!("source_freeze_transition:{violation:?}"))?;
            repository.save_feature_run(&frozen, persisted.revision)?;
            repository.freeze_source(&freeze)?;
            Ok(json!({"feature_run": frozen, "source_freeze": freeze, "created": true}))
        })();
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE freeze_feature_run_source; COMMIT")?;
                Ok(Some(value))
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK TO freeze_feature_run_source; RELEASE freeze_feature_run_source; ROLLBACK");
                Err(error)
            }
        }
    }

    pub(crate) fn classify_feature_run_readiness_value(
        &self,
        plan_id: &str,
        blocked: bool,
    ) -> Result<Option<Value>> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let Some(persisted) = repository.active_feature_run_for_plan(&project.id, plan_id)? else {
            return Ok(None);
        };
        if blocked
            && persisted.run.phase == FeatureRunPhase::Held
            && persisted.run.held_from_phase == Some(FeatureRunPhase::SourceFrozen)
            && persisted.run.hold_reason != Some(FeatureRunHoldReason::Capability)
        {
            let mut corrected = persisted.run.clone();
            corrected.hold_reason = Some(FeatureRunHoldReason::Capability);
            repository.save_feature_run(&corrected, persisted.revision)?;
            return Ok(Some(json!({
                "classification": "capability",
                "reason": "evidence_readiness_blocked",
                "feature_run": corrected,
            })));
        }
        let transition = if blocked && persisted.run.phase == FeatureRunPhase::SourceFrozen {
            Some(PhaseTransition {
                to: FeatureRunPhase::Held,
                cause: PhaseTransitionCause::CapabilityHold,
                reference: "evidence_readiness:capability_gap".to_string(),
                owner: None,
            })
        } else if !blocked
            && persisted.run.phase == FeatureRunPhase::Held
            && persisted.run.held_from_phase == Some(FeatureRunPhase::SourceFrozen)
        {
            Some(PhaseTransition {
                to: FeatureRunPhase::SourceFrozen,
                cause: PhaseTransitionCause::HoldResolved,
                reference: "evidence_readiness:capabilities_ready".to_string(),
                owner: None,
            })
        } else {
            None
        };
        let Some(transition) = transition else {
            return Ok(None);
        };
        let next = apply_phase_transition(&persisted.run, &transition)
            .map_err(|violation| anyhow!("readiness_hold_transition:{violation:?}"))?;
        repository.save_feature_run(&next, persisted.revision)?;
        Ok(Some(json!({
            "classification": if blocked { "capability" } else { "resolved" },
            "reason": if blocked { "evidence_readiness_blocked" } else { "capability_hold_resolved" },
            "feature_run": next,
        })))
    }

    pub(crate) fn route_evidence_product_finding_value(
        &self,
        run_id: &str,
        freeze_id: &str,
        obligation_id: &str,
    ) -> Result<Value> {
        self.route_evidence_product_findings_value(run_id, freeze_id, &[obligation_id.to_string()])
    }

    pub(crate) fn route_evidence_product_findings_value(
        &self,
        run_id: &str,
        freeze_id: &str,
        obligation_ids: &[String],
    ) -> Result<Value> {
        let obligation_ids = obligation_ids
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let first_obligation_id = obligation_ids
            .first()
            .ok_or_else(|| anyhow!("product_finding_requires_obligation:{run_id}"))?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository.feature_run(run_id)?;
        if persisted.run.phase != FeatureRunPhase::Verification {
            bail!("product_finding_requires_verification:{run_id}");
        }
        let verifier_worker_id = owner_for_role(&persisted.run, RunRole::Verifier)
            .ok_or_else(|| anyhow!("product_finding_missing_verifier:{run_id}"))?
            .worker_id
            .clone();
        let plan = self.get_plan(&persisted.run.plan_id)?;
        let verification_item_id = self.ready_or_owned_verification_item_for_plan_path(
            Some(plan.path.as_str()),
            &verifier_worker_id,
        )?;
        let maker_worker_id = self.conn.query_row(
            "SELECT worker_id FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'maker' ORDER BY lease_generation DESC LIMIT 1",
            [run_id],
            |row| row.get::<_, String>(0),
        )?;
        let mut affected_evidence_ids = obligation_ids.clone();
        let mut receipt_statement = self.conn.prepare(
            "SELECT id FROM evidence_receipts WHERE project_id = ?1 AND obligation_id = ?2 AND receipt_status = 'trusted' ORDER BY created_at, id",
        )?;
        for obligation_id in &obligation_ids {
            let receipt_rows = receipt_statement.query_map(
                rusqlite::params![persisted.project_id, obligation_id],
                |row| row.get::<_, String>(0),
            )?;
            affected_evidence_ids.extend(crate::util::collect_rows(receipt_rows)?);
        }
        drop(receipt_statement);
        let invalidation = EvidenceInvalidationRecord {
            id: short_id("invalidation"),
            run_id: run_id.to_string(),
            freeze_id: freeze_id.to_string(),
            finding_id: None,
            reason: "product_finding".to_string(),
            affected_evidence_ids,
        };
        let generation = self.conn.query_row(
            "SELECT COALESCE(MAX(lease_generation), 0) + 1 FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'maker'",
            [run_id],
            |row| row.get::<_, u64>(0),
        )?;
        let mut repair = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::Implementation,
                cause: PhaseTransitionCause::ProductFinding,
                reference: format!("evidence:{first_obligation_id}"),
                owner: Some(RoleOwner {
                    role: RunRole::Maker,
                    worker_id: maker_worker_id.clone(),
                    lease_generation: generation,
                }),
            },
        )
        .map_err(|violation| anyhow!("evidence_product_finding_transition:{violation:?}"))?;
        let batch = ExecutionBatch {
            id: short_id("batch"),
            run_id: run_id.to_string(),
            maker_worker_id: maker_worker_id.clone(),
            status: ExecutionBatchStatus::Active,
            settled_outcome_ids: Vec::new(),
            replacement: None,
        };
        repair.active_batch_id = Some(batch.id.clone());
        repair.batch_outcome_count = 0;
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT route_evidence_product_finding")?;
        let result = (|| -> Result<Value> {
            self.reconcile_active_phase_wall(run_id, BudgetPhase::Verification)?;
            repository.invalidate_source(&invalidation)?;
            repository.save_feature_run_with_new_batch(&repair, persisted.revision, &batch)?;
            if let Some(verification_item_id) = verification_item_id.as_deref() {
                let released = self.conn.execute(
                    "UPDATE items SET status = 'ready', worker_id = NULL, pick_token = NULL,
                         picked_at = NULL, last_heartbeat_at = NULL, updated_at = datetime('now')
                     WHERE id = ?1 AND work_type = 'verification'
                       AND (status = 'ready'
                            OR (status IN ('picked','running') AND worker_id = ?2))",
                    params![verification_item_id, verifier_worker_id],
                )?;
                if released != 1 {
                    bail!("product_finding_verifier_release_failed:{verification_item_id}");
                }
            }
            Ok(json!({
                "classification": "product_finding",
                "responsible_maker_id": maker_worker_id,
                "repair_id": invalidation.id,
                "verification_item_id": verification_item_id,
                "invalidation": invalidation,
                "selective_replay_obligation_ids": obligation_ids,
                "next": "maker_repair_then_plan_readiness_refreeze",
            }))
        })();
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE route_evidence_product_finding; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK TO route_evidence_product_finding; RELEASE route_evidence_product_finding; ROLLBACK");
                Err(error)
            }
        }
    }

    pub(crate) fn feature_run_budget_hold(
        &self,
        persisted: &PersistedFeatureRun,
        phase: BudgetPhase,
    ) -> Result<Option<Value>> {
        let now_unix_ms = unix_time_ms()?;
        let snapshot = match self.persisted_budget_snapshot(
            persisted,
            feature_run_budget_phase(phase),
            now_unix_ms,
        ) {
            Ok((_, snapshot)) => snapshot,
            Err(error) => {
                return self
                    .persist_budget_hold(
                        persisted,
                        BudgetTaskHoldReason::InvalidBudgetState,
                        format!("cannot derive persisted budget snapshot: {error}"),
                    )
                    .map(Some);
            }
        };
        if snapshot.mode == FeatureRunBudgetMode::Bounded
            && snapshot.available.is_some_and(|available| {
                available.wall_seconds == 0 || available.tool_calls == 0 || available.tokens == 0
            })
        {
            return self
                .persist_budget_hold(
                    persisted,
                    BudgetTaskHoldReason::BudgetExhausted,
                    "no complete bounded task can be admitted from the persisted budget snapshot",
                )
                .map(Some);
        }
        Ok(None)
    }
}

pub(crate) fn unix_time_ms() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| anyhow!("system clock before unix epoch: {error}"))?
        .as_millis()
        .try_into()
        .map_err(|_| anyhow!("system clock millisecond value overflow"))
}

const fn budget_phase_owner_role(phase: BudgetPhase) -> RunRole {
    match phase {
        BudgetPhase::Implementation | BudgetPhase::Repair => RunRole::Maker,
        BudgetPhase::Verification => RunRole::Verifier,
        BudgetPhase::Review => RunRole::Reviewer,
    }
}

fn held_feature_run_budget_phase(run: &crate::execution_run::FeatureRun) -> Result<BudgetPhase> {
    match run.held_from_phase {
        Some(FeatureRunPhase::Implementation) => Ok(BudgetPhase::Implementation),
        Some(FeatureRunPhase::RiskReview | FeatureRunPhase::FinalReview) => Ok(BudgetPhase::Review),
        Some(FeatureRunPhase::SourceFrozen | FeatureRunPhase::Verification) => {
            Ok(BudgetPhase::Verification)
        }
        _ => bail!("feature_run_budget_hold_resolution_rejected:invalid_held_origin"),
    }
}

const fn budget_phase_matches_held_origin(
    phase: BudgetPhase,
    held_from_phase: Option<FeatureRunPhase>,
) -> bool {
    matches!(
        (phase, held_from_phase),
        (
            BudgetPhase::Implementation | BudgetPhase::Repair,
            Some(FeatureRunPhase::Implementation)
        ) | (
            BudgetPhase::Review,
            Some(FeatureRunPhase::RiskReview | FeatureRunPhase::FinalReview)
        ) | (
            BudgetPhase::Verification,
            Some(FeatureRunPhase::SourceFrozen | FeatureRunPhase::Verification)
        )
    )
}

const fn feature_run_budget_phase(phase: BudgetPhase) -> FeatureRunBudgetPhase {
    match phase {
        BudgetPhase::Implementation => FeatureRunBudgetPhase::Maker,
        BudgetPhase::Verification => FeatureRunBudgetPhase::Verification,
        BudgetPhase::Review => FeatureRunBudgetPhase::Review,
        BudgetPhase::Repair => FeatureRunBudgetPhase::Repair,
    }
}

fn validate_usage_report(usage: &BudgetUsageReport) -> Result<()> {
    if usage.adapter_id.trim().is_empty()
        || !reported_dimension_is_consistent(usage.tool_calls_metering, usage.tool_calls)
        || !reported_dimension_is_consistent(usage.tokens_metering, usage.tokens)
    {
        bail!("budget_usage_report_provenance_mismatch");
    }
    Ok(())
}

const fn reported_dimension_is_consistent(metering: MeteringMode, value: Option<u64>) -> bool {
    match metering {
        MeteringMode::Unavailable => value.is_none(),
        MeteringMode::Estimated | MeteringMode::Trusted => value.is_some(),
    }
}

fn reconciled_dimension(
    observations: &[&BudgetObservationRecord],
    value: fn(&BudgetObservationRecord) -> Option<u64>,
    metering: fn(&BudgetObservationRecord) -> Option<MeteringMode>,
    declared_maximum: Option<u64>,
) -> Result<(u64, MeteringMode)> {
    let mut observed = 0_u64;
    let mut effective_metering = MeteringMode::Trusted;
    for observation in observations {
        let dimension_metering = metering(observation)
            .ok_or_else(|| anyhow!("budget observation is missing dimension provenance"))?;
        effective_metering = effective_metering.min(dimension_metering);
        if let Some(amount) = value(observation) {
            observed = observed
                .checked_add(amount)
                .ok_or_else(|| anyhow!("budget observation arithmetic overflow"))?;
        }
    }
    let consumed = match (effective_metering, declared_maximum) {
        (MeteringMode::Trusted, _) | (_, None) => observed,
        (MeteringMode::Unavailable | MeteringMode::Estimated, Some(maximum)) => {
            observed.max(maximum)
        }
    };
    Ok((consumed, effective_metering))
}

fn budget_amounts_are_positive(amounts: BudgetAmounts) -> bool {
    amounts.wall_seconds > 0 && amounts.tool_calls > 0 && amounts.tokens > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::execution_run::OutcomeSettlement;
    use crate::app::repository::execution_run::{
        ReviewAttemptRecord, ReviewGateRecord, ReviewScopeKind, ReviewVerdict,
    };
    use crate::evidence::model::{
        CapabilityBinding, PermissionState, RawResultRef, Sha256Digest, TrustedProvenance,
        TrustedReceiptInput, VantagePoint, build_trusted_receipt,
    };
    use crate::execution_run::FeatureRunTerminalReason;
    use crate::storage::ensure_schema;
    use rusqlite::{Connection, params};
    use serde_json::Map;
    use std::path::{Path, PathBuf};
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    fn test_app(root: PathBuf) -> App {
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
        App::new(conn, root, PathBuf::from("planr.sqlite"), true, false)
    }

    fn add_outcome(app: &App, id: &str) {
        let plan_path = app.get_plan("plan-a").expect("plan").path;
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at) VALUES (?1, 'project-a', ?1, 'outcome', 'picked', 'code', ?2, ?3, datetime('now'), datetime('now'))",
                params![id, worker_id(), plan_path],
            )
            .expect("outcome item");
    }

    fn add_verification_outcome(app: &App, id: &str) {
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at) VALUES (?1, 'project-a', ?1, 'verification', 'ready', 'verification', 'plan-a.md', datetime('now'), datetime('now'))",
                [id],
            )
            .expect("verification item");
    }

    fn add_verification_item(app: &App, id: &str) {
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at) VALUES (?1, 'project-a', ?1, 'verification', 'ready', 'verification', 'plan-a.md', datetime('now'), datetime('now'))",
                params![id],
            )
            .expect("verification item");
    }

    fn write_budget_policy(root: &Path) {
        std::fs::create_dir_all(root.join(".planr")).unwrap();
        std::fs::write(
            root.join(".planr/policy.toml"),
            r#"schema_version = 1
id = "feature-run-budget-test"
version = "1.0.0"
[usage]
max_active_agents = 3
max_parallel_readers = 2
max_parallel_writers = 1
max_depth = 1
max_attempts = 4
max_wall_time_seconds = 100
max_tool_calls = 100
max_tokens = 1000
budget_exhaustion = "stop"
metering = "trusted"
[usage.phase_reserves]
verification_percent = 20
review_percent = 10
repair_percent = 10
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
protected_risks = ["security_or_auth"]
[execution]
max_read_scope_entries = 4
max_write_scope_entries = 2
[execution.roles.worker]
tools = ["cargo"]
commands = [{ program = "cargo", args = ["test"] }]
[execution.roles.worker.filesystem]
read_roots = ["src"]
write_roots = ["src"]
allow_overwrite = true
"#,
        )
        .unwrap();
    }

    fn initialize_git(root: &Path) {
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Planr Test",
                "-c",
                "user.email=planr@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(root)
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }

    fn write_ready_evidence_policy(root: &Path) -> String {
        write_evidence_policy(root, "repeatable", "printf '{\"status\":\"ready\"}'")
    }

    fn write_evidence_policy(root: &Path, repeatability: &str, shell_script: &str) -> String {
        write_evidence_policy_with_probe(root, repeatability, shell_script, shell_script)
    }

    fn write_evidence_policy_with_probe(
        root: &Path,
        repeatability: &str,
        shell_script: &str,
        probe_shell_script: &str,
    ) -> String {
        let schema_path =
            root.join(".planr/evidence/schemas/com.example.ready.status.v1.schema.json");
        let manifest_path = root.join(".planr/evidence/adapters/ready.manifest.json");
        std::fs::create_dir_all(schema_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "com.example.ready.status@v1",
            "type": "object",
            "required": ["status"],
            "properties": {"status": {"const": "ready"}},
            "additionalProperties": false
        });
        std::fs::write(&schema_path, serde_json::to_vec_pretty(&schema).unwrap()).unwrap();
        let schema_digest = crate::canonical_json::sha256_json_digest(&schema).unwrap();
        let payload_schema = json!({
            "type": "com.example.ready.status",
            "schema_ref": "com.example.ready.status@v1",
            "schema_digest": schema_digest
        });
        let execution = json!({
            "kind": "process",
            "executable": "sh",
            "args": ["-c", shell_script],
            "working_directory": ".",
            "timeout_ms": 5000,
            "stdout_limit_bytes": 4096,
            "stderr_limit_bytes": 4096,
            "payload_schema": payload_schema
        });
        let probe_execution = json!({
            "kind": "process",
            "executable": "sh",
            "args": ["-c", probe_shell_script],
            "working_directory": ".",
            "timeout_ms": 5000,
            "stdout_limit_bytes": 4096,
            "stderr_limit_bytes": 4096,
            "payload_schema": payload_schema
        });
        let adapter_digest = crate::canonical_json::sha256_json_digest(&json!({
            "schema_version": "planr.process_adapter.binding.v1",
            "execution_contract": probe_execution,
            "file_arguments": []
        }))
        .unwrap();
        let manifest = json!({
            "id": "vcap-example-ready-v1",
            "schema_version": "evidence.contract.v1",
            "version": "1.0.0",
            "adapter_kind": "process",
            "adapter_digest": adapter_digest,
            "supported_surfaces": ["local-process"],
            "supported_observations": [payload_schema],
            "supported_interactions": ["process"],
            "supported_artifacts": ["stdout"],
            "runtime_targets": [{"kind": "process", "id": "ready"}],
            "provenance_path": "planr_observed_execution",
            "permissions": {"network": "none", "filesystem": "read_workspace"},
            "costs": {},
            "determinism": "deterministic",
            "repeatability": repeatability,
            "independence": "repository-owned test adapter",
            "blind_spots": [],
            "availability_probe": {"kind": "process", "execution": probe_execution}
        });
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let manifest_digest = crate::canonical_json::sha256_json_digest(&manifest).unwrap();
        let mut policy = json!({
            "id": "epolicy-ready-v1",
            "schema_version": "evidence.contract.v1",
            "policy_digest": "sha256:pending",
            "defaults": {"preset_id": "ready", "binding": true, "assurance_level": "standard"},
            "named_presets": [{
                "id": "ready",
                "schema_version": "evidence.contract.v1",
                "namespace": "com.example.ready",
                "observations": [{
                    "id": "ready",
                    "type": "com.example.ready.status",
                    "subject": "ready process",
                    "expected": {"status": "ready"},
                    "target": {"kind": "process", "uri": "local://ready"}
                }]
            }],
            "observation_schema_registrations": [{
                "type": "com.example.ready.status",
                "schema_ref": "com.example.ready.status@v1",
                "schema_digest": schema_digest,
                "owning_namespace": "com.example.ready"
            }],
            "adapter_registrations": [{
                "manifest_id": "vcap-example-ready-v1",
                "manifest_path": ".planr/evidence/adapters/ready.manifest.json",
                "manifest_digest": manifest_digest,
                "observation_types": ["com.example.ready.status"],
                "payload_schemas": [payload_schema],
                "provenance_path": "planr_observed_execution",
                "execution_contract": execution
            }],
            "extension_namespaces": ["com.example.ready"],
            "trust_policy": {
                "accepted_provenance": ["planr_observed_execution"],
                "min_receipt_status": "trusted",
                "allow_user_attestation": false
            },
            "freshness_policy": {"max_age_seconds": 3600, "invalidate_on": ["source_change"]},
            "fixture_policy": {"fixtures_allowed": false, "mocks_allowed": false, "disclosure_required": true},
            "completion_policy": {
                "require_satisfied_or_waived": true,
                "allow_inconclusive_completion": false,
                "require_review_evidence": true
            },
            "layering_policy": {
                "mode": "monotonic_strengthening",
                "weakening_requires_waiver": true,
                "layers": [{
                    "scope": {"kind": "plan", "id": "plan-a"},
                    "policy_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }]
            }
        });
        let mut digest_input = policy.clone();
        digest_input
            .as_object_mut()
            .unwrap()
            .remove("policy_digest");
        let policy_digest = crate::canonical_json::sha256_json_digest(&digest_input).unwrap();
        policy["policy_digest"] = json!(policy_digest);
        std::fs::write(
            root.join(".planr/evidence.yaml"),
            serde_json::to_vec_pretty(&policy).unwrap(),
        )
        .unwrap();
        policy_digest
    }

    fn add_product_failure_evidence_adapter(root: &Path, repeatability: &str) -> String {
        let policy_path = root.join(".planr/evidence.yaml");
        let schema_path =
            root.join(".planr/evidence/schemas/com.example.product.status.v1.schema.json");
        let manifest_path = root.join(".planr/evidence/adapters/product-failure.manifest.json");
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "com.example.product.status@v1",
            "type": "object",
            "required": ["status"],
            "properties": {"status": {"const": "ready"}},
            "additionalProperties": false
        });
        std::fs::write(&schema_path, serde_json::to_vec_pretty(&schema).unwrap()).unwrap();
        let schema_digest = crate::canonical_json::sha256_json_digest(&schema).unwrap();
        let payload_schema = json!({
            "type": "com.example.product.status",
            "schema_ref": "com.example.product.status@v1",
            "schema_digest": schema_digest
        });
        let execution = json!({
            "kind": "process",
            "executable": "sh",
            "args": ["-c", "exit 77"],
            "working_directory": ".",
            "timeout_ms": 5000,
            "stdout_limit_bytes": 4096,
            "stderr_limit_bytes": 4096,
            "payload_schema": payload_schema
        });
        let probe_execution = json!({
            "kind": "process",
            "executable": "sh",
            "args": ["-c", "printf ready"],
            "working_directory": ".",
            "timeout_ms": 5000,
            "stdout_limit_bytes": 4096,
            "stderr_limit_bytes": 4096,
            "payload_schema": payload_schema
        });
        let adapter_digest = crate::canonical_json::sha256_json_digest(&json!({
            "schema_version": "planr.process_adapter.binding.v1",
            "execution_contract": probe_execution,
            "file_arguments": []
        }))
        .unwrap();
        let manifest = json!({
            "id": "vcap-example-product-failure-v1",
            "schema_version": "evidence.contract.v1",
            "version": "1.0.0",
            "adapter_kind": "process",
            "adapter_digest": adapter_digest,
            "supported_surfaces": ["local-process"],
            "supported_observations": [payload_schema],
            "supported_interactions": ["process"],
            "supported_artifacts": ["stdout"],
            "runtime_targets": [{"kind": "process", "id": "product-failure"}],
            "provenance_path": "planr_observed_execution",
            "permissions": {"network": "none", "filesystem": "read_workspace"},
            "costs": {},
            "determinism": "deterministic",
            "repeatability": repeatability,
            "independence": "repository-owned product-failure adapter",
            "blind_spots": [],
            "availability_probe": {"kind": "process", "execution": probe_execution}
        });
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let manifest_digest = crate::canonical_json::sha256_json_digest(&manifest).unwrap();
        let mut policy: Value =
            serde_json::from_slice(&std::fs::read(&policy_path).unwrap()).unwrap();
        policy["observation_schema_registrations"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "type": "com.example.product.status",
                "schema_ref": "com.example.product.status@v1",
                "schema_digest": schema_digest,
                "owning_namespace": "com.example.product"
            }));
        policy["adapter_registrations"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "manifest_id": "vcap-example-product-failure-v1",
                "manifest_path": ".planr/evidence/adapters/product-failure.manifest.json",
                "manifest_digest": manifest_digest,
                "observation_types": ["com.example.product.status"],
                "payload_schemas": [payload_schema],
                "provenance_path": "planr_observed_execution",
                "execution_contract": execution
            }));
        policy["extension_namespaces"]
            .as_array_mut()
            .unwrap()
            .push(json!("com.example.product"));
        policy.as_object_mut().unwrap().remove("policy_digest");
        let policy_digest = crate::canonical_json::sha256_json_digest(&policy).unwrap();
        policy["policy_digest"] = json!(policy_digest);
        std::fs::write(&policy_path, serde_json::to_vec_pretty(&policy).unwrap()).unwrap();
        policy_digest
    }

    fn verification_fixture(
        with_item: bool,
        initial_verifier_pick: bool,
    ) -> (tempfile::TempDir, App, String, String) {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        write_ready_evidence_policy(root.path());
        let plan_path = root.path().join("plan-a.md");
        std::fs::write(
            &plan_path,
            crate::planpack::build_plan_body("Plan", "product-plan", "phase ready"),
        )
        .unwrap();
        initialize_git(root.path());
        let app = test_app(root.path().to_path_buf());
        app.conn
            .execute(
                "UPDATE plans SET path = ?1 WHERE id = 'plan-a'",
                [plan_path.to_string_lossy().as_ref()],
            )
            .unwrap();
        add_outcome(&app, "item-phase");
        if with_item {
            add_verification_item(&app, "verification-phase");
        }
        app.conn
            .execute(
                "UPDATE items SET plan_path = ?1 WHERE id IN ('item-phase', 'verification-phase')",
                [plan_path.to_string_lossy().as_ref()],
            )
            .unwrap();
        app.evidence_migration_value(
            json!({
                "schema_version": "planr.evidence.migration.v1",
                "plan_id": "plan-a",
                "obligations": [{
                    "id": "pob-phase-ready",
                    "schema_version": "evidence.contract.v1",
                    "criterion_id": "criterion-phase-ready",
                    "plan_id": "plan-a",
                    "item_id": "item-phase",
                    "title": "ready process",
                    "binding": true,
                    "observations": [{
                        "id": "obs-phase-ready",
                        "type": "com.example.ready.status",
                        "subject": "ready process",
                        "expected": {"status": "ready"},
                        "target": {"kind": "process", "uri": "local://ready"},
                        "payload_schema": {"schema_ref": "com.example.ready.status@v1"}
                    }],
                    "fixture_policy": {"fixtures_allowed": false, "mocks_allowed": false, "disclosure_required": true},
                    "freshness_policy": {"max_age_seconds": 3600, "invalidate_on": ["source_change"]},
                    "assurance_policy": {"retry_aggregation": "all_applicable_pass"}
                }]
            }),
            true,
        )
            .unwrap();
        let run = app
            .ensure_outcome_feature_run("item-phase")
            .unwrap()
            .unwrap();
        app.conn
            .execute(
                "UPDATE items SET status = 'closed', worker_id = NULL WHERE id = 'item-phase'",
                [],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = 'maker-other' WHERE run_id = ?1 AND role = 'maker' AND released_at IS NULL",
                [&run.run.id],
            )
            .unwrap();
        let frozen = app
            .freeze_feature_run_source_value("plan-a")
            .unwrap()
            .unwrap();
        let freeze_id = frozen["source_freeze"]["id"].as_str().unwrap().to_string();
        if !initial_verifier_pick {
            return (root, app, run.run.id, freeze_id);
        }
        let packet = app
            .verification_work_packet_value("plan-a", false)
            .unwrap()
            .unwrap();
        assert_eq!(packet["work_packet"]["kind"], "verification");
        assert!(packet["work_packet"].get("mode").is_none());
        assert!(packet["work_packet"].get("repair_id").is_none());
        assert!(
            packet["work_packet"]
                .get("selective_replay_obligation_ids")
                .is_none()
        );
        (root, app, run.run.id, freeze_id)
    }

    fn one_shot_verification_fixture(
        shell_script: &str,
        database_path: Option<PathBuf>,
    ) -> (tempfile::TempDir, App, String, Value) {
        one_shot_verification_fixture_with_probe(shell_script, shell_script, database_path)
    }

    fn one_shot_verification_fixture_with_probe(
        shell_script: &str,
        probe_shell_script: &str,
        database_path: Option<PathBuf>,
    ) -> (tempfile::TempDir, App, String, Value) {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        let policy_digest = write_evidence_policy_with_probe(
            root.path(),
            "non_repeatable_one_shot",
            shell_script,
            probe_shell_script,
        );
        initialize_git(root.path());
        let app = if let Some(database_path) = database_path {
            let conn = Connection::open(&database_path).unwrap();
            conn.busy_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            ensure_schema(&conn).unwrap();
            conn.execute(
                "INSERT INTO projects(id, name, root_path, status, created_at, updated_at) VALUES ('project-a', 'Project', '.', 'active', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at) VALUES ('plan-a', 'project-a', 'build', 'plan-a.md', 'Plan', 'plan', 'ok', 'sha256:plan', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
            App::new(conn, root.path().to_path_buf(), database_path, true, false)
        } else {
            test_app(root.path().to_path_buf())
        };
        add_outcome(&app, "item-one-shot");
        add_verification_outcome(&app, "item-one-shot-verifier");
        app.conn
            .execute(
                "INSERT INTO proof_obligations(
                  id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                  binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                  assurance_policy_json, retry_aggregation, policy_digest, config_digest,
                  source_digest, supersedes_obligation_id, created_at, obligation_shape
                ) VALUES (
                  'pob-one-shot', 'project-a', 'plan-a', 'item-one-shot', 'criterion-one-shot', 1,
                  'one-shot process', 1, ?1, '{}', '{}', '{}', 'all_applicable_pass', ?2, ?2, ?2, NULL,
                  datetime('now'), 'semantic_v1'
                )",
                params![
                    json!([{
                        "id": "obs-one-shot",
                        "type": "com.example.ready.status",
                        "subject": "one-shot process",
                        "expected": {"status": "ready"},
                        "target": {"kind": "process", "uri": "local://ready"},
                        "payload_schema": {"schema_ref": "com.example.ready.status@v1"}
                    }])
                    .to_string(),
                    policy_digest,
                ],
            )
            .unwrap();
        let run = app
            .ensure_outcome_feature_run("item-one-shot")
            .unwrap()
            .unwrap();
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = 'maker-other' WHERE run_id = ?1 AND role = 'maker' AND released_at IS NULL",
                [&run.run.id],
            )
            .unwrap();
        assert!(
            app.evidence_readiness_value(EvidenceCoverageScope::Plan, "plan-a")
                .unwrap_err()
                .to_string()
                .contains("--work-type verification --json")
        );
        let packet = app
            .verification_work_packet_value("plan-a", false)
            .unwrap()
            .unwrap();
        (
            root,
            app,
            run.run.id,
            packet["work_packet"]["sealed_run_index"].clone(),
        )
    }

    #[test]
    fn source_freeze_is_canonical_and_evidence_requires_the_live_verifier() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        initialize_git(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-freeze");
        let persisted = app
            .ensure_outcome_feature_run("item-freeze")
            .expect("ensure run")
            .expect("run");
        let repository = ExecutionRunRepository::new(&app.conn);
        assert!(
            app.resolve_feature_run_evidence_lease("project-a", "plan-a")
                .unwrap_err()
                .to_string()
                .contains("binding_evidence_requires_verification")
        );
        app.freeze_feature_run_source_value("plan-a")
            .expect("freeze")
            .expect("feature run");
        let freeze = repository
            .active_source_freeze(&persisted.run.id)
            .expect("freeze lookup")
            .expect("active freeze");
        assert!(
            app.resolve_feature_run_evidence_lease("project-a", "plan-a")
                .unwrap_err()
                .to_string()
                .contains("planr pick --plan plan-a --work-type verification --json")
        );
        let frozen = repository.feature_run(&persisted.run.id).expect("run");
        let verification = apply_phase_transition(
            &frozen.run,
            &PhaseTransition {
                to: FeatureRunPhase::Verification,
                cause: PhaseTransitionCause::VerificationStarted,
                reference: format!("source_freeze:{}", freeze.id),
                owner: Some(RoleOwner {
                    role: RunRole::Verifier,
                    worker_id: worker_id(),
                    lease_generation: 1,
                }),
            },
        )
        .expect("verification transition");
        repository
            .save_feature_run(&verification, frozen.revision)
            .expect("save verification");
        let resolved = app
            .resolve_feature_run_evidence_lease("project-a", "plan-a")
            .expect("canonical lease")
            .expect("active run");
        assert_eq!(resolved.run_id, persisted.run.id);
        assert_eq!(resolved.freeze_id, freeze.id);
        assert_eq!(resolved.lease_generation, 1);
    }

    #[test]
    fn verification_release_atomically_restores_the_source_frozen_boundary() {
        let (_root, app, run_id, _freeze_id) = verification_fixture(true, true);
        let released = app
            .release_verification_pick_value("verification-phase", false)
            .unwrap()
            .unwrap();
        assert_eq!(released["disposition"], "released");
        assert_eq!(released["item"]["status"], "ready");
        assert_eq!(released["feature_run"]["phase"], "source_frozen");

        let repository = ExecutionRunRepository::new(&app.conn);
        let persisted = repository.feature_run(&run_id).unwrap();
        assert_eq!(persisted.run.phase, FeatureRunPhase::SourceFrozen);
        assert!(persisted.run.role_owners.is_empty());
        let active_verifiers: u64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'verifier' AND released_at IS NULL",
                [&run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active_verifiers, 0);

        let repeated = app
            .release_verification_pick_value("verification-phase", false)
            .unwrap()
            .unwrap();
        assert_eq!(repeated["disposition"], "already_released");
    }

    #[test]
    fn exhausted_one_shot_atomically_fails_item_and_terminally_releases_lease() {
        let (_root, app, run_id, _freeze_id) = verification_fixture(true, true);
        let lease = app
            .resolve_feature_run_evidence_lease("project-a", "plan-a")
            .unwrap()
            .unwrap();

        app.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let repeatable = app
            .settle_exhausted_verification_attempt_in_transaction(
                &app.conn,
                &lease,
                VerificationAttemptExhaustion {
                    obligation_id: "pob-phase-ready",
                    attempt_id: "attempt-repeatable",
                    attempt_index: 0,
                    max_attempts: 1,
                    repeatability: "repeatable",
                },
            )
            .unwrap_err();
        app.conn.execute_batch("ROLLBACK").unwrap();
        assert!(
            repeatable
                .to_string()
                .contains("requires_explicit_one_shot")
        );
        let still_active = ExecutionRunRepository::new(&app.conn)
            .feature_run(&run_id)
            .unwrap();
        assert_eq!(still_active.run.phase, FeatureRunPhase::Verification);

        app.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let (item_id, settled_run_id) = app
            .settle_exhausted_verification_attempt_in_transaction(
                &app.conn,
                &lease,
                VerificationAttemptExhaustion {
                    obligation_id: "pob-phase-ready",
                    attempt_id: "attempt-final",
                    attempt_index: 0,
                    max_attempts: 1,
                    repeatability: "non_repeatable_one_shot",
                },
            )
            .unwrap();
        app.conn.execute_batch("COMMIT").unwrap();
        let item_id = item_id.expect("verification item projection");
        assert_eq!(app.get_item(&item_id).unwrap().status.as_str(), "failed");
        let settled = app
            .canonical_execution_state_value(&settled_run_id, None)
            .unwrap();
        assert_eq!(settled["reason_code"], "verification_attempts_exhausted");
        assert_eq!(settled["next_action"], "none");
        assert_eq!(
            settled["feature_run"]["terminal_reason"],
            "verification_attempts_exhausted"
        );
        let active_verifiers: u64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'verifier' AND released_at IS NULL",
                [&run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active_verifiers, 0);
    }

    #[test]
    fn verification_admission_repair_is_itemless_safe_seal_bound_and_atomic() {
        let protected_counts = |app: &App| {
            (
            ["evidence_attempts", "evidence_receipts", "coverage_verdicts", "coverage_verdict_history", "review_findings"]
                .map(|table| app.conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, u64>(0)).unwrap()),
            app.conn.query_row("SELECT COUNT(*) FROM feature_run_evidence_invalidations WHERE reason = 'product_finding'", [], |row| row.get::<_, u64>(0)).unwrap(),
        )
        };
        for reason in [
            VerificationAdmissionRepairReason::ReadinessBlocked,
            VerificationAdmissionRepairReason::RunIndexSealFailed,
        ] {
            assert!(!reason.requires_run_index_digest());
        }
        for reason in [
            VerificationAdmissionRepairReason::SealedRunRejected,
            VerificationAdmissionRepairReason::CapabilityAdmissionFailed,
        ] {
            assert!(reason.requires_run_index_digest());
        }
        let mut equivalent = Vec::new();
        for with_item in [false, true] {
            let (_root, app, run_id, freeze_id) = verification_fixture(with_item, true);
            let repository = ExecutionRunRepository::new(&app.conn);
            let admitted = repository
                .latest_verification_admission(&run_id, &freeze_id)
                .unwrap()
                .unwrap();
            let persisted = repository.feature_run(&run_id).unwrap();
            let request = VerificationAdmissionRepairRequest {
                plan_id: "plan-a".into(),
                run_id: run_id.clone(),
                freeze_id: freeze_id.clone(),
                run_revision: persisted.revision,
                reason: if with_item {
                    VerificationAdmissionRepairReason::CapabilityAdmissionFailed
                } else {
                    VerificationAdmissionRepairReason::SealedRunRejected
                },
                run_index_digest: Some(admitted.run_index_digest),
            };
            let before = protected_counts(&app);
            if !with_item {
                let stale = [
                    {
                        let mut value = request.clone();
                        value.plan_id = "plan-stale".into();
                        value
                    },
                    {
                        let mut value = request.clone();
                        value.run_id = "run-stale".into();
                        value
                    },
                    {
                        let mut value = request.clone();
                        value.freeze_id = "freeze-stale".into();
                        value
                    },
                    {
                        let mut value = request.clone();
                        value.run_revision += 1;
                        value
                    },
                    {
                        let mut value = request.clone();
                        value.run_index_digest = Some("sha256:stale".into());
                        value
                    },
                ];
                for candidate in stale {
                    assert!(app.repair_verification_admission_value(candidate).is_err());
                    assert_eq!(repository.feature_run(&run_id).unwrap(), persisted);
                    assert_eq!(protected_counts(&app), before);
                }
            }
            let repaired = app
                .repair_verification_admission_value(request.clone())
                .unwrap();
            let repaired_revision = repository.feature_run(&run_id).unwrap().revision;
            let repeated = app.repair_verification_admission_value(request).unwrap();
            assert_eq!(repaired["repair"], repeated["repair"]);
            assert_eq!(
                repository.feature_run(&run_id).unwrap().revision,
                repaired_revision
            );
            let transition: crate::execution_run::VerificationAdmissionRepairTransition =
                serde_json::from_value(repaired["repair"].clone()).unwrap();
            let current = repository.feature_run(&run_id).unwrap();
            let maker = owner_for_role(&current.run, RunRole::Maker).unwrap();
            assert_eq!(
                (maker.worker_id.as_str(), maker.lease_generation),
                ("maker-other", 2)
            );
            assert_eq!(
                repository.source_freeze(&freeze_id).unwrap().status,
                SourceFreezeStatus::Invalidated
            );
            assert_eq!(transition.facts.verification_item_id.is_some(), with_item);
            assert_eq!(
                repository
                    .verification_item_projection("plan-a")
                    .unwrap()
                    .is_some(),
                with_item
            );
            assert_eq!(
                repository
                    .batch(current.run.active_batch_id.as_deref().unwrap())
                    .unwrap()
                    .batch
                    .status,
                ExecutionBatchStatus::Active
            );
            assert!(owner_for_role(&current.run, RunRole::Verifier).is_none());
            let state = app.canonical_execution_state_value(&run_id, None).unwrap();
            assert_eq!(
                (state["phase"].clone(), state["next_action"].clone()),
                (json!("implementation"), json!("settle_next_outcome"))
            );
            assert_eq!(protected_counts(&app), before);
            assert_eq!(app.conn.query_row("SELECT COUNT(*) FROM events WHERE event_type = 'feature_run_verification_admission_repaired'", [], |row| row.get::<_, u64>(0)).unwrap(), 1);
            equivalent.push(json!({"phase": state["phase"], "next": state["next_action"], "maker": maker.worker_id, "generation": maker.lease_generation, "batch": "active"}));
        }
        assert_eq!(equivalent[0], equivalent[1]);

        let (_root, app, run_id, freeze_id) = verification_fixture(true, false);
        app.conn.execute("INSERT INTO proof_obligations(id, project_id, plan_id, item_id, criterion_id, obligation_version, title, binding, observation_requirements_json, fixture_policy_json, freshness_policy_json, assurance_policy_json, retry_aggregation, policy_digest, config_digest, source_digest, supersedes_obligation_id, created_at, obligation_shape) SELECT 'pob-readiness-conflict', project_id, plan_id, item_id, criterion_id, obligation_version + 1, title, binding, observation_requirements_json, fixture_policy_json, freshness_policy_json, assurance_policy_json, retry_aggregation, policy_digest, config_digest, source_digest, NULL, datetime('now'), obligation_shape FROM proof_obligations WHERE id = 'pob-phase-ready'", []).unwrap();
        let before = protected_counts(&app);
        assert!(
            app.verification_work_packet_value("plan-a", false)
                .unwrap_err()
                .to_string()
                .contains("verification_pick_readiness_blocked")
        );
        let repository = ExecutionRunRepository::new(&app.conn);
        let held = repository.feature_run(&run_id).unwrap();
        let diagnostic = repository
            .latest_verification_readiness_diagnostic(&run_id, &freeze_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            (held.run.phase, held.run.hold_reason),
            (
                FeatureRunPhase::Held,
                Some(FeatureRunHoldReason::Capability)
            )
        );
        assert!(owner_for_role(&held.run, RunRole::Verifier).is_none());
        let item = repository
            .verification_item_projection("plan-a")
            .unwrap()
            .unwrap();
        assert_eq!(
            (item.status, item.worker_id),
            (CurrentVerificationItemLeaseStatus::Ready, None)
        );
        assert_eq!(
            diagnostic.repair_request.reason,
            VerificationAdmissionRepairReason::ReadinessBlocked
        );
        assert!(diagnostic.repair_request.run_index_digest.is_none());
        let mut invalid = diagnostic.repair_request.clone();
        invalid.run_index_digest = Some("sha256:forbidden".into());
        assert!(app.repair_verification_admission_value(invalid).is_err());
        assert_eq!(repository.feature_run(&run_id).unwrap(), held);
        let repaired = app
            .repair_verification_admission_value(diagnostic.repair_request)
            .unwrap();
        assert_eq!(
            repaired["repair"]["repaired_run"]["phase"],
            "implementation"
        );
        assert_eq!(
            repository.source_freeze(&freeze_id).unwrap().status,
            SourceFreezeStatus::Invalidated
        );
        assert_eq!(protected_counts(&app), before);
    }

    #[test]
    fn frozen_plan_readiness_requires_the_verification_lease_before_side_effects() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        initialize_git(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-lease-first-readiness");
        let run = app
            .ensure_outcome_feature_run("item-lease-first-readiness")
            .unwrap()
            .unwrap();
        app.freeze_feature_run_source_value("plan-a")
            .unwrap()
            .unwrap();

        let counts = |app: &App| {
            [
                "verification_capability_manifests",
                "verification_capability_instances",
                "evidence_attempts",
                "evidence_receipts",
            ]
            .map(|table| {
                app.conn
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get::<_, u64>(0)
                    })
                    .unwrap()
            })
        };
        let before = counts(&app);
        let error = app
            .evidence_readiness_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap_err()
            .to_string();

        assert_eq!(
            error,
            "evidence_readiness_requires_verification_lease:phase=source_frozen:owner=unleased: run `planr pick --plan plan-a --work-type verification --json`"
        );
        assert_eq!(counts(&app), before);
        assert_eq!(
            ExecutionRunRepository::new(&app.conn)
                .feature_run(&run.run.id)
                .unwrap()
                .run
                .phase,
            FeatureRunPhase::SourceFrozen
        );
        assert!(!root.path().join(".planr/evidence/runs").exists());
    }

    #[test]
    fn plan_readiness_freezes_then_returns_the_exact_verification_pick_repair() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        initialize_git(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-readiness-ordering-repair");
        let run = app
            .ensure_outcome_feature_run("item-readiness-ordering-repair")
            .unwrap()
            .unwrap();

        let error = app
            .evidence_readiness_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap_err()
            .to_string();

        assert_eq!(
            error,
            "evidence_readiness_requires_verification_lease:phase=source_frozen:owner=unleased: run `planr pick --plan plan-a --work-type verification --json`"
        );
        let repository = ExecutionRunRepository::new(&app.conn);
        assert_eq!(
            repository.feature_run(&run.run.id).unwrap().run.phase,
            FeatureRunPhase::SourceFrozen
        );
        assert!(
            repository
                .active_source_freeze(&run.run.id)
                .unwrap()
                .is_some()
        );
        assert!(!root.path().join(".planr/evidence/runs").exists());
    }

    #[test]
    fn verifier_pick_unlocks_readiness_and_one_sealed_repository_run_index() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        let policy_digest = write_ready_evidence_policy(root.path());
        initialize_git(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-happy-path");
        app.conn
            .execute(
                "INSERT INTO proof_obligations(
                  id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                  binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                  assurance_policy_json, retry_aggregation, policy_digest, config_digest,
                  source_digest, supersedes_obligation_id, created_at, obligation_shape
                ) VALUES (
                  'pob-ready', 'project-a', 'plan-a', 'item-happy-path', 'criterion-ready', 1,
                  'ready process', 1, ?1, '{}', '{}', '{}', 'all_applicable_pass', ?2, ?2, ?2, NULL,
                  datetime('now'), 'semantic_v1'
                )",
                params![
                    json!([{
                        "id": "obs-ready",
                        "type": "com.example.ready.status",
                        "subject": "ready process",
                        "expected": {"status": "ready"},
                        "target": {"kind": "process", "uri": "local://ready"},
                        "payload_schema": {"schema_ref": "com.example.ready.status@v1"}
                    }])
                    .to_string(),
                    policy_digest,
                ],
            )
            .unwrap();
        let run = app
            .ensure_outcome_feature_run("item-happy-path")
            .unwrap()
            .unwrap();
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = 'maker-other' WHERE run_id = ?1 AND role = 'maker' AND released_at IS NULL",
                [&run.run.id],
            )
            .unwrap();

        let ordering = app
            .evidence_readiness_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap_err()
            .to_string();
        assert!(
            ordering.contains("--work-type verification --json"),
            "{ordering}"
        );
        let packet = app
            .verification_work_packet_value("plan-a", false)
            .unwrap()
            .unwrap();
        assert_eq!(packet["work_packet"]["kind"], "verification");
        assert_eq!(
            packet["work_packet"]["verification_lease"]["worker_id"],
            worker_id()
        );
        assert_eq!(
            packet["work_packet"]["sealed_run_index"]["schema_version"],
            "planr.evidence.run-index.v1"
        );
        assert!(
            packet["work_packet"]["sealed_run_index"]["repository_path"]
                .as_str()
                .unwrap()
                .starts_with(".planr/evidence/runs/")
        );

        let readiness = app
            .evidence_readiness_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap();
        assert_eq!(readiness["status"], "passed");
        let repository_path = readiness["run_index"]["repository_path"].as_str().unwrap();
        assert!(root.path().join(repository_path).is_file());
        assert_eq!(
            readiness["run_index"]["schema_version"],
            "planr.evidence.run-index.v1"
        );
        assert!(
            readiness["run_index"]["run_index_digest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );

        let run_result = app
            .evidence_run_value(readiness["run_index"].clone())
            .expect("admit and execute the sealed run index");
        assert_eq!(run_result["results"][0]["verdict"], "passed");
        let persisted = ExecutionRunRepository::new(&app.conn)
            .feature_run(&run.run.id)
            .unwrap();
        assert_eq!(persisted.run.phase, FeatureRunPhase::Verification);
        assert_eq!(persisted.run.role_owners[0].role, RunRole::Verifier);
    }

    #[test]
    fn one_shot_claim_survives_late_settlement_rollback_and_blocks_relaunch() {
        let (_root, app, run_id, run_index) =
            one_shot_verification_fixture("printf 'not-json'", None);
        app.conn
            .execute_batch(
                "CREATE TRIGGER reject_one_shot_settlement
                 BEFORE UPDATE OF status ON items
                 WHEN NEW.status = 'failed'
                 BEGIN SELECT RAISE(ABORT, 'injected terminal settlement failure'); END;",
            )
            .unwrap();
        let error = app.evidence_run_value(run_index.clone()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("injected terminal settlement failure"),
            "{error}"
        );
        for table in ["evidence_attempts", "evidence_receipts"] {
            let count: u64 = app
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table} must roll back with settlement");
        }
        assert_eq!(
            ExecutionRunRepository::new(&app.conn)
                .feature_run(&run_id)
                .unwrap()
                .run
                .phase,
            FeatureRunPhase::Verification
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM feature_run_one_shot_claims",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            1
        );

        app.conn
            .execute_batch("DROP TRIGGER reject_one_shot_settlement")
            .unwrap();
        let error = app.evidence_run_value(run_index).unwrap_err();
        assert!(
            error.to_string().contains("allowance already consumed"),
            "{error}"
        );
        assert_eq!(
            app.conn
                .query_row("SELECT COUNT(*) FROM evidence_attempts", [], |row| {
                    row.get::<_, u64>(0)
                })
                .unwrap(),
            0
        );
    }

    #[test]
    fn sealed_one_shot_nonpassing_receipt_atomically_settles_terminal_exhaustion() {
        let (_root, app, run_id, run_index) =
            one_shot_verification_fixture("printf 'not-json'", None);
        let result = app.evidence_run_value(run_index).unwrap();
        assert_eq!(result["verdict"], "failed");
        assert_eq!(
            result["terminal_exhaustion"]["status"],
            "terminal_non_covering"
        );
        assert_eq!(
            result["terminal_exhaustion"]["execution_state"]["reason_code"],
            "verification_attempts_exhausted"
        );
        assert_eq!(
            result["results"][0]["attempt"]["retry_lineage"]["max_attempts"],
            1
        );
        let persisted = ExecutionRunRepository::new(&app.conn)
            .feature_run(&run_id)
            .unwrap();
        assert_eq!(persisted.run.phase, FeatureRunPhase::Cancelled);
        assert!(persisted.run.role_owners.is_empty());
    }

    #[test]
    fn passed_one_shot_rejects_second_fresh_initial_before_adapter_launch() {
        let marker = tempfile::NamedTempFile::new().unwrap();
        let shell_script = format!(
            "printf launched >> '{}'; printf '{{\"status\":\"ready\"}}'",
            marker.path().display()
        );
        let (_root, app, _run_id, run_index) =
            one_shot_verification_fixture_with_probe(&shell_script, "printf ready", None);

        let first = app.evidence_run_value(run_index.clone()).unwrap();
        assert_eq!(first["verdict"], "passed");
        let second = app.evidence_run_value(run_index).unwrap_err();
        assert!(
            second.to_string().contains("allowance already consumed"),
            "{second}"
        );
        assert_eq!(std::fs::read_to_string(marker.path()).unwrap(), "launched");
        assert_eq!(
            app.conn
                .query_row("SELECT COUNT(*) FROM evidence_attempts", [], |row| {
                    row.get::<_, u64>(0)
                })
                .unwrap(),
            1
        );
    }

    #[test]
    fn product_failed_one_shot_is_terminal_without_repair_or_replay() {
        let (_root, app, run_id, run_index) =
            one_shot_verification_fixture_with_probe("exit 77", "printf ready", None);
        let result = app.evidence_run_value(run_index).unwrap();

        assert_eq!(result["verdict"], "failed");
        assert_eq!(
            result["terminal_exhaustion"]["execution_state"]["reason_code"],
            "verification_attempts_exhausted"
        );
        assert_eq!(
            result["results"][0]["receipt"]["proof_gaps"],
            json!(["product_failed"])
        );
        assert!(result["results"][0]["product_finding"].is_null());
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM feature_run_evidence_invalidations",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );
        assert_eq!(
            ExecutionRunRepository::new(&app.conn)
                .feature_run(&run_id)
                .unwrap()
                .run
                .terminal_reason,
            Some(FeatureRunTerminalReason::VerificationAttemptsExhausted)
        );
    }

    #[test]
    fn concurrent_one_shot_initials_claim_once_and_spawn_once() {
        use std::sync::{Arc, Barrier};

        let marker = tempfile::NamedTempFile::new().unwrap();
        let database_dir = tempfile::tempdir().unwrap();
        let database_path = database_dir.path().join("one-shot.sqlite");
        let shell_script = format!(
            "printf launched >> '{}'; sleep 0.2; printf '{{\"status\":\"ready\"}}'",
            marker.path().display()
        );
        let (root, app, _run_id, run_index) = one_shot_verification_fixture_with_probe(
            &shell_script,
            "printf ready",
            Some(database_path.clone()),
        );
        drop(app);

        let barrier = Arc::new(Barrier::new(3));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let barrier = Arc::clone(&barrier);
            let database_path = database_path.clone();
            let repository_root = root.path().to_path_buf();
            let run_index = run_index.clone();
            threads.push(std::thread::spawn(move || {
                let conn = crate::storage::open_db(&database_path).unwrap();
                let app = App::new(conn, repository_root, database_path, true, false);
                barrier.wait();
                app.evidence_run_value(run_index)
            }));
        }
        barrier.wait();
        let results = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            1,
            "{results:?}"
        );
        let rejected = results
            .iter()
            .find_map(|result| result.as_ref().err())
            .expect("one concurrent contender must be rejected");
        assert!(
            rejected.to_string().contains("allowance already consumed"),
            "{rejected}"
        );
        assert_eq!(std::fs::read_to_string(marker.path()).unwrap(), "launched");
        let conn = Connection::open(database_path).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM feature_run_one_shot_claims",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM evidence_attempts", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap(),
            1
        );
    }

    #[test]
    fn sealed_multi_run_defers_product_finding_until_every_adapter_finishes() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        write_ready_evidence_policy(root.path());
        let policy_digest = add_product_failure_evidence_adapter(root.path(), "repeatable");
        initialize_git(root.path());
        let app = test_app(root.path().to_path_buf());
        for item_id in ["item-batch-a", "item-batch-b"] {
            add_outcome(&app, item_id);
        }
        add_verification_outcome(&app, "item-verifier");
        for (obligation_id, criterion_id) in [
            ("pob-a-product-failure", "criterion-a-product-failure"),
            ("pob-b-ready", "criterion-b-ready"),
        ] {
            app.conn
                .execute(
                    "INSERT INTO proof_obligations(
                      id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                      binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                      assurance_policy_json, retry_aggregation, policy_digest, config_digest,
                      source_digest, supersedes_obligation_id, created_at, obligation_shape
                    ) VALUES (
                      ?1, 'project-a', 'plan-a', 'item-verifier', ?2, 1,
                      ?1, 1, ?3, '{}', '{}', '{}', 'all_applicable_pass', ?4, ?4, ?4, NULL,
                      datetime('now'), 'semantic_v1'
                    )",
                    params![
                        obligation_id,
                        criterion_id,
                        json!([{
                            "id": format!("obs-{obligation_id}"),
                            "type": if obligation_id == "pob-a-product-failure" {
                                "com.example.product.status"
                            } else {
                                "com.example.ready.status"
                            },
                            "subject": obligation_id,
                            "expected": {"status": "ready"},
                            "target": {"kind": "process", "uri": "local://ready"},
                            "payload_schema": {"schema_ref": if obligation_id == "pob-a-product-failure" {
                                "com.example.product.status@v1"
                            } else {
                                "com.example.ready.status@v1"
                            }}
                        }])
                        .to_string(),
                        policy_digest,
                    ],
                )
                .unwrap();
        }
        let run = app
            .ensure_outcome_feature_run("item-batch-a")
            .unwrap()
            .unwrap();
        let non_material =
            json!({"decision": {"material": false, "review": "none", "reasons": []}});
        for item_id in ["item-batch-a", "item-batch-b"] {
            app.settle_feature_run_outcome(OutcomeSettlement {
                item_id,
                summary: "compatible batched maker outcome",
                materiality: &non_material,
                escalation: None,
            })
            .unwrap();
        }
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = 'maker-other' WHERE run_id = ?1 AND role = 'maker' AND released_at IS NULL",
                [&run.run.id],
            )
            .unwrap();
        let frozen = app
            .freeze_feature_run_source_value("plan-a")
            .unwrap()
            .unwrap();
        assert_eq!(frozen["feature_run"]["phase"], "source_frozen");
        let packet = app
            .verification_work_packet_value("plan-a", false)
            .unwrap()
            .unwrap();
        assert_eq!(packet["work_packet"]["item_id"], "item-verifier");
        assert_eq!(
            packet["work_packet"]["execution_state"]["phase"],
            "verification"
        );
        let readiness = app
            .evidence_readiness_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap();
        assert_eq!(readiness["status"], "passed");
        assert_eq!(readiness["run_index"]["runs"].as_array().unwrap().len(), 2);

        let result = app
            .evidence_run_value(readiness["run_index"].clone())
            .expect("the sealed collector must not invalidate its own verifier lease");
        assert_eq!(result["verdict"], "failed");
        assert_eq!(
            crate::app::evidence::evidence_success_envelope("evidence.run", result.clone())["exit"]
                ["code"],
            crate::app::evidence::EVIDENCE_UNSATISFIED
        );
        assert_eq!(result["results"].as_array().unwrap().len(), 2);
        assert_eq!(result["results"][0]["verdict"], "failed");
        assert_eq!(result["results"][1]["verdict"], "passed", "{result}");
        assert_eq!(
            app.conn
                .query_row("SELECT COUNT(*) FROM evidence_attempts", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            2
        );
        let repaired = ExecutionRunRepository::new(&app.conn)
            .feature_run(&run.run.id)
            .unwrap();
        assert_eq!(repaired.run.phase, FeatureRunPhase::Implementation);
        assert_eq!(repaired.run.role_owners[0].role, RunRole::Maker);
    }

    fn seed_settlement_obligation(app: &App, policy_digest: &str) {
        seed_settlement_obligation_with_freshness(app, policy_digest, json!({"invalidate_on": []}));
    }

    fn seed_settlement_obligation_with_freshness(
        app: &App,
        policy_digest: &str,
        freshness_policy: Value,
    ) {
        app.conn
            .execute(
                "INSERT INTO proof_obligations(
                  id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                  binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                  assurance_policy_json, retry_aggregation, policy_digest, config_digest,
                  source_digest, supersedes_obligation_id, created_at, obligation_shape
                ) VALUES (
                  'pob-settle', 'project-a', 'plan-a', NULL, 'criterion-settle', 1,
                  'settlement process', 1, ?1, ?2, ?3, '{}', 'latest_applicable_pass', ?4, ?5, ?6, NULL,
                  datetime('now'), 'semantic_v1'
                )",
                params![
                    json!([{
                        "id": "obs-settle",
                        "type": "com.example.ready.status",
                        "subject": "settlement process",
                        "expected": {"status": "ready", "schema_ref": "com.example.ready.status@v1"},
                        "target": {"kind": "process", "uri": "local://ready"},
                        "payload_schema": {"schema_ref": "com.example.ready.status@v1"}
                    }])
                    .to_string(),
                    json!({"fixtures_allowed": false, "mocks_allowed": false}).to_string(),
                    freshness_policy.to_string(),
                    policy_digest,
                    "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                    "sha256:3333333333333333333333333333333333333333333333333333333333333333",
                ],
            )
            .unwrap();
    }

    fn seed_receipt_bound_settlement(app: &App, policy_digest: &str) -> String {
        seed_receipt_for_existing_settlement_obligation(app, policy_digest)
    }

    fn seed_receipt_for_existing_settlement_obligation(app: &App, policy_digest: &str) -> String {
        app.verification_work_packet_value("plan-a", false)
            .unwrap()
            .unwrap();
        let repository = ExecutionRunRepository::new(&app.conn);
        let run = repository
            .active_feature_run_for_plan("project-a", "plan-a")
            .unwrap()
            .unwrap();
        let freeze = repository
            .active_source_freeze(&run.run.id)
            .unwrap()
            .unwrap();
        let source = capture_repository_snapshot(&app.root).unwrap().source;
        assert_eq!(source.revision.as_str(), freeze.source_revision.as_str());
        assert_eq!(source.tree_digest.as_str(), freeze.source_digest.as_str());
        let (instance_id, manifest_id, manifest_digest): (String, String, String) = app
            .conn
            .query_row(
                "SELECT id, manifest_id, manifest_digest FROM verification_capability_instances ORDER BY created_at, id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let receipt_id = "erec-settle";
        let attempt_id = "eatt-settle";
        app.conn
            .execute(
                "INSERT INTO evidence_attempts(
                  id, project_id, obligation_id, capability_instance_id, attempt_status,
                  execution_contract_digest, resolved_command_json, environment_digest,
                  started_at, completed_at, exit_code, output_bounds_json, attempt_json, created_at
                ) VALUES (
                  ?1, 'project-a', 'pob-settle', ?2, 'passed',
                  ?3, '{}', ?4, datetime('now'), datetime('now'), 0, '{}', ?5, datetime('now')
                )",
                params![
                    attempt_id,
                    instance_id,
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "sha256:5555555555555555555555555555555555555555555555555555555555555555",
                    json!({
                        "id": attempt_id,
                        "status": "passed",
                        "exit": {"exit_code": 0, "signal": null, "error": null}
                    })
                    .to_string(),
                ],
            )
            .unwrap();
        let receipt = build_trusted_receipt(TrustedReceiptInput {
            id: crate::evidence::EvidenceId::parse(receipt_id).unwrap(),
            criterion_id: crate::evidence::EvidenceId::parse("criterion-settle").unwrap(),
            obligation_id: crate::evidence::EvidenceId::parse("pob-settle").unwrap(),
            source,
            target: crate::evidence::TargetBinding {
                kind: "process".to_string(),
                uri: Some("local://ready".to_string()),
                digest: None,
                deployment_id: None,
            },
            environment: crate::evidence::EnvironmentBinding {
                kind: "local".to_string(),
                id: crate::evidence::EvidenceId::parse("dev").unwrap(),
                digest: Sha256Digest::parse(
                    "sha256:5555555555555555555555555555555555555555555555555555555555555555",
                )
                .unwrap(),
            },
            vantage_point: VantagePoint {
                kind: "process_adapter".to_string(),
                identity: manifest_id.clone(),
            },
            capability: CapabilityBinding {
                manifest_id: crate::evidence::EvidenceId::parse(manifest_id).unwrap(),
                manifest_digest: Sha256Digest::parse(manifest_digest).unwrap(),
                instance_id: crate::evidence::EvidenceId::parse(instance_id).unwrap(),
                instance_digest: Sha256Digest::parse(
                    "sha256:7777777777777777777777777777777777777777777777777777777777777777",
                )
                .unwrap(),
            },
            provenance: TrustedProvenance::planr_observed_execution(attempt_id).unwrap(),
            observations: vec![crate::evidence::model::ObservationResult {
                requirement_id: crate::evidence::EvidenceId::parse("obs-settle").unwrap(),
                observation_type: crate::evidence::NamespacedIdentifier::parse(
                    "com.example.ready.status",
                )
                .unwrap(),
                outcome: crate::evidence::AttemptStatus::Passed,
                predicate: Map::from_iter([
                    ("status".to_string(), json!("ready")),
                    (
                        "schema_ref".to_string(),
                        json!("com.example.ready.status@v1"),
                    ),
                ]),
                actual: Map::from_iter([(
                    "schema_ref".to_string(),
                    json!("com.example.ready.status@v1"),
                )]),
            }],
            attempt_ids: vec![crate::evidence::EvidenceId::parse(attempt_id).unwrap()],
            retry_history: vec![],
            artifacts: vec![],
            raw_result: RawResultRef {
                kind: "inline".to_string(),
                digest: Sha256Digest::parse(
                    "sha256:9999999999999999999999999999999999999999999999999999999999999999",
                )
                .unwrap(),
                artifact_id: None,
                extra: Map::new(),
            },
            config_digest: Sha256Digest::parse(
                "sha256:2222222222222222222222222222222222222222222222222222222222222222",
            )
            .unwrap(),
            fixture_disclosure: crate::evidence::FixtureDisclosure {
                fixtures_used: false,
                mocks_used: false,
                fixture_refs: None,
                mock_refs: None,
            },
            permissions: PermissionState {
                network: "none".to_string(),
                filesystem: "none".to_string(),
                environment: None,
                secrets: None,
            },
            sandbox: crate::evidence::model::SandboxState {
                mode: "test".to_string(),
                limits: crate::evidence::model::SandboxLimits {
                    timeout_ms: 1,
                    stdout_bytes: 0,
                    stderr_bytes: 0,
                },
            },
            proof_gaps: vec![],
            started_at: "2026-07-28T12:00:00Z".to_string(),
            ended_at: "2026-07-28T12:00:01Z".to_string(),
        })
        .unwrap();
        let receipt_value = serde_json::to_value(&receipt).unwrap();
        let receipt_digest = receipt_value["receipt_digest"]
            .as_str()
            .unwrap()
            .to_string();
        let trusted_binding = json!({
            "source": receipt_value["source"],
            "target": receipt_value["target"],
            "environment": receipt_value["environment"],
            "capability": receipt_value["capability"],
            "policy_digest": policy_digest,
            "policy_source": "repository",
            "config_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222"
        });
        app.conn
            .execute(
                "INSERT INTO evidence_receipts(
                  id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
                  trusted_binding_json, observations_json, provenance_json, receipt_json, created_at
                ) VALUES (?1, 'project-a', 'pob-settle', ?2, 'trusted', ?3, ?4, ?5, ?6, ?7, datetime('now'))",
                params![
                    receipt_id,
                    attempt_id,
                    receipt_digest,
                    trusted_binding.to_string(),
                    receipt_value["observations"].to_string(),
                    receipt_value["provenance"].to_string(),
                    receipt_value.to_string(),
                ],
            )
            .unwrap();
        receipt_digest
    }

    fn settlement_app() -> (tempfile::TempDir, App, String) {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        let budget_policy_path = root.path().join(".planr/policy.toml");
        let unbounded_policy = std::fs::read_to_string(&budget_policy_path)
            .unwrap()
            .lines()
            .filter(|line| {
                !line.starts_with("max_wall_time_seconds")
                    && !line.starts_with("max_tool_calls")
                    && !line.starts_with("max_tokens")
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&budget_policy_path, unbounded_policy).unwrap();
        let policy_digest = write_ready_evidence_policy(root.path());
        let plan_path = root.path().join("plan-a.md");
        std::fs::write(
            &plan_path,
            crate::planpack::build_plan_body("Plan", "product-plan", "settle"),
        )
        .unwrap();
        std::fs::write(root.path().join(".gitignore"), "planr.sqlite*\n").unwrap();
        initialize_git(root.path());
        let database_path = root.path().join("planr.sqlite");
        let conn = Connection::open(&database_path).expect("database");
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
        let app = App::new(conn, root.path().to_path_buf(), database_path, true, false);
        app.conn
            .execute(
                "UPDATE plans SET path = ?1 WHERE id = 'plan-a'",
                [plan_path.to_string_lossy().as_ref()],
            )
            .unwrap();
        add_outcome(&app, "item-settle-maker");
        add_verification_item(&app, "item-settle-verification");
        app.conn
            .execute(
                "UPDATE items SET plan_path = ?1 WHERE id IN ('item-settle-maker', 'item-settle-verification')",
                [plan_path.to_string_lossy().as_ref()],
            )
            .unwrap();
        seed_settlement_obligation(&app, &policy_digest);
        let run = app
            .ensure_outcome_feature_run("item-settle-maker")
            .unwrap()
            .unwrap();
        app.close_item_value(
            "item-settle-maker",
            "initial ordinary outcome settled before source freeze",
        )
        .unwrap();
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = 'maker-other' WHERE run_id = ?1 AND role = 'maker' AND released_at IS NULL",
                [&run.run.id],
            )
            .unwrap();
        app.freeze_feature_run_source_value("plan-a")
            .unwrap()
            .unwrap();
        (root, app, policy_digest)
    }

    fn stranded_recovery_app() -> (tempfile::TempDir, App, Value) {
        let (root, app, policy_digest) = settlement_app();
        app.conn
            .execute(
                "UPDATE items SET status = 'closed', completed_at = datetime('now')
                 WHERE id = 'item-settle-maker'",
                [],
            )
            .unwrap();
        seed_receipt_bound_settlement(&app, &policy_digest);
        app.evidence_coverage_value(EvidenceCoverageScope::Plan, "plan-a")
            .expect("initial canonical settlement");
        add_outcome(&app, "item-after-verification");
        let repository = ExecutionRunRepository::new(&app.conn);
        let settled = repository
            .active_feature_run_for_plan("project-a", "plan-a")
            .unwrap()
            .unwrap();
        let freeze = repository
            .active_source_freeze(&settled.run.id)
            .unwrap()
            .unwrap();
        let binding: String = app
            .conn
            .query_row(
                "SELECT trusted_binding_json FROM evidence_receipts WHERE id = 'erec-settle'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut binding: Value = serde_json::from_str(&binding).unwrap();
        binding["source"]["revision"] = json!(freeze.source_revision);
        binding["source"]["tree_digest"] = json!(freeze.source_digest);
        app.conn
            .execute_batch("DROP TRIGGER evidence_receipts_no_update")
            .unwrap();
        app.conn
            .execute(
                "UPDATE evidence_receipts SET trusted_binding_json = ?1 WHERE id = 'erec-settle'",
                [binding.to_string()],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = ?2
                 WHERE run_id = ?1 AND role = 'maker'",
                params![settled.run.id, worker_id()],
            )
            .unwrap();
        let verifier_generation: u64 = app
            .conn
            .query_row(
                "SELECT COALESCE(MAX(lease_generation), 0) + 1
                 FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'verifier'",
                [&settled.run.id],
                |row| row.get(0),
            )
            .unwrap();
        app.conn
            .execute(
                "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation)
                 VALUES (?1, 'verifier', 'stranded-verifier', ?2)",
                params![settled.run.id, verifier_generation],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE feature_runs SET phase = 'verification', revision = revision + 1,
                 updated_at = datetime('now') WHERE id = ?1",
                [&settled.run.id],
            )
            .unwrap();
        let input = json!({
            "schema": "planr.evidence.recover_settlement.v1",
            "plan_id": "plan-a",
            "run_id": settled.run.id,
            "freeze_id": freeze.id,
            "receipt_id": "erec-settle",
            "verifier_worker_id": "stranded-verifier",
            "verifier_generation": verifier_generation,
            "next_item_id": "item-after-verification",
        });
        (root, app, input)
    }

    fn terminal_verified_continuation_app() -> (tempfile::TempDir, App, Value) {
        let (root, app, recovery_input) = stranded_recovery_app();
        app.recover_verification_settlement_value(recovery_input.clone())
            .expect("restore verified continuation");
        let repository = ExecutionRunRepository::new(&app.conn);
        let run = repository
            .active_feature_run_for_plan("project-a", "plan-a")
            .unwrap()
            .unwrap();
        let batch_id = run.run.active_batch_id.as_deref().unwrap();
        app.conn
            .execute(
                "INSERT INTO execution_run_outcomes(id, run_id, batch_id, item_id, ordinal, outcome_json)
                 VALUES ('outcome-item-after-verification', ?1, ?2, 'item-after-verification', 1, '{}')",
                params![run.run.id, batch_id],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE feature_runs SET outcomes_settled = outcomes_settled + 1,
                    batch_outcome_count = batch_outcome_count + 1, revision = revision + 1
                 WHERE id = ?1",
                [&run.run.id],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE items SET status = 'closed', worker_id = NULL WHERE id = 'item-after-verification'",
                [],
            )
            .unwrap();
        let input = json!({
            "schema": "planr.evidence.recover_verified_continuation.v1",
            "plan_id": "plan-a",
            "run_id": recovery_input["run_id"],
            "freeze_id": recovery_input["freeze_id"],
            "receipt_id": recovery_input["receipt_id"],
            "recovery_item_id": recovery_input["next_item_id"],
            "final_item_id": "item-after-verification"
        });
        (root, app, input)
    }

    fn historical_invalidation_reconciliation_app() -> (tempfile::TempDir, App, Value) {
        let (root, app, settlement_input) = stranded_recovery_app();
        app.recover_verification_settlement_value(settlement_input)
            .expect("restore ordinary maker continuation");
        let repository = ExecutionRunRepository::new(&app.conn);
        let run = repository
            .active_feature_run_for_plan("project-a", "plan-a")
            .unwrap()
            .unwrap();
        let active = repository
            .active_source_freeze(&run.run.id)
            .unwrap()
            .unwrap();
        app.conn
            .execute(
                "UPDATE feature_run_source_freezes SET created_at = '2000-01-02 00:00:00'
                 WHERE id = ?1",
                [&active.id],
            )
            .unwrap();
        let (receipt_json, trusted_binding): (String, String) = app
            .conn
            .query_row(
                "SELECT receipt_json, trusted_binding_json FROM evidence_receipts
                 WHERE id = 'erec-settle'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let mut receipt: Value = serde_json::from_str(&receipt_json).unwrap();
        receipt["source"]["revision"] = json!(active.source_revision);
        receipt["source"]["tree_digest"] = json!(active.source_digest);
        let receipt_digest = crate::canonical_json::sha256_json_digest_without_top_level_field(
            &receipt,
            "receipt_digest",
        )
        .unwrap();
        receipt["receipt_digest"] = json!(receipt_digest);
        let mut trusted_binding: Value = serde_json::from_str(&trusted_binding).unwrap();
        trusted_binding["source"] = receipt["source"].clone();
        app.conn
            .execute(
                "UPDATE evidence_receipts SET receipt_digest = ?1, receipt_json = ?2,
                   trusted_binding_json = ?3 WHERE id = 'erec-settle'",
                params![
                    receipt_digest,
                    receipt.to_string(),
                    trusted_binding.to_string(),
                ],
            )
            .unwrap();
        let evaluated_at = OffsetDateTime::now_utc().format(&Rfc3339).unwrap();
        let coverage = crate::evidence::coverage::evaluate_plan_coverage(
            &app.conn,
            "project-a",
            "plan-a",
            &evaluated_at,
        )
        .unwrap();
        app.conn
            .execute(
                "INSERT INTO feature_run_source_freezes(
                   id, run_id, source_revision, source_digest, status, created_at, invalidated_at
                 ) VALUES ('freeze-historical', ?1, 'historical-revision',
                   'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                   'invalidated', '2000-01-01 00:00:00', '2000-01-02 00:00:00')",
                [&run.run.id],
            )
            .unwrap();
        app.conn
            .execute(
                "INSERT INTO review_gates(
                   id, run_id, scope_kind, scope_id, kind, status, responsible_maker_id,
                   latest_attempt, source_revision, accepted_at, created_at, updated_at
                 ) VALUES ('gate-lineage', ?1, 'plan', 'plan-a', 'final_product', 'accepted',
                   ?2, 2, ?3, datetime('now'), '2000-01-01 00:00:00', datetime('now'))",
                params![run.run.id, worker_id(), active.source_revision],
            )
            .unwrap();
        app.conn
            .execute(
                "INSERT INTO review_attempts(
                   id, gate_id, attempt_number, reviewer_worker_id, reviewer_mode, verdict,
                   source_revision, artifacts_json, created_at
                 ) VALUES ('attempt-lineage', 'gate-lineage', 2, 'independent-reviewer',
                   'independent', 'accepted', ?1, '[]', datetime('now'))",
                [&active.source_revision],
            )
            .unwrap();
        app.conn
            .execute(
                "INSERT INTO review_findings(
                   id, run_id, gate_id, attempt_id, severity, target, owner_worker_id, status,
                   invalidated_evidence_ids_json, created_at, resolved_at
                 ) VALUES ('finding-historical', ?1, 'gate-lineage', 'attempt-lineage', 'high',
                   'src/settlement.rs', ?2, 'resolved', '[\"pob-settle\"]',
                   '2000-01-01 00:00:00', '2000-01-03 00:00:00')",
                params![run.run.id, worker_id()],
            )
            .unwrap();
        app.conn
            .execute(
                "INSERT INTO feature_run_evidence_invalidations(
                   id, run_id, freeze_id, finding_id, reason, affected_evidence_ids_json, created_at
                 ) VALUES ('invalidation-historical', ?1, 'freeze-historical',
                   'finding-historical', 'final_review_product_finding', '[\"pob-settle\"]',
                   '2000-01-02 00:00:00')",
                [&run.run.id],
            )
            .unwrap();
        app.conn
            .execute(
                "INSERT INTO final_review_source_bindings(
                   gate_id, freeze_id, source_revision, source_digest, receipt_lineage_json
                 ) VALUES ('gate-lineage', ?1, ?2, ?3, ?4)",
                params![
                    active.id,
                    active.source_revision,
                    active.source_digest,
                    coverage.receipt_lineage.to_string(),
                ],
            )
            .unwrap();
        let input = json!({
            "schema": "planr.evidence.reconcile_historical_invalidation.v1",
            "plan_id": "plan-a",
            "run_id": run.run.id,
            "invalidation_id": "invalidation-historical",
            "superseding_freeze_id": active.id,
            "review_gate_id": "gate-lineage",
            "receipt_id": "erec-settle",
            "next_item_id": "item-after-verification",
        });
        (root, app, input)
    }

    fn risk_checkpoint_historical_invalidation_reconciliation_app()
    -> (tempfile::TempDir, App, Value) {
        let (root, app, input) = historical_invalidation_reconciliation_app();
        app.conn
            .execute(
                "UPDATE review_gates SET kind = 'risk_checkpoint',
                   accepted_at = '2000-01-03 00:00:00' WHERE id = 'gate-lineage'",
                [],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE evidence_receipts SET created_at = '2000-01-04 00:00:00'
                 WHERE id = 'erec-settle'",
                [],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE final_review_source_bindings SET receipt_lineage_json = ?1,
                   created_at = '2000-01-02 00:00:00'
                 WHERE gate_id = 'gate-lineage'",
                [json!({
                    "kind": "product_repair",
                    "repair_id": "invalidation-historical",
                    "selective_obligation_ids": ["pob-settle"]
                })
                .to_string()],
            )
            .unwrap();
        (root, app, input)
    }

    fn risk_review_obligation_backfill_app() -> (tempfile::TempDir, App, Value, Value) {
        let (root, app, historical_input) =
            risk_checkpoint_historical_invalidation_reconciliation_app();
        app.conn
            .execute(
                "INSERT INTO proof_obligations(
                   id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                   binding, observation_requirements_json, fixture_policy_json,
                   freshness_policy_json, assurance_policy_json, policy_digest, config_digest,
                   source_digest, supersedes_obligation_id, created_at, retry_aggregation,
                   obligation_shape
                 ) SELECT 'pob-settle-active', project_id, plan_id,
                   'item-settle-verification', criterion_id, obligation_version + 1, title,
                   binding, observation_requirements_json, fixture_policy_json,
                   freshness_policy_json, assurance_policy_json, policy_digest, config_digest,
                   source_digest, 'pob-settle', datetime('now'), retry_aggregation,
                   obligation_shape
                 FROM proof_obligations WHERE id = 'pob-settle'",
                [],
            )
            .unwrap();
        let receipt_json: String = app
            .conn
            .query_row(
                "SELECT receipt_json FROM evidence_receipts WHERE id = 'erec-settle'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut receipt: Value = serde_json::from_str(&receipt_json).unwrap();
        receipt["obligation_id"] = json!("pob-settle-active");
        let receipt_digest = crate::canonical_json::sha256_json_digest_without_top_level_field(
            &receipt,
            "receipt_digest",
        )
        .unwrap();
        receipt["receipt_digest"] = json!(receipt_digest);
        app.conn
            .execute(
                "UPDATE evidence_receipts SET obligation_id = 'pob-settle-active',
                   receipt_digest = ?1, receipt_json = ?2 WHERE id = 'erec-settle'",
                params![receipt_digest, receipt.to_string()],
            )
            .unwrap();
        let backfill_input = json!({
            "schema": "planr.evidence.backfill_risk_review_obligations.v1",
            "plan_id": "plan-a",
            "run_id": historical_input["run_id"],
            "review_gate_id": "gate-lineage",
            "freeze_id": historical_input["superseding_freeze_id"],
            "receipt_id": "erec-settle",
            "verification_item_id": "item-settle-verification",
        });
        (root, app, backfill_input, historical_input)
    }

    #[test]
    fn legacy_risk_review_backfill_seals_active_obligation_once_then_reconciles() {
        let (_root, app, backfill_input, historical_input) = risk_review_obligation_backfill_app();

        let backfilled = app
            .recover_verification_settlement_value(backfill_input.clone())
            .expect("exact legacy binding backfills to the active obligation");
        assert_eq!(backfilled["created"], true);
        assert_eq!(
            backfilled["schema"],
            "planr.evidence.backfill_risk_review_obligations.result.v1"
        );
        assert_eq!(
            backfilled["active_obligation_ids"],
            json!(["pob-settle-active"])
        );
        let binding = ExecutionRunRepository::new(&app.conn)
            .review_source_binding("gate-lineage")
            .unwrap()
            .unwrap();
        assert_eq!(
            binding.receipt_lineage,
            json!({
                "kind": "risk_review_acceptance",
                "active_obligation_ids": ["pob-settle-active"]
            })
        );
        let repeated = app
            .recover_verification_settlement_value(backfill_input)
            .expect("backfill repeats without another write");
        assert_eq!(repeated["created"], false);

        let reconciled = app
            .recover_verification_settlement_value(historical_input)
            .expect("reviewed backfill feeds the existing historical reconciliation");
        assert_eq!(reconciled["created"], true);
    }

    #[test]
    fn legacy_risk_review_backfill_fails_closed_for_ambiguous_or_stale_state() {
        for case in [
            "wrong_attempt_source",
            "post_receipt_acceptance",
            "wrong_freeze",
            "cross_plan_item",
            "multiple_active",
            "unrelated_supersession",
            "missing_coverage",
            "waived_coverage",
        ] {
            let (_root, app, mut input, _historical_input) = risk_review_obligation_backfill_app();
            match case {
                "wrong_attempt_source" => {
                    app.conn
                        .execute(
                            "UPDATE review_attempts SET source_revision = 'wrong-source'
                             WHERE id = 'attempt-lineage'",
                            [],
                        )
                        .unwrap();
                }
                "post_receipt_acceptance" => {
                    app.conn
                        .execute(
                            "UPDATE review_gates SET accepted_at = '2000-01-05 00:00:00'
                             WHERE id = 'gate-lineage'",
                            [],
                        )
                        .unwrap();
                }
                "wrong_freeze" => input["freeze_id"] = json!("freeze-wrong"),
                "cross_plan_item" => {
                    app.conn
                        .execute(
                            "INSERT INTO plans(id, project_id, stage, path, title, slug,
                               parse_status, content_hash, created_at, updated_at)
                             VALUES ('plan-b', 'project-a', 'build', 'plan-b.md', 'Plan B',
                               'plan-b', 'ok', 'sha256:plan-b', datetime('now'), datetime('now'))",
                            [],
                        )
                        .unwrap();
                    app.conn
                        .execute(
                            "INSERT INTO items(id, project_id, title, description, status,
                               work_type, plan_path, created_at, updated_at)
                             VALUES ('item-plan-b-verification', 'project-a', 'verify b', 'verify b',
                               'closed', 'verification', 'plan-b.md', datetime('now'), datetime('now'))",
                            [],
                        )
                        .unwrap();
                    input["verification_item_id"] = json!("item-plan-b-verification");
                }
                "multiple_active" => {
                    app.conn
                        .execute(
                            "INSERT INTO proof_obligations(
                               id, project_id, plan_id, item_id, criterion_id,
                               obligation_version, title, binding, observation_requirements_json,
                               fixture_policy_json, freshness_policy_json, assurance_policy_json,
                               policy_digest, config_digest, source_digest,
                               supersedes_obligation_id, created_at, retry_aggregation,
                               obligation_shape
                             ) SELECT 'pob-settle-other', project_id, plan_id,
                               'item-settle-verification', criterion_id, 1, title, binding,
                               observation_requirements_json, fixture_policy_json,
                               freshness_policy_json, assurance_policy_json, policy_digest,
                               config_digest, source_digest, NULL, datetime('now'),
                               retry_aggregation, obligation_shape
                             FROM proof_obligations WHERE id = 'pob-settle'",
                            [],
                        )
                        .unwrap();
                }
                "unrelated_supersession" => {
                    app.conn
                        .execute(
                            "UPDATE final_review_source_bindings SET receipt_lineage_json = ?1
                             WHERE gate_id = 'gate-lineage'",
                            [json!({
                                "kind": "product_repair",
                                "selective_obligation_ids": ["pob-unrelated"]
                            })
                            .to_string()],
                        )
                        .unwrap();
                }
                "missing_coverage" | "waived_coverage" => {
                    let observations: String = app
                        .conn
                        .query_row(
                            "SELECT observation_requirements_json FROM proof_obligations
                             WHERE id = 'pob-settle-active'",
                            [],
                            |row| row.get(0),
                        )
                        .unwrap();
                    let mut observations: Value = serde_json::from_str(&observations).unwrap();
                    let mut extra = observations[0].clone();
                    extra["id"] = json!("obs-risk-backfill-extra");
                    observations.as_array_mut().unwrap().push(extra.clone());
                    app.conn
                        .execute_batch("DROP TRIGGER proof_obligations_no_update")
                        .unwrap();
                    app.conn
                        .execute(
                            "UPDATE proof_obligations SET observation_requirements_json = ?1
                             WHERE id = 'pob-settle-active'",
                            [observations.to_string()],
                        )
                        .unwrap();
                    if case == "waived_coverage" {
                        let source = capture_repository_snapshot(&app.root).unwrap().source;
                        let waiver = json!({
                            "id": "waiver-risk-backfill",
                            "schema_version": crate::evidence::model::EVIDENCE_CONTRACT_V1,
                            "scope": {"kind": "plan", "id": "plan-a"},
                            "observation_ids": ["obs-risk-backfill-extra"],
                            "source": source,
                            "target": extra["target"],
                            "reason": "risk backfill waiver must not authorize mutation",
                            "created_by": "reviewer",
                            "created_at": "2026-08-08T00:00:00Z",
                            "expires_at": "2099-01-01T00:00:00Z",
                            "approval_ref": "item-risk-backfill-waiver-approval",
                            "audit_trail": [
                                {"event": "created", "at": "2026-08-08T00:00:00Z"}
                            ]
                        });
                        let waiver_digest =
                            crate::canonical_json::sha256_json_digest(&waiver).unwrap();
                        app.conn
                            .execute(
                                "INSERT INTO items(
                                   id, project_id, title, description, status, work_type,
                                   worker_id, plan_path, approval_status, approved_by, created_at,
                                   updated_at, completed_at
                                 ) VALUES ('item-risk-backfill-waiver-approval', 'project-a',
                                   'waiver', 'waiver', 'closed', 'approval', 'reviewer',
                                   'plan-a.md', 'approved', 'reviewer', datetime('now'),
                                   datetime('now'), datetime('now'))",
                                [],
                            )
                            .unwrap();
                        app.conn
                            .execute(
                                "INSERT INTO evidence_waivers(
                                   id, project_id, approval_item_id, obligation_id,
                                   observation_id, scope_kind, scope_id, waiver_digest, reason,
                                   expires_at, created_by, waiver_json, created_at
                                 ) VALUES ('waiver-risk-backfill', 'project-a',
                                   'item-risk-backfill-waiver-approval', 'pob-settle-active',
                                   'obs-risk-backfill-extra', 'plan', 'plan-a', ?1,
                                   'risk backfill waiver must not authorize mutation',
                                   '2099-01-01T00:00:00Z', 'reviewer', ?2,
                                   '2026-08-08T00:00:00Z')",
                                params![waiver_digest, waiver.to_string()],
                            )
                            .unwrap();
                    }
                }
                _ => unreachable!(),
            }
            let error = app
                .recover_verification_settlement_value(input)
                .expect_err("ambiguous legacy state must fail closed");
            let expected = match case {
                "wrong_attempt_source" => "attempt_source_mismatch",
                "post_receipt_acceptance" => "acceptance_not_pre_receipt",
                "wrong_freeze" => "source_binding_mismatch",
                "cross_plan_item" => "verification_item_mismatch",
                "multiple_active" => "active_obligations_ambiguous",
                "unrelated_supersession" => "supersession_mismatch",
                "missing_coverage" | "waived_coverage" => "coverage_mismatch",
                _ => unreachable!(),
            };
            assert!(error.to_string().contains(expected), "case {case}: {error}");
            let events: i64 = app
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM events
                     WHERE event_type = 'risk_review_obligations_backfilled'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(events, 0, "case {case}");
        }
    }

    #[test]
    fn concurrent_legacy_risk_review_backfill_converges_once() {
        let (root, app, input, _historical_input) = risk_review_obligation_backfill_app();
        let database_path = app.db_path.clone();
        drop(app);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let database_path = database_path.clone();
            let repository_root = root.path().to_path_buf();
            let input = input.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                let conn = Connection::open(&database_path).unwrap();
                conn.busy_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                let app = App::new(conn, repository_root, database_path, true, false);
                barrier.wait();
                app.recover_verification_settlement_value(input).unwrap()["created"]
                    .as_bool()
                    .unwrap()
            }));
        }
        let mut created = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        created.sort();
        assert_eq!(created, vec![false, true]);
        let conn = Connection::open(database_path).unwrap();
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events
                 WHERE event_type = 'risk_review_obligations_backfilled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 1);
    }

    #[test]
    fn legacy_risk_review_backfill_rolls_back_late_failure() {
        let (_root, app, input, _historical_input) = risk_review_obligation_backfill_app();
        let before = ExecutionRunRepository::new(&app.conn)
            .review_source_binding("gate-lineage")
            .unwrap()
            .unwrap();
        app.conn
            .execute_batch(
                "CREATE TRIGGER reject_risk_review_backfill
                 BEFORE INSERT ON events
                 WHEN NEW.event_type = 'risk_review_obligations_backfilled'
                 BEGIN SELECT RAISE(ABORT, 'forced risk review backfill failure'); END;",
            )
            .unwrap();
        let error = app
            .recover_verification_settlement_value(input)
            .expect_err("late event failure rolls back binding update");
        assert!(
            error
                .to_string()
                .contains("forced risk review backfill failure")
        );
        let after = ExecutionRunRepository::new(&app.conn)
            .review_source_binding("gate-lineage")
            .unwrap()
            .unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn legacy_risk_review_backfill_mcp_and_http_share_typed_result() {
        let (_mcp_root, mcp_app, mcp_input, _) = risk_review_obligation_backfill_app();
        let mcp = mcp_app
            .mcp_evidence_tool_call(
                "planr_evidence_recover_settlement",
                json!({"input": mcp_input}),
            )
            .unwrap();
        let mcp_envelope: Value =
            serde_json::from_str(mcp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            mcp_envelope["object"]["schema"],
            "planr.evidence.backfill_risk_review_obligations.result.v1"
        );
        assert_eq!(mcp_envelope["object"]["created"], true);

        let (_http_root, http_app, http_input, _) = risk_review_obligation_backfill_app();
        let (status, body) = http_app
            .http_evidence_route(
                "POST",
                "/v1/evidence/recover-settlement",
                "",
                &json!({"input": http_input}),
            )
            .unwrap();
        let http_envelope: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(status, "200 OK");
        assert_eq!(
            http_envelope["object"]["schema"],
            mcp_envelope["object"]["schema"]
        );
        assert_eq!(http_envelope["object"]["created"], true);
    }

    #[test]
    fn exact_equal_timestamp_causal_refreeze_reconciles_once_and_leaves_ordinary_work() {
        let (_root, app, input) = historical_invalidation_reconciliation_app();
        let causal_boundary: (String, String, String) = app
            .conn
            .query_row(
                "SELECT old.invalidated_at, invalidation.created_at, active.created_at
                 FROM feature_run_evidence_invalidations invalidation
                 JOIN feature_run_source_freezes old ON old.id = invalidation.freeze_id
                 JOIN feature_run_source_freezes active ON active.id = ?1
                 WHERE invalidation.id = ?2",
                params![
                    input["superseding_freeze_id"].as_str().unwrap(),
                    input["invalidation_id"].as_str().unwrap(),
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(causal_boundary.0, causal_boundary.1);
        assert_eq!(causal_boundary.1, causal_boundary.2);
        let before = app
            .repair_work_packet_value("plan-a")
            .expect("historical invalidation has a canonical repair packet")
            .expect("unreconciled historical invalidation remains repairable");
        assert_eq!(before["work_packet"]["kind"], "outcome");
        assert_eq!(before["work_packet"]["mode"], "product_finding_repair");
        assert_eq!(before["work_packet"]["repair_id"], input["invalidation_id"]);
        assert!(before["work_packet"]["verification_item_id"].is_null());

        let reconciled = app
            .recover_verification_settlement_value(input.clone())
            .expect("equal-timestamp causal lineage reconciles historical invalidation");
        assert_eq!(reconciled["created"], true);
        assert_eq!(
            reconciled["schema"],
            "planr.evidence.reconcile_historical_invalidation.result.v1"
        );
        assert!(app.repair_work_packet_value("plan-a").unwrap().is_none());

        let repeated = app
            .recover_verification_settlement_value(input)
            .expect("repeat converges without another write");
        assert_eq!(repeated["created"], false);
        let events: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM events
                 WHERE event_type = 'historical_invalidation_reconciled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 1);
    }

    #[test]
    fn pre_evidence_risk_checkpoint_reconciles_only_its_reviewed_obligation() {
        let (_root, app, input) = risk_checkpoint_historical_invalidation_reconciliation_app();

        let reconciled = app
            .recover_verification_settlement_value(input.clone())
            .expect("later trusted receipt proves the exact pre-Evidence reviewed obligation");
        assert_eq!(reconciled["created"], true);

        let repeated = app
            .recover_verification_settlement_value(input)
            .expect("risk checkpoint reconciliation remains idempotent");
        assert_eq!(repeated["created"], false);
    }

    #[test]
    fn pre_evidence_risk_checkpoint_normalizes_timezone_and_fraction_boundaries() {
        let (_root, app, input) = risk_checkpoint_historical_invalidation_reconciliation_app();
        app.conn
            .execute(
                "UPDATE review_gates SET accepted_at = '2000-01-04T10:59:59.999999999+01:00'
                 WHERE id = 'gate-lineage'",
                [],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE final_review_source_bindings SET created_at = '2000-01-04 09:59:59'
                 WHERE gate_id = 'gate-lineage'",
                [],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE evidence_receipts SET created_at = '2000-01-04T10:00:00Z'
                 WHERE id = 'erec-settle'",
                [],
            )
            .unwrap();

        let reconciled = app
            .recover_verification_settlement_value(input)
            .expect("offset and fractional timestamps normalize to instants before comparison");
        assert_eq!(reconciled["created"], true);
    }

    #[test]
    fn repaired_risk_checkpoint_preserves_reviewed_obligation_through_supersession_lineage() {
        let (_root, app, input) = risk_checkpoint_historical_invalidation_reconciliation_app();
        app.conn
            .execute(
                "UPDATE final_review_source_bindings SET receipt_lineage_json = ?1
                 WHERE gate_id = 'gate-lineage'",
                [json!({
                    "kind": "risk_review_finding_repair",
                    "finding_ids": ["finding-historical"],
                    "supersedes": {
                        "kind": "product_repair",
                        "repair_id": "invalidation-historical",
                        "selective_obligation_ids": ["pob-settle"]
                    }
                })
                .to_string()],
            )
            .unwrap();

        let reconciled = app
            .recover_verification_settlement_value(input)
            .expect("risk finding repair keeps the pre-Evidence reviewed obligation lineage");
        assert_eq!(reconciled["created"], true);
    }

    #[test]
    fn final_review_still_requires_exact_covering_receipt_lineage() {
        let (_root, app, input) = historical_invalidation_reconciliation_app();
        app.conn
            .execute(
                "UPDATE final_review_source_bindings SET receipt_lineage_json = ?1
                 WHERE gate_id = 'gate-lineage'",
                [json!({
                    "kind": "product_repair",
                    "selective_obligation_ids": ["pob-settle"]
                })
                .to_string()],
            )
            .unwrap();

        let error = app
            .recover_verification_settlement_value(input)
            .expect_err("pre-Evidence risk lineage cannot stand in for final receipt lineage");
        assert!(
            error
                .to_string()
                .contains("review_receipt_lineage_mismatch")
        );
    }

    #[test]
    fn risk_checkpoint_reconciliation_rejects_unproven_obligation_lineage() {
        for case in [
            "empty",
            "unknown",
            "receipt_from_another_obligation",
            "superseded",
            "post_evidence_lineage",
            "post_evidence_binding",
            "same_day_post_evidence_acceptance_mixed_format",
            "same_day_post_evidence_binding_mixed_format",
            "equal_instant_timezone_boundary",
            "fractional_post_evidence_boundary",
            "invalid_acceptance_timestamp",
            "invalid_binding_timestamp",
            "invalid_receipt_timestamp",
        ] {
            let (_root, app, input) = risk_checkpoint_historical_invalidation_reconciliation_app();
            match case {
                "empty" => {
                    app.conn
                        .execute(
                            "UPDATE final_review_source_bindings SET receipt_lineage_json = ?1
                             WHERE gate_id = 'gate-lineage'",
                            [json!({
                                "kind": "product_repair",
                                "selective_obligation_ids": []
                            })
                            .to_string()],
                        )
                        .unwrap();
                }
                "unknown" => {
                    app.conn
                        .execute(
                            "UPDATE final_review_source_bindings SET receipt_lineage_json = ?1
                             WHERE gate_id = 'gate-lineage'",
                            [json!({
                                "kind": "product_repair",
                                "selective_obligation_ids": ["pob-unknown"]
                            })
                            .to_string()],
                        )
                        .unwrap();
                }
                "receipt_from_another_obligation" => {
                    insert_test_obligation_successor(&app, "pob-other", None);
                    app.conn
                        .execute(
                            "UPDATE evidence_receipts SET obligation_id = 'pob-other'
                             WHERE id = 'erec-settle'",
                            [],
                        )
                        .unwrap();
                }
                "superseded" => {
                    insert_test_obligation_successor(
                        &app,
                        "pob-settle-successor",
                        Some("pob-settle"),
                    );
                }
                "post_evidence_lineage" => {
                    let coverage = crate::evidence::coverage::evaluate_plan_coverage(
                        &app.conn,
                        "project-a",
                        "plan-a",
                        &OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),
                    )
                    .unwrap();
                    app.conn
                        .execute(
                            "UPDATE final_review_source_bindings SET receipt_lineage_json = ?1
                             WHERE gate_id = 'gate-lineage'",
                            [coverage.receipt_lineage.to_string()],
                        )
                        .unwrap();
                }
                "post_evidence_binding" => {
                    app.conn
                        .execute(
                            "UPDATE final_review_source_bindings
                             SET created_at = '2000-01-05 00:00:00'
                             WHERE gate_id = 'gate-lineage'",
                            [],
                        )
                        .unwrap();
                }
                "same_day_post_evidence_acceptance_mixed_format" => {
                    app.conn
                        .execute(
                            "UPDATE review_gates SET accepted_at = '2000-01-04 11:00:00'
                             WHERE id = 'gate-lineage'",
                            [],
                        )
                        .unwrap();
                    app.conn
                        .execute(
                            "UPDATE evidence_receipts SET created_at = '2000-01-04T10:00:00Z'
                             WHERE id = 'erec-settle'",
                            [],
                        )
                        .unwrap();
                }
                "same_day_post_evidence_binding_mixed_format" => {
                    app.conn
                        .execute(
                            "UPDATE review_gates SET accepted_at = '2000-01-04 09:00:00'
                             WHERE id = 'gate-lineage'",
                            [],
                        )
                        .unwrap();
                    app.conn
                        .execute(
                            "UPDATE final_review_source_bindings SET created_at = '2000-01-04 11:00:00'
                             WHERE gate_id = 'gate-lineage'",
                            [],
                        )
                        .unwrap();
                    app.conn
                        .execute(
                            "UPDATE evidence_receipts SET created_at = '2000-01-04T10:00:00Z'
                             WHERE id = 'erec-settle'",
                            [],
                        )
                        .unwrap();
                }
                "equal_instant_timezone_boundary" => {
                    app.conn
                        .execute(
                            "UPDATE review_gates SET accepted_at = '2000-01-04T11:00:00+01:00'
                             WHERE id = 'gate-lineage'",
                            [],
                        )
                        .unwrap();
                    app.conn
                        .execute(
                            "UPDATE evidence_receipts SET created_at = '2000-01-04T10:00:00Z'
                             WHERE id = 'erec-settle'",
                            [],
                        )
                        .unwrap();
                }
                "fractional_post_evidence_boundary" => {
                    app.conn
                        .execute(
                            "UPDATE review_gates SET accepted_at = '2000-01-04T09:59:59.999999999Z'
                             WHERE id = 'gate-lineage'",
                            [],
                        )
                        .unwrap();
                    app.conn
                        .execute(
                            "UPDATE final_review_source_bindings
                             SET created_at = '2000-01-04T10:00:00.000000001Z'
                             WHERE gate_id = 'gate-lineage'",
                            [],
                        )
                        .unwrap();
                    app.conn
                        .execute(
                            "UPDATE evidence_receipts SET created_at = '2000-01-04T10:00:00Z'
                             WHERE id = 'erec-settle'",
                            [],
                        )
                        .unwrap();
                }
                "invalid_acceptance_timestamp" => {
                    app.conn
                        .execute(
                            "UPDATE review_gates SET accepted_at = 'not-a-timestamp'
                             WHERE id = 'gate-lineage'",
                            [],
                        )
                        .unwrap();
                }
                "invalid_binding_timestamp" => {
                    app.conn
                        .execute(
                            "UPDATE final_review_source_bindings SET created_at = '2000-01-04'
                             WHERE gate_id = 'gate-lineage'",
                            [],
                        )
                        .unwrap();
                }
                "invalid_receipt_timestamp" => {
                    app.conn
                        .execute(
                            "UPDATE evidence_receipts SET created_at = '2000-01-04 10:00:00 UTC'
                             WHERE id = 'erec-settle'",
                            [],
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }

            let error = app
                .recover_verification_settlement_value(input)
                .expect_err("risk obligation lineage must fail closed");
            let expected = match case {
                "empty" => "risk_review_obligations_empty",
                "unknown" | "superseded" => "risk_reviewed_obligation_inactive",
                "receipt_from_another_obligation" => "risk_receipt_obligation_mismatch",
                "post_evidence_lineage" => "risk_review_lineage_ambiguous",
                "post_evidence_binding"
                | "same_day_post_evidence_acceptance_mixed_format"
                | "same_day_post_evidence_binding_mixed_format"
                | "equal_instant_timezone_boundary"
                | "fractional_post_evidence_boundary" => "risk_review_not_pre_evidence",
                "invalid_acceptance_timestamp" => "risk_review_acceptance_invalid",
                "invalid_binding_timestamp" => "risk_review_binding_timestamp_invalid",
                "invalid_receipt_timestamp" => "risk_receipt_timestamp_invalid",
                _ => unreachable!(),
            };
            assert!(error.to_string().contains(expected), "case {case}: {error}");
            let events: i64 = app
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM events
                     WHERE event_type = 'historical_invalidation_reconciled'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(events, 0, "case {case}");
        }
    }

    fn insert_test_obligation_successor(app: &App, id: &str, supersedes: Option<&str>) {
        app.conn
            .execute(
                "INSERT INTO proof_obligations(
                   id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                   binding, observation_requirements_json, fixture_policy_json,
                   freshness_policy_json, assurance_policy_json, policy_digest, config_digest,
                   source_digest, supersedes_obligation_id, created_at, retry_aggregation,
                   obligation_shape
                 ) SELECT ?1, project_id, plan_id, item_id, criterion_id,
                   obligation_version + 1, title, binding, observation_requirements_json,
                   fixture_policy_json, freshness_policy_json, assurance_policy_json,
                   policy_digest, config_digest, source_digest, ?2, datetime('now'),
                   retry_aggregation, obligation_shape
                 FROM proof_obligations WHERE id = 'pob-settle'",
                params![id, supersedes],
            )
            .unwrap();
    }

    #[test]
    fn historical_invalidation_reconciliation_fails_closed_for_unproven_lineage() {
        for case in [
            "genuine_product_repair",
            "unresolved_finding",
            "older_active_freeze",
            "future_active_freeze",
            "mismatched_invalidation_boundary",
            "unrelated_superseding_freeze",
            "stale_source",
            "review_source_mismatch",
            "missing_receipt",
            "receipt_source_mismatch",
            "waived_coverage",
        ] {
            let (_root, app, mut input) = historical_invalidation_reconciliation_app();
            match case {
                "genuine_product_repair" => {
                    app.conn
                        .execute(
                            "UPDATE feature_run_evidence_invalidations SET finding_id = NULL
                             WHERE id = 'invalidation-historical'",
                            [],
                        )
                        .unwrap();
                }
                "unresolved_finding" => {
                    app.conn
                        .execute(
                            "UPDATE review_findings SET status = 'open', resolved_at = NULL
                             WHERE id = 'finding-historical'",
                            [],
                        )
                        .unwrap();
                }
                "older_active_freeze" => {
                    app.conn
                        .execute(
                            "UPDATE feature_run_source_freezes
                             SET created_at = '2000-01-01 00:00:00'
                             WHERE id = ?1",
                            [input["superseding_freeze_id"].as_str().unwrap()],
                        )
                        .unwrap();
                }
                "future_active_freeze" => {
                    app.conn
                        .execute(
                            "UPDATE feature_run_source_freezes
                             SET created_at = '2000-01-03 00:00:00'
                             WHERE id = ?1",
                            [input["superseding_freeze_id"].as_str().unwrap()],
                        )
                        .unwrap();
                }
                "mismatched_invalidation_boundary" => {
                    app.conn
                        .execute(
                            "UPDATE feature_run_source_freezes
                             SET invalidated_at = '2000-01-03 00:00:00'
                             WHERE id = 'freeze-historical'",
                            [],
                        )
                        .unwrap();
                }
                "unrelated_superseding_freeze" => {
                    app.conn
                        .execute(
                            "INSERT INTO feature_run_source_freezes(
                               id, run_id, source_revision, source_digest, status, created_at,
                               invalidated_at
                             ) SELECT 'freeze-unrelated', run_id, source_revision, source_digest,
                               'invalidated', created_at, created_at
                               FROM feature_run_source_freezes WHERE id = ?1",
                            [input["superseding_freeze_id"].as_str().unwrap()],
                        )
                        .unwrap();
                    input["superseding_freeze_id"] = json!("freeze-unrelated");
                }
                "stale_source" => std::fs::write(app.root.join("stale.txt"), "stale").unwrap(),
                "review_source_mismatch" => {
                    app.conn
                        .execute(
                            "UPDATE final_review_source_bindings SET source_digest =
                             'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
                             WHERE gate_id = 'gate-lineage'",
                            [],
                        )
                        .unwrap();
                }
                "missing_receipt" => input["receipt_id"] = json!("missing-receipt"),
                "receipt_source_mismatch" => {
                    app.conn
                        .execute(
                            "UPDATE evidence_receipts SET trusted_binding_json =
                             json_set(trusted_binding_json, '$.source.revision', 'wrong-revision')
                             WHERE id = 'erec-settle'",
                            [],
                        )
                        .unwrap();
                }
                "waived_coverage" => {
                    let observations: String = app
                        .conn
                        .query_row(
                            "SELECT observation_requirements_json FROM proof_obligations
                             WHERE id = 'pob-settle'",
                            [],
                            |row| row.get(0),
                        )
                        .unwrap();
                    let mut observations: Value = serde_json::from_str(&observations).unwrap();
                    let mut waived = observations[0].clone();
                    waived["id"] = json!("obs-settle-waived");
                    observations.as_array_mut().unwrap().push(waived);
                    app.conn
                        .execute_batch("DROP TRIGGER proof_obligations_no_update")
                        .unwrap();
                    app.conn
                        .execute(
                            "UPDATE proof_obligations SET observation_requirements_json = ?1
                             WHERE id = 'pob-settle'",
                            [observations.to_string()],
                        )
                        .unwrap();
                    let source = capture_repository_snapshot(&app.root).unwrap().source;
                    let waiver = json!({
                        "id": "waiver-settle-lineage",
                        "schema_version": crate::evidence::model::EVIDENCE_CONTRACT_V1,
                        "scope": {"kind": "plan", "id": "plan-a"},
                        "observation_ids": ["obs-settle-waived"],
                        "source": source,
                        "target": {"kind": "process", "uri": "local://ready"},
                        "reason": "lineage test waiver",
                        "created_by": "reviewer",
                        "created_at": "2026-08-08T00:00:00Z",
                        "expires_at": "2099-01-01T00:00:00Z",
                        "approval_ref": "item-lineage-waiver-approval",
                        "audit_trail": [{"event": "created", "at": "2026-08-08T00:00:00Z"}]
                    });
                    let waiver_digest = crate::canonical_json::sha256_json_digest(&waiver).unwrap();
                    app.conn
                        .execute(
                            "INSERT INTO items(
                               id, project_id, title, description, status, work_type, worker_id,
                               plan_path, approval_status, approved_by, created_at, updated_at,
                               completed_at
                             ) VALUES ('item-lineage-waiver-approval', 'project-a', 'waiver',
                               'waiver', 'closed', 'approval', 'reviewer', 'plan-a.md', 'approved',
                               'reviewer', datetime('now'), datetime('now'), datetime('now'))",
                            [],
                        )
                        .unwrap();
                    app.conn
                        .execute(
                            "INSERT INTO evidence_waivers(
                               id, project_id, approval_item_id, obligation_id, observation_id,
                               scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
                               waiver_json, created_at
                             ) VALUES ('waiver-settle-lineage', 'project-a',
                               'item-lineage-waiver-approval', 'pob-settle', 'obs-settle-waived',
                               'plan', 'plan-a', ?1, 'lineage test waiver',
                               '2099-01-01T00:00:00Z', 'reviewer', ?2,
                               '2026-08-08T00:00:00Z')",
                            params![waiver_digest, waiver.to_string()],
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }
            app.recover_verification_settlement_value(input)
                .unwrap_err();
            let events: i64 = app
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM events
                     WHERE event_type = 'historical_invalidation_reconciled'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(events, 0, "case {case}");
        }
    }

    #[test]
    fn concurrent_historical_invalidation_reconciliation_converges_once() {
        let (root, app, input) = historical_invalidation_reconciliation_app();
        let database_path = app.db_path.clone();
        drop(app);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let database_path = database_path.clone();
            let repository_root = root.path().to_path_buf();
            let input = input.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                let conn = Connection::open(&database_path).unwrap();
                conn.busy_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                let app = App::new(conn, repository_root, database_path, true, false);
                barrier.wait();
                app.recover_verification_settlement_value(input).unwrap()["created"]
                    .as_bool()
                    .unwrap()
            }));
        }
        let mut created = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        created.sort();
        assert_eq!(created, vec![false, true]);

        let conn = Connection::open(database_path).unwrap();
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events
                 WHERE event_type = 'historical_invalidation_reconciled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 1);
    }

    #[test]
    fn historical_invalidation_reconciliation_rolls_back_late_failure() {
        let (_root, app, input) = historical_invalidation_reconciliation_app();
        app.conn
            .execute_batch(
                "CREATE TRIGGER reject_historical_reconciliation
                 BEFORE INSERT ON events
                 WHEN NEW.event_type = 'historical_invalidation_reconciled'
                 BEGIN SELECT RAISE(ABORT, 'forced historical reconciliation failure'); END;",
            )
            .unwrap();

        let error = app
            .recover_verification_settlement_value(input)
            .expect_err("late event write failure rolls back");
        assert!(
            error
                .to_string()
                .contains("forced historical reconciliation failure")
        );
        assert!(app.repair_work_packet_value("plan-a").is_err());
        let events: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM events
                 WHERE event_type = 'historical_invalidation_reconciled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 0);
    }

    #[test]
    fn historical_invalidation_reconciliation_mcp_and_http_share_typed_result() {
        let (_mcp_root, mcp_app, mcp_input) = historical_invalidation_reconciliation_app();
        let mcp = mcp_app
            .mcp_evidence_tool_call(
                "planr_evidence_recover_settlement",
                json!({"input": mcp_input}),
            )
            .unwrap();
        let mcp_envelope: Value =
            serde_json::from_str(mcp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            mcp_envelope["object"]["schema"],
            "planr.evidence.reconcile_historical_invalidation.result.v1"
        );
        assert_eq!(mcp_envelope["object"]["created"], true);

        let (_http_root, http_app, http_input) = historical_invalidation_reconciliation_app();
        let (status, body) = http_app
            .http_evidence_route(
                "POST",
                "/v1/evidence/recover-settlement",
                "",
                &json!({"input": http_input}),
            )
            .unwrap();
        let http_envelope: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(status, "200 OK");
        assert_eq!(
            http_envelope["object"]["schema"],
            mcp_envelope["object"]["schema"]
        );
        assert_eq!(http_envelope["object"]["created"], true);
    }

    #[test]
    fn exact_stranded_settlement_recovery_is_atomic_and_idempotent() {
        let (_root, app, input) = stranded_recovery_app();

        let recovered = app
            .recover_verification_settlement_value(input.clone())
            .expect("recover split verifier and maker state");
        assert_eq!(recovered["created"], true);
        assert_eq!(
            recovered["execution_state"]["feature_run"]["phase"],
            "implementation"
        );
        assert_eq!(
            recovered["execution_state"]["feature_run"]["role_owners"][0]["role"],
            "maker"
        );
        assert_eq!(
            recovered["execution_state"]["feature_run"]["role_owners"][0]["worker_id"],
            worker_id()
        );
        let item: (String, String) = app
            .conn
            .query_row(
                "SELECT status, worker_id FROM items WHERE id = 'item-after-verification'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(item, ("picked".into(), worker_id()));

        let repeated = app
            .recover_verification_settlement_value(input)
            .expect("repeat converges");
        assert_eq!(repeated["created"], false);
        let events: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'verification_settlement_recovered'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 1);
    }

    #[test]
    fn final_recovery_created_outcome_reuses_verified_lineage_and_returns_to_final_review() {
        let (_root, app, input) = stranded_recovery_app();
        app.recover_verification_settlement_value(input)
            .expect("restore verified continuation");
        let attempts_before: i64 = app
            .conn
            .query_row("SELECT COUNT(*) FROM evidence_attempts", [], |row| {
                row.get(0)
            })
            .unwrap();
        let non_material =
            json!({"decision": {"material": false, "review": "none", "reasons": []}});
        let settled = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-after-verification",
                summary: "finish verified continuation",
                materiality: &non_material,
                escalation: None,
            })
            .expect("terminal continuation settlement");
        assert_eq!(settled["transition"], "verified_continuation_complete");
        assert_eq!(
            settled["execution_state"]["feature_run"]["phase"],
            "source_frozen"
        );
        let (attempts_after, completions): (i64, i64) = app
            .conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM evidence_attempts),
                        (SELECT COUNT(*) FROM events WHERE event_type = 'verified_continuation_completed')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(attempts_after, attempts_before);
        assert_eq!(completions, 1);
        app.conn
            .execute(
                "UPDATE items SET status = 'closed', worker_id = NULL WHERE id = 'item-after-verification'",
                [],
            )
            .unwrap();
        let run = ExecutionRunRepository::new(&app.conn)
            .active_feature_run_for_plan("project-a", "plan-a")
            .unwrap()
            .unwrap();
        assert_eq!(run.run.phase, FeatureRunPhase::SourceFrozen);
        assert!(run.run.role_owners.is_empty());
    }

    #[test]
    fn exact_public_recovery_completes_a_legacy_stranded_continuation_once() {
        let (_root, app, input) = terminal_verified_continuation_app();
        let recovered = app
            .recover_verification_settlement_value(input.clone())
            .expect("public terminal recovery");
        assert_eq!(recovered["created"], true);
        assert_eq!(
            recovered["execution_state"]["feature_run"]["phase"],
            "source_frozen"
        );
        let repeated = app
            .recover_verification_settlement_value(input)
            .expect("public terminal recovery repeats safely");
        assert_eq!(repeated["created"], false);
        let completions: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'verified_continuation_completed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(completions, 1);
    }

    fn assert_verified_continuation_recovery_rolled_back(app: &App) {
        let state: (String, String, i64) = app
            .conn
            .query_row(
                "SELECT runs.phase, batches.status,
                    (SELECT COUNT(*) FROM events WHERE event_type = 'verified_continuation_completed')
                 FROM feature_runs runs JOIN execution_batches batches
                   ON batches.id = runs.active_batch_id WHERE runs.plan_id = 'plan-a'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, ("implementation".into(), "active".into(), 0));
    }

    #[test]
    fn verified_continuation_public_recovery_fails_closed_for_exact_lineage_and_proof_gaps() {
        for case in [
            "stale_source",
            "missing_freeze",
            "wrong_freeze",
            "missing_receipt",
            "wrong_receipt",
            "receipt_source",
            "recovery_lineage",
            "missing_verification",
            "open_verification",
            "active_adapter",
            "remaining_work",
            "waived_coverage",
            "missing_coverage",
            "wrong_maker",
            "wrong_run",
            "wrong_plan",
        ] {
            let (root, app, mut input) = terminal_verified_continuation_app();
            match case {
                "stale_source" => std::fs::write(root.path().join("stale.txt"), "stale").unwrap(),
                "missing_freeze" => {
                    app.conn.execute("UPDATE feature_run_source_freezes SET status = 'invalidated', invalidated_at = datetime('now') WHERE id = ?1", [input["freeze_id"].as_str().unwrap()]).unwrap();
                }
                "wrong_freeze" => input["freeze_id"] = json!("freeze-wrong"),
                "missing_receipt" => {
                    app.conn
                        .execute_batch("DROP TRIGGER evidence_receipts_no_delete")
                        .unwrap();
                    app.conn
                        .execute("DELETE FROM evidence_receipts WHERE id = 'erec-settle'", [])
                        .unwrap();
                }
                "wrong_receipt" => input["receipt_id"] = json!("erec-wrong"),
                "receipt_source" => {
                    app.conn
                        .execute_batch("DROP TRIGGER evidence_receipts_no_update")
                        .ok();
                    let binding: String = app.conn.query_row(
                        "SELECT trusted_binding_json FROM evidence_receipts WHERE id = 'erec-settle'",
                        [], |row| row.get(0)).unwrap();
                    let mut binding: Value = serde_json::from_str(&binding).unwrap();
                    binding["source"]["revision"] = json!("wrong-revision");
                    app.conn.execute("UPDATE evidence_receipts SET trusted_binding_json = ?1 WHERE id = 'erec-settle'", [binding.to_string()]).unwrap();
                }
                "recovery_lineage" => input["recovery_item_id"] = json!("other-item"),
                "missing_verification" => {
                    app.conn.execute("UPDATE items SET work_type = 'docs' WHERE id = 'item-settle-verification'", []).unwrap();
                }
                "open_verification" => {
                    app.conn.execute("UPDATE items SET status = 'ready' WHERE id = 'item-settle-verification'", []).unwrap();
                }
                "active_adapter" => {
                    app.conn
                        .execute_batch("DROP TRIGGER evidence_attempts_no_update")
                        .unwrap();
                    app.conn.execute("UPDATE evidence_attempts SET completed_at = NULL WHERE id = 'eatt-settle'", []).unwrap();
                }
                "remaining_work" => {
                    app.conn.execute("INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at) VALUES ('remaining-public', 'project-a', 'remaining', 'remaining', 'ready', 'code', 'plan-a.md', datetime('now'), datetime('now'))", []).unwrap();
                }
                "waived_coverage" => {
                    app.conn.execute("UPDATE coverage_verdicts SET waiver_digest_set = '[\"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"]' WHERE scope_kind = 'plan' AND scope_id = 'plan-a'", []).unwrap();
                }
                "missing_coverage" => {
                    app.conn.execute("UPDATE coverage_verdicts SET scope_id = 'other-plan' WHERE scope_kind = 'plan' AND scope_id = 'plan-a'", []).unwrap();
                }
                "wrong_maker" => {
                    app.conn.execute("UPDATE feature_run_role_leases SET worker_id = 'wrong-maker' WHERE run_id = ?1 AND role = 'maker' AND released_at IS NULL", [input["run_id"].as_str().unwrap()]).unwrap();
                }
                "wrong_run" => input["run_id"] = json!("run-wrong"),
                "wrong_plan" => input["plan_id"] = json!("plan-wrong"),
                _ => unreachable!(),
            }
            app.recover_verification_settlement_value(input)
                .expect_err(case);
            assert_verified_continuation_recovery_rolled_back(&app);
        }
    }

    #[test]
    fn concurrent_verified_continuation_public_recovery_converges_once() {
        let (root, app, input) = terminal_verified_continuation_app();
        let database_path = app.db_path.clone();
        drop(app);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let database_path = database_path.clone();
            let repository_root = root.path().to_path_buf();
            let input = input.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                let conn = Connection::open(&database_path).unwrap();
                conn.busy_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                let app = App::new(conn, repository_root, database_path, true, false);
                barrier.wait();
                app.recover_verification_settlement_value(input).unwrap()["created"]
                    .as_bool()
                    .unwrap()
            }));
        }
        let mut created = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        created.sort();
        assert_eq!(created, vec![false, true]);
        let conn = Connection::open(database_path).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'verified_continuation_completed'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn verified_continuation_public_recovery_rolls_back_late_failure() {
        let (_root, app, input) = terminal_verified_continuation_app();
        app.conn
            .execute_batch(
                "CREATE TRIGGER reject_verified_continuation_event BEFORE INSERT ON events
             WHEN NEW.event_type = 'verified_continuation_completed'
             BEGIN SELECT RAISE(ABORT, 'forced verified continuation event failure'); END;",
            )
            .unwrap();
        let error = app
            .recover_verification_settlement_value(input)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("forced verified continuation event failure")
        );
        assert_verified_continuation_recovery_rolled_back(&app);
    }

    #[test]
    fn verified_continuation_recovery_cli_mcp_and_http_share_typed_result() {
        let (_cli_root, cli_app, cli_input) = terminal_verified_continuation_app();
        let cli_object = cli_app
            .recover_verification_settlement_value(cli_input)
            .unwrap();
        let cli = crate::app::evidence::evidence_success_envelope(
            "evidence.recover_settlement",
            cli_object,
        );
        let (_mcp_root, mcp_app, mcp_input) = terminal_verified_continuation_app();
        let mcp = mcp_app
            .mcp_evidence_tool_call(
                "planr_evidence_recover_settlement",
                json!({"input": mcp_input}),
            )
            .unwrap();
        let mcp: Value = serde_json::from_str(mcp["content"][0]["text"].as_str().unwrap()).unwrap();
        let (_http_root, http_app, http_input) = terminal_verified_continuation_app();
        let (status, body) = http_app
            .http_evidence_route(
                "POST",
                "/v1/evidence/recover-settlement",
                "",
                &json!({"input": http_input}),
            )
            .unwrap();
        let http: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(status, "200 OK");
        for envelope in [&cli, &mcp, &http] {
            assert_eq!(
                envelope["object"]["schema"],
                "planr.evidence.recover_verified_continuation.result.v1"
            );
            assert_eq!(envelope["object"]["created"], true);
        }
    }

    #[test]
    fn verified_continuation_stays_open_for_remaining_code_and_fails_closed_for_waivers() {
        let non_material =
            json!({"decision": {"material": false, "review": "none", "reasons": []}});
        let (_remaining_root, remaining_app, remaining_input) = stranded_recovery_app();
        remaining_app
            .recover_verification_settlement_value(remaining_input)
            .unwrap();
        remaining_app
            .conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
                 VALUES ('remaining-code', 'project-a', 'remaining', 'remaining', 'ready', 'code', 'plan-a.md', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
        let settled = remaining_app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-after-verification",
                summary: "not terminal",
                materiality: &non_material,
                escalation: None,
            })
            .unwrap();
        assert_eq!(settled["transition"], "continue_batch");
        assert_eq!(
            settled["execution_state"]["feature_run"]["phase"],
            "implementation"
        );

        let (_waived_root, waived_app, waived_input) = stranded_recovery_app();
        waived_app
            .recover_verification_settlement_value(waived_input)
            .unwrap();
        waived_app
            .conn
            .execute(
                "UPDATE coverage_verdicts SET waiver_digest_set = '[\"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"]'
                 WHERE scope_kind = 'plan' AND scope_id = 'plan-a'",
                [],
            )
            .unwrap();
        let error = waived_app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-after-verification",
                summary: "waived coverage is not exact proof",
                materiality: &non_material,
                escalation: None,
            })
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("verified_continuation_coverage_mismatch")
        );
        let phase: String = waived_app
            .conn
            .query_row(
                "SELECT phase FROM feature_runs WHERE plan_id = 'plan-a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(phase, "implementation");
    }

    #[test]
    fn stranded_settlement_recovery_rejects_stale_identity_without_writes() {
        let (_root, app, mut input) = stranded_recovery_app();
        input["verifier_generation"] = json!(999);

        let error = app
            .recover_verification_settlement_value(input)
            .expect_err("stale verifier generation fails closed");
        assert!(
            error
                .to_string()
                .contains("verification_settlement_recovery_verifier_mismatch")
        );
        let phase: String = app
            .conn
            .query_row(
                "SELECT phase FROM feature_runs WHERE plan_id = 'plan-a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(phase, "verification");
        let events: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'verification_settlement_recovered'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 0);
    }

    #[test]
    fn stranded_settlement_recovery_rejects_untrusted_state_shapes() {
        for case in [
            "missing_receipt",
            "waived",
            "wrong_owner",
            "active_adapter",
            "blocked_graph",
        ] {
            let (_root, app, mut input) = stranded_recovery_app();
            match case {
                "missing_receipt" => input["receipt_id"] = json!("missing-receipt"),
                "waived" => {
                    app.conn
                        .execute(
                            "UPDATE coverage_verdicts SET waiver_digest_set = '[\"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"]'
                             WHERE scope_kind = 'plan' AND scope_id = 'plan-a'",
                            [],
                        )
                        .unwrap();
                }
                "wrong_owner" => {
                    app.conn
                        .execute(
                            "UPDATE items SET worker_id = 'other-maker'
                             WHERE id = 'item-after-verification'",
                            [],
                        )
                        .unwrap();
                }
                "active_adapter" => {
                    app.conn
                        .execute_batch("DROP TRIGGER evidence_attempts_no_update")
                        .unwrap();
                    app.conn
                        .execute(
                            "UPDATE evidence_attempts SET completed_at = NULL
                             WHERE id = 'eatt-settle'",
                            [],
                        )
                        .unwrap();
                }
                "blocked_graph" => {
                    app.conn
                        .execute(
                            "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
                             VALUES ('unresolved-blocker', 'project-a', 'blocker', 'blocker', 'ready', 'docs', 'plan-a.md', datetime('now'), datetime('now'))",
                            [],
                        )
                        .unwrap();
                    app.conn
                        .execute(
                            "INSERT INTO links(from_item, to_item, kind, condition)
                             VALUES ('unresolved-blocker', 'item-after-verification', 'blocks', 'all')",
                            [],
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }

            app.recover_verification_settlement_value(input)
                .unwrap_err();
            let state: (String, i64) = app
                .conn
                .query_row(
                    "SELECT phase,
                        (SELECT COUNT(*) FROM events WHERE event_type = 'verification_settlement_recovered')
                     FROM feature_runs WHERE plan_id = 'plan-a'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(state, ("verification".into(), 0), "case {case}");
        }
    }

    #[test]
    fn stranded_settlement_recovery_rolls_back_late_transition_failure() {
        let (_root, app, input) = stranded_recovery_app();
        app.conn
            .execute_batch(
                "CREATE TRIGGER reject_recovery_transition
                 BEFORE UPDATE OF phase ON feature_runs
                 WHEN OLD.phase = 'verification' AND NEW.phase = 'implementation'
                 BEGIN SELECT RAISE(ABORT, 'forced recovery transition failure'); END;",
            )
            .unwrap();

        let error = app
            .recover_verification_settlement_value(input)
            .expect_err("late transition failure rolls back");
        assert!(
            error
                .to_string()
                .contains("forced recovery transition failure")
        );
        let active_verifier: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM feature_run_role_leases
                 WHERE role = 'verifier' AND released_at IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let active_maker: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM feature_run_role_leases
                 WHERE role = 'maker' AND released_at IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!((active_verifier, active_maker), (1, 0));
    }

    #[test]
    fn concurrent_stranded_settlement_recovery_converges_once() {
        let (root, app, input) = stranded_recovery_app();
        let database_path = app.db_path.clone();
        drop(app);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let database_path = database_path.clone();
            let repository_root = root.path().to_path_buf();
            let input = input.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                let conn = Connection::open(&database_path).unwrap();
                conn.busy_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                let app = App::new(conn, repository_root, database_path, true, false);
                barrier.wait();
                app.recover_verification_settlement_value(input).unwrap()["created"]
                    .as_bool()
                    .unwrap()
            }));
        }
        let mut created = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        created.sort();
        assert_eq!(created, vec![false, true]);

        let conn = Connection::open(database_path).unwrap();
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'verification_settlement_recovered'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 1);
    }

    #[test]
    fn stranded_settlement_recovery_mcp_and_http_share_the_typed_result() {
        let (_mcp_root, mcp_app, mcp_input) = stranded_recovery_app();
        let mcp = mcp_app
            .mcp_evidence_tool_call(
                "planr_evidence_recover_settlement",
                json!({"input": mcp_input}),
            )
            .unwrap();
        let mcp_envelope: Value =
            serde_json::from_str(mcp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            mcp_envelope["object"]["schema"],
            "planr.evidence.recover_settlement.result.v1"
        );
        assert_eq!(mcp_envelope["object"]["created"], true);

        let (_http_root, http_app, http_input) = stranded_recovery_app();
        let (status, body) = http_app
            .http_evidence_route(
                "POST",
                "/v1/evidence/recover-settlement",
                "",
                &json!({"input": http_input}),
            )
            .unwrap();
        let http_envelope: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(status, "200 OK");
        assert_eq!(
            http_envelope["object"]["schema"],
            mcp_envelope["object"]["schema"]
        );
        assert_eq!(http_envelope["object"]["created"], true);
    }

    #[test]
    fn satisfied_plan_coverage_closes_verification_item_with_receipt_lineage() {
        let (root, app, policy_digest) = settlement_app();
        let receipt_digest = seed_receipt_bound_settlement(&app, &policy_digest);

        let coverage = app
            .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap();

        assert_eq!(coverage["status"], "satisfied");
        let settlement = &coverage["feature_run_verification_settlement"];
        assert_eq!(settlement["item_id"], "item-settle-verification");
        assert_eq!(
            settlement["coverage"]["coverage_id"],
            coverage["coverage_id"]
        );
        assert_eq!(
            settlement["coverage"]["receipt_digests"],
            coverage["receipt_digests"]
        );
        assert_eq!(
            settlement["coverage"]["receipt_lineage"],
            coverage["receipt_lineage"]
        );
        assert!(
            coverage["receipt_digests"]
                .as_array()
                .unwrap()
                .iter()
                .any(|digest| digest.as_str() == Some(receipt_digest.as_str()))
        );
        let item_status: String = app
            .conn
            .query_row(
                "SELECT status FROM items WHERE id = 'item-settle-verification'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(item_status, "closed");
        let repository = ExecutionRunRepository::new(&app.conn);
        let settled_run = repository
            .active_feature_run_for_plan("project-a", "plan-a")
            .unwrap()
            .unwrap();
        assert_eq!(settled_run.run.phase, FeatureRunPhase::SourceFrozen);
        assert!(settled_run.run.role_owners.is_empty());
        assert_eq!(settlement["next_action"], "planr plan final-review plan-a");
        let event: String = app
            .conn
            .query_row(
                "SELECT payload FROM events WHERE item_id = 'item-settle-verification' AND event_type = 'verification_item_closed' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let event: Value = serde_json::from_str(&event).unwrap();
        assert_eq!(event["coverage"]["coverage_id"], coverage["coverage_id"]);
        assert_eq!(
            event["coverage"]["receipt_digests"],
            coverage["receipt_digests"]
        );
        assert_eq!(
            event["coverage"]["receipt_lineage"],
            coverage["receipt_lineage"]
        );

        let final_gate = app
            .ensure_final_product_review_gate_value("plan-a")
            .expect("initial final review gate");
        let gate_id = final_gate["execution_state"]["review_gate"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        app.review_gate_pick_value("plan-a", false)
            .expect("review pick")
            .expect("review packet");
        let changes = app
            .complete_review_gate_value(
                &gate_id,
                ReviewVerdict::ChangesRequested,
                &["repair exact source binding".into()],
                Some(&worker_id()),
            )
            .expect("changes requested");
        let finding_id = changes["execution_state"]["findings"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        app.conn
            .execute(
                "UPDATE review_gates SET responsible_maker_id = ?2 WHERE id = ?1",
                params![gate_id, worker_id()],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = ?2 WHERE run_id = ?1 AND role = 'maker' AND released_at IS NULL",
                params![
                    changes["execution_state"]["feature_run"]["id"]
                        .as_str()
                        .unwrap(),
                    worker_id()
                ],
            )
            .unwrap();
        std::fs::write(app.root.join("review-finding-repair.txt"), "repaired").unwrap();
        app.resolve_review_gate_findings_value(&gate_id, std::slice::from_ref(&finding_id))
            .expect("resolved finding refreezes source");
        app.conn
            .execute(
                "UPDATE review_gates SET status = 'changes_requested' WHERE id = ?1",
                [&gate_id],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = 'maker-other' WHERE run_id = ?1 AND role = 'maker'",
                [changes["execution_state"]["feature_run"]["id"]
                    .as_str()
                    .unwrap()],
            )
            .unwrap();
        let packet = app
            .verification_work_packet_value("plan-a", false)
            .expect("review finding reverification packet")
            .expect("itemless packet");
        assert_eq!(packet["work_packet"]["item_id"], Value::Null);
        assert_eq!(
            packet["work_packet"]["mode"], "review_finding_reverification",
            "{packet}"
        );
        assert_eq!(packet["work_packet"]["review_gate_id"], gate_id);
        assert_eq!(
            packet["work_packet"]["review_finding_ids"],
            json!([finding_id])
        );
        assert_eq!(
            packet["work_packet"]["selective_replay_obligation_ids"],
            json!(["pob-settle"])
        );
        let run_id = packet["work_packet"]["execution_state"]["feature_run"]["id"]
            .as_str()
            .unwrap();

        let stale_path = app.root.join("stale-review-reverification-source.txt");
        std::fs::write(&stale_path, "stale").unwrap();
        let stale = app
            .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .expect_err("stale source must roll back gate settlement");
        assert!(
            stale
                .to_string()
                .contains("review_reverification_source_stale")
        );
        let gate = repository.review_gate(&gate_id).unwrap();
        assert_eq!(gate.status, ReviewGateStatus::ChangesRequested);
        std::fs::remove_file(stale_path).unwrap();

        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = 'wrong-verifier' WHERE run_id = ?1 AND role = 'verifier' AND released_at IS NULL",
                [run_id],
            )
            .unwrap();
        let wrong_worker = app
            .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .expect_err("wrong verifier cannot settle review repair");
        assert!(
            wrong_worker
                .to_string()
                .contains("review_reverification_verifier_conflict")
        );
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = ?2 WHERE run_id = ?1 AND role = 'verifier' AND released_at IS NULL",
                params![run_id, worker_id()],
            )
            .unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let barrier = barrier.clone();
            let database_path = app.db_path.clone();
            let repository_root = root.path().to_path_buf();
            threads.push(std::thread::spawn(move || {
                let connection = Connection::open(&database_path).unwrap();
                connection
                    .busy_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
                let concurrent = App::new(connection, repository_root, database_path, true, false);
                barrier.wait();
                concurrent
                    .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
                    .unwrap()
            }));
        }
        barrier.wait();
        let concurrent = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        let statuses = concurrent
            .iter()
            .map(|coverage| {
                coverage["feature_run_verification_settlement"]["status"]
                    .as_str()
                    .unwrap()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            statuses,
            ["already_settled", "settled"].into_iter().collect()
        );
        let settled = concurrent
            .iter()
            .find(|coverage| coverage["feature_run_verification_settlement"]["status"] == "settled")
            .unwrap();
        assert_eq!(
            settled["feature_run_verification_settlement"]["review_gate_id"],
            gate_id
        );
        let repeated = app
            .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .expect("sequential repeat is idempotent");
        assert_eq!(
            repeated["feature_run_verification_settlement"]["status"],
            "already_settled"
        );
        assert_eq!(
            repeated["feature_run_verification_settlement"]["coverage"]["receipt_lineage"],
            settled["feature_run_verification_settlement"]["coverage"]["receipt_lineage"]
        );
        let ready_gate = repository.review_gate(&gate_id).unwrap();
        assert_eq!(ready_gate.status, ReviewGateStatus::Pending);
    }

    #[test]
    fn satisfied_exact_coverage_routes_to_final_review_and_cannot_release_verification() {
        let (_root, app, policy_digest) = settlement_app();
        seed_receipt_bound_settlement(&app, &policy_digest);
        let coverage = app
            .evidence_coverage_value(EvidenceCoverageScope::Plan, "plan-a")
            .unwrap();
        assert_eq!(
            coverage["feature_run_verification_settlement"]["phase"],
            "source_frozen"
        );
        assert_eq!(
            coverage["feature_run_verification_settlement"]["next_action"],
            "planr plan final-review plan-a"
        );

        let repository = ExecutionRunRepository::new(&app.conn);
        let snapshot = || {
            let persisted = repository
                .active_feature_run_for_plan("project-a", "plan-a")
                .unwrap()
                .unwrap();
            let run_id = &persisted.run.id;
            let freeze = repository.active_source_freeze(run_id).unwrap().unwrap();
            let count = |sql| {
                app.conn
                    .query_row(sql, [run_id], |row| row.get::<_, u64>(0))
                    .unwrap()
            };
            let item: (String, Option<String>) = app
                .conn
                .query_row(
                    "SELECT status, worker_id FROM items WHERE id = 'item-settle-verification'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            json!({
                "revision": persisted.revision,
                "phase": persisted.run.phase,
                "role_owners": persisted.run.role_owners,
                "active_verifier_leases": count("SELECT COUNT(*) FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'verifier' AND released_at IS NULL"),
                "verification_budget_reservations": count("SELECT COUNT(*) FROM feature_run_budget_reservations WHERE run_id = ?1 AND phase = 'verification'"),
                "verification_budget_observations": count("SELECT COUNT(*) FROM feature_run_budget_observations WHERE run_id = ?1 AND phase = 'verification'"),
                "verification_item": item,
                "source_freeze": [freeze.id, freeze.source_revision, freeze.source_digest],
                "event_count": app.conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get::<_, u64>(0)).unwrap(),
            })
        };
        let before = snapshot();
        assert_eq!(before["phase"], "source_frozen");
        let run_id = repository
            .active_feature_run_for_plan("project-a", "plan-a")
            .unwrap()
            .unwrap()
            .run
            .id;
        let state = app.canonical_execution_state_value(&run_id, None).unwrap();
        assert_eq!(state["reason_code"], "binding_evidence_satisfied");
        assert_eq!(state["next_action"], "open_final_review");
        assert!(
            app.verification_work_packet_value("plan-a", false)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            snapshot(),
            before,
            "duplicate verification mutated settlement facts"
        );
    }

    #[test]
    fn satisfied_plan_coverage_atomically_resumes_remaining_code_without_leasing_it() {
        let (_root, app, policy_digest) = settlement_app();
        seed_receipt_bound_settlement(&app, &policy_digest);
        app.conn
            .execute(
                "UPDATE items SET status = 'closed', worker_id = NULL,
                     completed_at = datetime('now'), updated_at = datetime('now')
                 WHERE id = 'item-settle-maker'",
                [],
            )
            .unwrap();
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
                 VALUES ('item-after-verification', 'project-a', 'classification', 'remaining code', 'pending', 'code', 'plan-a.md', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
        app.conn
            .execute(
                "INSERT INTO links(from_item, to_item, kind, condition)
                 VALUES ('item-settle-verification', 'item-after-verification', 'blocks', 'all')",
                [],
            )
            .unwrap();

        let coverage = app
            .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap();
        let settlement = &coverage["feature_run_verification_settlement"];
        assert_eq!(settlement["phase"], "implementation");
        assert_eq!(settlement["next_code_item_id"], "item-after-verification");
        assert_eq!(
            settlement["next_action"],
            "planr pick --plan plan-a --work-type code --json"
        );

        let repository = ExecutionRunRepository::new(&app.conn);
        let run = repository
            .active_feature_run_for_plan("project-a", "plan-a")
            .unwrap()
            .unwrap();
        assert_eq!(run.run.phase, FeatureRunPhase::Implementation);
        assert_eq!(run.run.role_owners.len(), 1);
        assert_eq!(run.run.role_owners[0].role, RunRole::Maker);
        assert_eq!(run.run.role_owners[0].worker_id, "maker-other");
        let batch = repository
            .batch(run.run.active_batch_id.as_deref().unwrap())
            .unwrap();
        assert_eq!(batch.batch.status, ExecutionBatchStatus::Active);
        assert_eq!(batch.batch.maker_worker_id, "maker-other");
        let next: (String, Option<String>) = app
            .conn
            .query_row(
                "SELECT status, worker_id FROM items WHERE id = 'item-after-verification'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(next, ("ready".to_string(), None));
        let active_roles: Vec<(String, String)> = app
            .conn
            .prepare(
                "SELECT role, worker_id FROM feature_run_role_leases
                 WHERE run_id = ?1 AND released_at IS NULL ORDER BY role",
            )
            .unwrap()
            .query_map([&run.run.id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            active_roles,
            vec![("maker".to_string(), "maker-other".to_string())]
        );
        let active_verification_budgets: u64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM feature_run_budget_reservations
                 WHERE run_id = ?1 AND phase = 'verification' AND status = 'active'",
                [&run.run.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active_verification_budgets, 0);
    }

    #[test]
    fn post_verification_transition_failure_rolls_back_item_graph_log_and_roles() {
        let (_root, app, policy_digest) = settlement_app();
        seed_receipt_bound_settlement(&app, &policy_digest);
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
                 VALUES ('item-after-verification', 'project-a', 'classification', 'remaining code', 'pending', 'code', 'plan-a.md', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
        app.conn
            .execute(
                "INSERT INTO links(from_item, to_item, kind, condition)
                 VALUES ('item-settle-verification', 'item-after-verification', 'blocks', 'all')",
                [],
            )
            .unwrap();
        app.conn
            .execute_batch(
                "CREATE TRIGGER reject_post_verification_continuation
                 BEFORE UPDATE ON feature_runs
                 WHEN OLD.phase = 'verification' AND NEW.phase = 'implementation'
                 BEGIN
                   SELECT RAISE(ABORT, 'injected post-verification transition failure');
                 END;",
            )
            .unwrap();

        let error = app
            .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap_err()
            .to_string();
        assert!(error.contains("injected post-verification transition failure"));
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT status FROM items WHERE id = 'item-settle-verification'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "picked"
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT status FROM items WHERE id = 'item-after-verification'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "pending"
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM logs WHERE item_id = 'item-settle-verification'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );
        let repository = ExecutionRunRepository::new(&app.conn);
        let run = repository
            .active_feature_run_for_plan("project-a", "plan-a")
            .unwrap()
            .unwrap();
        assert_eq!(run.run.phase, FeatureRunPhase::Verification);
        assert_eq!(run.run.role_owners.len(), 1);
        assert_eq!(run.run.role_owners[0].role, RunRole::Verifier);
    }

    #[test]
    fn settlement_does_not_close_without_satisfied_leased_coverage() {
        let (_root, app, _policy_digest) = settlement_app();
        app.verification_work_packet_value("plan-a", false).unwrap();
        let missing = app
            .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap();
        assert_ne!(missing["status"], "satisfied");
        assert!(missing.get("feature_run_verification_settlement").is_none());
        let item_status: String = app
            .conn
            .query_row(
                "SELECT status FROM items WHERE id = 'item-settle-verification'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(item_status, "picked");

        let (_root, app, policy_digest) = settlement_app();
        seed_receipt_bound_settlement(&app, &policy_digest);
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = 'other-verifier' WHERE role = 'verifier'",
                [],
            )
            .unwrap();
        let error = app
            .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap_err()
            .to_string();
        assert!(error.contains("verification_coverage_requires_verifier_lease"));
        let item_status: String = app
            .conn
            .query_row(
                "SELECT status FROM items WHERE id = 'item-settle-verification'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(item_status, "picked");
    }

    #[test]
    fn stale_source_and_close_conflict_do_not_persist_verification_settlement() {
        let (root, app, policy_digest) = settlement_app();
        seed_receipt_for_existing_settlement_obligation(&app, &policy_digest);
        std::fs::write(root.path().join("after-freeze.txt"), "stale\n").unwrap();
        let stale = app
            .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap_err()
            .to_string();
        assert!(stale.contains("verification_coverage_source_freeze_stale"));
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT status FROM items WHERE id = 'item-settle-verification'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "picked"
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE item_id = 'item-settle-verification' AND event_type = 'verification_item_closed'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );

        let (_root, app, policy_digest) = settlement_app();
        seed_receipt_bound_settlement(&app, &policy_digest);
        app.conn
            .execute_batch(
                "CREATE TRIGGER test_conflict_after_settlement_log
                 AFTER INSERT ON logs
                 WHEN NEW.item_id = 'item-settle-verification'
                   AND NEW.summary = 'canonical plan Evidence coverage satisfied verification outcome'
                 BEGIN
                   UPDATE items
                   SET status = 'closed'
                   WHERE id = 'item-settle-verification';
                 END;",
            )
            .unwrap();
        let conflict = app
            .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap_err()
            .to_string();
        assert!(conflict.contains("verification_item_close_conflict"));
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT status FROM items WHERE id = 'item-settle-verification'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "picked"
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE item_id = 'item-settle-verification' AND event_type = 'verification_item_closed'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM logs WHERE item_id = 'item-settle-verification' AND summary = 'canonical plan Evidence coverage satisfied verification outcome'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn waived_only_plan_coverage_does_not_close_verification_item() {
        let (_root, app, _policy_digest) = settlement_app();
        app.verification_work_packet_value("plan-a", false)
            .unwrap()
            .unwrap();
        seed_waiver_for_settlement_obligation(&app);
        let waived = app
            .evidence_coverage_value(crate::cli::EvidenceCoverageScope::Plan, "plan-a")
            .unwrap();
        assert_eq!(waived["status"], "waived");
        assert!(waived["receipt_digests"].as_array().unwrap().is_empty());
        assert!(!waived["waiver_digests"].as_array().unwrap().is_empty());
        assert!(waived.get("feature_run_verification_settlement").is_none());
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT status FROM items WHERE id = 'item-settle-verification'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "picked"
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE item_id = 'item-settle-verification' AND event_type = 'verification_item_closed'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM logs WHERE item_id = 'item-settle-verification' AND summary = 'canonical plan Evidence coverage satisfied verification outcome'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );
    }

    fn seed_waiver_for_settlement_obligation(app: &App) {
        let source = capture_repository_snapshot(Path::new(".")).unwrap().source;
        let waiver_json = json!({
            "id": "waiver-settle",
            "schema_version": crate::evidence::model::EVIDENCE_CONTRACT_V1,
            "scope": {"kind": "plan", "id": "plan-a"},
            "observation_ids": ["obs-settle"],
            "source": source,
            "target": {"kind": "process", "uri": "local://ready"},
            "reason": "temporary exact-source verifier waiver",
            "created_by": "reviewer",
            "created_at": "2026-08-08T00:00:00Z",
            "expires_at": "2099-01-01T00:00:00Z",
            "approval_ref": "item-waiver-approval",
            "audit_trail": [{"event": "created", "at": "2026-08-08T00:00:00Z"}]
        });
        let waiver_digest = crate::canonical_json::sha256_json_digest(&waiver_json).unwrap();
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, approval_status, approved_by, created_at, updated_at, completed_at)
                 VALUES ('item-waiver-approval', 'project-a', 'waiver approval', 'approved waiver', 'closed', 'approval', 'reviewer', 'plan-a.md', 'approved', 'reviewer', datetime('now'), datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
        app.conn
            .execute(
                "INSERT INTO evidence_waivers(
                  id, project_id, approval_item_id, obligation_id, observation_id,
                  scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
                  waiver_json, created_at
                ) VALUES (
                  'waiver-settle', 'project-a', 'item-waiver-approval', 'pob-settle', 'obs-settle',
                  'plan', 'plan-a', ?1, 'temporary exact-source verifier waiver',
                  '2099-01-01T00:00:00Z', 'reviewer', ?2, '2026-08-08T00:00:00Z'
                )",
                params![waiver_digest, waiver_json.to_string()],
            )
            .unwrap();
    }

    #[test]
    fn capability_held_source_is_invalidated_and_refrozen_after_repair_changes() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        initialize_git(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-refreeze");
        let persisted = app
            .ensure_outcome_feature_run("item-refreeze")
            .unwrap()
            .unwrap();
        let first = app
            .freeze_feature_run_source_value("plan-a")
            .unwrap()
            .unwrap();
        let first_id = first["source_freeze"]["id"].as_str().unwrap();
        let repository = ExecutionRunRepository::new(&app.conn);
        let frozen = repository.feature_run(&persisted.run.id).unwrap();
        let held = apply_phase_transition(
            &frozen.run,
            &PhaseTransition {
                to: FeatureRunPhase::Held,
                cause: PhaseTransitionCause::CapabilityHold,
                reference: "test:missing_capability".to_string(),
                owner: None,
            },
        )
        .unwrap();
        repository.save_feature_run(&held, frozen.revision).unwrap();
        std::fs::write(root.path().join("repair.txt"), "repaired\n").unwrap();

        let replacement = app
            .freeze_feature_run_source_value("plan-a")
            .unwrap()
            .unwrap();
        assert!(replacement["created"].as_bool().unwrap());
        assert_ne!(replacement["source_freeze"]["id"], first_id);
        assert_eq!(
            repository.source_freeze(first_id).unwrap().status,
            SourceFreezeStatus::Invalidated
        );
        assert_eq!(
            repository
                .active_source_freeze(&persisted.run.id)
                .unwrap()
                .unwrap()
                .status,
            SourceFreezeStatus::Active
        );
    }

    #[test]
    fn persisted_contract_not_mutated_policy_controls_runtime_admission() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-policy-snapshot");
        let persisted = app
            .ensure_outcome_feature_run("item-policy-snapshot")
            .unwrap()
            .unwrap();
        let repository = ExecutionRunRepository::new(&app.conn);
        let original = repository.budget_contract(&persisted.run.id).unwrap();
        let mutated = std::fs::read_to_string(root.path().join(".planr/policy.toml"))
            .unwrap()
            .replace("max_wall_time_seconds = 100", "max_wall_time_seconds = 1")
            .replace("max_tool_calls = 100", "max_tool_calls = 1")
            .replace("max_tokens = 1000", "max_tokens = 1");
        std::fs::write(root.path().join(".planr/policy.toml"), mutated).unwrap();

        let reservation = app
            .admit_feature_run_budget(
                &persisted,
                BudgetPhase::Implementation,
                "implementation:policy-snapshot",
                &worker_id(),
                "test.policy_snapshot",
            )
            .unwrap();
        assert!(matches!(
            reservation,
            FeatureRunBudgetAdmission::Reserved(_)
        ));
        assert_eq!(
            repository
                .budget_contract(&persisted.run.id)
                .unwrap()
                .digest,
            original.digest
        );
    }

    #[test]
    fn compatible_budget_hold_resolution_is_idempotent_and_rollback_safe() {
        let root = tempfile::tempdir().unwrap();
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-budget-resume");
        let persisted = app
            .ensure_outcome_feature_run("item-budget-resume")
            .unwrap()
            .unwrap();
        let reservation = match app
            .admit_feature_run_budget(
                &persisted,
                BudgetPhase::Implementation,
                "implementation:item-budget-resume",
                &worker_id(),
                "test.resume",
            )
            .unwrap()
        {
            FeatureRunBudgetAdmission::Reserved(reservation) => reservation,
            FeatureRunBudgetAdmission::Held(_) => panic!("expected reservation"),
        };
        let repository = ExecutionRunRepository::new(&app.conn);
        let held = apply_phase_transition(
            &repository.feature_run(&persisted.run.id).unwrap().run,
            &PhaseTransition {
                to: FeatureRunPhase::Held,
                cause: PhaseTransitionCause::BudgetHold,
                reference: "test:stale_runtime_hold".to_string(),
                owner: None,
            },
        )
        .unwrap();
        repository
            .save_feature_run(&held, persisted.revision)
            .unwrap();

        let resumed = app.resolve_feature_run_budget_hold_value("plan-a").unwrap();
        assert_eq!(resumed["resolution"]["disposition"], "resumed");
        assert_eq!(
            resumed["resolution"]["cause"],
            "active_reservations_revalidated"
        );
        assert_eq!(resumed["execution_state"]["phase"], "implementation");
        let repeated = app.resolve_feature_run_budget_hold_value("plan-a").unwrap();
        assert_eq!(repeated["resolution"]["disposition"], "already_resumed");
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE event_type = 'feature_run_budget_hold_resolved'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            1
        );

        app.reconcile_feature_run_budget(&reservation, &BudgetUsageReport::application(Some(1)))
            .unwrap();
        let reconciled = repository.feature_run(&persisted.run.id).unwrap();
        let transient_hold = apply_phase_transition(
            &reconciled.run,
            &PhaseTransition {
                to: FeatureRunPhase::Held,
                cause: PhaseTransitionCause::BudgetHold,
                reference: "test:transient_contention".to_string(),
                owner: None,
            },
        )
        .unwrap();
        repository
            .save_feature_run(&transient_hold, reconciled.revision)
            .unwrap();
        let cleared = app.resolve_feature_run_budget_hold_value("plan-a").unwrap();
        assert_eq!(
            cleared["resolution"]["cause"],
            "transient_contention_cleared"
        );
        assert_eq!(cleared["resolution"]["active_reservation_ids"], json!([]));

        let current = repository.feature_run(&persisted.run.id).unwrap();
        let held_again = apply_phase_transition(
            &current.run,
            &PhaseTransition {
                to: FeatureRunPhase::Held,
                cause: PhaseTransitionCause::BudgetHold,
                reference: "test:rollback".to_string(),
                owner: None,
            },
        )
        .unwrap();
        repository
            .save_feature_run(&held_again, current.revision)
            .unwrap();
        app.conn
            .execute_batch(
                "CREATE TRIGGER fail_budget_hold_resolution BEFORE UPDATE ON feature_runs
                 WHEN OLD.id = 'frun-does-not-match' OR NEW.phase = 'implementation'
                 BEGIN SELECT RAISE(ABORT, 'injected budget hold resolution rollback'); END;",
            )
            .unwrap();
        let error = app
            .resolve_feature_run_budget_hold_value("plan-a")
            .unwrap_err()
            .to_string();
        assert!(error.contains("injected budget hold resolution rollback"));
        assert_eq!(
            repository.feature_run(&persisted.run.id).unwrap().run.phase,
            FeatureRunPhase::Held
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE event_type = 'feature_run_budget_hold_resolved'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            2
        );
    }

    #[test]
    fn budget_hold_resolution_rejects_unrepaired_and_incompatible_state() {
        let root = tempfile::tempdir().unwrap();
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-unrepaired-hold");
        let persisted = app
            .ensure_outcome_feature_run("item-unrepaired-hold")
            .unwrap()
            .unwrap();
        let repository = ExecutionRunRepository::new(&app.conn);
        let held = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::Held,
                cause: PhaseTransitionCause::BudgetHold,
                reference: "test:no_reservation".to_string(),
                owner: None,
            },
        )
        .unwrap();
        repository
            .save_feature_run(&held, persisted.revision)
            .unwrap();
        let unrepaired = app
            .resolve_feature_run_budget_hold_value("plan-a")
            .unwrap_err()
            .to_string();
        assert!(unrepaired.contains("unrepaired_ceiling_or_missing_reservation"));

        app.conn
            .execute_batch(
                "DROP TRIGGER feature_run_budget_contracts_no_delete;
                 DELETE FROM feature_run_budget_contracts WHERE run_id IN (SELECT id FROM feature_runs WHERE plan_id = 'plan-a');",
            )
            .unwrap();
        let incompatible = app
            .resolve_feature_run_budget_hold_value("plan-a")
            .unwrap_err()
            .to_string();
        assert!(incompatible.contains("feature_run_budget_hold_requires_restart"));
        assert!(incompatible.contains("restart_incompatible_feature_run"));
    }

    #[test]
    fn reconciliation_is_delta_based_transactional_and_never_launders_provenance() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-reconcile");
        let persisted = app
            .ensure_outcome_feature_run("item-reconcile")
            .unwrap()
            .unwrap();
        let released = match app
            .admit_feature_run_budget(
                &persisted,
                BudgetPhase::Implementation,
                "implementation:released",
                &worker_id(),
                "test.release",
            )
            .unwrap()
        {
            FeatureRunBudgetAdmission::Reserved(value) => value,
            FeatureRunBudgetAdmission::Held(_) => panic!("unexpected release fixture hold"),
        };
        app.release_feature_run_budget(&released).unwrap();

        let reservation = match app
            .admit_feature_run_budget(
                &persisted,
                BudgetPhase::Implementation,
                "implementation:reconcile",
                &worker_id(),
                "test.reconciliation",
            )
            .unwrap()
        {
            FeatureRunBudgetAdmission::Reserved(value) => value,
            FeatureRunBudgetAdmission::Held(_) => panic!("unexpected reconciliation hold"),
        };
        app.reconcile_feature_run_budget(&reservation, &BudgetUsageReport::application(Some(1)))
            .unwrap();

        let repository = ExecutionRunRepository::new(&app.conn);
        let observations = repository.budget_observations(&persisted.run.id).unwrap();
        assert_eq!(observations.len(), 1);
        assert!(observations.iter().all(|observation| {
            observation.metering == MeteringMode::Unavailable
                && observation.wall_metering == Some(MeteringMode::Trusted)
                && observation.tool_calls_metering == Some(MeteringMode::Estimated)
                && observation.tokens_metering == Some(MeteringMode::Unavailable)
                && observation.tool_calls == Some(1)
                && observation.tokens.is_none()
        }));
        let (_, snapshot) = app
            .persisted_budget_snapshot(
                &persisted,
                FeatureRunBudgetPhase::Maker,
                unix_time_ms().unwrap(),
            )
            .unwrap();
        assert_eq!(snapshot.consumed.tool_calls, 60);
        assert_eq!(snapshot.consumed.tokens, 600);
        assert_eq!(snapshot.metering.tool_calls, MeteringProvenance::Estimated);
        assert_eq!(snapshot.metering.tokens, MeteringProvenance::Unavailable);
    }

    #[test]
    fn concurrent_same_phase_reservations_are_serialized_against_one_snapshot() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        let database_path = root.path().join("concurrent.sqlite");
        let connection = Connection::open(&database_path).unwrap();
        ensure_schema(&connection).unwrap();
        connection.execute(
            "INSERT INTO projects(id, name, root_path, status, created_at, updated_at) VALUES ('project-a', 'Project', '.', 'active', datetime('now'), datetime('now'))",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at) VALUES ('plan-a', 'project-a', 'build', 'plan-a.md', 'Plan', 'plan', 'ok', 'sha256:plan', datetime('now'), datetime('now'))",
            [],
        ).unwrap();
        let setup = App::new(
            connection,
            root.path().to_path_buf(),
            database_path.clone(),
            true,
            false,
        );
        add_outcome(&setup, "item-concurrent");
        let run_id = setup
            .ensure_outcome_feature_run("item-concurrent")
            .unwrap()
            .unwrap()
            .run
            .id;
        drop(setup);

        let barrier = Arc::new(Barrier::new(2));
        let mut threads = Vec::new();
        for index in 0..2 {
            let barrier = Arc::clone(&barrier);
            let root_path = root.path().to_path_buf();
            let database_path = database_path.clone();
            let run_id = run_id.clone();
            threads.push(thread::spawn(move || {
                let connection = Connection::open(&database_path).unwrap();
                connection
                    .busy_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                let app = App::new(connection, root_path, database_path, true, false);
                let persisted = ExecutionRunRepository::new(&app.conn)
                    .feature_run(&run_id)
                    .unwrap();
                barrier.wait();
                app.admit_feature_run_budget(
                    &persisted,
                    BudgetPhase::Implementation,
                    &format!("implementation:concurrent-{index}"),
                    &worker_id(),
                    "test.concurrent",
                )
                .unwrap()
            }));
        }
        let results = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, FeatureRunBudgetAdmission::Reserved(_)))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, FeatureRunBudgetAdmission::Held(_)))
                .count(),
            1
        );
        let connection = Connection::open(&database_path).unwrap();
        let active: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM feature_run_budget_reservations WHERE run_id = ?1 AND status = 'active'",
                [&run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 1);
    }

    #[test]
    fn wall_consumption_is_anchored_to_persisted_run_start() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-wall-anchor");
        let persisted = app
            .ensure_outcome_feature_run("item-wall-anchor")
            .unwrap()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1_050));
        let (contract, snapshot) = app
            .persisted_budget_snapshot(
                &persisted,
                FeatureRunBudgetPhase::Maker,
                unix_time_ms().unwrap(),
            )
            .unwrap();
        assert!(contract.started_at_unix_ms > 0);
        assert!(snapshot.consumed.wall_seconds >= 2);
        assert!(
            ExecutionRunRepository::new(&app.conn)
                .budget_reservations(&persisted.run.id)
                .unwrap()
                .is_empty(),
            "run wall consumption must not depend on a task reservation start"
        );
    }

    #[test]
    fn product_finding_repair_is_itemless_safe_across_settlement_and_replay() {
        let mut canonical_results = Vec::new();
        for with_item in [false, true] {
            let (_root, app, run_id, freeze_id) = verification_fixture(with_item, true);
            app.conn.execute(
                "UPDATE feature_run_role_leases SET worker_id = ?1 WHERE run_id = ?2 AND role = 'maker'",
                params![worker_id(), run_id],
            ).unwrap();
            let item_count: u64 = app
                .conn
                .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
                .unwrap();
            let outcome_before: (String, Option<String>, String) = app
                .conn
                .query_row(
                    "SELECT status, worker_id, updated_at FROM items WHERE id = 'item-phase'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            let receipts_before: u64 = app
                .conn
                .query_row("SELECT COUNT(*) FROM evidence_receipts", [], |row| {
                    row.get(0)
                })
                .unwrap();

            let routed = app
                .route_evidence_product_finding_value(&run_id, &freeze_id, "pob-phase-ready")
                .unwrap();
            let repair_id = routed["repair_id"].as_str().unwrap();
            assert_eq!(repair_id, routed["invalidation"]["id"]);
            assert_eq!(routed["verification_item_id"].is_null(), !with_item);
            let repository = ExecutionRunRepository::new(&app.conn);
            let repair_run = repository.feature_run(&run_id).unwrap();
            let repair_batch_id = repair_run.run.active_batch_id.clone().unwrap();
            let repair = app.repair_work_packet_value("plan-a").unwrap().unwrap();
            assert_eq!(repair["work_packet"]["repair_id"], repair_id);
            assert_eq!(
                repair["work_packet"]["verification_item_id"].is_null(),
                !with_item
            );

            let refrozen = app
                .settle_product_finding_repair_value(
                    "plan-a",
                    repair_id,
                    "repair complete",
                    &["src/app/feature_run_evidence.rs".into()],
                    &["focused invariant".into()],
                    &["passed".into()],
                )
                .unwrap();
            assert_eq!(refrozen["work_packet"]["kind"], "verification_handoff");
            assert_eq!(refrozen["work_packet"]["mode"], "selective_replay");
            assert_eq!(
                refrozen["work_packet"]["verification_item_id"].is_null(),
                !with_item
            );
            assert_eq!(
                refrozen["work_packet"]["selective_replay_obligation_ids"],
                json!(["pob-phase-ready"])
            );
            assert_eq!(
                refrozen["work_packet"]["execution_state"]["phase"],
                "source_frozen"
            );

            let settlement = repository
                .product_repair_settlement(repair_id)
                .unwrap()
                .unwrap();
            assert!(
                serde_json::to_value(&settlement)
                    .unwrap()
                    .get("verification_item_id")
                    .is_none()
            );
            assert_eq!(settlement.responsible_maker_id, worker_id());
            assert_eq!(settlement.selective_obligation_ids, ["pob-phase-ready"]);
            let repeated = app
                .settle_product_finding_repair_value("plan-a", repair_id, "ignored", &[], &[], &[])
                .unwrap();
            assert_eq!(repeated["created"], false);
            assert_eq!(
                repeated["work_packet"]["verification_item_id"].is_null(),
                !with_item
            );
            assert_eq!(
                repeated["work_packet"]["source_freeze"]["id"],
                settlement.source_freeze_id
            );
            let selective = app
                .verification_work_packet_value("plan-a", true)
                .unwrap()
                .unwrap();
            assert_eq!(selective["work_packet"]["item_id"].is_null(), !with_item);
            assert_eq!(selective["work_packet"]["mode"], "selective_replay");
            assert_eq!(selective["work_packet"]["repair_id"], repair_id);
            assert_eq!(
                selective["work_packet"]["selective_replay_obligation_ids"],
                json!(["pob-phase-ready"])
            );

            let settled_run = repository.feature_run(&run_id).unwrap();
            let ended_batch = repository.batch(&repair_batch_id).unwrap();
            assert_eq!(
                repository.source_freeze(&freeze_id).unwrap().status,
                SourceFreezeStatus::Invalidated
            );
            assert_eq!(
                repository
                    .source_freeze(&settlement.source_freeze_id)
                    .unwrap()
                    .status,
                SourceFreezeStatus::Active
            );
            assert_eq!(ended_batch.batch.status, ExecutionBatchStatus::Ended);
            assert_eq!(ended_batch.batch.maker_worker_id, worker_id());
            assert_eq!(
                app.conn
                    .query_row("SELECT COUNT(*) FROM items", [], |row| row.get::<_, u64>(0))
                    .unwrap(),
                item_count
            );
            assert_eq!(
                app.conn
                    .query_row(
                        "SELECT status, worker_id, updated_at FROM items WHERE id = 'item-phase'",
                        [],
                        |row| Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?
                        )),
                    )
                    .unwrap(),
                outcome_before
            );
            if with_item {
                assert_eq!(
                    app.conn
                        .query_row(
                            "SELECT status, worker_id FROM items WHERE id = 'verification-phase'",
                            [],
                            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                        )
                        .unwrap(),
                    ("ready".into(), None)
                );
            }
            let receipts_after: u64 = app
                .conn
                .query_row("SELECT COUNT(*) FROM evidence_receipts", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!((receipts_before, receipts_after), (0, 0));
            assert!(
                app.load_active_budget_reservation(&run_id, &format!("repair:{run_id}"))
                    .unwrap()
                    .is_none()
            );
            canonical_results.push(json!({
                "phase": settled_run.run.phase,
                "batch_count": settled_run.run.batch_outcome_count, "owners": settled_run.run.role_owners,
                "old_freeze": "invalidated", "new_freeze": "active", "repair_batch": ended_batch.batch.status,
                "maker": settlement.responsible_maker_id, "obligations": settlement.selective_obligation_ids,
                "handoff_phase": refrozen["work_packet"]["execution_state"]["phase"], "receipts": receipts_after,
            }));
        }
        assert_eq!(canonical_results[0], canonical_results[1]);
    }

    #[test]
    fn product_repair_follows_invalidation_obligations_to_the_active_successor() {
        let (_root, app, _run_id, _freeze_id) = verification_fixture(true, true);
        app.conn
            .execute(
                "INSERT INTO proof_obligations(
               id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
               binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
               assurance_policy_json, retry_aggregation, policy_digest, config_digest,
               source_digest, supersedes_obligation_id, created_at, obligation_shape
             ) SELECT
               'pob-phase-successor', project_id, plan_id, item_id, criterion_id, 2, title,
               binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
               assurance_policy_json, retry_aggregation, policy_digest, config_digest,
               source_digest, id, datetime('now'), obligation_shape
             FROM proof_obligations WHERE id = 'pob-phase-ready'",
                [],
            )
            .unwrap();

        assert_eq!(
            app.product_repair_obligation_ids(&["pob-phase-ready".to_string()])
                .unwrap(),
            vec!["pob-phase-successor"]
        );
    }

    #[test]
    fn product_repair_reopens_the_same_material_gate_with_exact_source_binding() {
        let (_root, app, run_id, freeze_id) = verification_fixture(true, true);
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = ?1 WHERE run_id = ?2 AND role = 'maker'",
                params![worker_id(), run_id],
            )
            .unwrap();
        let repository = ExecutionRunRepository::new(&app.conn);
        let gate = ReviewGateRecord {
            id: "gate-material-repair".to_string(),
            run_id: run_id.clone(),
            scope_kind: ReviewScopeKind::Outcome,
            scope_id: "item-phase".to_string(),
            kind: ReviewGateKind::RiskCheckpoint,
            status: ReviewGateStatus::Pending,
            required_risk: Some("data_integrity_risk".to_string()),
            responsible_maker_id: worker_id(),
            latest_attempt: 0,
            source_revision: None,
        };
        repository.create_review_gate(&gate).unwrap();
        repository
            .append_review_attempt(
                &ReviewAttemptRecord {
                    id: "attempt-old".to_string(),
                    gate_id: gate.id.clone(),
                    attempt_number: 1,
                    reviewer_worker_id: "checker-old".to_string(),
                    reviewer_mode: "independent".to_string(),
                    verdict: ReviewVerdict::Accepted,
                    source_revision: "risk-gate:gate-material-repair".to_string(),
                    artifacts: Vec::new(),
                },
                &[],
                0,
            )
            .unwrap();
        let routed = app
            .route_evidence_product_finding_value(&run_id, &freeze_id, "pob-phase-ready")
            .unwrap();
        let repair_id = routed["repair_id"].as_str().unwrap();
        let reopened = app
            .settle_product_finding_repair_value(
                "plan-a",
                repair_id,
                "repair changed candidate inputs",
                &["preflight.json".to_string()],
                &["cargo test".to_string()],
                &["passed".to_string()],
            )
            .unwrap();
        assert_eq!(reopened["work_packet"]["kind"], "review_gate");
        assert_eq!(reopened["work_packet"]["gate_id"], gate.id);
        let repeated = app
            .settle_product_finding_repair_value("plan-a", repair_id, "ignored", &[], &[], &[])
            .unwrap();
        assert_eq!(repeated["created"], false);
        assert_eq!(repeated["work_packet"]["gate_id"], gate.id);

        let pending = repository.review_gate(&gate.id).unwrap();
        assert_eq!(pending.status, ReviewGateStatus::Pending);
        assert_eq!(pending.latest_attempt, 1);
        let binding = repository.review_source_binding(&gate.id).unwrap().unwrap();
        assert_eq!(
            pending.source_revision.as_deref(),
            Some(binding.source_revision.as_str())
        );
        assert_eq!(
            reopened["work_packet"]["execution_state"]["review_source_binding"]["freeze_id"],
            binding.freeze_id
        );
        assert_eq!(
            repository.feature_run(&run_id).unwrap().run.phase,
            FeatureRunPhase::Implementation
        );
        assert!(
            app.verification_work_packet_value("plan-a", true)
                .unwrap()
                .is_none(),
            "pending exact-source review must block verification"
        );

        repository
            .reopen_review_gate_with_source_binding(&binding)
            .unwrap();
        let mut conflicting = binding.clone();
        conflicting.source_digest = "sha256:conflict".to_string();
        assert!(
            repository
                .reopen_review_gate_with_source_binding(&conflicting)
                .unwrap_err()
                .to_string()
                .contains("review_gate_source_reopen_conflict")
        );
        conflicting.gate_id = "gate-wrong".to_string();
        assert!(
            repository
                .reopen_review_gate_with_source_binding(&conflicting)
                .is_err()
        );
    }
}
