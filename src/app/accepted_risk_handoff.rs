use super::App;
use super::lease::PickFilter;
use super::repository::execution_run::{
    EvidenceInvalidationRecord, ExecutionRunRepository, PersistedFeatureRun, ReviewGateKind,
    ReviewGateRecord, ReviewGateStatus, ReviewSourceBindingRecord, SourceFreezeRecord,
    SourceFreezeStatus,
};
use crate::app::execution_state::{
    CanonicalPlanrExecutableIdentity, observe_planr_executable_identity,
};
use crate::evidence::policy::capture_repository_snapshot;
use crate::execution_run::{
    ExecutionBatchStatus, FeatureRunPhase, PhaseTransition, PhaseTransitionCause, RunRole,
    apply_phase_transition,
};
use crate::usage_policy::BudgetPhase;
use crate::util::{short_id, worker_id};
use anyhow::{Result, anyhow, bail};
use rusqlite::params;
use serde_json::{Value, json};

impl App {
    fn active_risk_review_obligation_ids(
        &self,
        plan_id: &str,
        verification_item_id: Option<&str>,
    ) -> Result<Vec<String>> {
        let obligation_ids = self
            .conn
            .prepare(
                "SELECT obligations.id FROM proof_obligations obligations
                 WHERE obligations.plan_id = ?1
                   AND (?2 IS NULL OR obligations.item_id = ?2)
                   AND obligations.binding = 1
                   AND NOT EXISTS(
                     SELECT 1 FROM proof_obligations successors
                     WHERE successors.supersedes_obligation_id = obligations.id
                   )
                 ORDER BY obligations.id",
            )?
            .query_map(params![plan_id, verification_item_id], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if obligation_ids.is_empty()
            && let Some(verification_item_id) = verification_item_id
        {
            bail!("risk_review_active_obligations_missing:{verification_item_id}");
        }
        Ok(obligation_ids)
    }

    pub(crate) fn refresh_risk_review_binding_after_finding_repair(
        &self,
        repository: &ExecutionRunRepository<'_>,
        persisted: &PersistedFeatureRun,
        gate: &ReviewGateRecord,
        finding_ids: &[String],
    ) -> Result<()> {
        if gate.kind != ReviewGateKind::RiskCheckpoint {
            return Ok(());
        }
        let Some(binding) = repository.review_source_binding(&gate.id)? else {
            return Ok(());
        };
        let active = repository
            .active_source_freeze(&gate.run_id)?
            .ok_or_else(|| anyhow!("risk_review_repair_missing_bound_freeze:{}", gate.id))?;
        if active.id != binding.freeze_id
            || active.source_revision != binding.source_revision
            || active.source_digest != binding.source_digest
        {
            bail!("risk_review_repair_bound_freeze_mismatch:{}", gate.id);
        }
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("risk_review_repair_source_capture:{error}"))?;
        if snapshot.source.revision == active.source_revision
            && snapshot.source.tree_digest.as_str() == active.source_digest
        {
            return Ok(());
        }
        self.reconcile_active_phase_wall(&gate.run_id, BudgetPhase::Repair)?;
        let obligation_ids = self
            .conn
            .prepare(
                "SELECT id FROM proof_obligations WHERE plan_id = ?1 AND binding = 1 ORDER BY id",
            )?
            .query_map([&persisted.run.plan_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        repository.invalidate_source(&EvidenceInvalidationRecord {
            id: short_id("invalidation"),
            run_id: gate.run_id.clone(),
            freeze_id: active.id,
            finding_id: finding_ids.first().cloned(),
            reason: "resolved_risk_review_repair_source_changed".to_string(),
            affected_evidence_ids: obligation_ids,
        })?;
        let replacement = SourceFreezeRecord {
            id: short_id("freeze"),
            run_id: gate.run_id.clone(),
            source_revision: snapshot.source.revision,
            source_digest: snapshot.source.tree_digest.as_str().to_string(),
            status: SourceFreezeStatus::Active,
        };
        repository.freeze_source(&replacement)?;
        repository.rebind_review_gate_source(&ReviewSourceBindingRecord {
            gate_id: gate.id.clone(),
            freeze_id: replacement.id,
            source_revision: replacement.source_revision,
            source_digest: replacement.source_digest,
            receipt_lineage: json!({
                "kind": "risk_review_finding_repair",
                "finding_ids": finding_ids,
                "supersedes": binding.receipt_lineage,
            }),
        })?;
        Ok(())
    }

    fn accepted_risk_bound_freeze(
        &self,
        repository: &ExecutionRunRepository<'_>,
        run_id: &str,
        gate: &ReviewGateRecord,
        source_revision: &str,
        source_digest: &str,
    ) -> Result<Option<SourceFreezeRecord>> {
        let Some(binding) = repository.review_source_binding(&gate.id)? else {
            return Ok(None);
        };
        let freeze = repository
            .active_source_freeze(run_id)?
            .ok_or_else(|| anyhow!("accepted_risk_bound_source_freeze_missing:{}", gate.id))?;
        if binding.freeze_id != freeze.id
            || binding.source_revision != freeze.source_revision
            || binding.source_digest != freeze.source_digest
            || gate.source_revision.as_deref() != Some(binding.source_revision.as_str())
        {
            bail!("accepted_risk_bound_source_freeze_mismatch:{}", gate.id);
        }
        if source_revision != binding.source_revision || source_digest != binding.source_digest {
            bail!("accepted_risk_bound_source_freeze_stale:{}", gate.id);
        }
        Ok(Some(freeze))
    }

    pub(crate) fn accepted_risk_verification_handoff_locked(
        &self,
        persisted: PersistedFeatureRun,
        gate: &ReviewGateRecord,
        planr_executable: Option<&CanonicalPlanrExecutableIdentity>,
    ) -> Result<Option<Value>> {
        if gate.kind != ReviewGateKind::RiskCheckpoint || gate.status != ReviewGateStatus::Accepted
        {
            return Ok(None);
        }
        let plan = self.get_plan(&persisted.run.plan_id)?;
        let scope = self.get_item(&gate.scope_id)?;
        if scope.plan_path.as_deref() != Some(plan.path.as_str()) {
            bail!("risk_checkpoint_scope_plan_mismatch:{}", gate.id);
        }
        let repository = ExecutionRunRepository::new(&self.conn);
        if persisted.run.phase == FeatureRunPhase::SourceFrozen {
            let freeze = repository
                .active_source_freeze(&persisted.run.id)?
                .ok_or_else(|| anyhow!("source_frozen_run_missing_freeze:{}", persisted.run.id))?;
            let snapshot = capture_repository_snapshot(&self.root)
                .map_err(|error| anyhow!("checking accepted-risk source freeze: {error}"))?;
            self.accepted_risk_bound_freeze(
                &repository,
                &persisted.run.id,
                gate,
                &snapshot.source.revision,
                snapshot.source.tree_digest.as_str(),
            )?;
            if snapshot.source.revision != freeze.source_revision
                || snapshot.source.tree_digest.as_str() != freeze.source_digest
            {
                bail!("source_freeze_stale:{}", freeze.id);
            }
            let verification_item_id =
                self.ready_verification_item_for_plan_path(Some(plan.path.as_str()))?;
            let source_freeze = serde_json::to_value(freeze)?;
            return match planr_executable {
                Some(identity) => self.canonical_source_frozen_handoff_value_with_identity(
                    &persisted.run.plan_id,
                    verification_item_id,
                    source_freeze,
                    identity,
                ),
                None => self.canonical_source_frozen_handoff_value(
                    &persisted.run.plan_id,
                    verification_item_id,
                    source_freeze,
                ),
            }
            .map(Some);
        }
        if persisted.run.phase != FeatureRunPhase::Implementation {
            return Ok(None);
        }
        let maker = persisted
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Maker)
            .ok_or_else(|| anyhow!("feature_run_missing_maker:{}", persisted.run.id))?;
        if maker.worker_id != gate.responsible_maker_id {
            bail!("accepted_risk_gate_maker_mismatch:{}", gate.id);
        }
        if self
            .peek_next_ready_item_filtered(&PickFilter {
                exclude: None,
                work_type: Some("code"),
                plan_path: Some(plan.path.as_str()),
            })?
            .is_some()
        {
            return Ok(None);
        }
        let verification_item_id =
            self.ready_verification_item_for_plan_path(Some(plan.path.as_str()))?;
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("capturing accepted-risk source freeze: {error}"))?;
        let bound_freeze = self.accepted_risk_bound_freeze(
            &repository,
            &persisted.run.id,
            gate,
            &snapshot.source.revision,
            snapshot.source.tree_digest.as_str(),
        )?;
        let create_freeze = bound_freeze.is_none();
        let freeze = bound_freeze.unwrap_or_else(|| SourceFreezeRecord {
            id: short_id("freeze"),
            run_id: persisted.run.id.clone(),
            source_revision: snapshot.source.revision.clone(),
            source_digest: snapshot.source.tree_digest.as_str().to_string(),
            status: SourceFreezeStatus::Active,
        });
        self.reconcile_active_phase_wall(&persisted.run.id, BudgetPhase::Implementation)?;
        if let Some(batch_id) = persisted.run.active_batch_id.as_deref() {
            let batch = repository.batch(batch_id)?;
            if batch.batch.status == ExecutionBatchStatus::Active {
                let mut ended = batch.batch;
                ended.status = ExecutionBatchStatus::Ended;
                repository.save_batch(&ended, batch.revision)?;
            }
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
        .map_err(|violation| anyhow!("accepted_risk_source_freeze_transition:{violation:?}"))?;
        repository.save_feature_run(&frozen, persisted.revision)?;
        if create_freeze {
            repository.freeze_source(&freeze)?;
        }
        let active_binding = self.plan_has_active_binding_obligations(&persisted.run.plan_id)?;
        let active_obligation_ids = if active_binding {
            self.active_risk_review_obligation_ids(
                &persisted.run.plan_id,
                verification_item_id.as_deref(),
            )?
        } else {
            Vec::new()
        };
        repository.seal_accepted_risk_review_source_binding(&ReviewSourceBindingRecord {
            gate_id: gate.id.clone(),
            freeze_id: freeze.id.clone(),
            source_revision: freeze.source_revision.clone(),
            source_digest: freeze.source_digest.clone(),
            receipt_lineage: json!({
                "kind": "risk_review_acceptance",
                "active_obligation_ids": active_obligation_ids,
            }),
        })?;
        if let Some(verification_item_id) = verification_item_id.as_deref() {
            let released = self.conn.execute(
                "UPDATE items SET status = 'ready', worker_id = NULL, pick_token = NULL,
                        picked_at = NULL, last_heartbeat_at = NULL, updated_at = datetime('now')
                 WHERE id = ?1 AND plan_path = ?2 AND work_type = 'verification'
                   AND status = 'ready'",
                params![verification_item_id, plan.path],
            )?;
            if released != 1 {
                bail!("accepted_risk_verification_item_not_ready:{verification_item_id}");
            }
        }
        let source_freeze = serde_json::to_value(&freeze)?;
        let handoff = match planr_executable {
            Some(identity) => self.canonical_source_frozen_handoff_value_with_identity(
                &persisted.run.plan_id,
                verification_item_id,
                source_freeze,
                identity,
            ),
            None => self.canonical_source_frozen_handoff_value(
                &persisted.run.plan_id,
                verification_item_id,
                source_freeze,
            ),
        }?;
        let event_kind = if handoff["work_packet"]["kind"] == "verification_handoff" {
            "accepted_risk_verification_handoff"
        } else {
            "accepted_risk_final_review_handoff"
        };
        self.record_event(
            event_kind,
            Some(&gate.scope_id),
            json!({
                "gate_id": gate.id,
                "run_id": persisted.run.id,
                "verification_item_id": handoff["work_packet"]["verification_item_id"],
                "source_freeze_id": freeze.id,
            }),
        )?;
        Ok(Some(handoff))
    }

    pub(crate) fn resume_accepted_risk_verification_handoff_value(
        &self,
        plan_id: &str,
    ) -> Result<Option<Value>> {
        let project = self.default_project()?;
        let preflight_repository = ExecutionRunRepository::new(&self.conn);
        let Some(preflight_run) =
            preflight_repository.active_feature_run_for_plan(&project.id, plan_id)?
        else {
            return Ok(None);
        };
        let Some(preflight_gate) = preflight_repository
            .review_gates_for_run(&preflight_run.run.id, false)?
            .into_iter()
            .rev()
            .find(|gate| {
                gate.kind == ReviewGateKind::RiskCheckpoint
                    && gate.status == ReviewGateStatus::Accepted
            })
        else {
            return Ok(None);
        };
        if preflight_gate.responsible_maker_id != worker_id() {
            bail!(
                "accepted_risk_handoff_requires_responsible_maker:{}",
                preflight_gate.id
            );
        }
        if !self.conn.is_autocommit() {
            return self.accepted_risk_verification_handoff_locked(
                preflight_run,
                &preflight_gate,
                None,
            );
        }
        let planr_executable = observe_planr_executable_identity(&std::env::current_exe()?)?;
        self.conn.execute_batch(
            "BEGIN IMMEDIATE; SAVEPOINT resume_accepted_risk_verification_handoff",
        )?;
        let result = (|| -> Result<Option<Value>> {
            let repository = ExecutionRunRepository::new(&self.conn);
            let Some(persisted) = repository.active_feature_run_for_plan(&project.id, plan_id)?
            else {
                return Ok(None);
            };
            let gate = repository
                .review_gates_for_run(&persisted.run.id, false)?
                .into_iter()
                .rev()
                .find(|gate| {
                    gate.kind == ReviewGateKind::RiskCheckpoint
                        && gate.status == ReviewGateStatus::Accepted
                });
            let Some(gate) = gate else {
                return Ok(None);
            };
            if gate.responsible_maker_id != worker_id() {
                bail!(
                    "accepted_risk_handoff_requires_responsible_maker:{}",
                    gate.id
                );
            }
            self.accepted_risk_verification_handoff_locked(
                persisted,
                &gate,
                Some(&planr_executable),
            )
        })();
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE resume_accepted_risk_verification_handoff; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO resume_accepted_risk_verification_handoff; RELEASE resume_accepted_risk_verification_handoff; ROLLBACK",
                );
                Err(error)
            }
        }
    }
}
