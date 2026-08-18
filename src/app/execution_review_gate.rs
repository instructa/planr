//! Review-gate leasing, finding repair, and completion.

use super::App;
use super::feature_run_evidence::{BudgetUsageReport, FeatureRunBudgetAdmission};
use super::repository::execution_run::{
    EvidenceInvalidationRecord, ExecutionRunRepository, FindingRecord, FindingStatus,
    PersistedFeatureRun, ReviewAttemptRecord, ReviewGateKind, ReviewGateRecord, ReviewGateStatus,
    ReviewVerdict, SourceFreezeRecord, SourceFreezeStatus,
};
use crate::evidence::policy::capture_repository_snapshot;
use crate::execution_run::{
    ExecutionBatch, ExecutionBatchStatus, FeatureRunPhase, PhaseTransition, PhaseTransitionCause,
    RoleOwner, RunRole, apply_phase_transition, pause_batch_for_risk_review,
    resume_batch_after_risk_review,
};
use crate::usage_policy::BudgetPhase;
use crate::util::{short_id, worker_id};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

impl App {
    pub(crate) fn review_gate_pick_value(
        &self,
        plan_id: &str,
        peek: bool,
    ) -> Result<Option<Value>> {
        self.review_gate_pick_value_for_worker(plan_id, peek, &worker_id())
    }

