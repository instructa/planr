use super::App;
use super::repository::execution_run::{
    ExecutionRunRepository, PersistedExecutionBatch, PersistedFeatureRun, ReviewGateKind,
    ReviewGateStatus,
};
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
use std::collections::BTreeSet;
use time::{OffsetDateTime, PrimitiveDateTime, format_description::well_known::Rfc3339};

const RECOVERY_SCHEMA: &str = "planr.evidence.recover_settlement.v1";
const HISTORICAL_RECONCILIATION_SCHEMA: &str =
    "planr.evidence.reconcile_historical_invalidation.v1";
const RISK_REVIEW_OBLIGATION_BACKFILL_SCHEMA: &str =
    "planr.evidence.backfill_risk_review_obligations.v1";
const VERIFIED_CONTINUATION_RECOVERY_SCHEMA: &str =
    "planr.evidence.recover_verified_continuation.v1";
const SQLITE_UTC_TIMESTAMP_FORMAT: &str = "[year]-[month]-[day] [hour]:[minute]:[second]";

fn parse_persisted_timestamp(value: &str) -> Result<OffsetDateTime> {
    if let Ok(timestamp) = OffsetDateTime::parse(value, &Rfc3339) {
        return Ok(timestamp);
    }
    let format = time::format_description::parse(SQLITE_UTC_TIMESTAMP_FORMAT)
        .context("invalid persisted timestamp format description")?;
    let timestamp = PrimitiveDateTime::parse(value, &format)
        .map_err(|_| anyhow!("invalid or ambiguous persisted timestamp"))?;
    Ok(timestamp.assume_utc())
}

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RiskReviewObligationBackfillInput {
    schema: String,
    plan_id: String,
    run_id: String,
    review_gate_id: String,
    freeze_id: String,
    receipt_id: String,
    verification_item_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerifiedContinuationRecoveryInput {
    schema: String,
    plan_id: String,
    run_id: String,
    freeze_id: String,
    receipt_id: String,
    recovery_item_id: String,
    final_item_id: String,
}

impl App {
    pub(crate) fn complete_verified_continuation_after_outcome(
        &self,
        persisted: &PersistedFeatureRun,
        batch_id: &str,
        item_id: &str,
        transition: &'static str,
        checkpoint_required: bool,
    ) -> Result<(&'static str, u64)> {
        let next_revision = persisted.revision + 1;
        if checkpoint_required {
            return Ok((transition, next_revision));
        }
        let mut after_outcome = persisted.clone();
        after_outcome.revision = next_revision;
        after_outcome.run.outcomes_settled += 1;
        after_outcome.run.batch_outcome_count += 1;
        let batch = ExecutionRunRepository::new(&self.conn).batch(batch_id)?;
        if self
            .complete_verified_continuation(&after_outcome, &batch, item_id, None)?
            .is_some()
        {
            return Ok(("verified_continuation_complete", next_revision + 1));
        }
        Ok((transition, next_revision))
    }

    pub(crate) fn complete_verified_continuation(
        &self,
        persisted: &PersistedFeatureRun,
        batch: &PersistedExecutionBatch,
        final_item_id: &str,
        public_request: Option<&VerifiedContinuationRecoveryInput>,
    ) -> Result<Option<Value>> {
        let recovery_payload = self.existing_recovery_payload(&persisted.run.id)?;
        let Some(recovery_payload) = recovery_payload else {
            return Ok(None);
        };
        let request: VerificationSettlementRecoveryInput =
            serde_json::from_value(recovery_payload["request"].clone())
                .context("invalid persisted verification settlement recovery lineage")?;
        if let Some(public_request) = public_request {
            if public_request.plan_id != request.plan_id
                || public_request.run_id != request.run_id
                || public_request.freeze_id != request.freeze_id
                || public_request.receipt_id != request.receipt_id
                || public_request.recovery_item_id != request.next_item_id
            {
                bail!("verified_continuation_recovery_lineage_mismatch");
            }
        }
        if request.plan_id != persisted.run.plan_id || request.run_id != persisted.run.id {
            bail!("verified_continuation_lineage_run_mismatch");
        }
        if persisted.run.phase != FeatureRunPhase::Implementation
            || batch.batch.run_id != persisted.run.id
            || batch.batch.status != ExecutionBatchStatus::Active
        {
            bail!("verified_continuation_state_mismatch");
        }
        let maker = persisted
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Maker)
            .ok_or_else(|| anyhow!("verified_continuation_missing_maker"))?;
        if maker.worker_id != worker_id()
            || batch.batch.maker_worker_id != maker.worker_id
            || recovery_payload["maker_worker_id"].as_str() != Some(maker.worker_id.as_str())
            || recovery_payload["maker_generation"].as_u64() != Some(maker.lease_generation)
        {
            bail!("verified_continuation_maker_lineage_mismatch");
        }
        let project = self.default_project()?;
        let plan = self.get_plan(&request.plan_id)?;
        let remaining_work: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM items WHERE project_id = ?1 AND plan_path = ?2
             AND id <> ?3 AND work_type IN ('code','verification')
             AND status NOT IN ('closed','closed_partial','cancelled')",
            params![project.id, plan.path, final_item_id],
            |row| row.get(0),
        )?;
        if remaining_work != 0 {
            return Ok(None);
        }
        let (verification_items, closed_verification): (i64, i64) = self.conn.query_row(
            "SELECT COUNT(*), SUM(CASE WHEN status = 'closed' THEN 1 ELSE 0 END)
             FROM items WHERE project_id = ?1 AND plan_path = ?2 AND work_type = 'verification'",
            params![project.id, plan.path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if verification_items != 1 || closed_verification != 1 {
            bail!("verified_continuation_verification_not_closed");
        }
        let repository = ExecutionRunRepository::new(&self.conn);
        let freeze = repository
            .active_source_freeze(&persisted.run.id)?
            .ok_or_else(|| anyhow!("verified_continuation_missing_freeze"))?;
        if freeze.id != request.freeze_id {
            bail!("verified_continuation_freeze_mismatch");
        }
        let snapshot = capture_repository_snapshot(&self.root)
            .context("checking verified continuation source")?;
        if snapshot.source.revision != freeze.source_revision
            || snapshot.source.tree_digest.as_str() != freeze.source_digest
        {
            bail!("verified_continuation_source_stale");
        }
        let (receipt_digest, trusted_binding): (String, String) = self.conn.query_row(
            "SELECT receipts.receipt_digest, receipts.trusted_binding_json
             FROM evidence_receipts receipts
             JOIN proof_obligations obligations ON obligations.id = receipts.obligation_id
             WHERE receipts.id = ?1 AND receipts.project_id = ?2
               AND receipts.receipt_status = 'trusted' AND obligations.plan_id = ?3
               AND obligations.binding = 1",
            params![request.receipt_id, project.id, request.plan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let trusted_binding: Value = serde_json::from_str(&trusted_binding)?;
        if trusted_binding["source"]["revision"].as_str() != Some(freeze.source_revision.as_str())
            || trusted_binding["source"]["tree_digest"].as_str()
                != Some(freeze.source_digest.as_str())
        {
            bail!("verified_continuation_receipt_source_mismatch");
        }
        let (coverage_id, coverage_status, receipt_set, waiver_set): (
            String,
            String,
            String,
            String,
        ) = self.conn.query_row(
            "SELECT id, coverage_status, source_receipt_digest_set, waiver_digest_set
             FROM coverage_verdicts WHERE project_id = ?1 AND scope_kind = 'plan' AND scope_id = ?2
             ORDER BY computed_at DESC, id DESC LIMIT 1",
            params![project.id, request.plan_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let receipt_set: Vec<String> = serde_json::from_str(&receipt_set)?;
        let waiver_set: Vec<String> = serde_json::from_str(&waiver_set)?;
        if coverage_status != "satisfied"
            || !receipt_set.contains(&receipt_digest)
            || !waiver_set.is_empty()
        {
            bail!("verified_continuation_coverage_mismatch");
        }
        let active_attempts: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM evidence_attempts attempts
             JOIN proof_obligations obligations ON obligations.id = attempts.obligation_id
             WHERE attempts.project_id = ?1 AND obligations.plan_id = ?2
               AND attempts.completed_at IS NULL",
            params![project.id, request.plan_id],
            |row| row.get(0),
        )?;
        if active_attempts != 0 {
            bail!("verified_continuation_active_adapter");
        }
        let mut ended = batch.batch.clone();
        ended.status = ExecutionBatchStatus::Ended;
        repository.save_batch(&ended, batch.revision)?;
        let frozen = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::SourceFrozen,
                cause: PhaseTransitionCause::ImplementationSettled,
                reference: freeze.source_revision.clone(),
                owner: None,
            },
        )
        .map_err(|violation| anyhow!("verified_continuation_transition:{violation:?}"))?;
        repository.save_feature_run(&frozen, persisted.revision)?;
        self.record_event(
            "verified_continuation_completed",
            Some(final_item_id),
            json!({
                "plan_id": request.plan_id,
                "run_id": request.run_id,
                "freeze_id": request.freeze_id,
                "receipt_id": request.receipt_id,
                "receipt_digest": receipt_digest,
                "coverage_id": coverage_id,
                "verifier_worker_id": request.verifier_worker_id,
                "verifier_generation": request.verifier_generation,
                "recovery_item_id": request.next_item_id,
                "final_item_id": final_item_id,
                "batch_id": batch.batch.id,
            }),
        )?;
        Ok(Some(json!({
            "transition": "verified_continuation_complete",
            "phase": "source_frozen",
            "next_action": format!("planr plan final-review {}", request.plan_id),
            "freeze_id": freeze.id,
            "receipt_id": request.receipt_id,
            "coverage_id": coverage_id,
        })))
    }

    pub(crate) fn recover_verification_settlement_value(&self, input: Value) -> Result<Value> {
        if input["schema"].as_str() == Some(VERIFIED_CONTINUATION_RECOVERY_SCHEMA) {
            let input: VerifiedContinuationRecoveryInput = serde_json::from_value(input)
                .context("invalid verified continuation recovery input")?;
            return self.recover_verified_continuation_value(&input);
        }
        if input["schema"].as_str() == Some(HISTORICAL_RECONCILIATION_SCHEMA) {
            let input: HistoricalInvalidationReconciliationInput = serde_json::from_value(input)
                .context("invalid historical invalidation reconciliation input")?;
            return self.reconcile_historical_invalidation_value(&input);
        }
        if input["schema"].as_str() == Some(RISK_REVIEW_OBLIGATION_BACKFILL_SCHEMA) {
            let input: RiskReviewObligationBackfillInput = serde_json::from_value(input)
                .context("invalid risk review obligation backfill input")?;
            return self.backfill_risk_review_obligations_value(&input);
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

    fn recover_verified_continuation_value(
        &self,
        input: &VerifiedContinuationRecoveryInput,
    ) -> Result<Value> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT recover_verified_continuation")?;
        let result = (|| -> Result<Value> {
            if input.schema != VERIFIED_CONTINUATION_RECOVERY_SCHEMA {
                bail!("verified_continuation_recovery_schema_mismatch");
            }
            if let Some(payload) = self.conn.query_row(
                "SELECT payload FROM events WHERE event_type = 'verified_continuation_completed'
                 AND json_extract(payload, '$.run_id') = ?1 ORDER BY id DESC LIMIT 1",
                [&input.run_id],
                |row| row.get::<_, String>(0),
            ).optional()? {
                let payload: Value = serde_json::from_str(&payload)?;
                if payload["plan_id"] != input.plan_id
                    || payload["freeze_id"] != input.freeze_id
                    || payload["receipt_id"] != input.receipt_id
                    || payload["recovery_item_id"] != input.recovery_item_id
                    || payload["final_item_id"] != input.final_item_id
                {
                    bail!("verified_continuation_recovery_request_mismatch");
                }
                return Ok(json!({
                    "schema": "planr.evidence.recover_verified_continuation.result.v1",
                    "created": false,
                    "request": input,
                    "execution_state": self.canonical_execution_state_value(&input.run_id, None)?,
                }));
            }
            let project = self.default_project()?;
            let plan = self.get_plan(&input.plan_id)?;
            let repository = ExecutionRunRepository::new(&self.conn);
            let persisted = repository.feature_run(&input.run_id)?;
            if plan.project_id != project.id
                || persisted.project_id != project.id
                || persisted.run.plan_id != input.plan_id
                || persisted.run.phase != FeatureRunPhase::Implementation
            {
                bail!("verified_continuation_recovery_run_mismatch");
            }
            let batch_id = persisted
                .run
                .active_batch_id
                .clone()
                .ok_or_else(|| anyhow!("verified_continuation_recovery_missing_batch"))?;
            let batch = repository.batch(&batch_id)?;
            let outcome_exists: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM execution_run_outcomes WHERE run_id = ?1 AND item_id = ?2)",
                params![input.run_id, input.final_item_id], |row| row.get(0))?;
            let final_item: (String, String, Option<String>) = self.conn.query_row(
                "SELECT plan_path, status, worker_id FROM items WHERE id = ?1 AND project_id = ?2",
                params![input.final_item_id, project.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            if !outcome_exists || final_item.0 != plan.path || final_item.1 != "closed" {
                bail!("verified_continuation_recovery_final_outcome_mismatch");
            }
            self.complete_verified_continuation(
                &persisted,
                &batch,
                &input.final_item_id,
                Some(input),
            )?
            .ok_or_else(|| anyhow!("verified_continuation_recovery_not_terminal"))?;
            Ok(json!({
                "schema": "planr.evidence.recover_verified_continuation.result.v1",
                "created": true,
                "request": input,
                "execution_state": self.canonical_execution_state_value(&input.run_id, None)?,
            }))
        })();
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE recover_verified_continuation; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO recover_verified_continuation; RELEASE recover_verified_continuation; ROLLBACK",
                );
                Err(error)
            }
        }
    }

    fn backfill_risk_review_obligations_value(
        &self,
        input: &RiskReviewObligationBackfillInput,
    ) -> Result<Value> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT backfill_risk_review_obligations")?;
        let result = self.backfill_risk_review_obligations_locked(input);
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE backfill_risk_review_obligations; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO backfill_risk_review_obligations; RELEASE backfill_risk_review_obligations; ROLLBACK",
                );
                Err(error)
            }
        }
    }

    fn backfill_risk_review_obligations_locked(
        &self,
        input: &RiskReviewObligationBackfillInput,
    ) -> Result<Value> {
        if let Some(payload) = self
            .conn
            .query_row(
                "SELECT payload FROM events
             WHERE event_type = 'risk_review_obligations_backfilled'
               AND json_extract(payload, '$.request.run_id') = ?1
               AND json_extract(payload, '$.request.review_gate_id') = ?2
             ORDER BY id DESC LIMIT 1",
                params![input.run_id, input.review_gate_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let payload: Value = serde_json::from_str(&payload)?;
            let recorded: RiskReviewObligationBackfillInput =
                serde_json::from_value(payload["request"].clone())
                    .context("invalid persisted risk review obligation backfill event")?;
            if recorded != *input {
                bail!("risk_review_obligation_backfill_request_mismatch");
            }
            return Ok(json!({
                "schema": "planr.evidence.backfill_risk_review_obligations.result.v1",
                "created": false,
                "request": input,
                "active_obligation_ids": payload["active_obligation_ids"],
            }));
        }

        let project = self.default_project()?;
        let plan = self.get_plan(&input.plan_id)?;
        if plan.project_id != project.id {
            bail!("risk_review_obligation_backfill_plan_project_mismatch");
        }
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository.feature_run(&input.run_id)?;
        if persisted.project_id != project.id || persisted.run.plan_id != input.plan_id {
            bail!("risk_review_obligation_backfill_run_mismatch");
        }
        let gate = repository.review_gate(&input.review_gate_id)?;
        if gate.run_id != input.run_id
            || gate.kind != ReviewGateKind::RiskCheckpoint
            || gate.status != ReviewGateStatus::Accepted
        {
            bail!("risk_review_obligation_backfill_gate_not_accepted_risk");
        }
        let binding = repository
            .review_source_binding(&gate.id)?
            .ok_or_else(|| anyhow!("risk_review_obligation_backfill_binding_missing"))?;
        let freeze = repository
            .active_source_freeze(&input.run_id)?
            .ok_or_else(|| anyhow!("risk_review_obligation_backfill_active_freeze_missing"))?;
        if input.freeze_id != freeze.id
            || binding.freeze_id != freeze.id
            || binding.source_revision != freeze.source_revision
            || binding.source_digest != freeze.source_digest
            || gate.source_revision.as_deref() != Some(freeze.source_revision.as_str())
        {
            bail!("risk_review_obligation_backfill_source_binding_mismatch");
        }
        let snapshot = capture_repository_snapshot(&self.root)
            .context("checking risk review obligation backfill source")?;
        if snapshot.source.revision != freeze.source_revision
            || snapshot.source.tree_digest.as_str() != freeze.source_digest
        {
            bail!("risk_review_obligation_backfill_source_stale");
        }

        let (attempt_verdict, attempt_source_revision): (String, String) = self.conn.query_row(
            "SELECT verdict, source_revision FROM review_attempts
             WHERE gate_id = ?1 AND attempt_number = ?2",
            params![gate.id, gate.latest_attempt],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if attempt_verdict != "accepted" || attempt_source_revision != freeze.source_revision {
            bail!("risk_review_obligation_backfill_attempt_source_mismatch");
        }

        let (item_plan_path, item_work_type): (String, String) = self.conn.query_row(
            "SELECT plan_path, work_type FROM items WHERE id = ?1 AND project_id = ?2",
            params![input.verification_item_id, project.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if item_plan_path != plan.path || item_work_type != "verification" {
            bail!("risk_review_obligation_backfill_verification_item_mismatch");
        }
        let active_obligation_ids = self
            .conn
            .prepare(
                "SELECT obligations.id FROM proof_obligations obligations
             WHERE obligations.project_id = ?1 AND obligations.plan_id = ?2
               AND obligations.item_id = ?3 AND obligations.binding = 1
               AND NOT EXISTS(
                 SELECT 1 FROM proof_obligations successors
                 WHERE successors.supersedes_obligation_id = obligations.id
               )
             ORDER BY obligations.id",
            )?
            .query_map(
                params![project.id, input.plan_id, input.verification_item_id],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if active_obligation_ids.len() != 1 {
            bail!("risk_review_obligation_backfill_active_obligations_ambiguous");
        }
        let active_obligation_id = &active_obligation_ids[0];
        let legacy_ids = reviewed_risk_obligation_ids(&binding.receipt_lineage)?;
        if legacy_ids.len() != 1 || legacy_ids.contains(active_obligation_id) {
            bail!("risk_review_obligation_backfill_legacy_lineage_ambiguous");
        }
        let legacy_id = legacy_ids.iter().next().expect("one legacy obligation");
        let supersession_proven: bool = self.conn.query_row(
            "WITH RECURSIVE lineage(id, supersedes_obligation_id) AS (
               SELECT id, supersedes_obligation_id FROM proof_obligations WHERE id = ?1
               UNION ALL
               SELECT obligations.id, obligations.supersedes_obligation_id
               FROM proof_obligations obligations
               JOIN lineage ON obligations.id = lineage.supersedes_obligation_id
             )
             SELECT EXISTS(SELECT 1 FROM lineage WHERE id = ?2)",
            params![active_obligation_id, legacy_id],
            |row| row.get(0),
        )?;
        if !supersession_proven {
            bail!("risk_review_obligation_backfill_supersession_mismatch");
        }

        let (receipt_digest, trusted_binding, receipt_created_at): (String, String, String) =
            self.conn.query_row(
                "SELECT receipt_digest, trusted_binding_json, created_at
                 FROM evidence_receipts
                 WHERE id = ?1 AND project_id = ?2 AND obligation_id = ?3
                   AND receipt_status = 'trusted'",
                params![input.receipt_id, project.id, active_obligation_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        let trusted_binding: Value = serde_json::from_str(&trusted_binding)?;
        if trusted_binding["source"]["revision"].as_str() != Some(freeze.source_revision.as_str())
            || trusted_binding["source"]["tree_digest"].as_str()
                != Some(freeze.source_digest.as_str())
        {
            bail!("risk_review_obligation_backfill_receipt_source_mismatch");
        }
        let accepted_at: Option<String> = self.conn.query_row(
            "SELECT accepted_at FROM review_gates WHERE id = ?1",
            [&gate.id],
            |row| row.get(0),
        )?;
        let accepted_at = accepted_at
            .ok_or_else(|| anyhow!("risk_review_obligation_backfill_acceptance_missing"))?;
        let accepted_at = parse_persisted_timestamp(&accepted_at)
            .map_err(|_| anyhow!("risk_review_obligation_backfill_acceptance_invalid"))?;
        let receipt_created_at = parse_persisted_timestamp(&receipt_created_at)
            .map_err(|_| anyhow!("risk_review_obligation_backfill_receipt_timestamp_invalid"))?;
        if accepted_at >= receipt_created_at {
            bail!("risk_review_obligation_backfill_acceptance_not_pre_receipt");
        }
        let evaluated_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
        let coverage =
            evaluate_plan_coverage(&self.conn, &project.id, &input.plan_id, &evaluated_at)
                .map_err(|error| anyhow!("{error}"))?;
        if coverage.status.as_str() != "satisfied"
            || !coverage.waiver_digests.is_empty()
            || !coverage
                .receipt_digests
                .iter()
                .any(|digest| digest == &receipt_digest)
        {
            bail!("risk_review_obligation_backfill_coverage_mismatch");
        }

        let replacement = json!({
            "kind": "risk_review_acceptance",
            "active_obligation_ids": active_obligation_ids,
        });
        let previous = serde_json::to_string(&binding.receipt_lineage)?;
        let updated = self.conn.execute(
            "UPDATE final_review_source_bindings SET receipt_lineage_json = ?1
             WHERE gate_id = ?2 AND receipt_lineage_json = ?3",
            params![serde_json::to_string(&replacement)?, gate.id, previous],
        )?;
        if updated != 1 {
            bail!("risk_review_obligation_backfill_binding_conflict");
        }
        self.record_event(
            "risk_review_obligations_backfilled",
            Some(&input.verification_item_id),
            json!({
                "request": input,
                "legacy_obligation_id": legacy_id,
                "active_obligation_ids": active_obligation_ids,
                "receipt_digest": receipt_digest,
                "coverage_id": coverage.id,
            }),
        )?;
        Ok(json!({
            "schema": "planr.evidence.backfill_risk_review_obligations.result.v1",
            "created": true,
            "request": input,
            "active_obligation_ids": active_obligation_ids,
        }))
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

        let (gate_status, gate_run_id, gate_kind, gate_accepted_at): (
            String,
            String,
            String,
            Option<String>,
        ) = self.conn.query_row(
            "SELECT status, run_id, kind, accepted_at FROM review_gates WHERE id = ?1",
            [&input.review_gate_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
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
        let review_binding_created_at: String = self.conn.query_row(
            "SELECT created_at FROM final_review_source_bindings WHERE gate_id = ?1",
            [&input.review_gate_id],
            |row| row.get(0),
        )?;

        let (receipt_digest, trusted_binding, receipt_obligation_id, receipt_created_at): (
            String,
            String,
            String,
            String,
        ) = self.conn.query_row(
            "SELECT receipts.receipt_digest, receipts.trusted_binding_json,
                    receipts.obligation_id, receipts.created_at
             FROM evidence_receipts receipts
             JOIN proof_obligations obligations ON obligations.id = receipts.obligation_id
             WHERE receipts.id = ?1 AND receipts.project_id = ?2
               AND receipts.receipt_status = 'trusted' AND obligations.plan_id = ?3
               AND obligations.binding = 1",
            params![input.receipt_id, project.id, input.plan_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let trusted_binding: Value = serde_json::from_str(&trusted_binding)?;
        if trusted_binding["source"]["revision"].as_str()
            != Some(active_freeze.source_revision.as_str())
            || trusted_binding["source"]["tree_digest"].as_str()
                != Some(active_freeze.source_digest.as_str())
        {
            bail!("historical_invalidation_reconciliation_receipt_source_mismatch");
        }
        let risk_reviewed_obligation_ids = match gate_kind.as_str() {
            "final_product" => None,
            "risk_checkpoint" => {
                let accepted_at = gate_accepted_at.as_deref().ok_or_else(|| {
                    anyhow!("historical_invalidation_reconciliation_risk_review_acceptance_missing")
                })?;
                let accepted_at = parse_persisted_timestamp(accepted_at).map_err(|_| {
                    anyhow!("historical_invalidation_reconciliation_risk_review_acceptance_invalid")
                })?;
                let review_binding_created_at =
                    parse_persisted_timestamp(&review_binding_created_at).map_err(|_| {
                        anyhow!(
                            "historical_invalidation_reconciliation_risk_review_binding_timestamp_invalid"
                        )
                    })?;
                let receipt_created_at =
                    parse_persisted_timestamp(&receipt_created_at).map_err(|_| {
                        anyhow!(
                            "historical_invalidation_reconciliation_risk_receipt_timestamp_invalid"
                        )
                    })?;
                if accepted_at >= receipt_created_at
                    || review_binding_created_at >= receipt_created_at
                {
                    bail!("historical_invalidation_reconciliation_risk_review_not_pre_evidence");
                }
                let reviewed_obligation_ids =
                    reviewed_risk_obligation_ids(&review_binding.receipt_lineage)?;
                for obligation_id in &reviewed_obligation_ids {
                    let active = self.conn.query_row(
                        "SELECT EXISTS(
                           SELECT 1 FROM proof_obligations obligations
                           WHERE obligations.id = ?1 AND obligations.project_id = ?2
                             AND obligations.plan_id = ?3 AND obligations.binding = 1
                             AND NOT EXISTS(
                               SELECT 1 FROM proof_obligations successors
                               WHERE successors.supersedes_obligation_id = obligations.id
                             )
                         )",
                        params![obligation_id, project.id, input.plan_id],
                        |row| row.get::<_, bool>(0),
                    )?;
                    if !active {
                        bail!(
                            "historical_invalidation_reconciliation_risk_reviewed_obligation_inactive:{obligation_id}"
                        );
                    }
                }
                if !reviewed_obligation_ids.contains(&receipt_obligation_id) {
                    bail!(
                        "historical_invalidation_reconciliation_risk_receipt_obligation_mismatch"
                    );
                }
                Some(reviewed_obligation_ids)
            }
            _ => bail!("historical_invalidation_reconciliation_review_gate_kind_unsupported"),
        };
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
        if risk_reviewed_obligation_ids.is_none()
            && review_binding.receipt_lineage != coverage.receipt_lineage
        {
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

fn reviewed_risk_obligation_ids(receipt_lineage: &Value) -> Result<BTreeSet<String>> {
    fn collect(receipt_lineage: &Value, depth: usize) -> Result<BTreeSet<String>> {
        if depth > 8 {
            bail!("historical_invalidation_reconciliation_risk_review_lineage_too_deep");
        }
        let kind = receipt_lineage["kind"].as_str().ok_or_else(|| {
            anyhow!("historical_invalidation_reconciliation_risk_review_lineage_ambiguous")
        })?;
        match kind {
            "product_repair" => {
                let values = receipt_lineage["selective_obligation_ids"]
                    .as_array()
                    .ok_or_else(|| {
                        anyhow!(
                            "historical_invalidation_reconciliation_risk_review_obligations_missing"
                        )
                    })?;
                let mut ids = BTreeSet::new();
                for value in values {
                    let id = value.as_str().filter(|id| !id.trim().is_empty()).ok_or_else(|| {
                        anyhow!(
                            "historical_invalidation_reconciliation_risk_review_obligations_invalid"
                        )
                    })?;
                    if !ids.insert(id.to_string()) {
                        bail!(
                            "historical_invalidation_reconciliation_risk_review_obligations_duplicate"
                        );
                    }
                }
                if ids.is_empty() {
                    bail!("historical_invalidation_reconciliation_risk_review_obligations_empty");
                }
                Ok(ids)
            }
            "risk_review_acceptance" => {
                let values = receipt_lineage["active_obligation_ids"]
                    .as_array()
                    .ok_or_else(|| {
                        anyhow!(
                            "historical_invalidation_reconciliation_risk_review_obligations_missing"
                        )
                    })?;
                let ids = values
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .filter(|id| !id.trim().is_empty())
                            .map(str::to_string)
                            .ok_or_else(|| {
                                anyhow!(
                                    "historical_invalidation_reconciliation_risk_review_obligations_invalid"
                                )
                            })
                    })
                    .collect::<Result<BTreeSet<_>>>()?;
                if ids.len() != values.len() {
                    bail!(
                        "historical_invalidation_reconciliation_risk_review_obligations_duplicate"
                    );
                }
                if ids.is_empty() {
                    bail!("historical_invalidation_reconciliation_risk_review_obligations_empty");
                }
                Ok(ids)
            }
            "risk_review_finding_repair" => collect(&receipt_lineage["supersedes"], depth + 1),
            _ => bail!("historical_invalidation_reconciliation_risk_review_lineage_ambiguous"),
        }
    }

    collect(receipt_lineage, 0)
}
