use crate::execution_run::{
    CurrentVerificationAdmissionIdentity, CurrentVerificationInvariantFacts,
    CurrentVerificationItemLease, CurrentVerificationItemLeaseStatus, EvidenceInvalidationKind,
    ExecutionBatch, ExecutionBatchStatus, FeatureRun, FeatureRunBudgetContractCompatibility,
    FeatureRunHoldReason, FeatureRunPhase, FeatureRunRestartDisposition, FeatureRunRestartTransition,
    FeatureRunStatus, FeatureRunTerminalReason, MakerReplacement,
    HistoryIdentitySet, InconsistentVerificationBatchFacts,
    InconsistentVerificationRetirementTransition, MakerReplacementReason,
    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES,
    PrematureSourceFreezeRestartTransition, RoleOwner, RunRole, owner_for_role,
    VerificationAdmissionRepairRequest, VerificationAdmissionRepairTransition,
    classify_current_verification, repair_verification_admission,
    resolve_evidence_invalidation_kind,
    retire_inconsistent_verification_feature_run, retire_premature_source_freeze_feature_run,
    validate_feature_run,
};
use crate::canonical_json::sha256_json_digest;
use crate::usage_policy::{
    BudgetPhase, ExecutionBudget, FEATURE_RUN_BUDGET_CONTRACT_SCHEMA, FeatureRunBudgetContract,
    FeatureRunBudgetMode, MeteringMode, feature_run_budget_contract_digest,
    validate_execution_budget, validate_feature_run_budget_contract,
};
use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Params, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) struct ExecutionRunRepository<'conn> {
    conn: &'conn Connection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PersistedFeatureRun {
    pub(crate) project_id: String,
    pub(crate) run: FeatureRun,
    pub(crate) revision: u64,
    pub(crate) budget_projection_at_unix_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PersistedExecutionBatch {
    pub(crate) batch: ExecutionBatch,
    pub(crate) revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RunOutcomeRecord {
    pub(crate) id: String,
    pub(crate) run_id: String,
    pub(crate) batch_id: String,
    pub(crate) item_id: String,
    pub(crate) ordinal: u32,
    pub(crate) outcome: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewScopeKind {
    Outcome,
    Plan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewGateKind {
    RiskCheckpoint,
    FinalProduct,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewGateStatus {
    Pending,
    Leased,
    Accepted,
    ChangesRequested,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReviewGateRecord {
    pub(crate) id: String,
    pub(crate) run_id: String,
    pub(crate) scope_kind: ReviewScopeKind,
    pub(crate) scope_id: String,
    pub(crate) kind: ReviewGateKind,
    pub(crate) status: ReviewGateStatus,
    pub(crate) required_risk: Option<String>,
    pub(crate) responsible_maker_id: String,
    pub(crate) latest_attempt: u32,
    pub(crate) source_revision: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewVerdict {
    Accepted,
    ChangesRequested,
    Blocked,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReviewAttemptRecord {
    pub(crate) id: String,
    pub(crate) gate_id: String,
    pub(crate) attempt_number: u32,
    pub(crate) reviewer_worker_id: String,
    pub(crate) reviewer_mode: String,
    pub(crate) verdict: ReviewVerdict,
    pub(crate) source_revision: String,
    pub(crate) artifacts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ReviewSourceBindingRecord {
    pub(crate) gate_id: String,
    pub(crate) freeze_id: String,
    pub(crate) source_revision: String,
    pub(crate) source_digest: String,
    pub(crate) receipt_lineage: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FindingStatus {
    Open,
    Resolved,
    Dismissed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct FindingRecord {
    pub(crate) id: String,
    pub(crate) gate_id: String,
    pub(crate) attempt_id: String,
    pub(crate) severity: String,
    pub(crate) target: String,
    pub(crate) owner_worker_id: String,
    pub(crate) status: FindingStatus,
    pub(crate) invalidated_evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct BudgetObservationRecord {
    pub(crate) id: String,
    pub(crate) run_id: String,
    pub(crate) reservation_id: Option<String>,
    pub(crate) sequence: Option<u64>,
    pub(crate) phase: BudgetPhase,
    pub(crate) metering: MeteringMode,
    pub(crate) wall_metering: Option<MeteringMode>,
    pub(crate) tool_calls_metering: Option<MeteringMode>,
    pub(crate) tokens_metering: Option<MeteringMode>,
    pub(crate) wall_seconds: Option<u64>,
    pub(crate) tokens: Option<u64>,
    pub(crate) tool_calls: Option<u64>,
    pub(crate) credits_micros: Option<u64>,
    pub(crate) provenance: String,
    pub(crate) adapter_id: Option<String>,
    pub(crate) observed_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct BudgetReservationRecord {
    pub(crate) id: String,
    pub(crate) run_id: String,
    pub(crate) phase: BudgetPhase,
    pub(crate) boundary_key: String,
    pub(crate) owner_role: RunRole,
    pub(crate) owner_worker_id: String,
    pub(crate) lease_generation: u64,
    pub(crate) execution_budget: Option<ExecutionBudget>,
    pub(crate) started_at_unix_ms: u64,
    pub(crate) provenance: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BudgetReservationStatus {
    Active,
    Reconciled,
    Released,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PersistedBudgetReservation {
    pub(crate) reservation: BudgetReservationRecord,
    pub(crate) status: BudgetReservationStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SourceFreezeRecord {
    pub(crate) id: String,
    pub(crate) run_id: String,
    pub(crate) source_revision: String,
    pub(crate) source_digest: String,
    pub(crate) status: SourceFreezeStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SourceFreezeStatus {
    Active,
    Invalidated,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct EvidenceInvalidationRecord {
    pub(crate) id: String,
    pub(crate) run_id: String,
    pub(crate) freeze_id: String,
    pub(crate) finding_id: Option<String>,
    pub(crate) reason: String,
    pub(crate) affected_evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProductRepairSettlementRecord {
    pub(crate) invalidation_id: String,
    pub(crate) run_id: String,
    pub(crate) responsible_maker_id: String,
    pub(crate) selective_obligation_ids: Vec<String>,
    pub(crate) settlement: serde_json::Value,
    pub(crate) source_freeze_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct VerificationAdmissionRepairSettlementRecord {
    pub(crate) invalidation_id: String,
    pub(crate) run_id: String,
    pub(crate) repair_batch_id: String,
    pub(crate) responsible_maker_id: String,
    pub(crate) ended_revision: u64,
    pub(crate) settlement: serde_json::Value,
    pub(crate) source_freeze_id: String,
    pub(crate) source_revision: String,
    pub(crate) source_digest: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct VerificationAdmissionRecord {
    pub(crate) plan_id: String,
    pub(crate) run_id: String,
    pub(crate) freeze_id: String,
    pub(crate) run_revision: u64,
    pub(crate) verifier_worker_id: String,
    pub(crate) verifier_lease_generation: u64,
    pub(crate) verification_item_id: Option<String>,
    pub(crate) run_index_digest: String,
    pub(crate) sealed_run_index: Value,
}

impl VerificationAdmissionRecord {
    pub(crate) fn current_identity(&self) -> CurrentVerificationAdmissionIdentity {
        CurrentVerificationAdmissionIdentity {
            plan_id: self.plan_id.clone(),
            run_id: self.run_id.clone(),
            freeze_id: self.freeze_id.clone(),
            run_revision: self.run_revision,
            verifier_worker_id: self.verifier_worker_id.clone(),
            verifier_lease_generation: self.verifier_lease_generation,
            verification_item_id: self.verification_item_id.clone(),
            run_index_digest: self.run_index_digest.clone(),
            sealed_run_index_digest: self.sealed_run_index["run_index_digest"]
                .as_str()
                .map(str::to_string),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct VerificationReadinessDiagnosticRecord {
    pub(crate) repair_request: VerificationAdmissionRepairRequest,
    pub(crate) verifier_worker_id: String,
    pub(crate) diagnostic: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CurrentVerificationSnapshot {
    pub(crate) facts: CurrentVerificationInvariantFacts,
    pub(crate) admission: Option<VerificationAdmissionRecord>,
}

impl<'conn> ExecutionRunRepository<'conn> {
    pub(crate) const fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    pub(crate) fn active_feature_run_for_plan(
        &self,
        project_id: &str,
        plan_id: &str,
    ) -> Result<Option<PersistedFeatureRun>> {
        let run_id = self
            .conn
            .query_row(
                "SELECT id FROM feature_runs WHERE project_id = ?1 AND plan_id = ?2 AND status IN ('active','held') ORDER BY created_at DESC, id DESC LIMIT 1",
                params![project_id, plan_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        run_id.map(|id| self.feature_run(&id)).transpose()
    }

    pub(crate) fn verification_item_projection(
        &self,
        plan_id: &str,
    ) -> Result<Option<CurrentVerificationItemLease>> {
        let mut statement = self.conn.prepare(
            "SELECT items.id, items.status, items.worker_id
             FROM items JOIN plans ON plans.path = items.plan_path
             WHERE plans.id = ?1 AND items.work_type = 'verification'
               AND items.status IN ('ready','picked','running')
             ORDER BY items.created_at, items.id LIMIT 2",
        )?;
        let rows = statement
            .query_map([plan_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.len() > 1 {
            bail!("verification_admission_repair_ambiguous_items:{plan_id}");
        }
        rows.into_iter()
            .next()
            .map(|(id, status, worker_id)| {
                let status = match status.as_str() {
                    "ready" => CurrentVerificationItemLeaseStatus::Ready,
                    "picked" => CurrentVerificationItemLeaseStatus::Picked,
                    "running" => CurrentVerificationItemLeaseStatus::Running,
                    value => bail!("invalid current verification item status: {value}"),
                };
                Ok(CurrentVerificationItemLease {
                    id,
                    status,
                    worker_id,
                })
            })
            .transpose()
    }

    pub(crate) fn historical_maker_identity(&self, run_id: &str) -> Result<(String, u64)> {
        let (worker_id, generation): (String, u64) = self.conn.query_row(
            "SELECT worker_id, lease_generation FROM feature_run_role_leases
             WHERE run_id = ?1 AND role = 'maker'
             ORDER BY lease_generation DESC LIMIT 1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let next_generation = generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("verification_admission_repair_maker_generation_overflow:{run_id}"))?;
        Ok((worker_id, next_generation))
    }

    pub(crate) fn record_verification_admission(
        &self,
        record: &VerificationAdmissionRecord,
    ) -> Result<()> {
        if self.conn.is_autocommit() {
            bail!("verification_admission_requires_active_transaction");
        }
        let current = self.feature_run(&record.run_id)?;
        let verifier = owner_for_role(&current.run, RunRole::Verifier)
            .ok_or_else(|| anyhow!("verification_admission_missing_verifier:{}", record.run_id))?;
        let freeze = self
            .active_source_freeze(&record.run_id)?
            .ok_or_else(|| anyhow!("verification_admission_missing_freeze:{}", record.run_id))?;
        let item = self.verification_item_projection(&record.plan_id)?;
        if current.run.plan_id != record.plan_id
            || current.run.phase != FeatureRunPhase::Verification
            || current.revision != record.run_revision
            || freeze.id != record.freeze_id
            || verifier.worker_id != record.verifier_worker_id
            || verifier.lease_generation != record.verifier_lease_generation
            || item.as_ref().map(|item| item.id.as_str()) != record.verification_item_id.as_deref()
            || record.run_index_digest.trim().is_empty()
            || record.sealed_run_index["run_index_digest"].as_str()
                != Some(record.run_index_digest.as_str())
        {
            bail!("verification_admission_record_stale:{}", record.run_id);
        }
        self.conn.execute(
            "INSERT INTO events(project_id, item_id, worker_id, event_type, payload, timestamp)
             VALUES (?1, ?2, ?3, 'feature_run_verification_admitted', ?4, datetime('now'))",
            params![
                current.project_id,
                record.verification_item_id,
                record.verifier_worker_id,
                serde_json::to_string(record)?,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn latest_verification_admission(
        &self,
        run_id: &str,
        freeze_id: &str,
    ) -> Result<Option<VerificationAdmissionRecord>> {
        let payload = self
            .conn
            .query_row(
                "SELECT payload FROM events
                 WHERE event_type = 'feature_run_verification_admitted'
                   AND json_extract(payload, '$.run_id') = ?1
                   AND json_extract(payload, '$.freeze_id') = ?2
                 ORDER BY id DESC LIMIT 1",
                params![run_id, freeze_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn latest_verification_admission_for_run(
        &self,
        run_id: &str,
    ) -> Result<Option<VerificationAdmissionRecord>> {
        let payload = self
            .conn
            .query_row(
                "SELECT payload FROM events
                 WHERE event_type = 'feature_run_verification_admitted'
                   AND json_extract(payload, '$.run_id') = ?1
                 ORDER BY id DESC LIMIT 1",
                [run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn current_verification_snapshot(
        &self,
        persisted: &PersistedFeatureRun,
    ) -> Result<CurrentVerificationSnapshot> {
        if persisted.run.status != FeatureRunStatus::Active
            || persisted.run.phase != FeatureRunPhase::Verification
        {
            bail!("current_verification_requires_active_verification:{}", persisted.run.id);
        }
        let run_source_revision = persisted
            .run
            .source_revision
            .clone()
            .ok_or_else(|| anyhow!("current_verification_missing_run_source:{}", persisted.run.id))?;
        let freeze = self
            .active_source_freeze(&persisted.run.id)?
            .ok_or_else(|| anyhow!("current_verification_missing_freeze:{}", persisted.run.id))?;
        let verifier = owner_for_role(&persisted.run, RunRole::Verifier)
            .ok_or_else(|| anyhow!("current_verification_missing_verifier:{}", persisted.run.id))?;
        let verification_item = self.verification_item_projection(&persisted.run.plan_id)?;
        let admission = self.latest_verification_admission_for_run(&persisted.run.id)?;
        Ok(CurrentVerificationSnapshot {
            facts: CurrentVerificationInvariantFacts {
                plan_id: persisted.run.plan_id.clone(),
                run_id: persisted.run.id.clone(),
                run_revision: persisted.revision,
                run_source_revision,
                freeze_id: freeze.id,
                freeze_source_revision: freeze.source_revision,
                freeze_source_digest: freeze.source_digest,
                verifier_worker_id: verifier.worker_id.clone(),
                verifier_lease_generation: verifier.lease_generation,
                verification_item,
                admission: admission.as_ref().map(VerificationAdmissionRecord::current_identity),
            },
            admission,
        })
    }

    pub(crate) fn persist_verification_readiness_hold(
        &self,
        held_run: &FeatureRun,
        expected_run_revision: u64,
        freeze_id: &str,
        diagnostic: &VerificationReadinessDiagnosticRecord,
    ) -> Result<PersistedFeatureRun> {
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        let repository = ExecutionRunRepository::new(&tx);
        let current = repository.feature_run(&held_run.id)?;
        let freeze = repository
            .active_source_freeze(&held_run.id)?
            .ok_or_else(|| anyhow!("verification_readiness_hold_missing_freeze:{}", held_run.id))?;
        if current.revision != expected_run_revision
            || current.run.phase != FeatureRunPhase::SourceFrozen
            || freeze.id != freeze_id
            || diagnostic.repair_request.run_revision
                != expected_run_revision
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("verification_readiness_hold_revision_overflow:{}", held_run.id))?
            || diagnostic.repair_request.plan_id != current.run.plan_id
            || diagnostic.repair_request.run_id != current.run.id
            || diagnostic.repair_request.freeze_id != freeze.id
            || owner_for_role(&current.run, RunRole::Verifier).is_some()
        {
            bail!("verification_readiness_hold_stale:{}", held_run.id);
        }
        repository.save_feature_run(held_run, expected_run_revision)?;
        tx.execute(
            "INSERT INTO events(project_id, item_id, worker_id, event_type, payload, timestamp)
             VALUES (?1, NULL, ?2, 'feature_run_verification_readiness_held', ?3, datetime('now'))",
            params![
                current.project_id,
                diagnostic.verifier_worker_id,
                serde_json::to_string(diagnostic)?,
            ],
        )?;
        tx.commit()?;
        self.feature_run(&held_run.id)
    }

    pub(crate) fn latest_verification_readiness_diagnostic(
        &self,
        run_id: &str,
        freeze_id: &str,
    ) -> Result<Option<VerificationReadinessDiagnosticRecord>> {
        let payload = self
            .conn
            .query_row(
                "SELECT payload FROM events
                 WHERE event_type = 'feature_run_verification_readiness_held'
                   AND json_extract(payload, '$.repair_request.run_id') = ?1
                   AND json_extract(payload, '$.repair_request.freeze_id') = ?2
                 ORDER BY id DESC LIMIT 1",
                params![run_id, freeze_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn latest_verification_admission_repair(
        &self,
        request: &VerificationAdmissionRepairRequest,
    ) -> Result<Option<VerificationAdmissionRepairTransition>> {
        let mut statement = self.conn.prepare(
            "SELECT payload FROM events
             WHERE event_type = 'feature_run_verification_admission_repaired'
               AND json_extract(payload, '$.request.run_id') = ?1
             ORDER BY id DESC",
        )?;
        let payloads = statement
            .query_map([&request.run_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for payload in payloads {
            let transition: VerificationAdmissionRepairTransition =
                serde_json::from_str(&payload)?;
            if transition.request == *request {
                return Ok(Some(transition));
            }
        }
        Ok(None)
    }

    pub(crate) fn prove_verification_admission_repair_idempotent(
        &self,
        transition: &VerificationAdmissionRepairTransition,
    ) -> Result<()> {
        let current = self.feature_run(&transition.request.run_id)?;
        let freeze = self.source_freeze(&transition.request.freeze_id)?;
        let batch = self.batch(&transition.repair_batch.id)?;
        let item = self.verification_item_projection(&transition.request.plan_id)?;
        let mut event_statement = self.conn.prepare(
            "SELECT payload FROM events
             WHERE event_type = 'feature_run_verification_admission_repaired'
               AND json_extract(payload, '$.request.run_id') = ?1",
        )?;
        let event_payloads = event_statement
            .query_map([&transition.request.run_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut matching_event_count = 0;
        for payload in event_payloads {
            let candidate: VerificationAdmissionRepairTransition =
                serde_json::from_str(&payload)?;
            if candidate.request == transition.request {
                matching_event_count += 1;
            }
        }
        let invalidation_count: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM feature_run_evidence_invalidations
             WHERE id = ?1 AND run_id = ?2 AND freeze_id = ?3
               AND finding_id IS NULL AND reason = ?4
               AND affected_evidence_ids_json = '[]'",
            params![
                transition.facts.invalidation_id,
                transition.request.run_id,
                transition.request.freeze_id,
                format!(
                    "verification_admission_repair:{}",
                    transition.request.reason.as_str()
                ),
            ],
            |row| row.get(0),
        )?;
        if current.revision != transition.resulting_run_revision
            || current.run != transition.repaired_run
            || freeze.status != SourceFreezeStatus::Invalidated
            || batch.batch != transition.repair_batch
            || item.as_ref().map(|item| item.id.as_str())
                != transition.facts.verification_item_id.as_deref()
            || item.as_ref().is_some_and(|item| {
                item.status != CurrentVerificationItemLeaseStatus::Ready || item.worker_id.is_some()
            })
            || matching_event_count != 1
            || invalidation_count != 1
        {
            bail!("verification_admission_repair_idempotence_unproven:{}", transition.request.run_id);
        }
        Ok(())
    }

    pub(crate) fn persist_verification_admission_repair(
        &self,
        transition: &VerificationAdmissionRepairTransition,
        operator_worker_id: &str,
    ) -> Result<()> {
        if self.conn.is_autocommit() {
            bail!("verification_admission_repair_requires_active_transaction");
        }
        require_nonempty("operator_worker_id", operator_worker_id)?;
        let current = self.feature_run(&transition.request.run_id)?;
        let freeze = self
            .active_source_freeze(&current.run.id)?
            .ok_or_else(|| anyhow!("verification_admission_repair_missing_freeze:{}", current.run.id))?;
        if current.revision != transition.request.run_revision
            || current.run.plan_id != transition.request.plan_id
            || freeze.id != transition.request.freeze_id
            || operator_worker_id != transition.facts.requester_worker_id
        {
            bail!("verification_admission_repair_stale_identity:{}", current.run.id);
        }
        let (_, attempt_count, receipt_count) = self.source_freeze_verification_activity(
            &current.project_id,
            &current.run.plan_id,
            &current.run.id,
            &freeze,
        )?;
        let one_shot_count: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM feature_run_one_shot_claims WHERE freeze_id = ?1",
            [&freeze.id],
            |row| row.get(0),
        )?;
        let product_finding_count: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM feature_run_evidence_invalidations
             WHERE run_id = ?1 AND freeze_id = ?2 AND reason = 'product_finding'",
            params![current.run.id, freeze.id],
            |row| row.get(0),
        )?;
        if attempt_count != 0 || receipt_count != 0 || one_shot_count != 0 || product_finding_count != 0 {
            bail!("verification_admission_repair_not_pre_receipt:{}", current.run.id);
        }
        let (maker_worker_id, maker_generation) = self.historical_maker_identity(&current.run.id)?;
        let item = self.verification_item_projection(&current.run.plan_id)?;
        if maker_worker_id != transition.facts.historical_maker_worker_id
            || maker_generation != transition.facts.next_maker_lease_generation
            || item.as_ref().map(|item| item.id.as_str())
                != transition.facts.verification_item_id.as_deref()
        {
            bail!("verification_admission_repair_facts_changed:{}", current.run.id);
        }
        if transition.request.reason.requires_run_index_digest() {
            let verifier = owner_for_role(&current.run, RunRole::Verifier)
                .ok_or_else(|| anyhow!("verification_admission_repair_missing_verifier:{}", current.run.id))?;
            if verifier.worker_id != operator_worker_id
                || item.as_ref().is_some_and(|item| {
                    !matches!(item.status, CurrentVerificationItemLeaseStatus::Picked | CurrentVerificationItemLeaseStatus::Running)
                        || item.worker_id.as_deref() != Some(operator_worker_id)
                })
            {
                bail!("verification_admission_repair_wrong_owner:{}", current.run.id);
            }
            let admission = self
                .latest_verification_admission(&current.run.id, &freeze.id)?
                .ok_or_else(|| anyhow!("verification_admission_repair_seal_missing:{}", current.run.id))?;
            if admission.run_index_digest != transition.request.run_index_digest.as_deref().unwrap_or_default()
                || admission.run_revision != current.revision
                || admission.verifier_worker_id != operator_worker_id
                || admission.verification_item_id != transition.facts.verification_item_id
            {
                bail!("verification_admission_repair_seal_mismatch:{}", current.run.id);
            }
        } else {
            let diagnostic = self
                .latest_verification_readiness_diagnostic(&current.run.id, &freeze.id)?
                .ok_or_else(|| anyhow!("verification_admission_repair_diagnostic_missing:{}", current.run.id))?;
            if diagnostic.repair_request != transition.request
                || diagnostic.verifier_worker_id != operator_worker_id
                || item.as_ref().is_some_and(|item| {
                    item.status != CurrentVerificationItemLeaseStatus::Ready || item.worker_id.is_some()
                })
            {
                bail!("verification_admission_repair_diagnostic_mismatch:{}", current.run.id);
            }
        }
        let recomputed = repair_verification_admission(
            &current.run,
            current.revision,
            &transition.request,
            &transition.facts,
        )
        .map_err(|violation| anyhow!("verification_admission_repair_rejected:{violation:?}"))?;
        if recomputed != *transition {
            bail!("verification_admission_repair_transition_stale:{}", current.run.id);
        }
        invalidate_source_in(
            self.conn,
            &EvidenceInvalidationRecord {
                id: transition.facts.invalidation_id.clone(),
                run_id: current.run.id.clone(),
                freeze_id: freeze.id,
                finding_id: None,
                reason: format!(
                    "verification_admission_repair:{}",
                    transition.request.reason.as_str()
                ),
                affected_evidence_ids: Vec::new(),
            },
        )?;
        self.save_feature_run_with_new_batch(
            &transition.repaired_run,
            current.revision,
            &transition.repair_batch,
        )?;
        if let Some(item) = item {
            if matches!(
                item.status,
                CurrentVerificationItemLeaseStatus::Picked
                    | CurrentVerificationItemLeaseStatus::Running
            ) {
                let changed = self.conn.execute(
                    "UPDATE items SET status = 'ready', worker_id = NULL, pick_token = NULL,
                         picked_at = NULL, last_heartbeat_at = NULL, paused_at = NULL,
                         updated_at = datetime('now')
                     WHERE id = ?1 AND status IN ('picked','running') AND worker_id = ?2",
                    params![item.id, operator_worker_id],
                )?;
                if changed != 1 {
                    bail!("verification_admission_repair_item_release_failed:{}", item.id);
                }
            }
        }
        self.conn.execute(
            "INSERT INTO events(project_id, item_id, worker_id, event_type, payload, timestamp)
             VALUES (?1, ?2, ?3, 'feature_run_verification_admission_repaired', ?4, datetime('now'))",
            params![
                current.project_id,
                transition.facts.verification_item_id,
                operator_worker_id,
                serde_json::to_string(transition)?,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn open_ordinary_outcome_ids(&self, plan_id: &str) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(
            "SELECT items.id FROM items JOIN plans ON plans.path = items.plan_path
             WHERE plans.id = ?1 AND items.work_type IN (?2, ?3, ?4, ?5)
               AND items.status NOT IN ('closed','closed_partial','cancelled')
             ORDER BY items.created_at, items.id",
        )?;
        statement
            .query_map(
                params![
                    plan_id,
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[0],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[1],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[2],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[3],
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub(crate) fn active_ordinary_outcome_lease_ids(
        &self,
        plan_id: &str,
    ) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(
            "SELECT items.id FROM items JOIN plans ON plans.path = items.plan_path
             WHERE plans.id = ?1 AND items.work_type IN (?2, ?3, ?4, ?5)
               AND items.status IN ('picked','running')
             ORDER BY items.created_at, items.id",
        )?;
        statement
            .query_map(
                params![
                    plan_id,
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[0],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[1],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[2],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[3],
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub(crate) fn source_freeze_verification_activity(
        &self,
        project_id: &str,
        plan_id: &str,
        run_id: &str,
        freeze: &SourceFreezeRecord,
    ) -> Result<(u64, u64, u64)> {
        let role_admissions = self.conn.query_row(
            "SELECT COUNT(*) FROM feature_run_role_leases
             WHERE run_id = ?1 AND role = 'verifier'",
            [run_id],
            |row| row.get::<_, u64>(0),
        )?;
        let one_shot_admissions = self.conn.query_row(
            "SELECT COUNT(*) FROM feature_run_one_shot_claims WHERE freeze_id = ?1",
            [&freeze.id],
            |row| row.get::<_, u64>(0),
        )?;
        let attempts = self.conn.query_row(
            "SELECT COUNT(*) FROM evidence_attempts AS attempts
             JOIN proof_obligations AS obligations ON obligations.id = attempts.obligation_id
             WHERE attempts.project_id = ?1 AND obligations.plan_id = ?2
               AND attempts.created_at >= (
                 SELECT created_at FROM feature_run_source_freezes WHERE id = ?3
               )",
            params![project_id, plan_id, freeze.id],
            |row| row.get::<_, u64>(0),
        )?;
        let receipts = self.conn.query_row(
            "SELECT COUNT(*) FROM evidence_receipts AS receipts
             JOIN proof_obligations AS obligations ON obligations.id = receipts.obligation_id
             WHERE receipts.project_id = ?1 AND obligations.plan_id = ?2
               AND json_extract(receipts.trusted_binding_json, '$.source.revision') = ?3
               AND json_extract(receipts.trusted_binding_json, '$.source.tree_digest') = ?4",
            params![
                project_id,
                plan_id,
                freeze.source_revision,
                freeze.source_digest,
            ],
            |row| row.get::<_, u64>(0),
        )?;
        Ok((
            role_admissions + one_shot_admissions,
            attempts,
            receipts,
        ))
    }

    pub(crate) fn create_feature_run(
        &self,
        project_id: &str,
        run: &FeatureRun,
        budget_contract: &FeatureRunBudgetContract,
        initial_batch: Option<&ExecutionBatch>,
    ) -> Result<PersistedFeatureRun> {
        validate_feature_run(run)
            .map_err(|violation| anyhow!("invalid feature run: {violation:?}"))?;
        require_nonempty("project_id", project_id)?;
        validate_budget_contract_for_run(run, budget_contract)?;
        match (run.active_batch_id.as_deref(), initial_batch) {
            (Some(active_batch_id), Some(batch))
                if batch.id == active_batch_id && batch.run_id == run.id => {}
            (None, None) => {}
            _ => bail!("feature_run_initial_batch_mismatch:{}", run.id),
        }
        if let Some(batch) = initial_batch {
            validate_batch_for_persistence(batch)?;
            validate_batch_maker_owner(run, batch)?;
        }
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, budget_contract_digest, source_revision, active_batch_id, outcomes_settled, batch_outcome_count, held_from_phase, hold_reason, terminal_reason, revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0)",
            params![
                run.id,
                project_id,
                run.plan_id,
                feature_run_status_str(run.status),
                feature_run_phase_str(run.phase),
                run.policy_digest,
                budget_contract.digest,
                run.source_revision,
                run.active_batch_id,
                run.outcomes_settled,
                run.batch_outcome_count,
                run.held_from_phase.map(feature_run_phase_str),
                run.hold_reason.map(hold_reason_str),
                run.terminal_reason.map(terminal_reason_str),
            ],
        )?;
        insert_budget_contract(&tx, budget_contract)?;
        insert_initial_role_leases(&tx, &run.id, &run.role_owners)?;
        if let Some(batch) = initial_batch {
            insert_batch(&tx, batch)?;
        }
        tx.commit()?;
        self.feature_run(&run.id)
    }

    pub(crate) fn budget_contract(&self, run_id: &str) -> Result<FeatureRunBudgetContract> {
        let (bound_digest, schema, digest, contract_json): (String, String, String, String) = self
            .conn
            .query_row(
                "SELECT feature_runs.budget_contract_digest, contract.schema, contract.digest, contract.contract_json FROM feature_runs JOIN feature_run_budget_contracts AS contract ON contract.run_id = feature_runs.id WHERE feature_runs.id = ?1",
                [run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .with_context(|| format!("FeatureRun budget contract not found: {run_id}"))?;
        let contract: FeatureRunBudgetContract = serde_json::from_str(&contract_json)
            .with_context(|| format!("invalid persisted FeatureRun budget contract: {run_id}"))?;
        if bound_digest != digest
            || contract.schema != schema
            || contract.digest != digest
            || contract.run_id != run_id
        {
            bail!("feature_run_budget_contract_integrity_mismatch:{run_id}");
        }
        let diagnostics = validate_feature_run_budget_contract(&contract);
        if !diagnostics.is_empty() {
            bail!("feature_run_budget_contract_invalid:{run_id}:{diagnostics:?}");
        }
        Ok(contract)
    }

    pub(crate) fn budget_contract_compatibility(
        &self,
        run_id: &str,
    ) -> Result<FeatureRunBudgetContractCompatibility> {
        budget_contract_compatibility_in(self.conn, run_id)
    }

    pub(crate) fn retire_incompatible_feature_run(
        &self,
        transition: &FeatureRunRestartTransition,
        expected_run_revision: u64,
        operator_worker_id: &str,
    ) -> Result<PersistedFeatureRun> {
        if transition.disposition != FeatureRunRestartDisposition::Retired {
            bail!("feature_run_restart_transition_not_applicable");
        }
        require_nonempty("operator_worker_id", operator_worker_id)?;
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        let repository = ExecutionRunRepository::new(&tx);
        let current = repository.feature_run(&transition.retired_run.id)?;
        if current.revision != expected_run_revision {
            bail!("feature_run_revision_conflict:{}", current.run.id);
        }
        let current_compatibility = budget_contract_compatibility_in(&tx, &current.run.id)?;
        if current_compatibility != transition.incompatibility {
            bail!(
                "feature_run_budget_compatibility_changed:{}",
                current.run.id
            );
        }
        let recomputed = crate::execution_run::retire_incompatible_feature_run(
            &current.run,
            &transition.request,
            current_compatibility,
        )
        .map_err(|violation| anyhow!("feature_run_restart_rejected:{violation:?}"))?;
        if recomputed != *transition {
            bail!("feature_run_restart_transition_stale:{}", current.run.id);
        }

        if let Some(batch_id) = transition.ended_batch_id.as_deref() {
            let persisted_batch = repository.batch(batch_id)?;
            if persisted_batch.batch.run_id != current.run.id
                || persisted_batch.batch.status == ExecutionBatchStatus::Ended
            {
                bail!("feature_run_restart_batch_not_active:{batch_id}");
            }
            let mut ended_batch = persisted_batch.batch;
            ended_batch.status = ExecutionBatchStatus::Ended;
            ended_batch.replacement = None;
            repository.save_batch(&ended_batch, persisted_batch.revision)?;
        }
        repository.save_feature_run(&transition.retired_run, expected_run_revision)?;
        let payload = serde_json::to_string(transition)?;
        tx.execute(
            "INSERT INTO events(project_id, item_id, worker_id, event_type, payload, timestamp) VALUES (?1, NULL, ?2, 'feature_run_incompatible_budget_retired', ?3, datetime('now'))",
            params![current.project_id, operator_worker_id, payload],
        )?;
        tx.commit()?;
        self.feature_run(&transition.retired_run.id)
    }

    pub(crate) fn latest_incompatible_feature_run_restart(
        &self,
        project_id: &str,
        plan_id: &str,
    ) -> Result<Option<FeatureRunRestartTransition>> {
        let payload = self
            .conn
            .query_row(
                "SELECT payload FROM events WHERE project_id = ?1 AND event_type = 'feature_run_incompatible_budget_retired' AND json_extract(payload, '$.request.plan_id') = ?2 ORDER BY id DESC LIMIT 1",
                params![project_id, plan_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn retire_premature_source_freeze_feature_run(
        &self,
        transition: &PrematureSourceFreezeRestartTransition,
        expected_run_revision: u64,
        operator_worker_id: &str,
    ) -> Result<PersistedFeatureRun> {
        if transition.disposition != FeatureRunRestartDisposition::Retired {
            bail!("feature_run_restart_transition_not_applicable");
        }
        require_nonempty("operator_worker_id", operator_worker_id)?;
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        let repository = ExecutionRunRepository::new(&tx);
        let current = repository.feature_run(&transition.retired_run.id)?;
        if current.revision != expected_run_revision {
            bail!("feature_run_revision_conflict:{}", current.run.id);
        }
        let freeze = repository
            .active_source_freeze(&current.run.id)?
            .ok_or_else(|| anyhow!("feature_run_premature_source_freeze_missing:{}", current.run.id))?;
        let open_outcome_ids = repository.open_ordinary_outcome_ids(&current.run.plan_id)?;
        let released_outcome_ids =
            repository.active_ordinary_outcome_lease_ids(&current.run.plan_id)?;
        let (
            verification_admission_count,
            verification_attempt_count,
            verification_receipt_count,
        ) = repository.source_freeze_verification_activity(
            &current.project_id,
            &current.run.plan_id,
            &current.run.id,
            &freeze,
        )?;
        if freeze.id != transition.facts.freeze_id
            || freeze.source_revision != transition.facts.frozen_source_revision
            || freeze.source_digest != transition.facts.frozen_source_digest
            || open_outcome_ids != transition.facts.open_outcome_ids
            || released_outcome_ids != transition.facts.released_outcome_ids
            || verification_admission_count != transition.facts.verification_admission_count
            || verification_attempt_count != transition.facts.verification_attempt_count
            || verification_receipt_count != transition.facts.verification_receipt_count
        {
            bail!("feature_run_premature_source_freeze_facts_changed:{}", current.run.id);
        }
        let recomputed = retire_premature_source_freeze_feature_run(
            &current.run,
            &transition.request,
            &transition.facts,
        )
        .map_err(|violation| anyhow!("feature_run_restart_rejected:{violation:?}"))?;
        if recomputed != *transition {
            bail!("feature_run_restart_transition_stale:{}", current.run.id);
        }

        if let Some(batch_id) = transition.batch_id.as_deref() {
            let persisted_batch = repository.batch(batch_id)?;
            if persisted_batch.batch.run_id != current.run.id {
                bail!("feature_run_restart_batch_mismatch:{batch_id}");
            }
            if persisted_batch.batch.status != ExecutionBatchStatus::Ended {
                let mut ended_batch = persisted_batch.batch;
                ended_batch.status = ExecutionBatchStatus::Ended;
                ended_batch.replacement = None;
                repository.save_batch(&ended_batch, persisted_batch.revision)?;
            }
        }
        repository.save_feature_run(&transition.retired_run, expected_run_revision)?;
        for item_id in &transition.facts.released_outcome_ids {
            let changed = tx.execute(
                "UPDATE items
                 SET status = 'ready', worker_id = NULL, pick_token = NULL,
                     last_heartbeat_at = NULL, paused_at = NULL, updated_at = datetime('now')
                 WHERE id = ?1 AND status IN ('picked','running')
                   AND plan_path = (SELECT path FROM plans WHERE id = ?2)",
                params![item_id, transition.request.plan_id],
            )?;
            if changed != 1 {
                bail!("feature_run_restart_outcome_routing_conflict:{item_id}");
            }
        }
        let payload = serde_json::to_string(transition)?;
        tx.execute(
            "INSERT INTO events(project_id, item_id, worker_id, event_type, payload, timestamp)
             VALUES (?1, NULL, ?2, 'feature_run_premature_source_freeze_retired', ?3, datetime('now'))",
            params![current.project_id, operator_worker_id, payload],
        )?;
        tx.commit()?;
        self.feature_run(&transition.retired_run.id)
    }

    pub(crate) fn latest_premature_source_freeze_feature_run_restart(
        &self,
        project_id: &str,
        plan_id: &str,
    ) -> Result<Option<PrematureSourceFreezeRestartTransition>> {
        let payload = self
            .conn
            .query_row(
                "SELECT payload FROM events
                 WHERE project_id = ?1
                   AND event_type = 'feature_run_premature_source_freeze_retired'
                   AND json_extract(payload, '$.request.plan_id') = ?2
                 ORDER BY id DESC LIMIT 1",
                params![project_id, plan_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn active_verification_reservation_ids(&self, run_id: &str) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(
            "SELECT id FROM feature_run_budget_reservations
             WHERE run_id = ?1 AND phase = 'verification' AND status = 'active'
             ORDER BY started_at_unix_ms, id",
        )?;
        statement
            .query_map([run_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub(crate) fn inconsistent_verification_preserved_history(
        &self,
        project_id: &str,
        plan_id: &str,
        run_id: &str,
    ) -> Result<HistoryIdentitySet> {
        history_identity_set(
            self.conn,
            "WITH scope(plan_id, project_id, run_id) AS (VALUES (?1, ?2, ?3))
             SELECT identity FROM (
               SELECT 'proof_obligation:' || obligations.id AS identity FROM proof_obligations obligations, scope WHERE obligations.plan_id = scope.plan_id
               UNION ALL SELECT 'evidence_attempt:' || attempts.id FROM evidence_attempts attempts JOIN proof_obligations obligations ON obligations.id = attempts.obligation_id, scope WHERE obligations.plan_id = scope.plan_id
               UNION ALL SELECT 'attempt_artifact:' || links.id FROM evidence_attempt_artifacts links JOIN evidence_attempts attempts ON attempts.id = links.attempt_id JOIN proof_obligations obligations ON obligations.id = attempts.obligation_id, scope WHERE obligations.plan_id = scope.plan_id
               UNION ALL SELECT 'stored_artifact:' || artifacts.id FROM artifacts JOIN evidence_attempt_artifacts links ON links.artifact_id = artifacts.id JOIN evidence_attempts attempts ON attempts.id = links.attempt_id JOIN proof_obligations obligations ON obligations.id = attempts.obligation_id, scope WHERE obligations.plan_id = scope.plan_id
               UNION ALL SELECT 'evidence_receipt:' || receipts.id FROM evidence_receipts receipts JOIN proof_obligations obligations ON obligations.id = receipts.obligation_id, scope WHERE obligations.plan_id = scope.plan_id
               UNION ALL SELECT 'observation_result:' || results.id FROM evidence_observation_results results JOIN evidence_receipts receipts ON receipts.id = results.receipt_id JOIN proof_obligations obligations ON obligations.id = receipts.obligation_id, scope WHERE obligations.plan_id = scope.plan_id
               UNION ALL SELECT 'coverage_verdict:' || verdicts.id FROM coverage_verdicts verdicts, scope WHERE verdicts.project_id = scope.project_id
               UNION ALL SELECT 'coverage_history:' || history.id FROM coverage_verdict_history history, scope WHERE history.project_id = scope.project_id
               UNION ALL SELECT 'review_gate:' || gates.id FROM review_gates gates, scope WHERE gates.run_id = scope.run_id
               UNION ALL SELECT 'review_attempt:' || attempts.id FROM review_attempts attempts JOIN review_gates gates ON gates.id = attempts.gate_id, scope WHERE gates.run_id = scope.run_id
               UNION ALL SELECT 'review_finding:' || findings.id FROM review_findings findings, scope WHERE findings.run_id = scope.run_id
               UNION ALL SELECT 'review_binding:' || bindings.gate_id FROM final_review_source_bindings bindings JOIN review_gates gates ON gates.id = bindings.gate_id, scope WHERE gates.run_id = scope.run_id
               UNION ALL SELECT 'outcome:' || outcomes.id FROM execution_run_outcomes outcomes, scope WHERE outcomes.run_id = scope.run_id
               UNION ALL SELECT 'one_shot:' || claims.freeze_id FROM feature_run_one_shot_claims claims, scope WHERE claims.run_id = scope.run_id
               UNION ALL SELECT 'budget_observation:' || observations.id FROM feature_run_budget_observations observations, scope WHERE observations.run_id = scope.run_id
               UNION ALL SELECT 'run_event:' || events.id FROM events, scope WHERE events.project_id = scope.project_id AND events.event_type <> 'feature_run_inconsistent_verification_retired' AND json_valid(events.payload) = 1 AND (json_extract(events.payload, '$.run_id') = scope.run_id OR json_extract(events.payload, '$.request.run_id') = scope.run_id OR json_extract(events.payload, '$.retired_run.id') = scope.run_id)
             ) ORDER BY identity",
            params![plan_id, project_id, run_id],
        )
    }

    pub(crate) fn retire_inconsistent_verification_feature_run(
        &self,
        transition: &InconsistentVerificationRetirementTransition,
        operator_worker_id: &str,
    ) -> Result<PersistedFeatureRun> {
        if transition.disposition != FeatureRunRestartDisposition::Retired
            || transition.successor_run_id.is_some()
        {
            bail!("feature_run_restart_transition_not_applicable");
        }
        require_nonempty("operator_worker_id", operator_worker_id)?;
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        let repository = ExecutionRunRepository::new(&tx);
        let current = repository.feature_run(&transition.retired_run.id)?;
        if current.revision != transition.facts.diagnosis.facts.run_revision {
            bail!("feature_run_revision_conflict:{}", current.run.id);
        }
        let snapshot = repository.current_verification_snapshot(&current)?;
        let batch = current
            .run
            .active_batch_id
            .as_deref()
            .map(|id| repository.batch(id))
            .transpose()?
            .map(|persisted| InconsistentVerificationBatchFacts {
                id: persisted.batch.id,
                status: persisted.batch.status,
            });
        let revalidated_facts = crate::execution_run::InconsistentVerificationRetirementFacts {
            diagnosis: classify_current_verification(&snapshot.facts),
            batch,
            active_verification_reservation_ids: repository
                .active_verification_reservation_ids(&current.run.id)?,
            preserved_history: repository.inconsistent_verification_preserved_history(
                &current.project_id,
                &current.run.plan_id,
                &current.run.id,
            )?,
            invalidation_id: transition.facts.invalidation_id.clone(),
        };
        let recomputed = retire_inconsistent_verification_feature_run(
            &current.run,
            &transition.request,
            &revalidated_facts,
        )
        .map_err(|violation| anyhow!("feature_run_restart_rejected:{violation:?}"))?;
        if recomputed != *transition {
            bail!("feature_run_restart_transition_stale:{}", current.run.id);
        }
        if let Some(effect) = transition.batch_effect.as_ref() {
            let persisted_batch = repository.batch(&effect.id)?;
            if persisted_batch.batch.run_id != current.run.id
                || persisted_batch.batch.status != effect.previous_status
            {
                bail!("feature_run_restart_batch_mismatch:{}", effect.id);
            }
            if persisted_batch.batch.status != ExecutionBatchStatus::Ended {
                let mut ended = persisted_batch.batch;
                ended.status = ExecutionBatchStatus::Ended;
                ended.replacement = None;
                repository.save_batch(&ended, persisted_batch.revision)?;
            }
        }
        invalidate_source_in(
            &tx,
            &EvidenceInvalidationRecord {
                id: transition.facts.invalidation_id.clone(),
                run_id: current.run.id.clone(),
                freeze_id: transition.invalidated_freeze_id.clone(),
                finding_id: None,
                reason: "inconsistent_verification".to_string(),
                affected_evidence_ids: Vec::new(),
            },
        )?;
        for reservation_id in &transition.released_verification_reservation_ids {
            repository.release_budget_reservation(reservation_id, &current.run.id)?;
        }
        repository.save_feature_run(&transition.retired_run, current.revision)?;
        if let Some(item_id) = transition.released_verification_item_id.as_deref() {
            let changed = tx.execute(
                "UPDATE items SET status = 'ready', worker_id = NULL, pick_token = NULL,
                     picked_at = NULL, last_heartbeat_at = NULL, paused_at = NULL,
                     updated_at = datetime('now')
                 WHERE id = ?1 AND status IN ('picked','running') AND worker_id = ?2
                   AND plan_path = (SELECT path FROM plans WHERE id = ?3)",
                params![
                    item_id,
                    transition.facts.diagnosis.facts.verifier_worker_id,
                    transition.request.plan_id,
                ],
            )?;
            if changed != 1 {
                bail!("feature_run_restart_verification_item_conflict:{item_id}");
            }
        }
        tx.execute(
            "INSERT INTO events(project_id, item_id, worker_id, event_type, payload, timestamp)
             VALUES (?1, ?2, ?3, 'feature_run_inconsistent_verification_retired', ?4, datetime('now'))",
            params![
                current.project_id,
                transition.released_verification_item_id,
                operator_worker_id,
                serde_json::to_string(transition)?,
            ],
        )?;
        let active_successors: u64 = tx.query_row(
            "SELECT COUNT(*) FROM feature_runs
             WHERE project_id = ?1 AND plan_id = ?2 AND status IN ('active','held')",
            params![current.project_id, current.run.plan_id],
            |row| row.get(0),
        )?;
        let history_after = repository.inconsistent_verification_preserved_history(
            &current.project_id,
            &current.run.plan_id,
            &current.run.id,
        )?;
        let persisted_after = repository.feature_run(&current.run.id)?;
        let freeze_after = repository.source_freeze(&transition.invalidated_freeze_id)?;
        if active_successors != 0
            || history_after != transition.facts.preserved_history
            || persisted_after.run != transition.retired_run
            || persisted_after.revision != transition.resulting_run_revision
            || freeze_after.status != SourceFreezeStatus::Invalidated
        {
            bail!("feature_run_restart_atomic_postcondition_failed:{}", current.run.id);
        }
        tx.commit()?;
        self.feature_run(&transition.retired_run.id)
    }

    pub(crate) fn latest_inconsistent_verification_feature_run_restart(
        &self,
        project_id: &str,
        plan_id: &str,
    ) -> Result<Option<InconsistentVerificationRetirementTransition>> {
        let payload = self
            .conn
            .query_row(
                "SELECT payload FROM events
                 WHERE project_id = ?1
                   AND event_type = 'feature_run_inconsistent_verification_retired'
                   AND json_extract(payload, '$.request.plan_id') = ?2
                 ORDER BY id DESC LIMIT 1",
                params![project_id, plan_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn create_budget_reservation(
        &self,
        reservation: &BudgetReservationRecord,
    ) -> Result<()> {
        require_nonempty("budget_reservation.id", &reservation.id)?;
        require_nonempty("budget_reservation.run_id", &reservation.run_id)?;
        require_nonempty("budget_reservation.boundary_key", &reservation.boundary_key)?;
        require_nonempty(
            "budget_reservation.owner_worker_id",
            &reservation.owner_worker_id,
        )?;
        require_nonempty("budget_reservation.provenance", &reservation.provenance)?;
        if reservation.started_at_unix_ms == 0 || reservation.lease_generation == 0 {
            bail!(
                "budget_reservation_invalid_clock_or_lease:{}",
                reservation.id
            );
        }
        let contract = self.budget_contract(&reservation.run_id)?;
        match (contract.mode, reservation.execution_budget) {
            (FeatureRunBudgetMode::Bounded, Some(execution_budget)) => {
                let diagnostics = validate_execution_budget(&execution_budget);
                if !diagnostics.is_empty()
                    || ExecutionBudget::new(
                        reservation.started_at_unix_ms,
                        execution_budget.maxima(),
                    )? != execution_budget
                {
                    bail!(
                        "budget_reservation_invalid_execution_budget:{}",
                        reservation.id
                    );
                }
            }
            (FeatureRunBudgetMode::Unbounded, None) => {}
            (FeatureRunBudgetMode::Bounded, None) => {
                bail!(
                    "bounded_budget_reservation_missing_execution_budget:{}",
                    reservation.id
                )
            }
            (FeatureRunBudgetMode::Unbounded, Some(_)) => {
                bail!(
                    "unbounded_budget_reservation_has_execution_budget:{}",
                    reservation.id
                )
            }
        }
        let owner_matches = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM feature_run_role_leases WHERE run_id = ?1 AND role = ?2 AND worker_id = ?3 AND lease_generation = ?4 AND released_at IS NULL)",
            params![
                reservation.run_id,
                role_str(reservation.owner_role),
                reservation.owner_worker_id,
                reservation.lease_generation,
            ],
            |row| row.get::<_, bool>(0),
        )?;
        if !owner_matches {
            bail!("budget_reservation_owner_mismatch:{}", reservation.id);
        }
        let execution_budget = reservation.execution_budget;
        self.conn.execute(
            "INSERT INTO feature_run_budget_reservations(id, run_id, contract_digest, phase, boundary_key, owner_role, owner_worker_id, lease_generation, status, reserved_wall_seconds, reserved_tokens, reserved_tool_calls, deadline_unix_ms, started_at_unix_ms, provenance) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                reservation.id,
                reservation.run_id,
                contract.digest,
                budget_phase_str(reservation.phase),
                reservation.boundary_key,
                role_str(reservation.owner_role),
                reservation.owner_worker_id,
                reservation.lease_generation,
                execution_budget.map(|value| value.max_wall_seconds),
                execution_budget.map(|value| value.max_tokens),
                execution_budget.map(|value| value.max_tool_calls),
                execution_budget.map(|value| value.deadline_unix_ms),
                reservation.started_at_unix_ms,
                reservation.provenance,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn active_budget_reservation(
        &self,
        run_id: &str,
        boundary_key: &str,
    ) -> Result<Option<BudgetReservationRecord>> {
        self.budget_reservations(run_id).map(|reservations| {
            reservations
                .into_iter()
                .find(|persisted| {
                    persisted.status == BudgetReservationStatus::Active
                        && persisted.reservation.boundary_key == boundary_key
                })
                .map(|persisted| persisted.reservation)
        })
    }

    pub(crate) fn budget_reservations(
        &self,
        run_id: &str,
    ) -> Result<Vec<PersistedBudgetReservation>> {
        let contract = self.budget_contract(run_id)?;
        let mut statement = self.conn.prepare(
            "SELECT id, contract_digest, phase, boundary_key, owner_role, owner_worker_id,
                    lease_generation, status, reserved_wall_seconds, reserved_tokens,
                    reserved_tool_calls, deadline_unix_ms, started_at_unix_ms,
                    provenance
             FROM feature_run_budget_reservations
             WHERE run_id = ?1
             ORDER BY started_at_unix_ms, id",
        )?;
        let rows = statement.query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<u64>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<u64>>(8)?,
                row.get::<_, Option<u64>>(9)?,
                row.get::<_, Option<u64>>(10)?,
                row.get::<_, Option<u64>>(11)?,
                row.get::<_, u64>(12)?,
                row.get::<_, String>(13)?,
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                contract_digest,
                phase,
                boundary_key,
                owner_role,
                owner_worker_id,
                lease_generation,
                status,
                reserved_wall_seconds,
                reserved_tokens,
                reserved_tool_calls,
                deadline_unix_ms,
                started_at_unix_ms,
                provenance,
            ) = row?;
            if contract_digest.as_deref() != Some(contract.digest.as_str())
                || owner_role.as_deref().is_none_or(str::is_empty)
                || owner_worker_id.as_deref().is_none_or(str::is_empty)
                || lease_generation.is_none()
            {
                bail!("budget_reservation_invalid_v2_identity:{id}");
            }
            let execution_budget = match (
                reserved_wall_seconds,
                reserved_tool_calls,
                reserved_tokens,
                deadline_unix_ms,
            ) {
                (
                    Some(max_wall_seconds),
                    Some(max_tool_calls),
                    Some(max_tokens),
                    Some(deadline_unix_ms),
                ) => Some(ExecutionBudget {
                    max_wall_seconds,
                    max_tool_calls,
                    max_tokens,
                    deadline_unix_ms,
                }),
                (None, None, None, None) => None,
                _ => bail!("budget_reservation_incomplete_execution_budget:{id}"),
            };
            let reservation = BudgetReservationRecord {
                id,
                run_id: run_id.to_string(),
                phase: parse_budget_phase(&phase)?,
                boundary_key,
                owner_role: parse_role(owner_role.as_deref().expect("checked owner role"))?,
                owner_worker_id: owner_worker_id.expect("checked owner worker"),
                lease_generation: lease_generation.expect("checked lease generation"),
                execution_budget,
                started_at_unix_ms,
                provenance,
            };
            if let Some(execution_budget) = execution_budget {
                let diagnostics = validate_execution_budget(&execution_budget);
                if !diagnostics.is_empty()
                    || ExecutionBudget::new(started_at_unix_ms, execution_budget.maxima())?
                        != execution_budget
                {
                    bail!(
                        "budget_reservation_invalid_execution_budget:{}",
                        reservation.id
                    );
                }
            }
            Ok(PersistedBudgetReservation {
                reservation,
                status: parse_budget_reservation_status(&status)?,
            })
        })
        .collect()
    }

    pub(crate) fn release_budget_reservation(
        &self,
        reservation_id: &str,
        run_id: &str,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE feature_run_budget_reservations
             SET status = 'released', finished_at = datetime('now')
             WHERE id = ?1 AND run_id = ?2 AND status = 'active'",
            params![reservation_id, run_id],
        )?;
        if changed != 1 {
            bail!("budget_reservation_not_active:{reservation_id}");
        }
        Ok(())
    }

    pub(crate) fn reconcile_budget_reservation(
        &self,
        reservation_id: &str,
        run_id: &str,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE feature_run_budget_reservations
             SET status = 'reconciled', finished_at = datetime('now')
             WHERE id = ?1 AND run_id = ?2 AND status = 'active'",
            params![reservation_id, run_id],
        )?;
        if changed != 1 {
            bail!("budget_reservation_not_active:{reservation_id}");
        }
        Ok(())
    }

    pub(crate) fn feature_run(&self, run_id: &str) -> Result<PersistedFeatureRun> {
        let row = self
            .conn
            .query_row(
                "SELECT project_id, plan_id, status, phase, policy_digest, source_revision, active_batch_id, outcomes_settled, batch_outcome_count, held_from_phase, hold_reason, terminal_reason, revision,
                        MAX(
                            CAST(strftime('%s', updated_at) AS INTEGER) * 1000 + 999,
                            COALESCE((SELECT json_extract(contract_json, '$.started_at_unix_ms') FROM feature_run_budget_contracts WHERE run_id = ?1), 0),
                            COALESCE((SELECT MAX(started_at_unix_ms) FROM feature_run_budget_reservations WHERE run_id = ?1), 0),
                            COALESCE((SELECT MAX(observed_at_unix_ms) FROM feature_run_budget_observations WHERE run_id = ?1), 0)
                        )
                 FROM feature_runs WHERE id = ?1",
                [run_id],
                |row| {
                    Ok(RawRunRow {
                        project_id: row.get(0)?,
                        plan_id: row.get(1)?,
                        status: row.get(2)?,
                        phase: row.get(3)?,
                        policy_digest: row.get(4)?,
                        source_revision: row.get(5)?,
                        active_batch_id: row.get(6)?,
                        outcomes_settled: row.get(7)?,
                        batch_outcome_count: row.get(8)?,
                        held_from_phase: row.get(9)?,
                        hold_reason: row.get(10)?,
                        terminal_reason: row.get(11)?,
                        revision: row.get(12)?,
                        budget_projection_at_unix_ms: row.get(13)?,
                    })
                },
            )
            .with_context(|| format!("feature run not found: {run_id}"))?;
        let owners = active_role_owners(self.conn, run_id)?;
        let run = FeatureRun {
            id: run_id.to_string(),
            plan_id: row.plan_id,
            status: parse_feature_run_status(&row.status)?,
            phase: parse_feature_run_phase(&row.phase)?,
            policy_digest: row.policy_digest,
            source_revision: row.source_revision,
            active_batch_id: row.active_batch_id,
            role_owners: owners,
            outcomes_settled: to_u32(row.outcomes_settled, "outcomes_settled")?,
            batch_outcome_count: to_u32(row.batch_outcome_count, "batch_outcome_count")?,
            held_from_phase: row
                .held_from_phase
                .as_deref()
                .map(parse_feature_run_phase)
                .transpose()?,
            hold_reason: row
                .hold_reason
                .as_deref()
                .map(parse_hold_reason)
                .transpose()?
                .or_else(|| (row.status == "held").then_some(FeatureRunHoldReason::Budget)),
            terminal_reason: row
                .terminal_reason
                .as_deref()
                .map(parse_terminal_reason)
                .transpose()?,
        };
        validate_feature_run(&run)
            .map_err(|violation| anyhow!("persisted feature run is invalid: {violation:?}"))?;
        Ok(PersistedFeatureRun {
            project_id: row.project_id,
            run,
            revision: to_u64(row.revision, "revision")?,
            budget_projection_at_unix_ms: to_u64(
                row.budget_projection_at_unix_ms,
                "budget_projection_at_unix_ms",
            )?,
        })
    }

    pub(crate) fn save_feature_run(&self, run: &FeatureRun, expected_revision: u64) -> Result<u64> {
        validate_feature_run(run)
            .map_err(|violation| anyhow!("invalid feature run: {violation:?}"))?;
        if !self.conn.is_autocommit() {
            validate_active_batch_link(self.conn, run)?;
            update_feature_run_row(self.conn, run, expected_revision)?;
            synchronize_role_leases(self.conn, &run.id, &run.role_owners)?;
            return Ok(expected_revision + 1);
        }
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        validate_active_batch_link(&tx, run)?;
        update_feature_run_row(&tx, run, expected_revision)?;
        synchronize_role_leases(&tx, &run.id, &run.role_owners)?;
        tx.commit()?;
        Ok(expected_revision + 1)
    }

    pub(crate) fn save_feature_run_with_new_batch(
        &self,
        run: &FeatureRun,
        expected_revision: u64,
        batch: &ExecutionBatch,
    ) -> Result<u64> {
        validate_feature_run(run)
            .map_err(|violation| anyhow!("invalid feature run: {violation:?}"))?;
        if run.active_batch_id.as_deref() != Some(batch.id.as_str()) || batch.run_id != run.id {
            bail!("feature_run_active_batch_mismatch:{}", run.id);
        }
        validate_batch_for_persistence(batch)?;
        validate_batch_maker_owner(run, batch)?;
        if !self.conn.is_autocommit() {
            insert_batch(self.conn, batch)?;
            update_feature_run_row(self.conn, run, expected_revision)?;
            synchronize_role_leases(self.conn, &run.id, &run.role_owners)?;
            return Ok(expected_revision + 1);
        }
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        insert_batch(&tx, batch)?;
        update_feature_run_row(&tx, run, expected_revision)?;
        synchronize_role_leases(&tx, &run.id, &run.role_owners)?;
        tx.commit()?;
        Ok(expected_revision + 1)
    }

    pub(crate) fn roll_feature_run_batch(
        &self,
        ended_batch: &ExecutionBatch,
        expected_batch_revision: u64,
        run: &FeatureRun,
        expected_run_revision: u64,
        successor_batch: &ExecutionBatch,
    ) -> Result<u64> {
        if ended_batch.status != ExecutionBatchStatus::Ended || ended_batch.replacement.is_some() {
            bail!(
                "same_maker_roll_requires_ended_non_replacement_batch:{}",
                ended_batch.id
            );
        }
        if ended_batch.run_id != run.id
            || successor_batch.run_id != run.id
            || ended_batch.maker_worker_id != successor_batch.maker_worker_id
        {
            bail!("same_maker_roll_identity_mismatch:{}", run.id);
        }
        if successor_batch.status != ExecutionBatchStatus::Active
            || !successor_batch.settled_outcome_ids.is_empty()
            || successor_batch.replacement.is_some()
            || run.active_batch_id.as_deref() != Some(successor_batch.id.as_str())
            || run.batch_outcome_count != 0
        {
            bail!("same_maker_roll_invalid_successor:{}", run.id);
        }
        if !self.conn.is_autocommit() {
            self.save_batch(ended_batch, expected_batch_revision)?;
            return self.save_feature_run_with_new_batch(
                run,
                expected_run_revision,
                successor_batch,
            );
        }
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        let repository = ExecutionRunRepository::new(&tx);
        repository.save_batch(ended_batch, expected_batch_revision)?;
        let next_revision = repository.save_feature_run_with_new_batch(
            run,
            expected_run_revision,
            successor_batch,
        )?;
        tx.commit()?;
        Ok(next_revision)
    }

    pub(crate) fn save_batch(&self, batch: &ExecutionBatch, expected_revision: u64) -> Result<u64> {
        validate_batch_for_persistence(batch)?;
        let replacement = batch.replacement.as_ref();
        let changed = self.conn.execute(
            "UPDATE execution_batches SET status = ?1, replaced_maker_worker_id = ?2, successor_maker_worker_id = ?3, replacement_reason = ?4, replacement_reference = ?5, replacement_explanation = ?6, ended_at = CASE WHEN ?1 = 'ended' THEN COALESCE(ended_at, datetime('now')) ELSE NULL END, revision = revision + 1 WHERE id = ?7 AND run_id = ?8 AND maker_worker_id = ?9 AND revision = ?10",
            params![
                batch_status_str(batch.status),
                replacement.map(|value| value.replaced_maker_worker_id.as_str()),
                replacement.map(|value| value.successor_maker_worker_id.as_str()),
                replacement.map(|value| replacement_reason_str(value.reason)),
                replacement.map(|value| value.reference.as_str()),
                replacement.map(|value| value.explanation.as_str()),
                batch.id,
                batch.run_id,
                batch.maker_worker_id,
                expected_revision,
            ],
        )?;
        if changed != 1 {
            bail!("execution_batch_revision_conflict:{}", batch.id);
        }
        Ok(expected_revision + 1)
    }

    pub(crate) fn batch(&self, batch_id: &str) -> Result<PersistedExecutionBatch> {
        let raw = self.conn.query_row(
            "SELECT run_id, maker_worker_id, status, replaced_maker_worker_id, successor_maker_worker_id, replacement_reason, replacement_reference, replacement_explanation, revision FROM execution_batches WHERE id = ?1",
            [batch_id],
            |row| {
                Ok(RawBatchRow {
                    run_id: row.get(0)?,
                    maker_worker_id: row.get(1)?,
                    status: row.get(2)?,
                    replaced_maker_worker_id: row.get(3)?,
                    successor_maker_worker_id: row.get(4)?,
                    replacement_reason: row.get(5)?,
                    replacement_reference: row.get(6)?,
                    replacement_explanation: row.get(7)?,
                    revision: row.get(8)?,
                })
            },
        )?;
        let mut stmt = self.conn.prepare(
            "SELECT id FROM execution_run_outcomes WHERE batch_id = ?1 ORDER BY ordinal, id",
        )?;
        let settled_outcome_ids = stmt
            .query_map([batch_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let replacement = match raw.replacement_reason.as_deref() {
            Some(reason) => Some(MakerReplacement {
                replaced_maker_worker_id: required(raw.replaced_maker_worker_id, "replaced maker")?,
                successor_maker_worker_id: required(
                    raw.successor_maker_worker_id,
                    "successor maker",
                )?,
                reason: parse_replacement_reason(reason)?,
                reference: required(raw.replacement_reference, "replacement reference")?,
                explanation: required(raw.replacement_explanation, "replacement explanation")?,
            }),
            None => None,
        };
        Ok(PersistedExecutionBatch {
            batch: ExecutionBatch {
                id: batch_id.to_string(),
                run_id: raw.run_id,
                maker_worker_id: raw.maker_worker_id,
                status: parse_batch_status(&raw.status)?,
                settled_outcome_ids,
                replacement,
            },
            revision: to_u64(raw.revision, "batch revision")?,
        })
    }

    pub(crate) fn record_outcome(
        &self,
        outcome: &RunOutcomeRecord,
        expected_run_revision: u64,
    ) -> Result<u64> {
        if !self.conn.is_autocommit() {
            return record_outcome_in(self.conn, outcome, expected_run_revision);
        }
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        let next_revision = record_outcome_in(&tx, outcome, expected_run_revision)?;
        tx.commit()?;
        Ok(next_revision)
    }

    pub(crate) fn review_gate_for_scope(
        &self,
        run_id: &str,
        kind: ReviewGateKind,
        scope_kind: ReviewScopeKind,
        scope_id: &str,
    ) -> Result<Option<ReviewGateRecord>> {
        let gate_id = self
            .conn
            .query_row(
                "SELECT id FROM review_gates WHERE run_id = ?1 AND kind = ?2 AND scope_kind = ?3 AND scope_id = ?4 LIMIT 1",
                params![run_id, review_gate_kind_str(kind), review_scope_str(scope_kind), scope_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        gate_id.map(|id| self.review_gate(&id)).transpose()
    }

    pub(crate) fn review_gate_for_outcome_item(
        &self,
        item_id: &str,
    ) -> Result<Option<ReviewGateRecord>> {
        let gate_id = self
            .conn
            .query_row(
                "SELECT gates.id
                 FROM review_gates AS gates
                 JOIN execution_run_outcomes AS outcomes ON outcomes.run_id = gates.run_id
                 WHERE outcomes.item_id = ?1
                   AND gates.scope_kind = 'outcome'
                   AND gates.scope_id = outcomes.item_id
                 ORDER BY gates.created_at DESC, gates.id DESC
                 LIMIT 1",
                [item_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        gate_id.map(|id| self.review_gate(&id)).transpose()
    }

    pub(crate) fn pending_review_gate_for_plan(
        &self,
        project_id: &str,
        plan_id: &str,
    ) -> Result<Option<ReviewGateRecord>> {
        let gate_id = self
            .conn
            .query_row(
                "SELECT gates.id FROM review_gates AS gates JOIN feature_runs AS runs ON runs.id = gates.run_id WHERE runs.project_id = ?1 AND runs.plan_id = ?2 AND runs.status IN ('active','held') AND gates.status = 'pending' ORDER BY CASE gates.kind WHEN 'risk_checkpoint' THEN 0 ELSE 1 END, gates.created_at, gates.id LIMIT 1",
                params![project_id, plan_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        gate_id.map(|id| self.review_gate(&id)).transpose()
    }

    pub(crate) fn repair_review_gate_for_plan(
        &self,
        project_id: &str,
        plan_id: &str,
    ) -> Result<Option<ReviewGateRecord>> {
        let gate_id = self
            .conn
            .query_row(
                "SELECT gates.id FROM review_gates AS gates JOIN feature_runs AS runs ON runs.id = gates.run_id WHERE runs.project_id = ?1 AND runs.plan_id = ?2 AND runs.status IN ('active','held') AND gates.status = 'changes_requested' AND EXISTS (SELECT 1 FROM review_findings findings WHERE findings.gate_id = gates.id AND findings.status = 'open') ORDER BY gates.updated_at, gates.id LIMIT 1",
                params![project_id, plan_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        gate_id.map(|id| self.review_gate(&id)).transpose()
    }

    pub(crate) fn review_gates_for_plan(
        &self,
        project_id: &str,
        plan_id: &str,
        open_only: bool,
    ) -> Result<Vec<ReviewGateRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT gates.id FROM review_gates AS gates JOIN feature_runs AS runs ON runs.id = gates.run_id WHERE runs.project_id = ?1 AND runs.plan_id = ?2 AND (?3 = 0 OR gates.status NOT IN ('accepted','cancelled')) ORDER BY gates.created_at, gates.id",
        )?;
        let ids = stmt
            .query_map(params![project_id, plan_id, open_only], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| self.review_gate(&id)).collect()
    }

    pub(crate) fn review_gates_for_run(
        &self,
        run_id: &str,
        open_only: bool,
    ) -> Result<Vec<ReviewGateRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM review_gates WHERE run_id = ?1 AND (?2 = 0 OR status NOT IN ('accepted','cancelled')) ORDER BY created_at, id",
        )?;
        let ids = stmt
            .query_map(params![run_id, open_only], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| self.review_gate(&id)).collect()
    }

    pub(crate) fn review_gates_for_project(
        &self,
        project_id: &str,
        open_only: bool,
    ) -> Result<Vec<ReviewGateRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT gates.id FROM review_gates AS gates JOIN feature_runs AS runs ON runs.id = gates.run_id WHERE runs.project_id = ?1 AND (?2 = 0 OR gates.status NOT IN ('accepted','cancelled')) ORDER BY gates.created_at, gates.id",
        )?;
        let ids = stmt
            .query_map(params![project_id, open_only], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| self.review_gate(&id)).collect()
    }

    pub(crate) fn set_review_gate_status(
        &self,
        gate_id: &str,
        expected: ReviewGateStatus,
        next: ReviewGateStatus,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE review_gates SET status = ?1, updated_at = datetime('now') WHERE id = ?2 AND status = ?3",
            params![
                review_gate_status_str(next),
                gate_id,
                review_gate_status_str(expected)
            ],
        )?;
        if changed != 1 {
            bail!("review_gate_status_conflict:{gate_id}");
        }
        Ok(())
    }

    pub(crate) fn outcomes(&self, run_id: &str) -> Result<Vec<RunOutcomeRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, batch_id, item_id, ordinal, outcome_json FROM execution_run_outcomes WHERE run_id = ?1 ORDER BY settled_at, id",
        )?;
        stmt.query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .map(|row| {
            let (id, batch_id, item_id, ordinal, outcome_json) = row?;
            Ok(RunOutcomeRecord {
                id,
                run_id: run_id.to_string(),
                batch_id,
                item_id,
                ordinal: to_u32(ordinal, "outcome ordinal")?,
                outcome: serde_json::from_str(&outcome_json)?,
            })
        })
        .collect()
    }

    pub(crate) fn outcome_for_run_item(
        &self,
        run_id: &str,
        item_id: &str,
    ) -> Result<Option<RunOutcomeRecord>> {
        self.conn
            .query_row(
                "SELECT id, batch_id, ordinal, outcome_json FROM execution_run_outcomes WHERE run_id = ?1 AND item_id = ?2",
                params![run_id, item_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?
            .map(|(id, batch_id, ordinal, outcome_json)| {
                Ok(RunOutcomeRecord {
                    id,
                    run_id: run_id.to_string(),
                    batch_id,
                    item_id: item_id.to_string(),
                    ordinal: to_u32(ordinal, "outcome ordinal")?,
                    outcome: serde_json::from_str(&outcome_json)?,
                })
            })
            .transpose()
    }

    pub(crate) fn create_review_gate(&self, gate: &ReviewGateRecord) -> Result<()> {
        if gate.kind == ReviewGateKind::FinalProduct {
            require_nonempty(
                "final_product_gate.source_revision",
                gate.source_revision.as_deref().unwrap_or_default(),
            )?;
        }
        self.conn.execute(
            "INSERT INTO review_gates(id, run_id, scope_kind, scope_id, kind, status, required_risk, responsible_maker_id, latest_attempt, source_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                gate.id,
                gate.run_id,
                review_scope_str(gate.scope_kind),
                gate.scope_id,
                review_gate_kind_str(gate.kind),
                review_gate_status_str(gate.status),
                gate.required_risk,
                gate.responsible_maker_id,
                gate.latest_attempt,
                gate.source_revision,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn create_review_source_binding(
        &self,
        binding: &ReviewSourceBindingRecord,
    ) -> Result<()> {
        require_nonempty("final_review_binding.freeze_id", &binding.freeze_id)?;
        require_nonempty(
            "final_review_binding.source_revision",
            &binding.source_revision,
        )?;
        require_nonempty("final_review_binding.source_digest", &binding.source_digest)?;
        self.conn.execute(
            "INSERT INTO final_review_source_bindings(gate_id, freeze_id, source_revision, source_digest, receipt_lineage_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![binding.gate_id, binding.freeze_id, binding.source_revision, binding.source_digest, serde_json::to_string(&binding.receipt_lineage)?],
        )?;
        Ok(())
    }

    pub(crate) fn reopen_review_gate_with_source_binding(
        &self,
        binding: &ReviewSourceBindingRecord,
    ) -> Result<()> {
        require_nonempty("review_binding.source_revision", &binding.source_revision)?;
        require_nonempty("review_binding.source_digest", &binding.source_digest)?;
        let gate = self.review_gate(&binding.gate_id)?;
        if gate.status == ReviewGateStatus::Pending {
            let existing = self.review_source_binding(&binding.gate_id)?;
            if existing.as_ref() == Some(binding)
                && gate.source_revision.as_deref() == Some(binding.source_revision.as_str())
            {
                return Ok(());
            }
            bail!("review_gate_source_reopen_conflict:{}", binding.gate_id);
        }
        if gate.status != ReviewGateStatus::Accepted {
            bail!(
                "review_gate_source_reopen_requires_accepted:{}",
                binding.gate_id
            );
        }
        let changed = self.conn.execute(
            "UPDATE review_gates SET status = 'pending', source_revision = ?1,
                 accepted_at = NULL, updated_at = datetime('now')
             WHERE id = ?2 AND status = 'accepted' AND latest_attempt = ?3",
            params![
                binding.source_revision,
                binding.gate_id,
                gate.latest_attempt
            ],
        )?;
        if changed != 1 {
            bail!("review_gate_source_reopen_stale:{}", binding.gate_id);
        }
        self.conn.execute(
            "INSERT INTO final_review_source_bindings(gate_id, freeze_id, source_revision, source_digest, receipt_lineage_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(gate_id) DO UPDATE SET freeze_id = excluded.freeze_id,
               source_revision = excluded.source_revision, source_digest = excluded.source_digest,
               receipt_lineage_json = excluded.receipt_lineage_json, created_at = datetime('now')",
            params![binding.gate_id, binding.freeze_id, binding.source_revision,
                binding.source_digest, serde_json::to_string(&binding.receipt_lineage)?],
        )?;
        Ok(())
    }

    pub(crate) fn rebind_review_gate_source(
        &self,
        binding: &ReviewSourceBindingRecord,
    ) -> Result<()> {
        require_nonempty(
            "final_review_binding.source_revision",
            &binding.source_revision,
        )?;
        require_nonempty("final_review_binding.source_digest", &binding.source_digest)?;
        let updated = self.conn.execute(
            "UPDATE review_gates SET source_revision = ?1, updated_at = datetime('now') WHERE id = ?2 AND status = 'changes_requested'",
            params![binding.source_revision, binding.gate_id],
        )?;
        if updated != 1 {
            bail!("review_gate_rebind_rejected:{}", binding.gate_id);
        }
        self.conn.execute(
            "INSERT INTO final_review_source_bindings(gate_id, freeze_id, source_revision, source_digest, receipt_lineage_json) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(gate_id) DO UPDATE SET freeze_id = excluded.freeze_id, source_revision = excluded.source_revision, source_digest = excluded.source_digest, receipt_lineage_json = excluded.receipt_lineage_json, created_at = datetime('now')",
            params![binding.gate_id, binding.freeze_id, binding.source_revision, binding.source_digest, serde_json::to_string(&binding.receipt_lineage)?],
        )?;
        Ok(())
    }

    pub(crate) fn seal_accepted_risk_review_source_binding(
        &self,
        binding: &ReviewSourceBindingRecord,
    ) -> Result<()> {
        require_nonempty("risk_review_binding.freeze_id", &binding.freeze_id)?;
        require_nonempty(
            "risk_review_binding.source_revision",
            &binding.source_revision,
        )?;
        require_nonempty("risk_review_binding.source_digest", &binding.source_digest)?;
        let gate = self.review_gate(&binding.gate_id)?;
        if gate.kind != ReviewGateKind::RiskCheckpoint
            || gate.status != ReviewGateStatus::Accepted
            || gate
                .source_revision
                .as_deref()
                .is_some_and(|source| source != binding.source_revision)
        {
            bail!("risk_review_binding_seal_rejected:{}", binding.gate_id);
        }
        let updated = self.conn.execute(
            "UPDATE review_gates SET source_revision = ?1, updated_at = datetime('now')
             WHERE id = ?2 AND kind = 'risk_checkpoint' AND status = 'accepted'
               AND (source_revision IS NULL OR source_revision = ?1)",
            params![binding.source_revision, binding.gate_id],
        )?;
        if updated != 1 {
            bail!("risk_review_binding_seal_stale:{}", binding.gate_id);
        }
        self.conn.execute(
            "INSERT INTO final_review_source_bindings(
               gate_id, freeze_id, source_revision, source_digest, receipt_lineage_json
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(gate_id) DO UPDATE SET freeze_id = excluded.freeze_id,
               source_revision = excluded.source_revision, source_digest = excluded.source_digest,
               receipt_lineage_json = excluded.receipt_lineage_json, created_at = datetime('now')",
            params![
                binding.gate_id,
                binding.freeze_id,
                binding.source_revision,
                binding.source_digest,
                serde_json::to_string(&binding.receipt_lineage)?
            ],
        )?;
        Ok(())
    }

    pub(crate) fn review_source_binding(
        &self,
        gate_id: &str,
    ) -> Result<Option<ReviewSourceBindingRecord>> {
        self.conn
            .query_row(
                "SELECT freeze_id, source_revision, source_digest, receipt_lineage_json FROM final_review_source_bindings WHERE gate_id = ?1",
                [gate_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?
            .map(|raw| {
                Ok(ReviewSourceBindingRecord {
                    gate_id: gate_id.to_string(),
                    freeze_id: raw.0,
                    source_revision: raw.1,
                    source_digest: raw.2,
                    receipt_lineage: serde_json::from_str(&raw.3)?,
                })
            })
            .transpose()
    }

    pub(crate) fn append_review_attempt(
        &self,
        attempt: &ReviewAttemptRecord,
        findings: &[FindingRecord],
        expected_latest_attempt: u32,
    ) -> Result<()> {
        if attempt.attempt_number != expected_latest_attempt + 1 {
            bail!("review_attempt_sequence_conflict:{}", attempt.gate_id);
        }
        require_nonempty("review_attempt.source_revision", &attempt.source_revision)?;
        if !self.conn.is_autocommit() {
            return append_review_attempt_in(self.conn, attempt, findings, expected_latest_attempt);
        }
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        let (run_id, kind, status, responsible_maker, source_revision): (
            String,
            String,
            String,
            String,
            Option<String>,
        ) = tx.query_row(
            "SELECT run_id, kind, status, responsible_maker_id, source_revision FROM review_gates WHERE id = ?1",
            [&attempt.gate_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
        if matches!(status.as_str(), "accepted" | "cancelled") {
            bail!("review_gate_terminal:{}", attempt.gate_id);
        }
        if kind == "final_product" && source_revision.is_none() {
            bail!("final_product_review_gate_unbound:{}", attempt.gate_id);
        }
        if source_revision
            .as_deref()
            .is_some_and(|revision| revision != attempt.source_revision)
        {
            bail!(
                "review_attempt_source_revision_mismatch:{}",
                attempt.gate_id
            );
        }
        if kind == "final_product" && responsible_maker == attempt.reviewer_worker_id {
            bail!("final_product_review_requires_independent_reviewer");
        }
        tx.execute(
            "INSERT INTO review_attempts(id, gate_id, attempt_number, reviewer_worker_id, reviewer_mode, verdict, source_revision, artifacts_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                attempt.id,
                attempt.gate_id,
                attempt.attempt_number,
                attempt.reviewer_worker_id,
                attempt.reviewer_mode,
                review_verdict_str(attempt.verdict),
                attempt.source_revision,
                serde_json::to_string(&attempt.artifacts)?,
            ],
        )?;
        for finding in findings {
            if finding.gate_id != attempt.gate_id || finding.attempt_id != attempt.id {
                bail!("finding_attempt_mismatch:{}", finding.id);
            }
            tx.execute(
                "INSERT INTO review_findings(id, run_id, gate_id, attempt_id, severity, target, owner_worker_id, status, invalidated_evidence_ids_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    finding.id,
                    run_id,
                    finding.gate_id,
                    finding.attempt_id,
                    finding.severity,
                    finding.target,
                    finding.owner_worker_id,
                    finding_status_str(finding.status),
                    serde_json::to_string(&finding.invalidated_evidence_ids)?,
                ],
            )?;
        }
        let gate_status = match attempt.verdict {
            ReviewVerdict::Accepted => ReviewGateStatus::Accepted,
            ReviewVerdict::ChangesRequested => ReviewGateStatus::ChangesRequested,
            ReviewVerdict::Blocked => ReviewGateStatus::Pending,
        };
        let changed = tx.execute(
            "UPDATE review_gates SET latest_attempt = ?1, status = ?2, accepted_at = CASE WHEN ?2 = 'accepted' THEN datetime('now') ELSE NULL END, updated_at = datetime('now') WHERE id = ?3 AND latest_attempt = ?4",
            params![
                attempt.attempt_number,
                review_gate_status_str(gate_status),
                attempt.gate_id,
                expected_latest_attempt,
            ],
        )?;
        if changed != 1 {
            bail!("review_attempt_sequence_conflict:{}", attempt.gate_id);
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn review_gate(&self, gate_id: &str) -> Result<ReviewGateRecord> {
        let raw = self.conn.query_row(
            "SELECT run_id, scope_kind, scope_id, kind, status, required_risk, responsible_maker_id, latest_attempt, source_revision FROM review_gates WHERE id = ?1",
            [gate_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )?;
        Ok(ReviewGateRecord {
            id: gate_id.to_string(),
            run_id: raw.0,
            scope_kind: parse_review_scope(&raw.1)?,
            scope_id: raw.2,
            kind: parse_review_gate_kind(&raw.3)?,
            status: parse_review_gate_status(&raw.4)?,
            required_risk: raw.5,
            responsible_maker_id: raw.6,
            latest_attempt: to_u32(raw.7, "latest_attempt")?,
            source_revision: raw.8,
        })
    }

    pub(crate) fn review_attempts(&self, gate_id: &str) -> Result<Vec<ReviewAttemptRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, attempt_number, reviewer_worker_id, reviewer_mode, verdict, source_revision, artifacts_json FROM review_attempts WHERE gate_id = ?1 ORDER BY attempt_number, id",
        )?;
        stmt.query_map([gate_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .map(|row| {
            let (id, number, reviewer, mode, verdict, source, artifacts) = row?;
            Ok(ReviewAttemptRecord {
                id,
                gate_id: gate_id.to_string(),
                attempt_number: to_u32(number, "attempt number")?,
                reviewer_worker_id: reviewer,
                reviewer_mode: mode,
                verdict: parse_review_verdict(&verdict)?,
                source_revision: source,
                artifacts: serde_json::from_str(&artifacts)?,
            })
        })
        .collect()
    }

    pub(crate) fn findings(&self, gate_id: &str) -> Result<Vec<FindingRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, attempt_id, severity, target, owner_worker_id, status, invalidated_evidence_ids_json FROM review_findings WHERE gate_id = ?1 ORDER BY created_at, id",
        )?;
        stmt.query_map([gate_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .map(|row| {
            let (id, attempt_id, severity, target, owner, status, invalidated) = row?;
            Ok(FindingRecord {
                id,
                gate_id: gate_id.to_string(),
                attempt_id,
                severity,
                target,
                owner_worker_id: owner,
                status: parse_finding_status(&status)?,
                invalidated_evidence_ids: serde_json::from_str(&invalidated)?,
            })
        })
        .collect()
    }

    pub(crate) fn set_finding_status(
        &self,
        finding_id: &str,
        expected: FindingStatus,
        next: FindingStatus,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE review_findings SET status = ?1, resolved_at = CASE WHEN ?1 = 'resolved' THEN datetime('now') ELSE NULL END WHERE id = ?2 AND status = ?3",
            params![finding_status_str(next), finding_id, finding_status_str(expected)],
        )?;
        if changed != 1 {
            bail!("finding_status_conflict:{finding_id}");
        }
        Ok(())
    }

    pub(crate) fn record_budget_observation(
        &self,
        observation: &BudgetObservationRecord,
    ) -> Result<()> {
        require_nonempty("budget.provenance", &observation.provenance)?;
        if observation.reservation_id.is_some() {
            if observation.sequence.is_none()
                || observation.wall_metering.is_none()
                || observation.tool_calls_metering.is_none()
                || observation.tokens_metering.is_none()
                || observation
                    .adapter_id
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
                || observation.observed_at_unix_ms.is_none()
            {
                bail!(
                    "budget_observation_incomplete_ledger_identity:{}",
                    observation.id
                );
            }
            let wall_metering = observation.wall_metering.expect("checked wall metering");
            let tool_calls_metering = observation
                .tool_calls_metering
                .expect("checked tool-call metering");
            let tokens_metering = observation.tokens_metering.expect("checked token metering");
            let aggregate_metering = wall_metering.min(tool_calls_metering).min(tokens_metering);
            if observation.metering != aggregate_metering
                || !observation_dimension_is_consistent(wall_metering, observation.wall_seconds)
                || !observation_dimension_is_consistent(tool_calls_metering, observation.tool_calls)
                || !observation_dimension_is_consistent(tokens_metering, observation.tokens)
            {
                bail!("budget_observation_provenance_mismatch:{}", observation.id);
            }
        }
        self.conn.execute(
            "INSERT INTO feature_run_budget_observations(id, run_id, reservation_id, sequence, phase, metering, wall_metering, tool_calls_metering, tokens_metering, wall_seconds, tokens, tool_calls, credits_micros, provenance, adapter_id, observed_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                observation.id,
                observation.run_id,
                observation.reservation_id,
                observation.sequence,
                budget_phase_str(observation.phase),
                metering_str(observation.metering),
                observation.wall_metering.map(metering_str),
                observation.tool_calls_metering.map(metering_str),
                observation.tokens_metering.map(metering_str),
                observation.wall_seconds,
                observation.tokens,
                observation.tool_calls,
                observation.credits_micros,
                observation.provenance,
                observation.adapter_id,
                observation.observed_at_unix_ms,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn budget_observations(&self, run_id: &str) -> Result<Vec<BudgetObservationRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, reservation_id, sequence, phase, metering, wall_metering, tool_calls_metering, tokens_metering, wall_seconds, tokens, tool_calls, credits_micros, provenance, adapter_id, observed_at_unix_ms FROM feature_run_budget_observations WHERE run_id = ?1 ORDER BY observed_at, id",
        )?;
        stmt.query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, Option<i64>>(14)?,
            ))
        })?
        .map(|row| {
            let (
                id,
                reservation_id,
                sequence,
                phase,
                metering,
                wall_metering,
                tool_calls_metering,
                tokens_metering,
                wall,
                tokens,
                tools,
                credits,
                provenance,
                adapter_id,
                observed_at_unix_ms,
            ) = row?;
            Ok(BudgetObservationRecord {
                id,
                run_id: run_id.to_string(),
                reservation_id,
                sequence: optional_u64(sequence, "sequence")?,
                phase: parse_budget_phase(&phase)?,
                metering: parse_metering(&metering)?,
                wall_metering: wall_metering.as_deref().map(parse_metering).transpose()?,
                tool_calls_metering: tool_calls_metering
                    .as_deref()
                    .map(parse_metering)
                    .transpose()?,
                tokens_metering: tokens_metering.as_deref().map(parse_metering).transpose()?,
                wall_seconds: optional_u64(wall, "wall_seconds")?,
                tokens: optional_u64(tokens, "tokens")?,
                tool_calls: optional_u64(tools, "tool_calls")?,
                credits_micros: optional_u64(credits, "credits_micros")?,
                provenance,
                adapter_id,
                observed_at_unix_ms: optional_u64(observed_at_unix_ms, "observed_at_unix_ms")?,
            })
        })
        .collect()
    }

    pub(crate) fn freeze_source(&self, freeze: &SourceFreezeRecord) -> Result<()> {
        require_nonempty("freeze.source_revision", &freeze.source_revision)?;
        require_nonempty("freeze.source_digest", &freeze.source_digest)?;
        if freeze.status != SourceFreezeStatus::Active {
            bail!("new source freeze must be active");
        }
        self.conn.execute(
            "INSERT INTO feature_run_source_freezes(id, run_id, source_revision, source_digest, status) VALUES (?1, ?2, ?3, ?4, 'active')",
            params![freeze.id, freeze.run_id, freeze.source_revision, freeze.source_digest],
        )?;
        Ok(())
    }

    pub(crate) fn source_freeze(&self, freeze_id: &str) -> Result<SourceFreezeRecord> {
        let raw = self.conn.query_row(
            "SELECT run_id, source_revision, source_digest, status FROM feature_run_source_freezes WHERE id = ?1",
            [freeze_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )?;
        Ok(SourceFreezeRecord {
            id: freeze_id.to_string(),
            run_id: raw.0,
            source_revision: raw.1,
            source_digest: raw.2,
            status: parse_source_freeze_status(&raw.3)?,
        })
    }

    pub(crate) fn active_source_freeze(&self, run_id: &str) -> Result<Option<SourceFreezeRecord>> {
        let freeze_id = self
            .conn
            .query_row(
                "SELECT id FROM feature_run_source_freezes WHERE run_id = ?1 AND status = 'active' ORDER BY created_at DESC, id DESC LIMIT 1",
                [run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        freeze_id.map(|id| self.source_freeze(&id)).transpose()
    }

    pub(crate) fn invalidate_source(
        &self,
        invalidation: &EvidenceInvalidationRecord,
    ) -> Result<()> {
        require_nonempty("invalidation.reason", &invalidation.reason)?;
        if !self.conn.is_autocommit() {
            return invalidate_source_in(self.conn, invalidation);
        }
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        invalidate_source_in(&tx, invalidation)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn invalidations(&self, run_id: &str) -> Result<Vec<EvidenceInvalidationRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, freeze_id, finding_id, reason, affected_evidence_ids_json FROM feature_run_evidence_invalidations WHERE run_id = ?1 ORDER BY created_at, id",
        )?;
        stmt.query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .map(|row| {
            let (id, freeze_id, finding_id, reason, affected) = row?;
            Ok(EvidenceInvalidationRecord {
                id,
                run_id: run_id.to_string(),
                freeze_id,
                finding_id,
                reason,
                affected_evidence_ids: serde_json::from_str(&affected)?,
            })
        })
        .collect()
    }

    pub(crate) fn product_repair_settlement(
        &self,
        invalidation_id: &str,
    ) -> Result<Option<ProductRepairSettlementRecord>> {
        let raw = self
            .conn
            .query_row(
                "SELECT run_id, responsible_maker_id, selective_obligation_ids_json,
                        settlement_json, source_freeze_id
                 FROM feature_run_product_repair_settlements WHERE invalidation_id = ?1",
                [invalidation_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        raw.map(|raw| {
            Ok(ProductRepairSettlementRecord {
                invalidation_id: invalidation_id.to_string(),
                run_id: raw.0,
                responsible_maker_id: raw.1,
                selective_obligation_ids: serde_json::from_str(&raw.2)?,
                settlement: serde_json::from_str(&raw.3)?,
                source_freeze_id: raw.4,
            })
        })
        .transpose()
    }

    pub(crate) fn product_repair_settlement_for_source_freeze(
        &self,
        run_id: &str,
        source_freeze_id: &str,
    ) -> Result<Option<ProductRepairSettlementRecord>> {
        let invalidation_id = self
            .conn
            .query_row(
                "SELECT invalidation_id FROM feature_run_product_repair_settlements
                 WHERE run_id = ?1 AND source_freeze_id = ?2",
                params![run_id, source_freeze_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        invalidation_id
            .map(|id| self.product_repair_settlement(&id))
            .transpose()
            .map(Option::flatten)
    }

    pub(crate) fn record_product_repair_settlement(
        &self,
        settlement: &ProductRepairSettlementRecord,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO feature_run_product_repair_settlements(
               invalidation_id, run_id, responsible_maker_id,
               selective_obligation_ids_json, settlement_json, source_freeze_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                settlement.invalidation_id,
                settlement.run_id,
                settlement.responsible_maker_id,
                serde_json::to_string(&settlement.selective_obligation_ids)?,
                serde_json::to_string(&settlement.settlement)?,
                settlement.source_freeze_id,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn verification_admission_repair_settlement(
        &self,
        invalidation_id: &str,
    ) -> Result<Option<VerificationAdmissionRepairSettlementRecord>> {
        let raw = self
            .conn
            .query_row(
                "SELECT run_id, repair_batch_id, responsible_maker_id, ended_revision,
                        settlement_json, source_freeze_id, source_revision, source_digest
                 FROM feature_run_verification_admission_repair_settlements
                 WHERE invalidation_id = ?1",
                [invalidation_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()?;
        raw.map(|raw| {
            Ok(VerificationAdmissionRepairSettlementRecord {
                invalidation_id: invalidation_id.to_string(),
                run_id: raw.0,
                repair_batch_id: raw.1,
                responsible_maker_id: raw.2,
                ended_revision: to_u64(raw.3, "verification admission repair ended revision")?,
                settlement: serde_json::from_str(&raw.4)?,
                source_freeze_id: raw.5,
                source_revision: raw.6,
                source_digest: raw.7,
            })
        })
        .transpose()
    }

    pub(crate) fn persist_verification_admission_repair_settlement(
        &self,
        invalidation: &EvidenceInvalidationRecord,
        expected_run_revision: u64,
        expected_verification_item_id: Option<&str>,
        settlement: &VerificationAdmissionRepairSettlementRecord,
        freeze: &SourceFreezeRecord,
        frozen_run: &FeatureRun,
        operator_worker_id: &str,
    ) -> Result<()> {
        if self.conn.is_autocommit() {
            bail!("verification_admission_repair_settlement_requires_active_transaction");
        }
        for (field, value) in [
            ("invalidation_id", settlement.invalidation_id.as_str()),
            ("run_id", settlement.run_id.as_str()),
            ("repair_batch_id", settlement.repair_batch_id.as_str()),
            ("responsible_maker_id", settlement.responsible_maker_id.as_str()),
            ("source_freeze_id", settlement.source_freeze_id.as_str()),
            ("source_revision", settlement.source_revision.as_str()),
            ("source_digest", settlement.source_digest.as_str()),
            ("operator_worker_id", operator_worker_id),
        ] {
            require_nonempty(field, value)?;
        }
        if resolve_evidence_invalidation_kind(
            &invalidation.reason,
            invalidation.finding_id.as_deref(),
            &invalidation.affected_evidence_ids,
        )
        .map_err(|violation| {
            anyhow!(
                "verification_admission_repair_invalidation_rejected:{}:{violation:?}",
                invalidation.id
            )
        })? != Some(EvidenceInvalidationKind::VerificationAdmission)
        {
            bail!(
                "verification_admission_repair_invalidation_kind_mismatch:{}",
                invalidation.id
            );
        }
        let current = self.feature_run(&settlement.run_id)?;
        if current.revision != expected_run_revision
            || current.run.phase != FeatureRunPhase::Implementation
            || current.run.status != FeatureRunStatus::Active
            || current.run.active_batch_id.as_deref()
                != Some(settlement.repair_batch_id.as_str())
            || current.run.batch_outcome_count != 0
            || current.run.role_owners.len() != 1
            || owner_for_role(&current.run, RunRole::Maker)
                .is_none_or(|maker| maker.worker_id != operator_worker_id)
            || settlement.invalidation_id != invalidation.id
            || settlement.responsible_maker_id != operator_worker_id
            || settlement.ended_revision != expected_run_revision.saturating_add(1)
            || settlement.source_freeze_id != freeze.id
            || settlement.source_revision != freeze.source_revision
            || settlement.source_digest != freeze.source_digest
        {
            bail!(
                "verification_admission_repair_settlement_stale_identity:{}",
                invalidation.id
            );
        }
        let persisted_invalidation = self
            .invalidations(&current.run.id)?
            .into_iter()
            .find(|candidate| candidate.id == invalidation.id)
            .ok_or_else(|| {
                anyhow!(
                    "verification_admission_repair_invalidation_missing:{}",
                    invalidation.id
                )
            })?;
        if persisted_invalidation != *invalidation
            || invalidation.run_id != current.run.id
            || self
                .verification_admission_repair_settlement(&invalidation.id)?
                .is_some()
            || self.active_source_freeze(&current.run.id)?.is_some()
        {
            bail!(
                "verification_admission_repair_settlement_facts_changed:{}",
                invalidation.id
            );
        }
        let invalidated_freeze = self.source_freeze(&invalidation.freeze_id)?;
        if invalidated_freeze.run_id != current.run.id
            || invalidated_freeze.status != SourceFreezeStatus::Invalidated
        {
            bail!(
                "verification_admission_repair_invalidated_freeze_mismatch:{}",
                invalidation.id
            );
        }
        let item = self.verification_item_projection(&current.run.plan_id)?;
        if item.as_ref().map(|item| item.id.as_str()) != expected_verification_item_id
            || item.as_ref().is_some_and(|item| {
                item.status != CurrentVerificationItemLeaseStatus::Ready
                    || item.worker_id.is_some()
            })
        {
            bail!(
                "verification_admission_repair_verification_item_mismatch:{}",
                invalidation.id
            );
        }
        let persisted_batch = self.batch(&settlement.repair_batch_id)?;
        if persisted_batch.batch.run_id != current.run.id
            || persisted_batch.batch.maker_worker_id != operator_worker_id
            || persisted_batch.batch.status != ExecutionBatchStatus::Active
            || !persisted_batch.batch.settled_outcome_ids.is_empty()
            || persisted_batch.batch.replacement.is_some()
        {
            bail!(
                "verification_admission_repair_batch_mismatch:{}",
                settlement.repair_batch_id
            );
        }
        if frozen_run.id != current.run.id
            || frozen_run.plan_id != current.run.plan_id
            || frozen_run.phase != FeatureRunPhase::SourceFrozen
            || frozen_run.status != FeatureRunStatus::Active
            || frozen_run.active_batch_id.as_deref()
                != Some(settlement.repair_batch_id.as_str())
            || frozen_run.source_revision.as_deref() != Some(freeze.source_revision.as_str())
            || !frozen_run.role_owners.is_empty()
        {
            bail!(
                "verification_admission_repair_frozen_run_mismatch:{}",
                current.run.id
            );
        }
        let mut ended_batch = persisted_batch.batch;
        ended_batch.status = ExecutionBatchStatus::Ended;
        self.save_batch(&ended_batch, persisted_batch.revision)?;
        self.save_feature_run(frozen_run, expected_run_revision)?;
        self.freeze_source(freeze)?;
        self.conn.execute(
            "INSERT INTO feature_run_verification_admission_repair_settlements(
               invalidation_id, run_id, repair_batch_id, responsible_maker_id,
               ended_revision, settlement_json, source_freeze_id, source_revision, source_digest
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                settlement.invalidation_id,
                settlement.run_id,
                settlement.repair_batch_id,
                settlement.responsible_maker_id,
                settlement.ended_revision,
                serde_json::to_string(&settlement.settlement)?,
                settlement.source_freeze_id,
                settlement.source_revision,
                settlement.source_digest,
            ],
        )?;
        self.conn.execute(
            "INSERT INTO events(project_id, item_id, worker_id, event_type, payload, timestamp)
             VALUES (?1, ?2, ?3, 'feature_run_verification_admission_repair_settled', ?4, datetime('now'))",
            params![
                current.project_id,
                expected_verification_item_id,
                operator_worker_id,
                serde_json::to_string(settlement)?,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn prove_verification_admission_repair_settlement(
        &self,
        invalidation: &EvidenceInvalidationRecord,
        settlement: &VerificationAdmissionRepairSettlementRecord,
        operator_worker_id: &str,
    ) -> Result<()> {
        if resolve_evidence_invalidation_kind(
            &invalidation.reason,
            invalidation.finding_id.as_deref(),
            &invalidation.affected_evidence_ids,
        )
        .map_err(|violation| {
            anyhow!(
                "verification_admission_repair_invalidation_rejected:{}:{violation:?}",
                invalidation.id
            )
        })? != Some(EvidenceInvalidationKind::VerificationAdmission)
        {
            bail!(
                "verification_admission_repair_invalidation_kind_mismatch:{}",
                invalidation.id
            );
        }
        let current = self.feature_run(&settlement.run_id)?;
        let persisted_invalidation = self
            .invalidations(&current.run.id)?
            .into_iter()
            .find(|candidate| candidate.id == invalidation.id);
        let batch = self.batch(&settlement.repair_batch_id)?;
        let active_freeze = self.active_source_freeze(&current.run.id)?;
        if persisted_invalidation.as_ref() != Some(invalidation)
            || settlement.invalidation_id != invalidation.id
            || settlement.responsible_maker_id != operator_worker_id
            || current.run.phase != FeatureRunPhase::SourceFrozen
            || current.run.status != FeatureRunStatus::Active
            || current.revision != settlement.ended_revision
            || current.run.active_batch_id.as_deref()
                != Some(settlement.repair_batch_id.as_str())
            || current.run.source_revision.as_deref()
                != Some(settlement.source_revision.as_str())
            || !current.run.role_owners.is_empty()
            || batch.batch.run_id != current.run.id
            || batch.batch.maker_worker_id != settlement.responsible_maker_id
            || batch.batch.status != ExecutionBatchStatus::Ended
            || batch.batch.replacement.is_some()
            || active_freeze.as_ref().is_none_or(|freeze| {
                freeze.id != settlement.source_freeze_id
                    || freeze.source_revision != settlement.source_revision
                    || freeze.source_digest != settlement.source_digest
                    || freeze.status != SourceFreezeStatus::Active
            })
        {
            bail!(
                "verification_admission_repair_settlement_replay_unproven:{}",
                invalidation.id
            );
        }
        let mut events = self.conn.prepare(
            "SELECT payload FROM events
             WHERE event_type = 'feature_run_verification_admission_repair_settled'
               AND json_extract(payload, '$.invalidation_id') = ?1
             ORDER BY id",
        )?;
        let payloads = events
            .query_map([&invalidation.id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if payloads.len() != 1
            || serde_json::from_str::<VerificationAdmissionRepairSettlementRecord>(&payloads[0])?
                != *settlement
        {
            bail!(
                "verification_admission_repair_settlement_event_unproven:{}",
                invalidation.id
            );
        }
        Ok(())
    }
}

fn invalidate_source_in(
    conn: &Connection,
    invalidation: &EvidenceInvalidationRecord,
) -> Result<()> {
    if let Some(finding_id) = invalidation.finding_id.as_deref() {
        let finding_run_id: String = conn
            .query_row(
                "SELECT gates.run_id FROM review_findings AS findings JOIN review_gates AS gates ON gates.id = findings.gate_id WHERE findings.id = ?1",
                [finding_id],
                |row| row.get(0),
            )
            .with_context(|| format!("review finding not found: {finding_id}"))?;
        if finding_run_id != invalidation.run_id {
            bail!("invalidation_finding_run_mismatch:{finding_id}");
        }
    }
    let changed = conn.execute(
        "UPDATE feature_run_source_freezes SET status = 'invalidated', invalidated_at = datetime('now') WHERE id = ?1 AND run_id = ?2 AND status = 'active'",
        params![invalidation.freeze_id, invalidation.run_id],
    )?;
    if changed != 1 {
        bail!("active_source_freeze_not_found:{}", invalidation.freeze_id);
    }
    conn.execute(
        "INSERT INTO feature_run_evidence_invalidations(id, run_id, freeze_id, finding_id, reason, affected_evidence_ids_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            invalidation.id,
            invalidation.run_id,
            invalidation.freeze_id,
            invalidation.finding_id,
            invalidation.reason,
            serde_json::to_string(&invalidation.affected_evidence_ids)?,
        ],
    )?;
    Ok(())
}

fn history_identity_set<P: Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<HistoryIdentitySet> {
    let mut statement = conn.prepare(sql)?;
    let identities = statement
        .query_map(params, |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(HistoryIdentitySet {
        count: u64::try_from(identities.len())
            .map_err(|_| anyhow!("history_identity_count_overflow"))?,
        identity_digest: sha256_json_digest(&json!(identities))?,
    })
}

struct RawRunRow {
    project_id: String,
    plan_id: String,
    status: String,
    phase: String,
    policy_digest: String,
    source_revision: Option<String>,
    active_batch_id: Option<String>,
    outcomes_settled: i64,
    batch_outcome_count: i64,
    held_from_phase: Option<String>,
    hold_reason: Option<String>,
    terminal_reason: Option<String>,
    revision: i64,
    budget_projection_at_unix_ms: i64,
}

struct RawBatchRow {
    run_id: String,
    maker_worker_id: String,
    status: String,
    replaced_maker_worker_id: Option<String>,
    successor_maker_worker_id: Option<String>,
    replacement_reason: Option<String>,
    replacement_reference: Option<String>,
    replacement_explanation: Option<String>,
    revision: i64,
}

fn record_outcome_in(
    conn: &Connection,
    outcome: &RunOutcomeRecord,
    expected_run_revision: u64,
) -> Result<u64> {
    let batch_status: String = conn.query_row(
        "SELECT status FROM execution_batches WHERE id = ?1 AND run_id = ?2",
        params![outcome.batch_id, outcome.run_id],
        |row| row.get(0),
    )?;
    if batch_status != "active" {
        bail!("execution_batch_not_active:{}", outcome.batch_id);
    }
    conn.execute(
        "INSERT INTO execution_run_outcomes(id, run_id, batch_id, item_id, ordinal, outcome_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            outcome.id,
            outcome.run_id,
            outcome.batch_id,
            outcome.item_id,
            outcome.ordinal,
            serde_json::to_string(&outcome.outcome)?,
        ],
    )?;
    let changed = conn.execute(
        "UPDATE feature_runs SET outcomes_settled = outcomes_settled + 1, batch_outcome_count = batch_outcome_count + 1, revision = revision + 1, updated_at = datetime('now') WHERE id = ?1 AND revision = ?2",
        params![outcome.run_id, expected_run_revision],
    )?;
    if changed != 1 {
        bail!("feature_run_revision_conflict:{}", outcome.run_id);
    }
    Ok(expected_run_revision + 1)
}

fn append_review_attempt_in(
    conn: &Connection,
    attempt: &ReviewAttemptRecord,
    findings: &[FindingRecord],
    expected_latest_attempt: u32,
) -> Result<()> {
    let (run_id, kind, status, responsible_maker, source_revision): (
        String,
        String,
        String,
        String,
        Option<String>,
    ) = conn.query_row(
        "SELECT run_id, kind, status, responsible_maker_id, source_revision FROM review_gates WHERE id = ?1",
        [&attempt.gate_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    if matches!(status.as_str(), "accepted" | "cancelled") {
        bail!("review_gate_terminal:{}", attempt.gate_id);
    }
    if kind == "final_product" && source_revision.is_none() {
        bail!("final_product_review_gate_unbound:{}", attempt.gate_id);
    }
    if source_revision
        .as_deref()
        .is_some_and(|revision| revision != attempt.source_revision)
    {
        bail!(
            "review_attempt_source_revision_mismatch:{}",
            attempt.gate_id
        );
    }
    if kind == "final_product" && responsible_maker == attempt.reviewer_worker_id {
        bail!("final_product_review_requires_independent_reviewer");
    }
    conn.execute(
        "INSERT INTO review_attempts(id, gate_id, attempt_number, reviewer_worker_id, reviewer_mode, verdict, source_revision, artifacts_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            attempt.id,
            attempt.gate_id,
            attempt.attempt_number,
            attempt.reviewer_worker_id,
            attempt.reviewer_mode,
            review_verdict_str(attempt.verdict),
            attempt.source_revision,
            serde_json::to_string(&attempt.artifacts)?,
        ],
    )?;
    for finding in findings {
        if finding.gate_id != attempt.gate_id || finding.attempt_id != attempt.id {
            bail!("finding_attempt_mismatch:{}", finding.id);
        }
        conn.execute(
            "INSERT INTO review_findings(id, run_id, gate_id, attempt_id, severity, target, owner_worker_id, status, invalidated_evidence_ids_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                finding.id,
                run_id,
                finding.gate_id,
                finding.attempt_id,
                finding.severity,
                finding.target,
                finding.owner_worker_id,
                finding_status_str(finding.status),
                serde_json::to_string(&finding.invalidated_evidence_ids)?,
            ],
        )?;
    }
    let gate_status = match attempt.verdict {
        ReviewVerdict::Accepted => ReviewGateStatus::Accepted,
        ReviewVerdict::ChangesRequested => ReviewGateStatus::ChangesRequested,
        ReviewVerdict::Blocked => ReviewGateStatus::Pending,
    };
    let changed = conn.execute(
        "UPDATE review_gates SET latest_attempt = ?1, status = ?2, accepted_at = CASE WHEN ?2 = 'accepted' THEN datetime('now') ELSE NULL END, updated_at = datetime('now') WHERE id = ?3 AND latest_attempt = ?4",
        params![
            attempt.attempt_number,
            review_gate_status_str(gate_status),
            attempt.gate_id,
            expected_latest_attempt,
        ],
    )?;
    if changed != 1 {
        bail!("review_attempt_sequence_conflict:{}", attempt.gate_id);
    }
    Ok(())
}

fn insert_batch(conn: &Connection, batch: &ExecutionBatch) -> Result<()> {
    validate_batch_for_persistence(batch)?;
    let replacement = batch.replacement.as_ref();
    conn.execute(
        "INSERT INTO execution_batches(id, run_id, maker_worker_id, status, replaced_maker_worker_id, successor_maker_worker_id, replacement_reason, replacement_reference, replacement_explanation, ended_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, CASE WHEN ?4 = 'ended' THEN datetime('now') ELSE NULL END)",
        params![
            batch.id,
            batch.run_id,
            batch.maker_worker_id,
            batch_status_str(batch.status),
            replacement.map(|value| value.replaced_maker_worker_id.as_str()),
            replacement.map(|value| value.successor_maker_worker_id.as_str()),
            replacement.map(|value| replacement_reason_str(value.reason)),
            replacement.map(|value| value.reference.as_str()),
            replacement.map(|value| value.explanation.as_str()),
        ],
    )?;
    Ok(())
}

fn validate_budget_contract_for_run(
    run: &FeatureRun,
    contract: &FeatureRunBudgetContract,
) -> Result<()> {
    if contract.run_id != run.id {
        bail!("feature_run_budget_contract_run_mismatch:{}", run.id);
    }
    let diagnostics = validate_feature_run_budget_contract(contract);
    if !diagnostics.is_empty() {
        bail!(
            "feature_run_budget_contract_invalid:{}:{diagnostics:?}",
            run.id
        );
    }
    Ok(())
}

fn budget_contract_compatibility_in(
    conn: &Connection,
    run_id: &str,
) -> Result<FeatureRunBudgetContractCompatibility> {
    let row = conn
        .query_row(
            "SELECT runs.budget_contract_digest, contract.schema, contract.digest, contract.contract_json
             FROM feature_runs AS runs
             LEFT JOIN feature_run_budget_contracts AS contract ON contract.run_id = runs.id
             WHERE runs.id = ?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .with_context(|| format!("feature run not found: {run_id}"))?;
    let (Some(bound_digest), Some(schema), Some(digest), Some(contract_json)) = row else {
        return Ok(FeatureRunBudgetContractCompatibility::Missing);
    };
    let Ok(contract) = serde_json::from_str::<FeatureRunBudgetContract>(&contract_json) else {
        return Ok(FeatureRunBudgetContractCompatibility::Invalid);
    };
    if schema != FEATURE_RUN_BUDGET_CONTRACT_SCHEMA
        || contract.schema != FEATURE_RUN_BUDGET_CONTRACT_SCHEMA
        || contract.run_id != run_id
    {
        return Ok(FeatureRunBudgetContractCompatibility::Invalid);
    }
    let Ok(canonical_digest) = feature_run_budget_contract_digest(&contract) else {
        return Ok(FeatureRunBudgetContractCompatibility::Invalid);
    };
    if bound_digest != digest || contract.digest != digest || canonical_digest != digest {
        return Ok(FeatureRunBudgetContractCompatibility::DigestMismatch);
    }
    if !validate_feature_run_budget_contract(&contract).is_empty() {
        return Ok(FeatureRunBudgetContractCompatibility::Invalid);
    }
    Ok(FeatureRunBudgetContractCompatibility::Compatible)
}

const fn observation_dimension_is_consistent(metering: MeteringMode, value: Option<u64>) -> bool {
    match metering {
        MeteringMode::Unavailable => value.is_none(),
        MeteringMode::Estimated | MeteringMode::Trusted => value.is_some(),
    }
}

fn insert_budget_contract(conn: &Connection, contract: &FeatureRunBudgetContract) -> Result<()> {
    let contract_json = serde_json::to_string(contract)?;
    conn.execute(
        "INSERT INTO feature_run_budget_contracts(run_id, schema, digest, contract_json) VALUES (?1, ?2, ?3, ?4)",
        params![contract.run_id, contract.schema, contract.digest, contract_json],
    )?;
    Ok(())
}

fn validate_batch_for_persistence(batch: &ExecutionBatch) -> Result<()> {
    require_nonempty("batch.id", &batch.id)?;
    require_nonempty("batch.run_id", &batch.run_id)?;
    require_nonempty("batch.maker_worker_id", &batch.maker_worker_id)?;
    match (&batch.replacement, batch.status) {
        (None, _) => Ok(()),
        (Some(_), ExecutionBatchStatus::Active | ExecutionBatchStatus::PausedForRiskReview) => {
            bail!("batch_replacement_requires_ended_status:{}", batch.id)
        }
        (Some(replacement), ExecutionBatchStatus::Ended) => {
            require_nonempty(
                "replacement.replaced_maker_worker_id",
                &replacement.replaced_maker_worker_id,
            )?;
            require_nonempty(
                "replacement.successor_maker_worker_id",
                &replacement.successor_maker_worker_id,
            )?;
            require_nonempty("replacement.reference", &replacement.reference)?;
            require_nonempty("replacement.explanation", &replacement.explanation)?;
            if replacement.replaced_maker_worker_id != batch.maker_worker_id {
                bail!("batch_replacement_source_mismatch:{}", batch.id);
            }
            if replacement.replaced_maker_worker_id == replacement.successor_maker_worker_id {
                bail!("batch_replacement_same_worker:{}", batch.id);
            }
            Ok(())
        }
    }
}

fn validate_batch_maker_owner(run: &FeatureRun, batch: &ExecutionBatch) -> Result<()> {
    let maker = owner_for_role(run, RunRole::Maker)
        .ok_or_else(|| anyhow!("feature_run_missing_maker_owner:{}", run.id))?;
    if maker.worker_id != batch.maker_worker_id {
        bail!("feature_run_batch_maker_mismatch:{}", run.id);
    }
    Ok(())
}

fn validate_active_batch_link(conn: &Connection, run: &FeatureRun) -> Result<()> {
    let Some(active_batch_id) = run.active_batch_id.as_deref() else {
        return Ok(());
    };
    let (batch_run_id, maker_worker_id): (String, String) = conn
        .query_row(
            "SELECT run_id, maker_worker_id FROM execution_batches WHERE id = ?1",
            [active_batch_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .with_context(|| format!("active execution batch not found: {active_batch_id}"))?;
    if batch_run_id != run.id {
        bail!("feature_run_active_batch_run_mismatch:{}", run.id);
    }
    if owner_for_role(run, RunRole::Maker).is_some_and(|maker| maker.worker_id != maker_worker_id) {
        bail!("feature_run_batch_maker_mismatch:{}", run.id);
    }
    Ok(())
}

fn update_feature_run_row(
    conn: &Connection,
    run: &FeatureRun,
    expected_revision: u64,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE feature_runs SET status = ?1, phase = ?2, policy_digest = ?3, source_revision = ?4, active_batch_id = ?5, outcomes_settled = ?6, batch_outcome_count = ?7, held_from_phase = ?8, hold_reason = ?9, terminal_reason = ?10, revision = revision + 1, updated_at = datetime('now') WHERE id = ?11 AND revision = ?12",
        params![
            feature_run_status_str(run.status),
            feature_run_phase_str(run.phase),
            run.policy_digest,
            run.source_revision,
            run.active_batch_id,
            run.outcomes_settled,
            run.batch_outcome_count,
            run.held_from_phase.map(feature_run_phase_str),
            run.hold_reason.map(hold_reason_str),
            run.terminal_reason.map(terminal_reason_str),
            run.id,
            expected_revision,
        ],
    )?;
    if changed != 1 {
        bail!("feature_run_revision_conflict:{}", run.id);
    }
    Ok(())
}

fn insert_initial_role_leases(conn: &Connection, run_id: &str, owners: &[RoleOwner]) -> Result<()> {
    for owner in owners {
        conn.execute(
            "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation) VALUES (?1, ?2, ?3, ?4)",
            params![run_id, role_str(owner.role), owner.worker_id, owner.lease_generation],
        )?;
    }
    Ok(())
}

fn synchronize_role_leases(conn: &Connection, run_id: &str, owners: &[RoleOwner]) -> Result<()> {
    let existing = active_role_owners(conn, run_id)?;
    for current in &existing {
        if !owners.contains(current) {
            conn.execute(
                "UPDATE feature_run_role_leases SET released_at = datetime('now') WHERE run_id = ?1 AND role = ?2 AND lease_generation = ?3 AND released_at IS NULL",
                params![run_id, role_str(current.role), current.lease_generation],
            )?;
        }
    }
    for owner in owners {
        if existing.contains(owner) {
            continue;
        }
        let max_generation: Option<i64> = conn.query_row(
            "SELECT MAX(lease_generation) FROM feature_run_role_leases WHERE run_id = ?1 AND role = ?2",
            params![run_id, role_str(owner.role)],
            |row| row.get(0),
        )?;
        if max_generation.is_some_and(|value| owner.lease_generation <= value as u64) {
            bail!(
                "role_lease_generation_not_monotonic:{}",
                role_str(owner.role)
            );
        }
        conn.execute(
            "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation) VALUES (?1, ?2, ?3, ?4)",
            params![run_id, role_str(owner.role), owner.worker_id, owner.lease_generation],
        )?;
    }
    Ok(())
}

fn active_role_owners(conn: &Connection, run_id: &str) -> Result<Vec<RoleOwner>> {
    let mut stmt = conn.prepare(
        "SELECT role, worker_id, lease_generation FROM feature_run_role_leases WHERE run_id = ?1 AND released_at IS NULL ORDER BY role",
    )?;
    stmt.query_map([run_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?
    .map(|row| {
        let (role, worker_id, generation) = row?;
        Ok(RoleOwner {
            role: parse_role(&role)?,
            worker_id,
            lease_generation: to_u64(generation, "lease_generation")?,
        })
    })
    .collect()
}

fn require_nonempty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} must not be empty");
    }
    Ok(())
}

fn required(value: Option<String>, field: &str) -> Result<String> {
    value.with_context(|| format!("persisted {field} is missing"))
}

fn to_u32(value: i64, field: &str) -> Result<u32> {
    value.try_into().with_context(|| format!("invalid {field}"))
}

fn to_u64(value: i64, field: &str) -> Result<u64> {
    value.try_into().with_context(|| format!("invalid {field}"))
}

fn optional_u64(value: Option<i64>, field: &str) -> Result<Option<u64>> {
    value.map(|value| to_u64(value, field)).transpose()
}

macro_rules! string_enum {
    ($as_fn:ident, $parse_fn:ident, $ty:ty, {$($variant:path => $value:literal),+ $(,)?}) => {
        fn $as_fn(value: $ty) -> &'static str {
            match value {$($variant => $value),+}
        }
        fn $parse_fn(value: &str) -> Result<$ty> {
            match value {$($value => Ok($variant),)+ other => bail!("invalid persisted {}: {other}", stringify!($ty))}
        }
    };
}

string_enum!(feature_run_status_str, parse_feature_run_status, FeatureRunStatus, {
    FeatureRunStatus::Active => "active", FeatureRunStatus::Held => "held",
    FeatureRunStatus::Complete => "complete", FeatureRunStatus::Cancelled => "cancelled"
});
string_enum!(feature_run_phase_str, parse_feature_run_phase, FeatureRunPhase, {
    FeatureRunPhase::Implementation => "implementation", FeatureRunPhase::RiskReview => "risk_review",
    FeatureRunPhase::SourceFrozen => "source_frozen", FeatureRunPhase::Verification => "verification",
    FeatureRunPhase::FinalReview => "final_review", FeatureRunPhase::Complete => "complete",
    FeatureRunPhase::Held => "held", FeatureRunPhase::Cancelled => "cancelled"
});
string_enum!(terminal_reason_str, parse_terminal_reason, FeatureRunTerminalReason, {
    FeatureRunTerminalReason::Completed => "completed", FeatureRunTerminalReason::UserCancelled => "user_cancelled",
    FeatureRunTerminalReason::PolicyCancelled => "policy_cancelled",
    FeatureRunTerminalReason::VerificationAttemptsExhausted => "verification_attempts_exhausted"
});
string_enum!(hold_reason_str, parse_hold_reason, FeatureRunHoldReason, {
    FeatureRunHoldReason::Budget => "budget", FeatureRunHoldReason::Capability => "capability"
});
string_enum!(role_str, parse_role, RunRole, {
    RunRole::Maker => "maker", RunRole::Verifier => "verifier", RunRole::Reviewer => "reviewer"
});
string_enum!(batch_status_str, parse_batch_status, ExecutionBatchStatus, {
    ExecutionBatchStatus::Active => "active", ExecutionBatchStatus::PausedForRiskReview => "paused_for_risk_review",
    ExecutionBatchStatus::Ended => "ended"
});
string_enum!(replacement_reason_str, parse_replacement_reason, MakerReplacementReason, {
    MakerReplacementReason::Unavailable => "unavailable", MakerReplacementReason::ContextLost => "context_lost",
    MakerReplacementReason::OwnershipIncompatible => "ownership_incompatible", MakerReplacementReason::BatchCapReached => "batch_cap_reached"
});
string_enum!(review_scope_str, parse_review_scope, ReviewScopeKind, {
    ReviewScopeKind::Outcome => "outcome", ReviewScopeKind::Plan => "plan"
});
string_enum!(review_gate_kind_str, parse_review_gate_kind, ReviewGateKind, {
    ReviewGateKind::RiskCheckpoint => "risk_checkpoint", ReviewGateKind::FinalProduct => "final_product"
});
string_enum!(review_gate_status_str, parse_review_gate_status, ReviewGateStatus, {
    ReviewGateStatus::Pending => "pending", ReviewGateStatus::Leased => "leased",
    ReviewGateStatus::Accepted => "accepted", ReviewGateStatus::ChangesRequested => "changes_requested",
    ReviewGateStatus::Cancelled => "cancelled"
});
string_enum!(review_verdict_str, parse_review_verdict, ReviewVerdict, {
    ReviewVerdict::Accepted => "accepted", ReviewVerdict::ChangesRequested => "changes_requested",
    ReviewVerdict::Blocked => "blocked"
});
string_enum!(finding_status_str, parse_finding_status, FindingStatus, {
    FindingStatus::Open => "open", FindingStatus::Resolved => "resolved", FindingStatus::Dismissed => "dismissed"
});
string_enum!(budget_phase_str, parse_budget_phase, BudgetPhase, {
    BudgetPhase::Implementation => "implementation", BudgetPhase::Verification => "verification",
    BudgetPhase::Review => "review", BudgetPhase::Repair => "repair"
});
string_enum!(metering_str, parse_metering, MeteringMode, {
    MeteringMode::Unavailable => "unavailable", MeteringMode::Estimated => "estimated", MeteringMode::Trusted => "trusted"
});
string_enum!(budget_reservation_status_str, parse_budget_reservation_status, BudgetReservationStatus, {
    BudgetReservationStatus::Active => "active", BudgetReservationStatus::Reconciled => "reconciled",
    BudgetReservationStatus::Released => "released"
});
string_enum!(source_freeze_status_str, parse_source_freeze_status, SourceFreezeStatus, {
    SourceFreezeStatus::Active => "active", SourceFreezeStatus::Invalidated => "invalidated"
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_run::{
        FeatureRunHoldReason, FeatureRunRestartReason, FeatureRunRestartRequest, PhaseTransition,
        PhaseTransitionCause, apply_phase_transition, replace_batch_maker,
        retire_incompatible_feature_run, roll_batch_for_same_maker,
    };
    use crate::storage::{ensure_schema, open_db};
    use crate::usage_policy::{
        BudgetAmounts, BudgetProvenance, FeatureRunPhaseReserves, MeteringProvenance,
    };
    use serde_json::json;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use tempfile::tempdir;

    fn owner(role: RunRole, worker_id: &str, generation: u64) -> RoleOwner {
        RoleOwner {
            role,
            worker_id: worker_id.into(),
            lease_generation: generation,
        }
    }

    fn feature_run(id: &str) -> FeatureRun {
        FeatureRun {
            id: id.into(),
            plan_id: "plan-a".into(),
            status: FeatureRunStatus::Active,
            phase: FeatureRunPhase::Implementation,
            policy_digest: "sha256:policy".into(),
            source_revision: None,
            active_batch_id: Some(format!("batch-{id}")),
            role_owners: vec![owner(RunRole::Maker, "maker-a", 1)],
            outcomes_settled: 0,
            batch_outcome_count: 0,
            held_from_phase: None,
            hold_reason: None,
            terminal_reason: None,
        }
    }

    fn batch(run_id: &str) -> ExecutionBatch {
        ExecutionBatch {
            id: format!("batch-{run_id}"),
            run_id: run_id.into(),
            maker_worker_id: "maker-a".into(),
            status: ExecutionBatchStatus::Active,
            settled_outcome_ids: Vec::new(),
            replacement: None,
        }
    }

    fn budget_contract(run_id: &str) -> FeatureRunBudgetContract {
        FeatureRunBudgetContract::unbounded(
            run_id,
            1_700_000_000_000,
            BudgetProvenance {
                wall_seconds: MeteringProvenance::Trusted,
                tool_calls: MeteringProvenance::Unavailable,
                tokens: MeteringProvenance::Unavailable,
            },
        )
        .expect("test budget contract")
    }

    fn bounded_budget_contract(run_id: &str) -> FeatureRunBudgetContract {
        let limits = BudgetAmounts {
            wall_seconds: 100,
            tool_calls: 50,
            tokens: 1_000,
        };
        FeatureRunBudgetContract::bounded(
            run_id,
            1_700_000_000_000,
            limits,
            FeatureRunPhaseReserves {
                maker: limits,
                verification: BudgetAmounts::ZERO,
                review: BudgetAmounts::ZERO,
                repair: BudgetAmounts::ZERO,
                release: BudgetAmounts::ZERO,
            },
            BudgetProvenance {
                wall_seconds: MeteringProvenance::Trusted,
                tool_calls: MeteringProvenance::Trusted,
                tokens: MeteringProvenance::Trusted,
            },
        )
        .expect("bounded test budget contract")
    }

    fn seed_incompatible_run(conn: &Connection, run_id: &str) {
        let batch_id = format!("batch-{run_id}");
        conn.execute_batch("BEGIN IMMEDIATE")
            .expect("begin legacy run seed");
        conn.execute(
            "INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, active_batch_id) VALUES (?1, 'project-a', 'plan-a', 'active', 'implementation', 'sha256:legacy', ?2)",
            params![run_id, batch_id],
        )
        .expect("legacy run");
        conn.execute(
            "INSERT INTO execution_batches(id, run_id, maker_worker_id, status) VALUES (?1, ?2, 'maker-a', 'active')",
            params![batch_id, run_id],
        )
        .expect("legacy batch");
        conn.execute(
            "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation) VALUES (?1, 'maker', 'maker-a', 1)",
            [run_id],
        )
        .expect("legacy lease");
        conn.execute_batch("COMMIT")
            .expect("commit legacy run seed");
    }

    #[test]
    fn budget_storage_contract_is_atomic_insert_only_and_tamper_evident() {
        let conn = Connection::open_in_memory().expect("database");
        ensure_schema(&conn).expect("schema");
        let repo = ExecutionRunRepository::new(&conn);
        let run = feature_run("run-budget-storage");
        let contract = budget_contract(&run.id);
        repo.create_feature_run("project-a", &run, &contract, Some(&batch(&run.id)))
            .expect("atomic FeatureRun and contract");
        assert_eq!(
            repo.budget_contract(&run.id).expect("load contract"),
            contract
        );
        assert!(
            conn.execute(
                "UPDATE feature_run_budget_contracts SET schema = 'tampered' WHERE run_id = ?1",
                [&run.id],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "DELETE FROM feature_run_budget_contracts WHERE run_id = ?1",
                [&run.id],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "UPDATE feature_runs SET budget_contract_digest = 'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE id = ?1",
                [&run.id],
            )
            .is_err()
        );

        let rollback_run = feature_run("run-budget-rollback");
        conn.execute_batch(
            "CREATE TRIGGER fail_budget_storage_batch BEFORE INSERT ON execution_batches WHEN NEW.run_id = 'run-budget-rollback' BEGIN SELECT RAISE(ABORT, 'injected budget storage rollback'); END;",
        )
        .expect("rollback trigger");
        assert!(
            repo.create_feature_run(
                "project-a",
                &rollback_run,
                &budget_contract(&rollback_run.id),
                Some(&batch(&rollback_run.id)),
            )
            .is_err()
        );
        for table in [
            "feature_runs",
            "feature_run_budget_contracts",
            "feature_run_role_leases",
            "execution_batches",
        ] {
            let count: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table} WHERE {} = ?1",
                        if table == "feature_runs" {
                            "id"
                        } else {
                            "run_id"
                        }
                    ),
                    [&rollback_run.id],
                    |row| row.get(0),
                )
                .expect("rollback count");
            assert_eq!(count, 0, "{table} rolled back");
        }

        let mut corrupt = budget_contract("run-corrupt-contract");
        corrupt.digest =
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
        assert!(
            repo.create_feature_run(
                "project-a",
                &feature_run("run-corrupt-contract"),
                &corrupt,
                Some(&batch("run-corrupt-contract")),
            )
            .is_err()
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM feature_runs WHERE id = 'run-corrupt-contract'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("corrupt rollback"),
            0
        );
    }

    #[test]
    fn incompatible_feature_run_retirement_is_atomic_and_preserves_history() {
        let conn = Connection::open_in_memory().expect("database");
        ensure_schema(&conn).expect("schema");
        seed_incompatible_run(&conn, "run-incompatible");
        conn.execute(
            "INSERT INTO execution_run_outcomes(id, run_id, batch_id, item_id, ordinal, outcome_json) VALUES ('outcome-history', 'run-incompatible', 'batch-run-incompatible', 'item-history', 1, '{}')",
            [],
        )
        .expect("outcome history");
        conn.execute(
            "INSERT INTO review_gates(id, run_id, scope_kind, scope_id, kind, status, responsible_maker_id) VALUES ('gate-history', 'run-incompatible', 'outcome', 'item-history', 'risk_checkpoint', 'changes_requested', 'maker-a')",
            [],
        )
        .expect("review gate history");
        conn.execute(
            "INSERT INTO review_attempts(id, gate_id, attempt_number, reviewer_worker_id, reviewer_mode, verdict, source_revision, artifacts_json) VALUES ('attempt-history', 'gate-history', 1, 'reviewer-a', 'independent', 'changes_requested', 'sha256:source', '[]')",
            [],
        )
        .expect("review attempt history");
        conn.execute(
            "INSERT INTO review_findings(id, run_id, gate_id, attempt_id, severity, target, owner_worker_id, status, invalidated_evidence_ids_json) VALUES ('finding-history', 'run-incompatible', 'gate-history', 'attempt-history', 'high', 'src/history.rs', 'maker-a', 'open', '[]')",
            [],
        )
        .expect("finding history");
        conn.execute(
            "INSERT INTO feature_run_budget_reservations(id, run_id, phase, boundary_key, status, started_at_unix_ms, provenance) VALUES ('reservation-history', 'run-incompatible', 'implementation', 'implementation:item-history', 'active', 1700000000000, 'legacy.history')",
            [],
        )
        .expect("metering reservation history");
        conn.execute(
            "INSERT INTO feature_run_budget_observations(id, run_id, phase, metering, wall_seconds, provenance) VALUES ('observation-history', 'run-incompatible', 'implementation', 'trusted', 4, 'legacy.history')",
            [],
        )
        .expect("metering observation history");
        conn.execute(
            "INSERT INTO logs(id, project_id, item_id, run_id, kind, summary, created_at) VALUES ('log-history', 'project-a', 'item-history', 'run-incompatible', 'completion', 'history', datetime('now'))",
            [],
        )
        .expect("log history");

        let history_tables = [
            "execution_run_outcomes",
            "review_gates",
            "review_attempts",
            "review_findings",
            "feature_run_budget_reservations",
            "feature_run_budget_observations",
            "logs",
        ];
        let before = history_tables
            .iter()
            .map(|table| (*table, count(&conn, table)))
            .collect::<Vec<_>>();
        let repository = ExecutionRunRepository::new(&conn);
        let persisted = repository
            .feature_run("run-incompatible")
            .expect("active legacy run");
        let compatibility = repository
            .budget_contract_compatibility(&persisted.run.id)
            .expect("compatibility diagnosis");
        assert_eq!(
            compatibility,
            FeatureRunBudgetContractCompatibility::Missing
        );
        let transition = retire_incompatible_feature_run(
            &persisted.run,
            &FeatureRunRestartRequest {
                plan_id: "plan-a".into(),
                reason: FeatureRunRestartReason::IncompatibleBudget,
            },
            compatibility,
        )
        .expect("pure retirement");
        assert!(
            repository
                .retire_incompatible_feature_run(
                    &transition,
                    persisted.revision + 1,
                    "operator-stale",
                )
                .unwrap_err()
                .to_string()
                .contains("feature_run_revision_conflict")
        );
        assert_eq!(
            repository
                .feature_run("run-incompatible")
                .expect("stale write rolled back")
                .run
                .status,
            FeatureRunStatus::Active
        );
        let retired = repository
            .retire_incompatible_feature_run(&transition, persisted.revision, "operator-a")
            .expect("atomic retirement");

        assert_eq!(retired.run.status, FeatureRunStatus::Cancelled);
        assert_eq!(
            retired.run.terminal_reason,
            Some(FeatureRunTerminalReason::PolicyCancelled)
        );
        assert_eq!(retired.run.active_batch_id, None);
        assert!(retired.run.role_owners.is_empty());
        assert_eq!(
            conn.query_row(
                "SELECT status FROM execution_batches WHERE id = 'batch-run-incompatible'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("ended batch"),
            "ended"
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM feature_run_role_leases WHERE run_id = 'run-incompatible' AND released_at IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("released leases"),
            0
        );
        for (table, expected) in before {
            assert_eq!(count(&conn, table), expected, "{table} history preserved");
        }
        assert_eq!(
            repository
                .latest_incompatible_feature_run_restart("project-a", "plan-a")
                .expect("restart provenance")
                .expect("persisted restart"),
            transition
        );

        seed_incompatible_run(&conn, "run-rollback");
        conn.execute_batch(
            "CREATE TRIGGER fail_incompatible_retirement BEFORE UPDATE ON feature_runs WHEN OLD.id = 'run-rollback' BEGIN SELECT RAISE(ABORT, 'injected restart rollback'); END;",
        )
        .expect("rollback trigger");
        let rollback_run = repository
            .feature_run("run-rollback")
            .expect("rollback run");
        let rollback_transition = retire_incompatible_feature_run(
            &rollback_run.run,
            &FeatureRunRestartRequest {
                plan_id: "plan-a".into(),
                reason: FeatureRunRestartReason::IncompatibleBudget,
            },
            FeatureRunBudgetContractCompatibility::Missing,
        )
        .expect("rollback transition");
        assert!(
            repository
                .retire_incompatible_feature_run(
                    &rollback_transition,
                    rollback_run.revision,
                    "operator-a",
                )
                .is_err()
        );
        assert_eq!(
            conn.query_row(
                "SELECT status FROM feature_runs WHERE id = 'run-rollback'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("run rollback"),
            "active"
        );
        assert_eq!(
            conn.query_row(
                "SELECT status FROM execution_batches WHERE id = 'batch-run-rollback'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("batch rollback"),
            "active"
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM feature_run_role_leases WHERE run_id = 'run-rollback' AND released_at IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("lease rollback"),
            1
        );
    }

    #[test]
    fn concurrent_incompatible_feature_run_retirement_has_one_atomic_winner() {
        let dir = tempdir().expect("tempdir");
        let db = dir.path().join("restart-race.sqlite");
        let conn = open_db(&db).expect("database");
        ensure_schema(&conn).expect("schema");
        seed_incompatible_run(&conn, "run-race-restart");
        let repository = ExecutionRunRepository::new(&conn);
        let persisted = repository
            .feature_run("run-race-restart")
            .expect("race run");
        let transition = retire_incompatible_feature_run(
            &persisted.run,
            &FeatureRunRestartRequest {
                plan_id: "plan-a".into(),
                reason: FeatureRunRestartReason::IncompatibleBudget,
            },
            FeatureRunBudgetContractCompatibility::Missing,
        )
        .expect("race transition");
        let expected_revision = persisted.revision;
        drop(conn);

        let barrier = Arc::new(Barrier::new(2));
        let results = (0..2)
            .map(|ordinal| {
                let db = db.clone();
                let barrier = Arc::clone(&barrier);
                let transition = transition.clone();
                thread::spawn(move || {
                    let conn = open_db(&db).expect("race connection");
                    let repository = ExecutionRunRepository::new(&conn);
                    barrier.wait();
                    repository
                        .retire_incompatible_feature_run(
                            &transition,
                            expected_revision,
                            &format!("operator-{ordinal}"),
                        )
                        .map(|_| ())
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().expect("race thread"))
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);

        let conn = open_db(&db).expect("final database");
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'feature_run_incompatible_budget_retired'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("one event"),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM feature_runs WHERE id = 'run-race-restart' AND status = 'cancelled' AND terminal_reason = 'policy_cancelled'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("one retired run"),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM feature_run_role_leases WHERE run_id = 'run-race-restart' AND released_at IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("no active leases"),
            0
        );
    }

    #[test]
    fn budget_storage_ledgers_enforce_owner_sequence_and_append_only_integrity() {
        let conn = Connection::open_in_memory().expect("database");
        ensure_schema(&conn).expect("schema");
        let repo = ExecutionRunRepository::new(&conn);
        let run = feature_run("run-budget-ledger");
        repo.create_feature_run(
            "project-a",
            &run,
            &bounded_budget_contract(&run.id),
            Some(&batch(&run.id)),
        )
        .expect("run");
        let started_at_unix_ms = 1_700_000_001_000;
        let execution_budget = ExecutionBudget::new(
            started_at_unix_ms,
            BudgetAmounts {
                wall_seconds: 10,
                tool_calls: 5,
                tokens: 100,
            },
        )
        .expect("execution budget");
        let reservation = BudgetReservationRecord {
            id: "reservation-ledger".into(),
            run_id: run.id.clone(),
            phase: BudgetPhase::Implementation,
            boundary_key: "implementation:item-a".into(),
            owner_role: RunRole::Maker,
            owner_worker_id: "maker-a".into(),
            lease_generation: 1,
            execution_budget: Some(execution_budget),
            started_at_unix_ms,
            provenance: "codex.adapter".into(),
        };
        repo.create_budget_reservation(&reservation)
            .expect("owned reservation");
        let mut wrong_owner = reservation.clone();
        wrong_owner.id = "reservation-wrong-owner".into();
        wrong_owner.owner_worker_id = "maker-other".into();
        assert!(repo.create_budget_reservation(&wrong_owner).is_err());

        for sequence in 1..=2 {
            repo.record_budget_observation(&BudgetObservationRecord {
                id: format!("observation-{sequence}"),
                run_id: run.id.clone(),
                reservation_id: Some(reservation.id.clone()),
                sequence: Some(sequence),
                phase: BudgetPhase::Implementation,
                metering: MeteringMode::Estimated,
                wall_metering: Some(MeteringMode::Trusted),
                tool_calls_metering: Some(MeteringMode::Trusted),
                tokens_metering: Some(MeteringMode::Estimated),
                wall_seconds: Some(sequence),
                tokens: Some(sequence * 10),
                tool_calls: Some(sequence),
                credits_micros: None,
                provenance: "host observation".into(),
                adapter_id: Some("codex.adapter".into()),
                observed_at_unix_ms: Some(started_at_unix_ms + sequence),
            })
            .expect("monotone observation");
        }
        let mut skipped = repo
            .budget_observations(&run.id)
            .expect("observations")
            .pop()
            .expect("latest observation");
        skipped.id = "observation-skipped".into();
        skipped.sequence = Some(4);
        assert!(repo.record_budget_observation(&skipped).is_err());
        assert!(
            conn.execute(
                "UPDATE feature_run_budget_observations SET tokens = 999 WHERE id = 'observation-1'",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "DELETE FROM feature_run_budget_observations WHERE id = 'observation-1'",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "UPDATE feature_run_budget_reservations SET owner_worker_id = 'maker-other' WHERE id = ?1",
                [&reservation.id],
            )
            .is_err()
        );
        assert_eq!(
            conn.execute(
                "UPDATE feature_run_budget_reservations SET status = 'reconciled', finished_at = datetime('now') WHERE id = ?1",
                [&reservation.id],
            )
            .expect("terminal reservation transition"),
            1
        );
        assert!(
            conn.execute(
                "UPDATE feature_run_budget_reservations SET status = 'released' WHERE id = ?1",
                [&reservation.id],
            )
            .is_err()
        );
    }

    fn legal_restart_run(
        id: &str,
        phase: FeatureRunPhase,
        held_from_phase: Option<FeatureRunPhase>,
        terminal_reason: Option<FeatureRunTerminalReason>,
    ) -> FeatureRun {
        let effective_phase = held_from_phase.unwrap_or(phase);
        let role_owners = match effective_phase {
            FeatureRunPhase::Implementation => vec![owner(RunRole::Maker, "maker-a", 1)],
            FeatureRunPhase::RiskReview => vec![
                owner(RunRole::Maker, "maker-a", 1),
                owner(RunRole::Reviewer, "reviewer-a", 1),
            ],
            FeatureRunPhase::Verification => vec![owner(RunRole::Verifier, "verifier-a", 1)],
            FeatureRunPhase::FinalReview => vec![owner(RunRole::Reviewer, "reviewer-a", 1)],
            FeatureRunPhase::SourceFrozen
            | FeatureRunPhase::Complete
            | FeatureRunPhase::Cancelled
            | FeatureRunPhase::Held => Vec::new(),
        };
        let has_active_batch = matches!(
            effective_phase,
            FeatureRunPhase::Implementation | FeatureRunPhase::RiskReview
        );
        FeatureRun {
            id: id.into(),
            plan_id: "plan-a".into(),
            status: match phase {
                FeatureRunPhase::Held => FeatureRunStatus::Held,
                FeatureRunPhase::Complete => FeatureRunStatus::Complete,
                FeatureRunPhase::Cancelled => FeatureRunStatus::Cancelled,
                _ => FeatureRunStatus::Active,
            },
            phase,
            policy_digest: "sha256:policy".into(),
            source_revision: matches!(
                effective_phase,
                FeatureRunPhase::SourceFrozen
                    | FeatureRunPhase::Verification
                    | FeatureRunPhase::FinalReview
                    | FeatureRunPhase::Complete
            )
            .then(|| "source-revision-a".into()),
            active_batch_id: has_active_batch.then(|| format!("batch-{id}")),
            role_owners,
            outcomes_settled: 0,
            batch_outcome_count: 0,
            held_from_phase,
            hold_reason: (phase == FeatureRunPhase::Held).then_some(FeatureRunHoldReason::Budget),
            terminal_reason,
        }
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count rows")
    }

    #[test]
    fn representative_v1_database_upgrades_without_promoting_historical_workflow_rows() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("planr.sqlite");
        let conn = open_db(&path).expect("open legacy database");
        ensure_schema(&conn).expect("create representative v1 base");
        conn.execute_batch(
            r#"
DROP TABLE feature_run_evidence_invalidations;
DROP TABLE review_findings;
DROP TABLE review_attempts;
DROP TABLE review_gates;
DROP TABLE feature_run_budget_reservations;
DROP TABLE feature_run_budget_observations;
DROP TABLE feature_run_budget_contracts;
DROP TABLE feature_run_product_repair_settlements;
DROP TABLE feature_run_source_freezes;
DROP TABLE execution_run_outcomes;
DROP TABLE feature_run_role_leases;
DROP TABLE execution_batches;
DROP TABLE feature_runs;
UPDATE meta SET value = '1' WHERE key = 'schema_version';
INSERT INTO items(id, project_id, title, description, status, work_type, priority, created_at, updated_at) VALUES
  ('legacy-review', 'project-a', 'Legacy review', 'historical', 'closed', 'review', 0, datetime('now'), datetime('now')),
  ('legacy-fix', 'project-a', 'Legacy fix', 'historical', 'closed', 'fix', 0, datetime('now'), datetime('now'));
INSERT INTO runs(id, project_id, item_id, worker_id, client, status) VALUES
  ('legacy-run', 'project-a', 'legacy-review', 'worker-a', 'codex', 'complete');
INSERT INTO logs(id, project_id, item_id, kind, summary, created_at) VALUES
  ('legacy-log', 'project-a', 'legacy-review', 'review', 'historical review', datetime('now'));
"#,
        )
        .expect("legacy fixture");

        ensure_schema(&conn).expect("upgrade legacy database");
        ensure_schema(&conn).expect("upgrade is idempotent");
        assert_eq!(count(&conn, "items"), 2);
        assert_eq!(count(&conn, "runs"), 1);
        assert_eq!(count(&conn, "logs"), 1);
        assert_eq!(count(&conn, "feature_runs"), 0);
        assert_eq!(count(&conn, "review_gates"), 0);
        let version: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema version");
        assert_eq!(version, "2");
        let compatibility_views: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'view' AND (name LIKE '%feature_run%' OR name LIKE '%review_gate%')",
                [],
                |row| row.get(0),
            )
            .expect("compatibility objects");
        assert_eq!(compatibility_views, 0);
    }

    #[test]
    fn canonical_records_survive_restart_with_role_history_and_all_child_domains() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("planr.sqlite");
        {
            let conn = open_db(&path).expect("open database");
            ensure_schema(&conn).expect("schema");
            let repo = ExecutionRunRepository::new(&conn);
            let run = feature_run("run-a");
            repo.create_feature_run(
                "project-a",
                &run,
                &budget_contract("run-a"),
                Some(&batch("run-a")),
            )
            .expect("create run");
            let revision = repo
                .record_outcome(
                    &RunOutcomeRecord {
                        id: "outcome-a".into(),
                        run_id: "run-a".into(),
                        batch_id: "batch-run-a".into(),
                        item_id: "item-a".into(),
                        ordinal: 1,
                        outcome: json!({"status": "settled"}),
                    },
                    0,
                )
                .expect("record outcome");
            assert_eq!(revision, 1);
            let persisted_batch = repo.batch("batch-run-a").expect("load batch");
            let ended_batch = replace_batch_maker(
                &persisted_batch.batch,
                Some(MakerReplacement {
                    replaced_maker_worker_id: "maker-a".into(),
                    successor_maker_worker_id: "maker-b".into(),
                    reason: MakerReplacementReason::Unavailable,
                    reference: "worker:maker-a".into(),
                    explanation: "maker became unavailable".into(),
                }),
                3,
            )
            .expect("end batch with replacement");
            assert_eq!(
                repo.save_batch(&ended_batch, persisted_batch.revision)
                    .expect("save batch"),
                1
            );

            let persisted = repo.feature_run("run-a").expect("load run");
            let frozen = apply_phase_transition(
                &persisted.run,
                &PhaseTransition {
                    to: FeatureRunPhase::SourceFrozen,
                    cause: PhaseTransitionCause::ImplementationSettled,
                    reference: "source-revision-a".into(),
                    owner: None,
                },
            )
            .expect("freeze transition");
            assert_eq!(repo.save_feature_run(&frozen, revision).expect("save"), 2);

            repo.create_review_gate(&ReviewGateRecord {
                id: "gate-a".into(),
                run_id: "run-a".into(),
                scope_kind: ReviewScopeKind::Plan,
                scope_id: "plan-a".into(),
                kind: ReviewGateKind::FinalProduct,
                status: ReviewGateStatus::Pending,
                required_risk: None,
                responsible_maker_id: "maker-a".into(),
                latest_attempt: 0,
                source_revision: Some("source-revision-a".into()),
            })
            .expect("create gate");
            repo.append_review_attempt(
                &ReviewAttemptRecord {
                    id: "attempt-a".into(),
                    gate_id: "gate-a".into(),
                    attempt_number: 1,
                    reviewer_worker_id: "reviewer-a".into(),
                    reviewer_mode: "independent".into(),
                    verdict: ReviewVerdict::ChangesRequested,
                    source_revision: "source-revision-a".into(),
                    artifacts: vec!["artifact-a".into()],
                },
                &[FindingRecord {
                    id: "finding-a".into(),
                    gate_id: "gate-a".into(),
                    attempt_id: "attempt-a".into(),
                    severity: "high".into(),
                    target: "item-a".into(),
                    owner_worker_id: "maker-a".into(),
                    status: FindingStatus::Open,
                    invalidated_evidence_ids: vec!["evidence-a".into()],
                }],
                0,
            )
            .expect("append review attempt");
            repo.record_budget_observation(&BudgetObservationRecord {
                id: "budget-a".into(),
                run_id: "run-a".into(),
                reservation_id: None,
                sequence: None,
                phase: BudgetPhase::Verification,
                metering: MeteringMode::Trusted,
                wall_metering: None,
                tool_calls_metering: None,
                tokens_metering: None,
                wall_seconds: Some(10),
                tokens: Some(20),
                tool_calls: Some(3),
                credits_micros: Some(40),
                provenance: "trusted-host-meter".into(),
                adapter_id: None,
                observed_at_unix_ms: None,
            })
            .expect("budget observation");
            repo.freeze_source(&SourceFreezeRecord {
                id: "freeze-a".into(),
                run_id: "run-a".into(),
                source_revision: "source-revision-a".into(),
                source_digest: "sha256:source".into(),
                status: SourceFreezeStatus::Active,
            })
            .expect("freeze source");
            repo.invalidate_source(&EvidenceInvalidationRecord {
                id: "invalidation-a".into(),
                run_id: "run-a".into(),
                freeze_id: "freeze-a".into(),
                finding_id: Some("finding-a".into()),
                reason: "product_finding".into(),
                affected_evidence_ids: vec!["evidence-a".into()],
            })
            .expect("invalidate source");
        }

        let conn = open_db(&path).expect("reopen database");
        ensure_schema(&conn).expect("schema after restart");
        let repo = ExecutionRunRepository::new(&conn);
        let persisted = repo.feature_run("run-a").expect("resume run");
        assert_eq!(persisted.revision, 2);
        assert_eq!(persisted.run.phase, FeatureRunPhase::SourceFrozen);
        assert_eq!(persisted.run.outcomes_settled, 1);
        assert!(persisted.run.role_owners.is_empty());
        let persisted_batch = repo.batch("batch-run-a").expect("batch");
        assert_eq!(persisted_batch.revision, 1);
        assert_eq!(persisted_batch.batch.settled_count(), 1);
        assert_eq!(persisted_batch.batch.status, ExecutionBatchStatus::Ended);
        assert_eq!(
            persisted_batch
                .batch
                .replacement
                .expect("replacement")
                .successor_maker_worker_id,
            "maker-b"
        );
        assert_eq!(
            repo.review_gate("gate-a").expect("gate").status,
            ReviewGateStatus::ChangesRequested
        );
        assert_eq!(
            repo.outcomes("run-a").expect("outcomes")[0].item_id,
            "item-a"
        );
        assert_eq!(
            repo.review_attempts("gate-a").expect("attempts")[0].verdict,
            ReviewVerdict::ChangesRequested
        );
        assert_eq!(
            repo.findings("gate-a").expect("findings")[0].invalidated_evidence_ids,
            vec!["evidence-a"]
        );
        assert_eq!(
            repo.budget_observations("run-a").expect("budgets")[0].tokens,
            Some(20)
        );
        assert_eq!(
            repo.source_freeze("freeze-a").expect("freeze").status,
            SourceFreezeStatus::Invalidated
        );
        assert_eq!(
            repo.invalidations("run-a").expect("invalidations")[0].affected_evidence_ids,
            vec!["evidence-a"]
        );
        repo.set_finding_status("finding-a", FindingStatus::Open, FindingStatus::Resolved)
            .expect("resolve finding");
        assert_eq!(
            repo.findings("gate-a").expect("resolved findings")[0].status,
            FindingStatus::Resolved
        );
        assert_eq!(count(&conn, "feature_run_role_leases"), 1);
        let released: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM feature_run_role_leases WHERE released_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .expect("released leases");
        assert_eq!(released, 1);
        assert_eq!(count(&conn, "review_attempts"), 1);
        assert_eq!(count(&conn, "review_findings"), 1);
        assert_eq!(count(&conn, "feature_run_budget_observations"), 1);
        assert_eq!(count(&conn, "feature_run_source_freezes"), 1);
        assert_eq!(count(&conn, "feature_run_evidence_invalidations"), 1);
    }

    #[test]
    fn stale_outcome_and_invalid_review_or_invalidation_roll_back_atomically() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("planr.sqlite");
        let conn = open_db(&path).expect("open database");
        ensure_schema(&conn).expect("schema");
        let repo = ExecutionRunRepository::new(&conn);
        repo.create_feature_run(
            "project-a",
            &feature_run("run-rollback"),
            &budget_contract("run-rollback"),
            Some(&batch("run-rollback")),
        )
        .expect("create run");

        let mut new_batch_run = repo.feature_run("run-rollback").expect("load run").run;
        new_batch_run.active_batch_id = Some("batch-new".into());
        let new_batch = ExecutionBatch {
            id: "batch-new".into(),
            run_id: "run-rollback".into(),
            maker_worker_id: "maker-a".into(),
            status: ExecutionBatchStatus::Active,
            settled_outcome_ids: Vec::new(),
            replacement: None,
        };
        assert!(
            repo.save_feature_run_with_new_batch(&new_batch_run, 99, &new_batch)
                .is_err()
        );
        let leaked_batch: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM execution_batches WHERE id = 'batch-new'",
                [],
                |row| row.get(0),
            )
            .expect("new batch rollback");
        assert_eq!(leaked_batch, 0);

        let stale = repo.record_outcome(
            &RunOutcomeRecord {
                id: "outcome-stale".into(),
                run_id: "run-rollback".into(),
                batch_id: "batch-run-rollback".into(),
                item_id: "item-stale".into(),
                ordinal: 1,
                outcome: json!({"status": "settled"}),
            },
            99,
        );
        assert!(stale.unwrap_err().to_string().contains("revision_conflict"));
        assert_eq!(count(&conn, "execution_run_outcomes"), 0);

        repo.create_review_gate(&ReviewGateRecord {
            id: "gate-rollback".into(),
            run_id: "run-rollback".into(),
            scope_kind: ReviewScopeKind::Outcome,
            scope_id: "item-a".into(),
            kind: ReviewGateKind::RiskCheckpoint,
            status: ReviewGateStatus::Pending,
            required_risk: Some("schema_or_migration".into()),
            responsible_maker_id: "maker-a".into(),
            latest_attempt: 0,
            source_revision: None,
        })
        .expect("create gate");
        let attempt = ReviewAttemptRecord {
            id: "attempt-rollback".into(),
            gate_id: "gate-rollback".into(),
            attempt_number: 1,
            reviewer_worker_id: "reviewer-a".into(),
            reviewer_mode: "independent".into(),
            verdict: ReviewVerdict::ChangesRequested,
            source_revision: "source-a".into(),
            artifacts: Vec::new(),
        };
        let invalid_finding = FindingRecord {
            id: "finding-invalid".into(),
            gate_id: "gate-rollback".into(),
            attempt_id: "attempt-rollback".into(),
            severity: "not-a-severity".into(),
            target: "item-a".into(),
            owner_worker_id: "maker-a".into(),
            status: FindingStatus::Open,
            invalidated_evidence_ids: Vec::new(),
        };
        assert!(
            repo.append_review_attempt(&attempt, &[invalid_finding], 0)
                .is_err()
        );
        assert_eq!(count(&conn, "review_attempts"), 0);
        assert_eq!(count(&conn, "review_findings"), 0);
        assert_eq!(repo.review_gate("gate-rollback").unwrap().latest_attempt, 0);

        repo.freeze_source(&SourceFreezeRecord {
            id: "freeze-rollback".into(),
            run_id: "run-rollback".into(),
            source_revision: "source-a".into(),
            source_digest: "sha256:a".into(),
            status: SourceFreezeStatus::Active,
        })
        .expect("freeze");
        assert!(
            repo.invalidate_source(&EvidenceInvalidationRecord {
                id: "invalidation-invalid".into(),
                run_id: "run-rollback".into(),
                freeze_id: "freeze-rollback".into(),
                finding_id: Some("missing-finding".into()),
                reason: "product_finding".into(),
                affected_evidence_ids: Vec::new(),
            })
            .is_err()
        );
        let status: String = conn
            .query_row(
                "SELECT status FROM feature_run_source_freezes WHERE id = 'freeze-rollback'",
                [],
                |row| row.get(0),
            )
            .expect("freeze status");
        assert_eq!(status, "active");
        assert_eq!(count(&conn, "feature_run_evidence_invalidations"), 0);
    }

    #[test]
    fn same_maker_roll_is_atomic_under_stale_revision_and_concurrent_double_roll() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("planr.sqlite");
        {
            let conn = open_db(&path).expect("open database");
            ensure_schema(&conn).expect("schema");
            let repo = ExecutionRunRepository::new(&conn);
            let run = feature_run("run-roll");
            let capped = batch("run-roll");
            repo.create_feature_run(
                "project-a",
                &run,
                &budget_contract("run-roll"),
                Some(&capped),
            )
            .expect("create capped run");
            for (ordinal, item_id) in ["one", "two", "three"].into_iter().enumerate() {
                repo.record_outcome(
                    &RunOutcomeRecord {
                        id: format!("outcome-{item_id}"),
                        run_id: "run-roll".into(),
                        batch_id: "batch-run-roll".into(),
                        item_id: item_id.into(),
                        ordinal: u32::try_from(ordinal + 1).unwrap(),
                        outcome: json!({"status": "settled"}),
                    },
                    u64::try_from(ordinal).unwrap(),
                )
                .expect("record capped outcome");
            }
            let persisted = repo.feature_run("run-roll").expect("run");
            let persisted_batch = repo.batch("batch-run-roll").expect("batch");
            let ended = roll_batch_for_same_maker(&persisted_batch.batch, "maker-a", 3)
                .expect("domain roll");
            let mut next_run = persisted.run.clone();
            next_run.active_batch_id = Some("batch-stale-successor".into());
            next_run.batch_outcome_count = 0;
            let successor = ExecutionBatch {
                id: "batch-stale-successor".into(),
                run_id: "run-roll".into(),
                maker_worker_id: "maker-a".into(),
                status: ExecutionBatchStatus::Active,
                settled_outcome_ids: Vec::new(),
                replacement: None,
            };
            assert!(
                repo.roll_feature_run_batch(
                    &ended,
                    persisted_batch.revision,
                    &next_run,
                    persisted.revision + 1,
                    &successor,
                )
                .is_err()
            );
            assert_eq!(
                repo.batch("batch-run-roll").unwrap().batch.status,
                ExecutionBatchStatus::Active
            );
            assert_eq!(count(&conn, "execution_batches"), 1);
        }

        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for suffix in ["a", "b"] {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                let conn = open_db(&path).expect("concurrent database");
                let repo = ExecutionRunRepository::new(&conn);
                let persisted = repo.feature_run("run-roll").expect("run");
                let persisted_batch = repo.batch("batch-run-roll").expect("batch");
                let ended = roll_batch_for_same_maker(&persisted_batch.batch, "maker-a", 3)
                    .expect("domain roll");
                let successor = ExecutionBatch {
                    id: format!("batch-successor-{suffix}"),
                    run_id: "run-roll".into(),
                    maker_worker_id: "maker-a".into(),
                    status: ExecutionBatchStatus::Active,
                    settled_outcome_ids: Vec::new(),
                    replacement: None,
                };
                let mut next_run = persisted.run.clone();
                next_run.active_batch_id = Some(successor.id.clone());
                next_run.batch_outcome_count = 0;
                barrier.wait();
                repo.roll_feature_run_batch(
                    &ended,
                    persisted_batch.revision,
                    &next_run,
                    persisted.revision,
                    &successor,
                )
            }));
        }
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("join"))
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);

        let conn = open_db(&path).expect("final database");
        let repo = ExecutionRunRepository::new(&conn);
        let run = repo.feature_run("run-roll").expect("final run");
        assert_eq!(run.run.batch_outcome_count, 0);
        assert_eq!(
            run.run.role_owners,
            vec![owner(RunRole::Maker, "maker-a", 1)]
        );
        assert_eq!(count(&conn, "execution_batches"), 2);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM execution_batches WHERE status = 'ended' AND replacement_reason IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM feature_run_role_leases WHERE run_id = 'run-roll' AND role = 'maker' AND worker_id = 'maker-a' AND released_at IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn competing_generation_bound_role_updates_admit_exactly_one_writer() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("planr.sqlite");
        let setup = open_db(&path).expect("open setup database");
        ensure_schema(&setup).expect("schema");
        let repo = ExecutionRunRepository::new(&setup);
        repo.create_feature_run(
            "project-a",
            &feature_run("run-race"),
            &budget_contract("run-race"),
            Some(&batch("run-race")),
        )
        .expect("create run");
        let baseline = repo.feature_run("run-race").expect("load baseline");
        drop(setup);

        let candidates = ["reviewer-a", "reviewer-b"].map(|reviewer| {
            apply_phase_transition(
                &baseline.run,
                &PhaseTransition {
                    to: FeatureRunPhase::RiskReview,
                    cause: PhaseTransitionCause::ProtectedRiskDiscovered,
                    reference: format!("risk:{reviewer}"),
                    owner: Some(owner(RunRole::Reviewer, reviewer, 1)),
                },
            )
            .expect("candidate transition")
        });
        let barrier = Arc::new(Barrier::new(2));
        let connections = [
            open_db(&path).expect("open first competing connection"),
            open_db(&path).expect("open second competing connection"),
        ];
        let handles = candidates
            .into_iter()
            .zip(connections)
            .map(|(candidate, conn)| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    ExecutionRunRepository::new(&conn).save_feature_run(&candidate, 0)
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("writer thread"))
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);

        let conn = open_db(&path).expect("reopen final database");
        let persisted = ExecutionRunRepository::new(&conn)
            .feature_run("run-race")
            .expect("load winner");
        assert_eq!(persisted.revision, 1);
        assert_eq!(persisted.run.phase, FeatureRunPhase::RiskReview);
        assert_eq!(persisted.run.role_owners.len(), 2);
        assert_eq!(
            count(&conn, "feature_run_role_leases"),
            2,
            "losing writer must not leak a lease generation"
        );
    }

    #[test]
    fn every_legal_phase_shape_round_trips_across_restart() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("planr.sqlite");
        let mut expected = vec![
            legal_restart_run(
                "implementation",
                FeatureRunPhase::Implementation,
                None,
                None,
            ),
            legal_restart_run("risk-review", FeatureRunPhase::RiskReview, None, None),
            legal_restart_run("source-frozen", FeatureRunPhase::SourceFrozen, None, None),
            legal_restart_run("verification", FeatureRunPhase::Verification, None, None),
            legal_restart_run("final-review", FeatureRunPhase::FinalReview, None, None),
            legal_restart_run(
                "complete",
                FeatureRunPhase::Complete,
                None,
                Some(FeatureRunTerminalReason::Completed),
            ),
            legal_restart_run(
                "cancelled-user",
                FeatureRunPhase::Cancelled,
                None,
                Some(FeatureRunTerminalReason::UserCancelled),
            ),
            legal_restart_run(
                "cancelled-policy",
                FeatureRunPhase::Cancelled,
                None,
                Some(FeatureRunTerminalReason::PolicyCancelled),
            ),
        ];
        for (id, origin) in [
            ("held-implementation", FeatureRunPhase::Implementation),
            ("held-risk-review", FeatureRunPhase::RiskReview),
            ("held-source-frozen", FeatureRunPhase::SourceFrozen),
            ("held-verification", FeatureRunPhase::Verification),
            ("held-final-review", FeatureRunPhase::FinalReview),
        ] {
            expected.push(legal_restart_run(
                id,
                FeatureRunPhase::Held,
                Some(origin),
                None,
            ));
        }
        {
            let conn = open_db(&path).expect("open database");
            ensure_schema(&conn).expect("schema");
            let repo = ExecutionRunRepository::new(&conn);
            for run in &expected {
                let initial_batch = run.active_batch_id.as_ref().map(|_| batch(&run.id));
                repo.create_feature_run(
                    "project-a",
                    run,
                    &budget_contract(&run.id),
                    initial_batch.as_ref(),
                )
                .unwrap_or_else(|error| panic!("create {}: {error:#}", run.id));
            }
        }

        let conn = open_db(&path).expect("reopen database");
        ensure_schema(&conn).expect("schema after restart");
        let repo = ExecutionRunRepository::new(&conn);
        for run in &expected {
            let persisted = repo
                .feature_run(&run.id)
                .unwrap_or_else(|error| panic!("reload {}: {error:#}", run.id));
            assert_eq!(&persisted.run, run, "phase shape {}", run.id);
            assert_eq!(persisted.revision, 0);
        }
        assert_eq!(count(&conn, "feature_run_role_leases"), 10);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM feature_run_role_leases WHERE released_at IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("active leases"),
            10
        );
    }

    #[test]
    fn active_batch_run_and_maker_ownership_are_transactional_invariants() {
        let conn = Connection::open_in_memory().expect("database");
        ensure_schema(&conn).expect("schema");
        let repo = ExecutionRunRepository::new(&conn);

        let run_a = feature_run("run-a");
        let run_b = feature_run("run-b");
        repo.create_feature_run(
            "project-a",
            &run_a,
            &budget_contract("run-a"),
            Some(&batch("run-a")),
        )
        .expect("run a");
        repo.create_feature_run(
            "project-a",
            &run_b,
            &budget_contract("run-b"),
            Some(&batch("run-b")),
        )
        .expect("run b");

        let mut cross_run = run_a.clone();
        cross_run.active_batch_id = Some("batch-run-b".into());
        let error = repo
            .save_feature_run(&cross_run, 0)
            .expect_err("cross-run batch must be rejected");
        assert!(error.to_string().contains("active_batch_run_mismatch"));
        assert_eq!(
            repo.feature_run("run-a").expect("unchanged run").revision,
            0
        );
        assert!(
            conn.execute(
                "UPDATE feature_runs SET active_batch_id = 'batch-run-b' WHERE id = 'run-a'",
                [],
            )
            .is_err(),
            "composite schema FK must reject cross-run links"
        );

        let mut wrong_saved_owner = run_a.clone();
        wrong_saved_owner.role_owners[0].worker_id = "maker-b".into();
        assert!(
            repo.save_feature_run(&wrong_saved_owner, 0)
                .unwrap_err()
                .to_string()
                .contains("batch_maker_mismatch")
        );
        assert_eq!(
            repo.feature_run("run-a").expect("unchanged run").revision,
            0
        );

        let mut wrong_initial = feature_run("wrong-initial");
        wrong_initial.role_owners[0].worker_id = "maker-b".into();
        assert!(
            repo.create_feature_run(
                "project-a",
                &wrong_initial,
                &budget_contract("wrong-initial"),
                Some(&batch("wrong-initial")),
            )
            .unwrap_err()
            .to_string()
            .contains("batch_maker_mismatch")
        );
        assert_eq!(count(&conn, "feature_runs"), 2);

        let mut next = run_a.clone();
        next.active_batch_id = Some("batch-new".into());
        let mut wrong_new_batch = batch("run-a");
        wrong_new_batch.id = "batch-new".into();
        wrong_new_batch.maker_worker_id = "maker-b".into();
        assert!(
            repo.save_feature_run_with_new_batch(&next, 0, &wrong_new_batch)
                .unwrap_err()
                .to_string()
                .contains("batch_maker_mismatch")
        );
        assert_eq!(count(&conn, "execution_batches"), 2);
        assert_eq!(
            repo.feature_run("run-a").expect("unchanged run").revision,
            0
        );
    }

    #[test]
    fn direct_batch_writes_enforce_replacement_state_and_provenance() {
        let conn = Connection::open_in_memory().expect("database");
        ensure_schema(&conn).expect("schema");
        let repo = ExecutionRunRepository::new(&conn);
        repo.create_feature_run(
            "project-a",
            &feature_run("run-batch"),
            &budget_contract("run-batch"),
            Some(&batch("run-batch")),
        )
        .expect("create run");
        let replacement = MakerReplacement {
            replaced_maker_worker_id: "maker-a".into(),
            successor_maker_worker_id: "maker-b".into(),
            reason: MakerReplacementReason::Unavailable,
            reference: "worker:maker-a".into(),
            explanation: "maker unavailable".into(),
        };

        let mut active_with_replacement = batch("run-batch");
        active_with_replacement.replacement = Some(replacement.clone());
        assert!(
            repo.save_batch(&active_with_replacement, 0)
                .unwrap_err()
                .to_string()
                .contains("requires_ended_status")
        );

        let mut wrong_source = batch("run-batch");
        wrong_source.status = ExecutionBatchStatus::Ended;
        let mut wrong_replacement = replacement;
        wrong_replacement.replaced_maker_worker_id = "maker-x".into();
        wrong_source.replacement = Some(wrong_replacement);
        assert!(
            repo.save_batch(&wrong_source, 0)
                .unwrap_err()
                .to_string()
                .contains("source_mismatch")
        );
        assert_eq!(repo.batch("batch-run-batch").expect("batch").revision, 0);

        let direct_sql = conn.execute(
            "UPDATE execution_batches SET status = 'active', replaced_maker_worker_id = 'maker-a', successor_maker_worker_id = 'maker-b', replacement_reason = 'unavailable', replacement_reference = 'worker:maker-a', replacement_explanation = 'unavailable' WHERE id = 'batch-run-batch'",
            [],
        );
        assert!(
            direct_sql.is_err(),
            "schema must reject invalid replacement state"
        );
        for invalid_assignment in [
            "maker_worker_id = ' '",
            "successor_maker_worker_id = ' '",
            "replacement_reference = ' '",
            "replacement_explanation = ' '",
        ] {
            let statement = format!(
                "UPDATE execution_batches SET status = 'ended', replaced_maker_worker_id = 'maker-a', successor_maker_worker_id = 'maker-b', replacement_reason = 'unavailable', replacement_reference = 'worker:maker-a', replacement_explanation = 'unavailable', {invalid_assignment} WHERE id = 'batch-run-batch'"
            );
            assert!(
                conn.execute(&statement, []).is_err(),
                "schema must reject blank replacement provenance: {invalid_assignment}"
            );
        }
        assert_eq!(repo.batch("batch-run-batch").expect("batch").revision, 0);
    }

    #[test]
    fn stale_source_and_terminal_review_attempts_roll_back() {
        let conn = Connection::open_in_memory().expect("database");
        ensure_schema(&conn).expect("schema");
        let repo = ExecutionRunRepository::new(&conn);
        repo.create_feature_run(
            "project-a",
            &feature_run("run-review"),
            &budget_contract("run-review"),
            Some(&batch("run-review")),
        )
        .expect("create run");
        let unbound_gate = ReviewGateRecord {
            id: "gate-unbound".into(),
            run_id: "run-review".into(),
            scope_kind: ReviewScopeKind::Plan,
            scope_id: "plan-unbound".into(),
            kind: ReviewGateKind::FinalProduct,
            status: ReviewGateStatus::Pending,
            required_risk: None,
            responsible_maker_id: "maker-a".into(),
            latest_attempt: 0,
            source_revision: None,
        };
        assert!(repo.create_review_gate(&unbound_gate).is_err());
        let mut blank_gate = unbound_gate.clone();
        blank_gate.id = "gate-blank".into();
        blank_gate.source_revision = Some("  ".into());
        assert!(repo.create_review_gate(&blank_gate).is_err());
        assert_eq!(count(&conn, "review_gates"), 0);
        assert!(
            conn.execute(
                "INSERT INTO review_gates(id, run_id, scope_kind, scope_id, kind, status, responsible_maker_id, source_revision) VALUES ('gate-direct-unbound', 'run-review', 'plan', 'plan-direct', 'final_product', 'pending', 'maker-a', NULL)",
                [],
            )
            .is_err(),
            "schema must reject an unbound final-product gate"
        );

        repo.create_review_gate(&ReviewGateRecord {
            id: "gate-risk-unbound".into(),
            run_id: "run-review".into(),
            scope_kind: ReviewScopeKind::Outcome,
            scope_id: "item-risk".into(),
            kind: ReviewGateKind::RiskCheckpoint,
            status: ReviewGateStatus::Pending,
            required_risk: Some("schema_or_migration".into()),
            responsible_maker_id: "maker-a".into(),
            latest_attempt: 0,
            source_revision: None,
        })
        .expect("mutable-source risk gate remains valid");
        assert!(
            repo.append_review_attempt(
                &ReviewAttemptRecord {
                    id: "attempt-blank-source".into(),
                    gate_id: "gate-risk-unbound".into(),
                    attempt_number: 1,
                    reviewer_worker_id: "reviewer-a".into(),
                    reviewer_mode: "independent".into(),
                    verdict: ReviewVerdict::Blocked,
                    source_revision: "  ".into(),
                    artifacts: Vec::new(),
                },
                &[],
                0,
            )
            .is_err()
        );
        assert_eq!(count(&conn, "review_attempts"), 0);
        assert_eq!(
            repo.review_gate("gate-risk-unbound")
                .expect("risk gate")
                .latest_attempt,
            0
        );
        assert!(
            conn.execute(
                "INSERT INTO review_attempts(id, gate_id, attempt_number, reviewer_worker_id, reviewer_mode, verdict, source_revision, artifacts_json) VALUES ('attempt-direct-blank', 'gate-risk-unbound', 1, 'reviewer-a', 'independent', 'blocked', ' ', '[]')",
                [],
            )
            .is_err(),
            "schema must reject blank attempt revisions"
        );
        repo.create_review_gate(&ReviewGateRecord {
            id: "gate-review".into(),
            run_id: "run-review".into(),
            scope_kind: ReviewScopeKind::Plan,
            scope_id: "plan-a".into(),
            kind: ReviewGateKind::FinalProduct,
            status: ReviewGateStatus::Pending,
            required_risk: None,
            responsible_maker_id: "maker-a".into(),
            latest_attempt: 0,
            source_revision: Some("source-a".into()),
        })
        .expect("gate");
        let mut attempt = ReviewAttemptRecord {
            id: "attempt-stale".into(),
            gate_id: "gate-review".into(),
            attempt_number: 1,
            reviewer_worker_id: "reviewer-a".into(),
            reviewer_mode: "independent".into(),
            verdict: ReviewVerdict::Accepted,
            source_revision: "source-stale".into(),
            artifacts: Vec::new(),
        };
        assert!(
            repo.append_review_attempt(&attempt, &[], 0)
                .unwrap_err()
                .to_string()
                .contains("source_revision_mismatch")
        );
        assert_eq!(count(&conn, "review_attempts"), 0);
        assert_eq!(repo.review_gate("gate-review").unwrap().latest_attempt, 0);

        attempt.id = "attempt-accepted".into();
        attempt.source_revision = "source-a".into();
        repo.append_review_attempt(&attempt, &[], 0)
            .expect("accept current source");
        let terminal_attempt = ReviewAttemptRecord {
            id: "attempt-after-terminal".into(),
            attempt_number: 2,
            ..attempt
        };
        assert!(
            repo.append_review_attempt(&terminal_attempt, &[], 1)
                .unwrap_err()
                .to_string()
                .contains("review_gate_terminal")
        );
        assert_eq!(count(&conn, "review_attempts"), 1);
        assert_eq!(repo.review_gate("gate-review").unwrap().latest_attempt, 1);

        repo.create_review_gate(&ReviewGateRecord {
            id: "gate-cancelled".into(),
            run_id: "run-review".into(),
            scope_kind: ReviewScopeKind::Outcome,
            scope_id: "item-a".into(),
            kind: ReviewGateKind::RiskCheckpoint,
            status: ReviewGateStatus::Cancelled,
            required_risk: Some("schema_or_migration".into()),
            responsible_maker_id: "maker-a".into(),
            latest_attempt: 0,
            source_revision: Some("source-a".into()),
        })
        .expect("cancelled gate");
        let cancelled_attempt = ReviewAttemptRecord {
            id: "attempt-cancelled".into(),
            gate_id: "gate-cancelled".into(),
            attempt_number: 1,
            reviewer_worker_id: "reviewer-a".into(),
            reviewer_mode: "independent".into(),
            verdict: ReviewVerdict::Blocked,
            source_revision: "source-a".into(),
            artifacts: Vec::new(),
        };
        assert!(
            repo.append_review_attempt(&cancelled_attempt, &[], 0)
                .is_err()
        );
        assert_eq!(count(&conn, "review_attempts"), 1);
    }

    #[test]
    fn evidence_invalidation_rejects_findings_from_another_run_atomically() {
        let conn = Connection::open_in_memory().expect("database");
        ensure_schema(&conn).expect("schema");
        let repo = ExecutionRunRepository::new(&conn);
        for run_id in ["run-freeze", "run-finding"] {
            repo.create_feature_run(
                "project-a",
                &feature_run(run_id),
                &budget_contract(run_id),
                Some(&batch(run_id)),
            )
            .expect("create run");
        }
        repo.create_review_gate(&ReviewGateRecord {
            id: "gate-finding".into(),
            run_id: "run-finding".into(),
            scope_kind: ReviewScopeKind::Plan,
            scope_id: "plan-a".into(),
            kind: ReviewGateKind::FinalProduct,
            status: ReviewGateStatus::Pending,
            required_risk: None,
            responsible_maker_id: "maker-a".into(),
            latest_attempt: 0,
            source_revision: Some("source-a".into()),
        })
        .expect("gate");
        repo.append_review_attempt(
            &ReviewAttemptRecord {
                id: "attempt-finding".into(),
                gate_id: "gate-finding".into(),
                attempt_number: 1,
                reviewer_worker_id: "reviewer-a".into(),
                reviewer_mode: "independent".into(),
                verdict: ReviewVerdict::ChangesRequested,
                source_revision: "source-a".into(),
                artifacts: Vec::new(),
            },
            &[FindingRecord {
                id: "finding-other-run".into(),
                gate_id: "gate-finding".into(),
                attempt_id: "attempt-finding".into(),
                severity: "high".into(),
                target: "item-a".into(),
                owner_worker_id: "maker-a".into(),
                status: FindingStatus::Open,
                invalidated_evidence_ids: Vec::new(),
            }],
            0,
        )
        .expect("finding");
        repo.freeze_source(&SourceFreezeRecord {
            id: "freeze-a".into(),
            run_id: "run-freeze".into(),
            source_revision: "source-a".into(),
            source_digest: "sha256:a".into(),
            status: SourceFreezeStatus::Active,
        })
        .expect("freeze");

        assert!(
            repo.invalidate_source(&EvidenceInvalidationRecord {
                id: "invalid-cross-run".into(),
                run_id: "run-freeze".into(),
                freeze_id: "freeze-a".into(),
                finding_id: Some("finding-other-run".into()),
                reason: "product_finding".into(),
                affected_evidence_ids: Vec::new(),
            })
            .unwrap_err()
            .to_string()
            .contains("finding_run_mismatch")
        );
        assert_eq!(
            repo.source_freeze("freeze-a").expect("freeze").status,
            SourceFreezeStatus::Active
        );
        assert_eq!(count(&conn, "feature_run_evidence_invalidations"), 0);
    }
}