    pub(super) fn reconcile_final_review_repair_source(
        &self,
        repository: &ExecutionRunRepository<'_>,
        persisted: &PersistedFeatureRun,
        gate: &ReviewGateRecord,
    ) -> Result<()> {
        if gate.kind != ReviewGateKind::FinalProduct
            || !matches!(
                persisted.run.phase,
                FeatureRunPhase::Verification | FeatureRunPhase::SourceFrozen
            )
        {
            return Ok(());
        }
        let Some(active) = repository.active_source_freeze(&gate.run_id)? else {
            return Ok(());
        };
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("final_review_repair_source_capture:{error}"))?;
        if snapshot.source.revision == active.source_revision
            && snapshot.source.tree_digest.as_str() == active.source_digest
        {
            return Ok(());
        }
        if persisted.run.phase == FeatureRunPhase::Verification {
            self.reconcile_active_phase_wall(&gate.run_id, BudgetPhase::Repair)?;
            self.reconcile_active_phase_wall(&gate.run_id, BudgetPhase::Verification)?;
        }
        let obligation_ids = self
            .conn
            .prepare(
                "SELECT id FROM proof_obligations WHERE plan_id = ?1 AND binding = 1 ORDER BY id",
            )?
            .query_map([&gate.scope_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        repository.invalidate_source(&EvidenceInvalidationRecord {
            id: short_id("invalidation"),
            run_id: gate.run_id.clone(),
            freeze_id: active.id,
            finding_id: repository
                .findings(&gate.id)?
                .first()
                .map(|finding| finding.id.clone()),
            reason: "resolved_final_review_repair_source_changed".to_string(),
            affected_evidence_ids: obligation_ids,
        })?;
        let replacement = SourceFreezeRecord {
            id: short_id("freeze"),
            run_id: gate.run_id.clone(),
            source_revision: snapshot.source.revision.clone(),
            source_digest: snapshot.source.tree_digest.as_str().to_string(),
            status: SourceFreezeStatus::Active,
        };
        repository.freeze_source(&replacement)?;
        let mut refrozen = if persisted.run.phase == FeatureRunPhase::Verification {
            apply_phase_transition(
                &persisted.run,
                &PhaseTransition {
                    to: FeatureRunPhase::SourceFrozen,
                    cause: PhaseTransitionCause::SourceInvalidated,
                    reference: format!("source_freeze:{}", replacement.id),
                    owner: None,
                },
            )
            .map_err(|violation| anyhow!("final_review_repair_refreeze:{violation:?}"))?
        } else {
            persisted.run.clone()
        };
        refrozen.source_revision = Some(replacement.source_revision);
        repository.save_feature_run(&refrozen, persisted.revision)?;
        Ok(())
    }

    pub(crate) fn resolve_review_gate_findings_value(
        &self,
        gate_id: &str,
        finding_ids: &[String],
    ) -> Result<Value> {
        if finding_ids.is_empty() {
            bail!("finding_resolution_requires_ids:{gate_id}");
        }
        let repository = ExecutionRunRepository::new(&self.conn);
        let gate = repository.review_gate(gate_id)?;
        if gate.status != ReviewGateStatus::ChangesRequested {
            bail!("review_gate_not_awaiting_repair:{gate_id}");
        }
        if gate.responsible_maker_id != worker_id() {
            bail!("finding_repair_requires_responsible_maker:{gate_id}");
        }
        if gate.kind == ReviewGateKind::FinalProduct
            && repository.active_source_freeze(&gate.run_id)?.is_none()
        {
            self.freeze_feature_run_source_value(&gate.scope_id)?
                .ok_or_else(|| anyhow!("final_review_repair_refreeze_failed:{gate_id}"))?;
        }
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT resolve_review_findings")?;
        let result = (|| -> Result<Value> {
            let persisted = repository.feature_run(&gate.run_id)?;
            self.refresh_risk_review_binding_after_finding_repair(
                &repository,
                &persisted,
                &gate,
                finding_ids,
            )?;
            self.reconcile_final_review_repair_source(&repository, &persisted, &gate)?;
            for finding_id in finding_ids {
                let current = repository
                    .findings(gate_id)?
                    .into_iter()
                    .find(|finding| finding.id == *finding_id)
                    .ok_or_else(|| anyhow!("review_finding_not_found:{finding_id}"))?;
                if current.status == FindingStatus::Open {
                    repository.set_finding_status(
                        finding_id,
                        FindingStatus::Open,
                        FindingStatus::Resolved,
                    )?;
                    self.record_event(
                        "finding_resolved",
                        Some(&gate.scope_id),
                        json!({"gate_id": gate_id, "finding_id": finding_id}),
                    )?;
                } else if current.status != FindingStatus::Resolved {
                    bail!("review_finding_not_repairable:{finding_id}");
                }
            }
            let remaining = repository
                .findings(gate_id)?
                .into_iter()
                .filter(|finding| finding.status == FindingStatus::Open)
                .collect::<Vec<_>>();
            if !remaining.is_empty() {
                bail!("review_gate_has_open_findings:{gate_id}");
            }
            let ready_for_review = if gate.kind == ReviewGateKind::FinalProduct {
                match self.capture_final_review_source_binding(
                    &gate.id,
                    &gate.run_id,
                    &gate.scope_id,
                ) {
                    Ok(binding) => {
                        repository.rebind_review_gate_source(&binding)?;
                        true
                    }
                    Err(error)
                        if error.to_string().starts_with(
                            "final_product_review_requires_satisfied_exact_source_coverage:",
                        ) =>
                    {
                        false
                    }
                    Err(error) => return Err(error),
                }
            } else {
                true
            };
            if ready_for_review {
                repository.set_review_gate_status(
                    gate_id,
                    ReviewGateStatus::ChangesRequested,
                    ReviewGateStatus::Pending,
                )?;
            }
            Ok(json!({
                "execution_state": self.canonical_execution_state_value(&gate.run_id, Some(gate_id))?,
                "resolved_finding_ids": finding_ids,
                "verification_required": !ready_for_review,
            }))
        })();
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE resolve_review_findings; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO resolve_review_findings; RELEASE resolve_review_findings; ROLLBACK",
                );
                Err(error)
            }
        }
    }

    pub(super) fn review_gate_pick_value_for_worker(
        &self,
        plan_id: &str,
        peek: bool,
        reviewer: &str,
    ) -> Result<Option<Value>> {
        let plan = self.get_plan(plan_id)?;
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let Some(gate) = repository.pending_review_gate_for_plan(&project.id, plan_id)? else {
            return Ok(None);
        };
        if peek {
            return Ok(Some(json!({
                "work_packet": {"kind": "review_gate", "execution_state": self.canonical_execution_state_value(&gate.run_id, Some(&gate.id))?},
                "peek": true,
                "remaining": self.progress_value()?,
            })));
        }
        let reviewer = reviewer.to_string();
        if reviewer == gate.responsible_maker_id {
            bail!("review_gate_requires_independent_reviewer:{}", gate.id);
        }
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT lease_review_gate")?;
        let mut gate_leased = false;
        let result = (|| -> Result<Value> {
            let persisted = repository.feature_run(&gate.run_id)?;
            if gate.kind == ReviewGateKind::RiskCheckpoint {
                let batch_id =
                    persisted.run.active_batch_id.as_deref().ok_or_else(|| {
                        anyhow!("feature_run_missing_active_batch:{}", gate.run_id)
                    })?;
                let batch = repository.batch(batch_id)?;
                if batch.batch.status == ExecutionBatchStatus::Active {
                    let paused = pause_batch_for_risk_review(&batch.batch)
                        .map_err(|violation| anyhow!("batch_pause_rejected:{violation:?}"))?;
                    repository.save_batch(&paused, batch.revision)?;
                }
            }
            let (to, cause) = match gate.kind {
                ReviewGateKind::RiskCheckpoint => (
                    FeatureRunPhase::RiskReview,
                    PhaseTransitionCause::ProtectedRiskDiscovered,
                ),
                ReviewGateKind::FinalProduct => (
                    FeatureRunPhase::FinalReview,
                    PhaseTransitionCause::VerificationPassed,
                ),
            };
            if persisted.run.phase == FeatureRunPhase::Verification {
                self.reconcile_active_phase_wall(&persisted.run.id, BudgetPhase::Verification)?;
            }
            let transitioned = apply_phase_transition(
                &persisted.run,
                &PhaseTransition {
                    to,
                    cause,
                    reference: format!("review_gate:{}", gate.id),
                    owner: Some(RoleOwner {
                        role: RunRole::Reviewer,
                        worker_id: reviewer.clone(),
                        lease_generation: self.conn.query_row(
                            "SELECT COALESCE(MAX(lease_generation), 0) + 1 FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'reviewer'",
                            [&gate.run_id],
                            |row| row.get::<_, u64>(0),
                        )?,
                    }),
                },
            )
            .map_err(|violation| anyhow!("review_gate_lease_transition:{violation:?}"))?;
            repository.save_feature_run(&transitioned, persisted.revision)?;
            let transitioned = repository.feature_run(&gate.run_id)?;
            match self.admit_feature_run_budget(
                &transitioned,
                BudgetPhase::Review,
                &format!("review:{}", gate.id),
                &reviewer,
                "review.dispatch",
            )? {
                FeatureRunBudgetAdmission::Held(hold) => return Ok(hold),
                FeatureRunBudgetAdmission::Reserved(_) => {}
            }
            let current = repository.review_gate(&gate.id)?;
            repository.set_review_gate_status(
                &gate.id,
                current.status,
                ReviewGateStatus::Leased,
            )?;
            gate_leased = true;
            Ok(json!({
                "work_packet": {
                    "kind": "review_gate",
                    "execution_state": self.canonical_execution_state_value(&gate.run_id, Some(&gate.id))?,
                    "reviewer_worker_id": reviewer,
                },
                "plan": plan,
                "remaining": self.progress_value()?,
            }))
        })();
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE lease_review_gate; COMMIT")?;
                if gate_leased {
                    self.record_event(
                        "review_gate_leased",
                        None,
                        json!({"gate_id": gate.id, "reviewer_worker_id": reviewer}),
                    )?;
                }
                Ok(Some(value))
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO lease_review_gate; RELEASE lease_review_gate; ROLLBACK",
                );
                Err(error)
            }
        }
    }

    pub(crate) fn complete_review_gate_value(
        &self,
        gate_id: &str,
        verdict: ReviewVerdict,
        findings: &[String],
        reviewer: Option<&str>,
    ) -> Result<Value> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT complete_review_gate")?;
        let result = self.complete_review_gate_locked(gate_id, verdict, findings, reviewer);
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE complete_review_gate; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO complete_review_gate; RELEASE complete_review_gate; ROLLBACK",
                );
                Err(error)
            }
        }
    }

    fn complete_review_gate_locked(
        &self,
        gate_id: &str,
        verdict: ReviewVerdict,
        findings: &[String],
        reviewer: Option<&str>,
    ) -> Result<Value> {
        let repository = ExecutionRunRepository::new(&self.conn);
        let gate = repository.review_gate(gate_id)?;
        if gate.status != ReviewGateStatus::Leased {
            bail!("review_gate_not_leased:{gate_id}");
        }
        self.validate_review_source_binding(&repository, &gate)?;
        if gate.kind == ReviewGateKind::FinalProduct {
            let stored = repository
                .review_source_binding(gate_id)?
                .expect("final review binding checked");
            let current =
                self.capture_final_review_source_binding(gate_id, &gate.run_id, &gate.scope_id)?;
            if stored != current
                || gate.source_revision.as_deref() != Some(stored.source_revision.as_str())
            {
                bail!("final_product_review_source_binding_stale:{gate_id}");
            }
        }
        let review_reservation =
            self.load_active_budget_reservation(&gate.run_id, &format!("review:{}", gate.id))?;
        let reviewer = reviewer
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(worker_id);
        if reviewer == gate.responsible_maker_id {
            bail!("review_gate_requires_independent_reviewer:{gate_id}");
        }
        let leased_run = repository.feature_run(&gate.run_id)?;
        let leased_reviewer = leased_run
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Reviewer)
            .ok_or_else(|| anyhow!("review_gate_missing_reviewer_lease:{gate_id}"))?;
        if leased_reviewer.worker_id != reviewer {
            bail!("review_gate_reviewer_lease_mismatch:{gate_id}");
        }
        if let Some(hold) = self.continue_review_budget(&leased_run, &gate.id, reviewer.as_str())? {
            return Ok(hold);
        }
        let attempt_id = short_id("attempt");
        let bounded = findings.iter().take(20).collect::<Vec<_>>();
        let finding_records = bounded
            .iter()
            .map(|_finding| FindingRecord {
                id: short_id("finding"),
                gate_id: gate.id.clone(),
                attempt_id: attempt_id.clone(),
                severity: "high".to_string(),
                target: gate.scope_id.clone(),
                owner_worker_id: gate.responsible_maker_id.clone(),
                status: FindingStatus::Open,
                invalidated_evidence_ids: Vec::new(),
            })
            .collect::<Vec<_>>();
        repository.append_review_attempt(
            &ReviewAttemptRecord {
                id: attempt_id,
                gate_id: gate.id.clone(),
                attempt_number: gate.latest_attempt + 1,
                reviewer_worker_id: reviewer.clone(),
                reviewer_mode: "independent".to_string(),
                verdict,
                source_revision: gate
                    .source_revision
                    .clone()
                    .unwrap_or_else(|| format!("risk-gate:{}", gate.id)),
                artifacts: bounded.iter().map(|value| (*value).clone()).collect(),
            },
            &finding_records,
            gate.latest_attempt,
        )?;
        self.record_event(
            "review_attempt_recorded",
            Some(&gate.scope_id),
            json!({"gate_id": &gate.id, "attempt": gate.latest_attempt + 1, "reviewer_worker_id": &reviewer, "verdict": verdict}),
        )?;
        for finding in &finding_records {
            self.record_event(
                "finding_opened",
                Some(&gate.scope_id),
                json!({"gate_id": &gate.id, "finding_id": &finding.id, "owner_worker_id": &finding.owner_worker_id}),
            )?;
        }
        let mut persisted = repository.feature_run(&gate.run_id)?;
        if gate.kind == ReviewGateKind::RiskCheckpoint {
            let transitioned = apply_phase_transition(
                &persisted.run,
                &PhaseTransition {
                    to: FeatureRunPhase::Implementation,
                    cause: PhaseTransitionCause::RiskCheckpointAccepted,
                    reference: format!("review_gate:{}", gate.id),
                    owner: None,
                },
            )
            .map_err(|violation| anyhow!("review_gate_completion_transition:{violation:?}"))?;
            repository.save_feature_run(&transitioned, persisted.revision)?;
            let batch_id = transitioned
                .active_batch_id
                .as_deref()
                .ok_or_else(|| anyhow!("feature_run_missing_active_batch:{}", transitioned.id))?;
            let batch = repository.batch(batch_id)?;
            let resumed = resume_batch_after_risk_review(&batch.batch, &gate.responsible_maker_id)
                .map_err(|violation| anyhow!("batch_resume_rejected:{violation:?}"))?;
            repository.save_batch(&resumed, batch.revision)?;
            persisted = repository.feature_run(&gate.run_id)?;
            self.record_event(
                "maker_resumed_after_review",
                Some(&gate.scope_id),
                json!({"gate_id": gate.id, "run_id": gate.run_id, "maker_worker_id": gate.responsible_maker_id, "batch_outcome_count": persisted.run.batch_outcome_count}),
            )?;
            if verdict == ReviewVerdict::Accepted {
                let accepted_gate = repository.review_gate(&gate.id)?;
                if let Some(handoff) = self.accepted_risk_verification_handoff_locked(
                    persisted.clone(),
                    &accepted_gate,
                    None,
                )? {
                    persisted = repository.feature_run(&gate.run_id)?;
                    if let Some(reservation) = review_reservation {
                        self.reconcile_feature_run_budget(
                            &reservation,
                            &BudgetUsageReport::application(Some(1)),
                        )?;
                    }
                    return Ok(json!({
                        "execution_state": self.canonical_execution_state_value(&persisted.run.id, Some(gate_id))?,
                        "created_map_items": [],
                        "verification_handoff": handoff,
                    }));
                }
            }
        } else if verdict == ReviewVerdict::Accepted {
            let completed = apply_phase_transition(
                &persisted.run,
                &PhaseTransition {
                    to: FeatureRunPhase::Complete,
                    cause: PhaseTransitionCause::FinalReviewAccepted,
                    reference: format!("review_gate:{}", gate.id),
                    owner: None,
                },
            )
            .map_err(|violation| anyhow!("final_review_completion_transition:{violation:?}"))?;
            repository.save_feature_run(&completed, persisted.revision)?;
            persisted = repository.feature_run(&gate.run_id)?;
        } else {
            let maker_generation = self.conn.query_row(
                "SELECT COALESCE(MAX(lease_generation), 0) + 1 FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'maker'",
                [&gate.run_id],
                |row| row.get::<_, u64>(0),
            )?;
            let mut repair = apply_phase_transition(
                &persisted.run,
                &PhaseTransition {
                    to: FeatureRunPhase::Implementation,
                    cause: PhaseTransitionCause::ProductFinding,
                    reference: format!("review_gate:{}", gate.id),
                    owner: Some(RoleOwner {
                        role: RunRole::Maker,
                        worker_id: gate.responsible_maker_id.clone(),
                        lease_generation: maker_generation,
                    }),
                },
            )
            .map_err(|violation| anyhow!("final_review_finding_transition:{violation:?}"))?;
            let batch = ExecutionBatch {
                id: short_id("batch"),
                run_id: repair.id.clone(),
                maker_worker_id: gate.responsible_maker_id.clone(),
                status: ExecutionBatchStatus::Active,
                settled_outcome_ids: Vec::new(),
                replacement: None,
            };
            repair.active_batch_id = Some(batch.id.clone());
            repair.batch_outcome_count = 0;
            if let Some(freeze) = repository.active_source_freeze(&gate.run_id)? {
                repository.invalidate_source(&EvidenceInvalidationRecord {
                    id: short_id("invalidation"),
                    run_id: gate.run_id.clone(),
                    freeze_id: freeze.id,
                    finding_id: finding_records.first().map(|finding| finding.id.clone()),
                    reason: "final_review_product_finding".to_string(),
                    affected_evidence_ids: Vec::new(),
                })?;
            }
            repository.save_feature_run_with_new_batch(&repair, persisted.revision, &batch)?;
            persisted = repository.feature_run(&gate.run_id)?;
        }
        if let Some(reservation) = review_reservation {
            self.reconcile_feature_run_budget(
                &reservation,
                &BudgetUsageReport::application(Some(1)),
            )?;
        }
        Ok(json!({
            "execution_state": self.canonical_execution_state_value(&persisted.run.id, Some(gate_id))?,
            "created_map_items": [],
        }))
    }
}
