use super::App;
use super::repository::execution_run::{ExecutionRunRepository, ReviewGateStatus};
use crate::evidence::coverage::evaluate_plan_coverage;
use crate::evidence::policy::capture_repository_snapshot;
use crate::execution_run::{
    ExecutionBatch, ExecutionBatchStatus, FeatureRunPhase, PhaseTransition, PhaseTransitionCause,
    RoleOwner, RunRole, apply_phase_transition,
};
use crate::usage_policy::BudgetPhase;
use crate::util::{short_id, worker_id};
use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const RECOVERY_SCHEMA: &str = "planr.evidence.recover_settlement.v1";
const HISTORICAL_RECONCILIATION_SCHEMA: &str =
    "planr.evidence.reconcile_historical_invalidation.v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct VerificationSettlementRecoveryInput {
    schema: String,
    plan_id: String,
    run_id: String,
    freeze_id: String,
    receipt_id: String,
    verifier_worker_id: String,
    verifier_generation: u64,
    next_item_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct HistoricalInvalidationReconciliationInput {
    schema: String,
    plan_id: String,
    run_id: String,
    invalidation_id: String,
    superseding_freeze_id: String,
    review_gate_id: String,
    receipt_id: String,
    next_item_id: String,
}

impl App {
    pub(crate) fn recover_verification_settlement_value(&self, input: Value) -> Result<Value> {
        if input["schema"].as_str() == Some(HISTORICAL_RECONCILIATION_SCHEMA) {
            let input: HistoricalInvalidationReconciliationInput = serde_json::from_value(input)
                .context("invalid historical invalidation reconciliation input")?;
            return self.reconcile_historical_invalidation_value(&input);
        }
        let input: VerificationSettlementRecoveryInput = serde_json::from_value(input)
            .context("invalid verification settlement recovery input")?;
        if input.schema != RECOVERY_SCHEMA {
            bail!("verification_settlement_recovery_schema_mismatch");
        }
        let current_worker = worker_id();
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT recover_verification_settlement")?;
        let result = self.recover_verification_settlement_locked(&input, &current_worker);
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE recover_verification_settlement; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO recover_verification_settlement; RELEASE recover_verification_settlement; ROLLBACK",
                );
                Err(error)
            }
        }
    }

    fn reconcile_historical_invalidation_value(
        &self,
        input: &HistoricalInvalidationReconciliationInput,
    ) -> Result<Value> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT reconcile_historical_invalidation")?;
        let result = self.reconcile_historical_invalidation_locked(input);
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE reconcile_historical_invalidation; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO reconcile_historical_invalidation; RELEASE reconcile_historical_invalidation; ROLLBACK",
                );
                Err(error)
            }
        }
    }

    fn reconcile_historical_invalidation_locked(
        &self,
        input: &HistoricalInvalidationReconciliationInput,
    ) -> Result<Value> {
        let project = self.default_project()?;
        let plan = self.get_plan(&input.plan_id)?;
        if plan.project_id != project.id {
            bail!("historical_invalidation_reconciliation_plan_project_mismatch");
        }
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository.feature_run(&input.run_id)?;
        if persisted.project_id != project.id || persisted.run.plan_id != input.plan_id {
            bail!("historical_invalidation_reconciliation_run_mismatch");
        }
        if let Some(existing) = self
            .historical_invalidation_reconciliation_payload(&input.run_id, &input.invalidation_id)?
        {
            let recorded: HistoricalInvalidationReconciliationInput =
                serde_json::from_value(existing["request"].clone())
                    .context("invalid persisted historical invalidation reconciliation event")?;
            if recorded != *input {
                bail!("historical_invalidation_reconciliation_request_mismatch");
            }
            return self.historical_invalidation_reconciliation_result(false, input);
        }
        if persisted.run.phase != FeatureRunPhase::Implementation {
            bail!("historical_invalidation_reconciliation_requires_implementation");
        }
        let maker = persisted
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Maker)
            .ok_or_else(|| anyhow!("historical_invalidation_reconciliation_missing_maker"))?;
        if maker.worker_id != worker_id() {
            bail!("historical_invalidation_reconciliation_wrong_maker");
        }
        let (item_plan_path, item_work_type, item_status, item_worker): (
            String,
            String,
            String,
            Option<String>,
        ) = self.conn.query_row(
            "SELECT plan_path, work_type, status, worker_id FROM items
             WHERE id = ?1 AND project_id = ?2",
            params![input.next_item_id, project.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        if item_plan_path != plan.path
            || item_work_type != "code"
            || !matches!(item_status.as_str(), "picked" | "running")
            || item_worker.as_deref() != Some(maker.worker_id.as_str())
        {
            bail!("historical_invalidation_reconciliation_next_item_mismatch");
        }

        let (old_freeze_id, finding_id, reason, invalidation_created_at): (
            String,
            Option<String>,
            String,
            String,
        ) = self.conn.query_row(
            "SELECT freeze_id, finding_id, reason, created_at
             FROM feature_run_evidence_invalidations
             WHERE id = ?1 AND run_id = ?2",
            params![input.invalidation_id, input.run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let finding_id = finding_id
            .ok_or_else(|| anyhow!("historical_invalidation_reconciliation_not_review_finding"))?;
        let (finding_status, finding_gate_id): (String, String) = self.conn.query_row(
            "SELECT status, gate_id FROM review_findings WHERE id = ?1 AND run_id = ?2",
            params![finding_id, input.run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if finding_status != "resolved" || finding_gate_id != input.review_gate_id {
            bail!("historical_invalidation_reconciliation_finding_unresolved_or_mismatched");
        }
        if repository
            .product_repair_settlement(&input.invalidation_id)?
            .is_some()
        {
            bail!("historical_invalidation_reconciliation_has_product_repair_settlement");
        }

        let active_freeze = repository
            .active_source_freeze(&input.run_id)?
            .ok_or_else(|| {
                anyhow!("historical_invalidation_reconciliation_missing_active_freeze")
            })?;
        if active_freeze.id != input.superseding_freeze_id || active_freeze.id == old_freeze_id {
            bail!("historical_invalidation_reconciliation_freeze_lineage_mismatch");
        }
        let (old_status, old_invalidated_at, active_created_at): (String, Option<String>, String) =
            self.conn.query_row(
                "SELECT old.status, old.invalidated_at, active.created_at
                 FROM feature_run_source_freezes old
                 JOIN feature_run_source_freezes active ON active.id = ?2
                 WHERE old.id = ?1 AND old.run_id = ?3 AND active.run_id = old.run_id",
                params![old_freeze_id, active_freeze.id, input.run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        if old_status != "invalidated"
            || old_invalidated_at.as_deref() != Some(invalidation_created_at.as_str())
            || active_created_at != invalidation_created_at
        {
            bail!("historical_invalidation_reconciliation_causal_boundary_mismatch");
        }
        let snapshot = capture_repository_snapshot(&self.root)
            .context("checking historical invalidation reconciliation source")?;
        if snapshot.source.revision != active_freeze.source_revision
            || snapshot.source.tree_digest.as_str() != active_freeze.source_digest
        {
            bail!("historical_invalidation_reconciliation_source_stale");
        }

        let (gate_status, gate_run_id): (String, String) = self.conn.query_row(
            "SELECT status, run_id FROM review_gates WHERE id = ?1",
            [&input.review_gate_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if gate_status != "accepted" || gate_run_id != input.run_id {
            bail!("historical_invalidation_reconciliation_review_not_accepted");
        }
        let review_binding = repository
            .review_source_binding(&input.review_gate_id)?
            .ok_or_else(|| {
                anyhow!("historical_invalidation_reconciliation_review_binding_missing")
            })?;
        if review_binding.freeze_id != active_freeze.id
            || review_binding.source_revision != active_freeze.source_revision
            || review_binding.source_digest != active_freeze.source_digest
        {
            bail!("historical_invalidation_reconciliation_review_source_mismatch");
        }

        let (receipt_digest, trusted_binding): (String, String) = self.conn.query_row(
            "SELECT receipts.receipt_digest, receipts.trusted_binding_json
             FROM evidence_receipts receipts
             JOIN proof_obligations obligations ON obligations.id = receipts.obligation_id
             WHERE receipts.id = ?1 AND receipts.project_id = ?2
               AND receipts.receipt_status = 'trusted' AND obligations.plan_id = ?3
               AND obligations.binding = 1",
            params![input.receipt_id, project.id, input.plan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let trusted_binding: Value = serde_json::from_str(&trusted_binding)?;
        if trusted_binding["source"]["revision"].as_str()
            != Some(active_freeze.source_revision.as_str())
            || trusted_binding["source"]["tree_digest"].as_str()
                != Some(active_freeze.source_digest.as_str())
        {
            bail!("historical_invalidation_reconciliation_receipt_source_mismatch");
        }
        let evaluated_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
        let coverage =
            evaluate_plan_coverage(&self.conn, &project.id, &input.plan_id, &evaluated_at)
                .map_err(|error| anyhow!("{error}"))?;
        if coverage.status.as_str() != "satisfied"
            || !coverage
                .receipt_digests
                .iter()
                .any(|digest| digest == &receipt_digest)
            || !coverage.waiver_digests.is_empty()
        {
            bail!("historical_invalidation_reconciliation_coverage_mismatch");
        }
        if review_binding.receipt_lineage != coverage.receipt_lineage {
            bail!("historical_invalidation_reconciliation_review_receipt_lineage_mismatch");
        }
        let active_adapter: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM evidence_attempts attempts
             JOIN proof_obligations obligations ON obligations.id = attempts.obligation_id
             WHERE attempts.project_id = ?1 AND obligations.plan_id = ?2
               AND attempts.completed_at IS NULL",
            params![project.id, input.plan_id],
            |row| row.get(0),
        )?;
        if active_adapter != 0 {
            bail!("historical_invalidation_reconciliation_active_adapter");
        }

        self.record_event(
            "historical_invalidation_reconciled",
            Some(&input.next_item_id),
            json!({
                "request": input,
                "finding_id": finding_id,
                "reason": reason,
                "old_freeze_id": old_freeze_id,
                "superseding_freeze_id": active_freeze.id,
                "receipt_digest": receipt_digest,
                "coverage_id": coverage.id,
                "receipt_lineage": coverage.receipt_lineage,
            }),
        )?;
        self.historical_invalidation_reconciliation_result(true, input)
    }

    pub(crate) fn historical_invalidation_reconciliation_payload(
        &self,
        run_id: &str,
        invalidation_id: &str,
    ) -> Result<Option<Value>> {
        self.conn
            .query_row(
                "SELECT payload FROM events
                 WHERE event_type = 'historical_invalidation_reconciled'
                   AND json_extract(payload, '$.request.run_id') = ?1
                   AND json_extract(payload, '$.request.invalidation_id') = ?2
                 ORDER BY id DESC LIMIT 1",
                params![run_id, invalidation_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }

    fn historical_invalidation_reconciliation_result(
        &self,
        created: bool,
        input: &HistoricalInvalidationReconciliationInput,
    ) -> Result<Value> {
        Ok(json!({
            "schema": "planr.evidence.reconcile_historical_invalidation.result.v1",
            "created": created,
            "request": input,
            "next_item_id": input.next_item_id,
            "execution_state": self.canonical_execution_state_value(&input.run_id, None)?,
        }))
    }

    fn recover_verification_settlement_locked(
        &self,
        input: &VerificationSettlementRecoveryInput,
        current_worker: &str,
    ) -> Result<Value> {
        let project = self.default_project()?;
        let plan = self.get_plan(&input.plan_id)?;
        if plan.project_id != project.id {
            bail!("verification_settlement_recovery_plan_project_mismatch");
        }
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository.feature_run(&input.run_id)?;
        if persisted.project_id != project.id || persisted.run.plan_id != input.plan_id {
            bail!("verification_settlement_recovery_run_mismatch");
        }

        if let Some(payload) = self.existing_recovery_payload(&input.run_id)? {
            let recorded: VerificationSettlementRecoveryInput =
                serde_json::from_value(payload["request"].clone())
                    .context("invalid persisted verification settlement recovery event")?;
            if recorded != *input {
                bail!("verification_settlement_recovery_request_mismatch");
            }
            let maker = persisted
                .run
                .role_owners
                .iter()
                .find(|owner| owner.role == RunRole::Maker)
                .ok_or_else(|| anyhow!("verification_settlement_recovery_missing_maker"))?;
            if persisted.run.phase != FeatureRunPhase::Implementation
                || maker.worker_id != current_worker
            {
                bail!("verification_settlement_recovery_idempotence_state_mismatch");
            }
            return self.recovery_result(false, input, &persisted.run.id);
        }

        if persisted.run.phase != FeatureRunPhase::Verification {
            bail!("verification_settlement_recovery_requires_verification_phase");
        }
        let verifier = persisted
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Verifier)
            .ok_or_else(|| anyhow!("verification_settlement_recovery_missing_verifier"))?;
        if verifier.worker_id != input.verifier_worker_id
            || verifier.lease_generation != input.verifier_generation
        {
            bail!("verification_settlement_recovery_verifier_mismatch");
        }

        let (item_plan_path, item_work_type, item_status, item_worker): (
            String,
            String,
            String,
            Option<String>,
        ) = self.conn.query_row(
            "SELECT plan_path, work_type, status, worker_id FROM items
             WHERE id = ?1 AND project_id = ?2",
            params![input.next_item_id, project.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        if item_plan_path != plan.path
            || item_work_type != "code"
            || !matches!(item_status.as_str(), "picked" | "running")
            || item_worker.as_deref() != Some(current_worker)
        {
            bail!("verification_settlement_recovery_next_item_mismatch");
        }
        let historical_maker: String = self.conn.query_row(
            "SELECT worker_id FROM feature_run_role_leases
             WHERE run_id = ?1 AND role = 'maker'
             ORDER BY lease_generation DESC LIMIT 1",
            [&input.run_id],
            |row| row.get(0),
        )?;
        if historical_maker != current_worker {
            bail!("verification_settlement_recovery_wrong_maker");
        }
        let competing_code: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM items WHERE project_id = ?1 AND plan_path = ?2
             AND work_type = 'code' AND status IN ('picked','running') AND id <> ?3",
            params![project.id, plan.path, input.next_item_id],
            |row| row.get(0),
        )?;
        if competing_code != 0 {
            bail!("verification_settlement_recovery_competing_code_lease");
        }
        let unresolved_predecessors: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM links JOIN items predecessor ON predecessor.id = links.from_item
             WHERE links.to_item = ?1 AND links.kind IN ('blocks','requires')
               AND predecessor.status <> 'closed'",
            [&input.next_item_id],
            |row| row.get(0),
        )?;
        if unresolved_predecessors != 0 {
            bail!("verification_settlement_recovery_unresolved_graph");
        }
        let closed_verification: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM items WHERE project_id = ?1 AND plan_path = ?2
             AND work_type = 'verification' AND status = 'closed'",
            params![project.id, plan.path],
            |row| row.get(0),
        )?;
        if closed_verification == 0 {
            bail!("verification_settlement_recovery_verification_not_settled");
        }

        let freeze = repository
            .active_source_freeze(&input.run_id)?
            .ok_or_else(|| anyhow!("verification_settlement_recovery_missing_freeze"))?;
        if freeze.id != input.freeze_id {
            bail!("verification_settlement_recovery_freeze_mismatch");
        }
        let snapshot = capture_repository_snapshot(&self.root)
            .context("checking verification settlement recovery source")?;
        if snapshot.source.revision != freeze.source_revision
            || snapshot.source.tree_digest.as_str() != freeze.source_digest
        {
            bail!("verification_settlement_recovery_source_stale");
        }

        let (receipt_digest, trusted_binding): (String, String) = self.conn.query_row(
            "SELECT receipts.receipt_digest, receipts.trusted_binding_json
             FROM evidence_receipts receipts
             JOIN proof_obligations obligations ON obligations.id = receipts.obligation_id
             WHERE receipts.id = ?1 AND receipts.project_id = ?2
               AND receipts.receipt_status = 'trusted' AND obligations.plan_id = ?3
               AND obligations.binding = 1",
            params![input.receipt_id, project.id, input.plan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let trusted_binding: Value = serde_json::from_str(&trusted_binding)?;
        if trusted_binding["source"]["revision"].as_str() != Some(freeze.source_revision.as_str())
            || trusted_binding["source"]["tree_digest"].as_str()
                != Some(freeze.source_digest.as_str())
        {
            bail!("verification_settlement_recovery_receipt_source_mismatch");
        }
        let (coverage_status, receipt_set, waiver_set): (String, String, String) =
            self.conn.query_row(
                "SELECT coverage_status, source_receipt_digest_set, waiver_digest_set
             FROM coverage_verdicts WHERE project_id = ?1 AND scope_kind = 'plan' AND scope_id = ?2
             ORDER BY computed_at DESC, id DESC LIMIT 1",
                params![project.id, input.plan_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        let receipt_set: Vec<String> = serde_json::from_str(&receipt_set)?;
        let waiver_set: Vec<String> = serde_json::from_str(&waiver_set)?;
        if coverage_status != "satisfied"
            || !receipt_set.iter().any(|digest| digest == &receipt_digest)
            || !waiver_set.is_empty()
        {
            bail!("verification_settlement_recovery_coverage_mismatch");
        }
        let active_adapter: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM evidence_attempts attempts
             JOIN proof_obligations obligations ON obligations.id = attempts.obligation_id
             WHERE attempts.project_id = ?1 AND obligations.plan_id = ?2
               AND attempts.completed_at IS NULL",
            params![project.id, input.plan_id],
            |row| row.get(0),
        )?;
        if active_adapter != 0 {
            bail!("verification_settlement_recovery_active_adapter");
        }
        if repository
            .review_gates_for_run(&input.run_id, false)?
            .iter()
            .any(|gate| {
                matches!(
                    gate.status,
                    ReviewGateStatus::Pending | ReviewGateStatus::Leased
                )
            })
        {
            bail!("verification_settlement_recovery_open_review_gate");
        }

        let maker_generation: u64 = self.conn.query_row(
            "SELECT COALESCE(MAX(lease_generation), 0) + 1 FROM feature_run_role_leases
             WHERE run_id = ?1 AND role = 'maker'",
            [&input.run_id],
            |row| row.get(0),
        )?;
        let mut continued = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::Implementation,
                cause: PhaseTransitionCause::VerificationPassed,
                reference: format!("verification_settlement_recovery:{}", input.receipt_id),
                owner: Some(RoleOwner {
                    role: RunRole::Maker,
                    worker_id: current_worker.to_string(),
                    lease_generation: maker_generation,
                }),
            },
        )
        .map_err(|violation| {
            anyhow!("verification_settlement_recovery_transition:{violation:?}")
        })?;
        let batch = ExecutionBatch {
            id: short_id("batch"),
            run_id: input.run_id.clone(),
            maker_worker_id: current_worker.to_string(),
            status: ExecutionBatchStatus::Active,
            settled_outcome_ids: Vec::new(),
            replacement: None,
        };
        continued.active_batch_id = Some(batch.id.clone());
        continued.batch_outcome_count = 0;
        self.reconcile_active_phase_wall(&input.run_id, BudgetPhase::Verification)?;
        repository.save_feature_run_with_new_batch(&continued, persisted.revision, &batch)?;
        self.record_event(
            "verification_settlement_recovered",
            Some(&input.next_item_id),
            json!({
                "request": input,
                "receipt_digest": receipt_digest,
                "maker_worker_id": current_worker,
                "maker_generation": maker_generation,
                "batch_id": batch.id,
            }),
        )?;
        self.recovery_result(true, input, &input.run_id)
    }

    fn existing_recovery_payload(&self, run_id: &str) -> Result<Option<Value>> {
        self.conn
            .query_row(
                "SELECT payload FROM events WHERE event_type = 'verification_settlement_recovered'
                 AND json_extract(payload, '$.request.run_id') = ?1 ORDER BY id DESC LIMIT 1",
                [run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }

    fn recovery_result(
        &self,
        created: bool,
        input: &VerificationSettlementRecoveryInput,
        run_id: &str,
    ) -> Result<Value> {
        Ok(json!({
            "schema": "planr.evidence.recover_settlement.result.v1",
            "created": created,
            "request": input,
            "next_item_id": input.next_item_id,
            "execution_state": self.canonical_execution_state_value(run_id, None)?,
        }))
    }
}
