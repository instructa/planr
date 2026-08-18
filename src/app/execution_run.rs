use super::App;
use super::feature_run_evidence::{BudgetUsageReport, FeatureRunBudgetAdmission};
use super::repository::execution_run::{
    EvidenceInvalidationRecord, ExecutionRunRepository, FindingRecord, FindingStatus,
    PersistedFeatureRun, ReviewAttemptRecord, ReviewGateKind, ReviewGateRecord, ReviewGateStatus,
    ReviewScopeKind, ReviewVerdict, RunOutcomeRecord, SourceFreezeRecord, SourceFreezeStatus,
};
use crate::canonical_json::sha256_json_digest;
use crate::cli::{RunBatchCommand, RunCommand};
use crate::evidence::policy::capture_repository_snapshot;
use crate::execution_run::{
    DEFAULT_BATCH_OUTCOME_CAP, ExecutionBatch, ExecutionBatchStatus, FeatureRun, FeatureRunPhase,
    FeatureRunRestartDisposition, FeatureRunRestartReason, FeatureRunRestartRequest,
    FeatureRunStatus, MakerReplacement, MakerReplacementReason, PhaseTransition,
    PhaseTransitionCause, PrematureSourceFreezeRestartFacts, RoleOwner, RunRole,
    VerificationAdmissionRepairReason, VerificationAdmissionRepairRequest, apply_phase_transition,
    is_ordinary_implementation_work_type, pause_batch_for_risk_review, replace_batch_maker,
    resume_batch_after_risk_review, retire_incompatible_feature_run,
    retire_premature_source_freeze_feature_run, roll_batch_for_same_maker,
};
use crate::model::ItemStatus;
use crate::usage_policy::{
    BudgetPhase, FeatureRunBudgetContract, PolicyLoad, ReviewEscalation, ReviewInterruptDecision,
    ReviewInterruptRequest, admit_review_interrupt, feature_run_budget_contract_from_policy,
    load_policy, preview_policy_upgrade,
};
use crate::util::{short_id, worker_id};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use time::OffsetDateTime;
#[derive(Clone, Debug)]
pub(crate) struct OutcomeSettlement<'a> {
    pub(crate) item_id: &'a str,
    pub(crate) summary: &'a str,
    pub(crate) materiality: &'a Value,
    pub(crate) escalation: Option<ReviewEscalation>,
}

pub(crate) struct ExistingOutcomeSettlement<'a> {
    pub(crate) item_id: &'a str,
    pub(crate) summary: &'a str,
    pub(crate) claimed_files: &'a [String],
    pub(crate) escalation: Option<ReviewEscalation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutcomeSettlementDisposition {
    FreshlyRecorded,
    AlreadySettled,
}

#[derive(Clone, Debug)]
pub(crate) struct OutcomeSettlementTransition {
    pub(crate) disposition: OutcomeSettlementDisposition,
    pub(crate) materiality: Value,
    work_packet: Value,
}

impl OutcomeSettlementTransition {
    fn freshly_recorded(work_packet: Value, materiality: &Value) -> Self {
        Self {
            disposition: OutcomeSettlementDisposition::FreshlyRecorded,
            materiality: materiality.clone(),
            work_packet,
        }
    }

    fn already_settled(work_packet: Value, materiality: Value) -> Self {
        Self {
            disposition: OutcomeSettlementDisposition::AlreadySettled,
            materiality,
            work_packet,
        }
    }

    pub(crate) fn into_work_packet(self) -> Value {
        self.work_packet
    }
}

impl std::ops::Deref for OutcomeSettlementTransition {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        &self.work_packet
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AlreadySettledOutcomeViolation {
    MissingOutcome,
    ItemNotTerminal,
    PhaseMismatch,
    RunStatusMismatch,
    MakerCardinality,
    MakerMismatch,
    ActiveBatchMissing,
    OutcomeIdMismatch,
    OutcomeRunMismatch,
    OutcomeBatchMismatch,
    OutcomeItemMismatch,
    OutcomeOrdinalMismatch,
    OutcomePayloadShapeMismatch,
    OutcomeSummaryMismatch,
    OutcomeEscalationMismatch,
    OutcomeClaimedFilesMismatch,
    ExistingOutcomeRequiresRetryAdmission,
    BatchRunMismatch,
    BatchStatusMismatch,
    BatchMakerMismatch,
    BatchCounterMismatch,
    BatchMembershipMismatch,
    RunCounterMismatch,
}

fn already_settled_outcome_error(
    run_id: &str,
    item_id: &str,
    violation: AlreadySettledOutcomeViolation,
) -> anyhow::Error {
    anyhow!("already_settled_outcome_rejected:{run_id}:{item_id}:{violation:?}")
}

fn persisted_outcome_materiality(
    outcome: &Value,
    summary: &str,
    escalation: &Option<ReviewEscalation>,
    claimed_files: &[String],
) -> std::result::Result<Value, AlreadySettledOutcomeViolation> {
    let payload = outcome
        .as_object()
        .ok_or(AlreadySettledOutcomeViolation::OutcomePayloadShapeMismatch)?;
    if payload.len() != 3
        || !payload.contains_key("summary")
        || !payload.contains_key("materiality")
        || !payload.contains_key("escalation")
    {
        return Err(AlreadySettledOutcomeViolation::OutcomePayloadShapeMismatch);
    }
    if payload.get("summary").and_then(Value::as_str) != Some(summary) {
        return Err(AlreadySettledOutcomeViolation::OutcomeSummaryMismatch);
    }
    let expected_escalation = json!(escalation);
    if payload.get("escalation") != Some(&expected_escalation) {
        return Err(AlreadySettledOutcomeViolation::OutcomeEscalationMismatch);
    }
    let materiality = payload
        .get("materiality")
        .filter(|value| value.is_object())
        .ok_or(AlreadySettledOutcomeViolation::OutcomePayloadShapeMismatch)?;
    let persisted_files = materiality
        .pointer("/change_summary/files")
        .and_then(Value::as_array)
        .ok_or(AlreadySettledOutcomeViolation::OutcomePayloadShapeMismatch)?;
    if !persisted_files.iter().all(Value::is_string) {
        return Err(AlreadySettledOutcomeViolation::OutcomePayloadShapeMismatch);
    }
    if persisted_files.len() != claimed_files.len()
        || persisted_files
            .iter()
            .zip(claimed_files)
            .any(|(persisted, claimed)| persisted.as_str() != Some(claimed.as_str()))
    {
        return Err(AlreadySettledOutcomeViolation::OutcomeClaimedFilesMismatch);
    }
    Ok(materiality.clone())
}

fn admitted_outcome_escalation(
    escalation: Option<ReviewEscalation>,
) -> Result<Option<ReviewEscalation>> {
    escalation
        .map(|escalation| {
            match admit_review_interrupt(&ReviewInterruptRequest::StructuredEscalation {
                escalation: escalation.clone(),
            }) {
                ReviewInterruptDecision::OpenCheckpoint { escalation } => Ok(escalation),
                decision => bail!("review_escalation_rejected:{decision:?}"),
            }
        })
        .transpose()
}
impl App {
    pub(crate) fn execution_run(&self, command: RunCommand) -> Result<()> {
        match command {
            RunCommand::Batch(args) => match args.command {
                RunBatchCommand::Roll(args) => {
                    let value = self.roll_feature_run_batch_value(&args.plan, &worker_id())?;
                    self.emit(value, "feature run batch rolled for same maker".to_string())
                }
                RunBatchCommand::Replace(args) => {
                    let reason = match args.reason {
                        crate::cli::MakerReplacementReasonArg::Unavailable => {
                            MakerReplacementReason::Unavailable
                        }
                        crate::cli::MakerReplacementReasonArg::ContextLost => {
                            MakerReplacementReason::ContextLost
                        }
                        crate::cli::MakerReplacementReasonArg::OwnershipIncompatible => {
                            MakerReplacementReason::OwnershipIncompatible
                        }
                        crate::cli::MakerReplacementReasonArg::BatchCapReached => {
                            MakerReplacementReason::BatchCapReached
                        }
                    };
                    let value = self.replace_feature_run_maker_value(
                        &args.plan,
                        &args.successor_maker,
                        reason,
                        &args.reference,
                        &args.explanation,
                    )?;
                    self.emit(value, "feature run maker replaced".to_string())
                }
            },
            RunCommand::Restart(args) => {
                let reason = match args.reason {
                    crate::cli::FeatureRunRestartReasonArg::IncompatibleBudget => {
                        FeatureRunRestartReason::IncompatibleBudget
                    }
                    crate::cli::FeatureRunRestartReasonArg::PrematureSourceFreeze => {
                        FeatureRunRestartReason::PrematureSourceFreeze
                    }
                    crate::cli::FeatureRunRestartReasonArg::InconsistentVerification => {
                        FeatureRunRestartReason::InconsistentVerification
                    }
                };
                let value = self.restart_feature_run_value(&args.plan, reason)?;
                self.emit(value, "feature run retired for typed restart".to_string())
            }
            RunCommand::ResolveBudgetHold(args) => {
                let value = self.resolve_feature_run_budget_hold_value(&args.plan)?;
                self.emit(value, "feature run budget hold resolved".to_string())
            }
            RunCommand::RepairVerificationAdmission(args) => {
                let reason = match args.reason {
                    crate::cli::VerificationAdmissionRepairReasonArg::ReadinessBlocked => {
                        VerificationAdmissionRepairReason::ReadinessBlocked
                    }
                    crate::cli::VerificationAdmissionRepairReasonArg::RunIndexSealFailed => {
                        VerificationAdmissionRepairReason::RunIndexSealFailed
                    }
                    crate::cli::VerificationAdmissionRepairReasonArg::SealedRunRejected => {
                        VerificationAdmissionRepairReason::SealedRunRejected
                    }
                    crate::cli::VerificationAdmissionRepairReasonArg::CapabilityAdmissionFailed => {
                        VerificationAdmissionRepairReason::CapabilityAdmissionFailed
                    }
                };
                let value =
                    self.repair_verification_admission_value(VerificationAdmissionRepairRequest {
                        plan_id: args.plan,
                        run_id: args.run,
                        freeze_id: args.freeze,
                        run_revision: args.revision,
                        reason,
                        run_index_digest: args.run_index_digest,
                    })?;
                self.emit(value, "verification admission repaired".to_string())
            }
            RunCommand::SettleRepair(args) => {
                let value = self.settle_repair_value(
                    &args.plan,
                    &args.invalidation,
                    &args.summary,
                    &args.files,
                    &args.cmd,
                    &args.tests,
                )?;
                self.emit(value, "repair settled and source refrozen".to_string())
            }
        }
    }
    pub(crate) fn review_gate_projection_value(&self, gate: &ReviewGateRecord) -> Result<Value> {
        let mut state = self.canonical_execution_state_value(&gate.run_id, Some(&gate.id))?;
        let latest = state["review_attempts"]
            .as_array()
            .and_then(|values| values.last());
        let independent = latest.is_some_and(|attempt| {
            attempt["reviewer_mode"] == "independent"
                && attempt["reviewer_worker_id"] != gate.responsible_maker_id
        });
        let accepted = gate.status == ReviewGateStatus::Accepted
            && latest.is_some_and(|attempt| attempt["verdict"] == "accepted")
            && independent;
        state["latest_attempt"] = latest.cloned().unwrap_or(Value::Null);
        state["independent"] = json!(independent);
        state["accepted"] = json!(accepted);
        Ok(state)
    }
    pub(crate) fn final_product_review_projection_value(&self, plan_id: &str) -> Result<Value> {
        let plan = self.get_plan(plan_id)?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let mut entries = Vec::new();
        if let Some(run_id) = self.canonical_execution_run_id_for_plan(plan_id)? {
            for gate in repository.review_gates_for_run(&run_id, false)? {
                if gate.kind != ReviewGateKind::FinalProduct
                    || gate.scope_kind != ReviewScopeKind::Plan
                    || gate.scope_id != plan_id
                {
                    continue;
                }
                let mut entry = self.review_gate_projection_value(&gate)?;
                entry["accepted"] = json!(
                    entry["accepted"] == true
                        && entry["feature_run"]["status"] == "complete"
                        && entry["feature_run"]["phase"] == "complete"
                );
                entries.push(entry);
            }
        }
        Ok(json!({
            "plan": plan,
            "review_gates": entries,
            "current": entries.first().cloned(),
        }))
    }
    pub(crate) fn final_product_review_clause_value(&self, plan_id: &str) -> Result<Value> {
        let projection = self.final_product_review_projection_value(plan_id)?;
        let entries = projection["review_gates"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let pass = entries.len() == 1 && entries[0]["accepted"] == true;
        Ok(json!({
            "clause": "final_product_review_complete",
            "pass": pass,
            "required": true,
            "detail": "exactly one plan-scoped durable final ReviewGate must be accepted by an independent reviewer against the frozen source",
            "open": if pass { json!([]) } else { json!(entries.clone()) },
            "review_gates": entries,
            "next": if projection["review_gates"].as_array().is_none_or(Vec::is_empty) {
                json!(format!("planr plan final-review {plan_id}"))
            } else {
                json!(format!("planr pick --plan {plan_id} --work-type review --json"))
            },
        }))
    }
    fn feature_run_policy_snapshot(
        &self,
        run_id: &str,
        started_at_unix_ms: u64,
    ) -> Result<(String, FeatureRunBudgetContract)> {
        if let Some(upgrade) = preview_policy_upgrade(&self.root)? {
            bail!(
                "policy_upgrade_required:{} -> {}; run `planr policy upgrade` before FeatureRun start",
                upgrade.from_shape,
                upgrade.to_shape
            );
        }
        let (value, contract) = match load_policy(&self.root) {
            PolicyLoad::Loaded(policy) => {
                let contract = feature_run_budget_contract_from_policy(
                    run_id,
                    started_at_unix_ms,
                    Some(&policy),
                )
                .map_err(|diagnostics| {
                    anyhow!("feature_run_budget_contract_invalid:{diagnostics}")
                })?;
                (serde_json::to_value(&*policy)?, contract)
            }
            PolicyLoad::Missing => (
                json!({"policy": "missing"}),
                feature_run_budget_contract_from_policy(run_id, started_at_unix_ms, None).map_err(
                    |diagnostics| anyhow!("feature_run_budget_contract_invalid:{diagnostics}"),
                )?,
            ),
            PolicyLoad::Invalid(diagnostics) => {
                bail!("feature_run_policy_invalid:{diagnostics}")
            }
        };
        Ok((sha256_json_digest(&value)?, contract))
    }
    pub(crate) fn ensure_outcome_feature_run(
        &self,
        item_id: &str,
    ) -> Result<Option<PersistedFeatureRun>> {
        let item = self.get_item(item_id)?;
        if !is_ordinary_implementation_work_type(&item.work_type) {
            return Ok(None);
        }
        if let Some(hold) = self.binding_evidence_hold_for_item(item_id)? {
            bail!(
                "binding_evidence_obligations_missing:{item_id}; next action: {}",
                hold["next_action"]
                    .as_str()
                    .unwrap_or("planr evidence migrate --input <migration-file> --apply")
            );
        }
        let Some(plan_path) = item.plan_path.as_deref() else {
            return Ok(None);
        };
        let Some(plan_id) = self.plan_id_for_path(plan_path)? else {
            return Ok(None);
        };
        let repository = ExecutionRunRepository::new(&self.conn);
        if let Some(run) = repository.active_feature_run_for_plan(&item.project_id, &plan_id)? {
            return Ok(Some(run));
        }
        let maker = item.worker_id.clone().unwrap_or_else(worker_id);
        let run_id = short_id("frun");
        let batch_id = short_id("batch");
        let started_at_unix_ms =
            u64::try_from(OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000)
                .map_err(|_| anyhow!("feature_run_clock_before_unix_epoch"))?;
        let (policy_digest, budget_contract) =
            self.feature_run_policy_snapshot(&run_id, started_at_unix_ms)?;
        let run = FeatureRun {
            id: run_id.clone(),
            plan_id: plan_id.clone(),
            status: FeatureRunStatus::Active,
            phase: FeatureRunPhase::Implementation,
            policy_digest,
            source_revision: None,
            active_batch_id: Some(batch_id.clone()),
            role_owners: vec![RoleOwner {
                role: RunRole::Maker,
                worker_id: maker.clone(),
                lease_generation: 1,
            }],
            outcomes_settled: 0,
            batch_outcome_count: 0,
            held_from_phase: None,
            hold_reason: None,
            terminal_reason: None,
        };
        let batch = ExecutionBatch {
            id: batch_id,
            run_id: run_id.clone(),
            maker_worker_id: maker.clone(),
            status: ExecutionBatchStatus::Active,
            settled_outcome_ids: Vec::new(),
            replacement: None,
        };
        let persisted = repository.create_feature_run(
            &item.project_id,
            &run,
            &budget_contract,
            Some(&batch),
        )?;
        self.record_event(
            "feature_run_started",
            Some(item_id),
            json!({"run_id": run_id, "plan_id": plan_id, "maker_worker_id": maker, "batch_id": batch.id}),
        )?;
        Ok(Some(persisted))
    }
    pub(crate) fn restart_feature_run_value(
        &self,
        plan_id: &str,
        reason: FeatureRunRestartReason,
    ) -> Result<Value> {
        if reason == FeatureRunRestartReason::InconsistentVerification {
            return self.restart_inconsistent_verification_feature_run_value(plan_id);
        }
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let Some(persisted) = repository.active_feature_run_for_plan(&project.id, plan_id)? else {
            return match reason {
                FeatureRunRestartReason::IncompatibleBudget => {
                    let mut previous = repository
                        .latest_incompatible_feature_run_restart(&project.id, plan_id)?
                        .ok_or_else(|| anyhow!("feature_run_not_found_for_plan:{plan_id}"))?;
                    previous.disposition = FeatureRunRestartDisposition::AlreadyRetired;
                    let execution_state =
                        self.canonical_execution_state_value(&previous.retired_run.id, None)?;
                    Ok(json!({"schema_version": "planr.feature_run_restart.v1",
                        "restart": previous, "execution_state": execution_state}))
                }
                FeatureRunRestartReason::PrematureSourceFreeze => {
                    let mut previous = repository
                        .latest_premature_source_freeze_feature_run_restart(&project.id, plan_id)?
                        .ok_or_else(|| anyhow!("feature_run_not_found_for_plan:{plan_id}"))?;
                    previous.disposition = FeatureRunRestartDisposition::AlreadyRetired;
                    let execution_state =
                        self.canonical_execution_state_value(&previous.retired_run.id, None)?;
                    Ok(json!({"schema_version": "planr.feature_run_restart.v1",
                        "restart": previous, "execution_state": execution_state}))
                }
                FeatureRunRestartReason::InconsistentVerification => {
                    unreachable!("dispatched above")
                }
            };
        };
        let request = FeatureRunRestartRequest {
            plan_id: plan_id.to_string(),
            reason,
        };
        let transition = match reason {
            FeatureRunRestartReason::IncompatibleBudget => {
                let compatibility = repository.budget_contract_compatibility(&persisted.run.id)?;
                let transition =
                    retire_incompatible_feature_run(&persisted.run, &request, compatibility)
                        .map_err(|violation| {
                            anyhow!("feature_run_restart_rejected:{violation:?}")
                        })?;
                repository.retire_incompatible_feature_run(
                    &transition,
                    persisted.revision,
                    &worker_id(),
                )?;
                serde_json::to_value(transition)?
            }
            FeatureRunRestartReason::PrematureSourceFreeze => {
                let facts = self
                    .premature_source_freeze_restart_facts(&persisted)?
                    .ok_or_else(|| {
                        anyhow!(
                            "feature_run_premature_source_freeze_restart_not_required:{plan_id}"
                        )
                    })?;
                let transition =
                    retire_premature_source_freeze_feature_run(&persisted.run, &request, &facts)
                        .map_err(|violation| {
                            anyhow!("feature_run_restart_rejected:{violation:?}")
                        })?;
                repository.retire_premature_source_freeze_feature_run(
                    &transition,
                    persisted.revision,
                    &worker_id(),
                )?;
                serde_json::to_value(transition)?
            }
            FeatureRunRestartReason::InconsistentVerification => unreachable!("dispatched above"),
        };
        let execution_state = self.canonical_execution_state_value(&persisted.run.id, None)?;
        Ok(json!({
            "schema_version": "planr.feature_run_restart.v1",
            "restart": transition,
            "execution_state": execution_state,
        }))
    }

    pub(crate) fn premature_source_freeze_restart_facts(
        &self,
        persisted: &PersistedFeatureRun,
    ) -> Result<Option<PrematureSourceFreezeRestartFacts>> {
        if persisted.run.status != FeatureRunStatus::Active
            || persisted.run.phase != FeatureRunPhase::SourceFrozen
        {
            return Ok(None);
        }
        let repository = ExecutionRunRepository::new(&self.conn);
        let Some(freeze) = repository.active_source_freeze(&persisted.run.id)? else {
            return Ok(None);
        };
        let open_outcome_ids = repository.open_ordinary_outcome_ids(&persisted.run.plan_id)?;
        if open_outcome_ids.is_empty() {
            return Ok(None);
        }
        let released_outcome_ids =
            repository.active_ordinary_outcome_lease_ids(&persisted.run.plan_id)?;
        let (verification_admission_count, verification_attempt_count, verification_receipt_count) =
            repository.source_freeze_verification_activity(
                &persisted.project_id,
                &persisted.run.plan_id,
                &persisted.run.id,
                &freeze,
            )?;
        if verification_admission_count != 0
            || verification_attempt_count != 0
            || verification_receipt_count != 0
        {
            return Ok(None);
        }
        Ok(Some(PrematureSourceFreezeRestartFacts {
            freeze_id: freeze.id,
            frozen_source_revision: freeze.source_revision,
            frozen_source_digest: freeze.source_digest,
            open_outcome_ids,
            released_outcome_ids,
            verification_admission_count,
            verification_attempt_count,
            verification_receipt_count,
        }))
    }

    pub(crate) fn premature_source_freeze_restart_hold_for_run(
        &self,
        persisted: &PersistedFeatureRun,
    ) -> Result<Option<Value>> {
        let Some(facts) = self.premature_source_freeze_restart_facts(persisted)? else {
            return Ok(None);
        };
        let command = format!(
            "planr --json run restart --plan {} --reason premature-source-freeze",
            persisted.run.plan_id
        );
        Ok(Some(json!({
            "item": null,
            "reason": "source_freeze_premature",
            "repair": [command],
            "work_packet": {
                "kind": "hold",
                "classification": "source_freeze_premature",
                "reason_code": "source_freeze_premature",
                "next_action": command,
                "restart_facts": facts,
                "execution_state": self.canonical_execution_state_value(&persisted.run.id, None)?,
            },
            "remaining": self.progress_value()?,
        })))
    }
    pub(crate) fn outcome_work_packet(&self, item_id: &str) -> Result<Value> {
        let run = self.ensure_outcome_feature_run(item_id)?;
        let Some(run) = run else {
            return Ok(json!({"kind": "outcome", "item_id": item_id}));
        };
        if let Some(hold) = self.incompatible_feature_run_hold_value(item_id, &run)? {
            return Ok(hold);
        }
        let boundary_key = format!("implementation:{item_id}");
        let reservation = match self.admit_feature_run_budget(
            &run,
            BudgetPhase::Implementation,
            &boundary_key,
            &worker_id(),
            "implementation.dispatch",
        )? {
            FeatureRunBudgetAdmission::Held(hold) => return Ok(hold["work_packet"].clone()),
            FeatureRunBudgetAdmission::Reserved(reservation) => reservation,
        };
        let batch = run
            .run
            .active_batch_id
            .as_deref()
            .map(|id| ExecutionRunRepository::new(&self.conn).batch(id))
            .transpose();
        let _batch = match batch {
            Ok(batch) => batch,
            Err(error) => {
                self.release_feature_run_budget(&reservation)?;
                return Err(error);
            }
        };
        Ok(json!({
            "kind": "outcome",
            "item_id": item_id,
            "execution_state": self.canonical_execution_state_value(&run.run.id, None)?,
        }))
    }

    pub(crate) fn incompatible_feature_run_hold_value(
        &self,
        item_id: &str,
        run: &PersistedFeatureRun,
    ) -> Result<Option<Value>> {
        let compatibility =
            ExecutionRunRepository::new(&self.conn).budget_contract_compatibility(&run.run.id)?;
        if !compatibility.is_incompatible() {
            return Ok(None);
        }
        let execution_state = self.canonical_execution_state_value(&run.run.id, None)?;
        Ok(Some(json!({
            "kind": "hold",
            "item_id": item_id,
            "classification": "incompatible_feature_run_budget_contract",
            "reason_code": execution_state["reason_code"],
            "next_action": execution_state["next_action"],
            "execution_state": execution_state,
        })))
    }

    pub(crate) fn settle_existing_feature_run_outcome(
        &self,
        input: ExistingOutcomeSettlement<'_>,
    ) -> Result<OutcomeSettlementTransition> {
        let Some(persisted) = self.ensure_outcome_feature_run(input.item_id)? else {
            return Err(already_settled_outcome_error(
                "unplanned",
                input.item_id,
                AlreadySettledOutcomeViolation::MissingOutcome,
            ));
        };
        if self
            .incompatible_feature_run_hold_value(input.item_id, &persisted)?
            .is_some()
        {
            bail!(
                "feature_run_budget_contract_incompatible:{}",
                persisted.run.id
            );
        }
        let repository = ExecutionRunRepository::new(&self.conn);
        let existing_outcome = repository
            .outcome_for_run_item(&persisted.run.id, input.item_id)?
            .ok_or_else(|| {
                already_settled_outcome_error(
                    &persisted.run.id,
                    input.item_id,
                    AlreadySettledOutcomeViolation::MissingOutcome,
                )
            })?;
        let item = self.get_item(input.item_id)?;
        let reject =
            |violation| already_settled_outcome_error(&persisted.run.id, input.item_id, violation);
        if persisted.run.phase != FeatureRunPhase::Implementation {
            return Err(reject(AlreadySettledOutcomeViolation::PhaseMismatch));
        }
        if persisted.run.status != FeatureRunStatus::Active {
            return Err(reject(AlreadySettledOutcomeViolation::RunStatusMismatch));
        }
        if !matches!(item.status, ItemStatus::Closed | ItemStatus::ClosedPartial) {
            return Err(reject(AlreadySettledOutcomeViolation::ItemNotTerminal));
        }
        let makers = persisted
            .run
            .role_owners
            .iter()
            .filter(|owner| owner.role == RunRole::Maker)
            .collect::<Vec<_>>();
        if makers.len() != 1 {
            return Err(reject(AlreadySettledOutcomeViolation::MakerCardinality));
        }
        let maker = makers[0];
        if maker.worker_id != worker_id() {
            return Err(reject(AlreadySettledOutcomeViolation::MakerMismatch));
        }
        let batch_id = persisted
            .run
            .active_batch_id
            .as_deref()
            .ok_or_else(|| reject(AlreadySettledOutcomeViolation::ActiveBatchMissing))?;
        let batch = repository
            .batch(batch_id)
            .map_err(|_| reject(AlreadySettledOutcomeViolation::ActiveBatchMissing))?;
        let escalation = admitted_outcome_escalation(input.escalation)?;
        if existing_outcome.id != format!("outcome-{}", input.item_id) {
            return Err(reject(AlreadySettledOutcomeViolation::OutcomeIdMismatch));
        }
        if existing_outcome.run_id != persisted.run.id {
            return Err(reject(AlreadySettledOutcomeViolation::OutcomeRunMismatch));
        }
        if existing_outcome.batch_id != batch_id {
            return Err(reject(AlreadySettledOutcomeViolation::OutcomeBatchMismatch));
        }
        if existing_outcome.item_id != input.item_id {
            return Err(reject(AlreadySettledOutcomeViolation::OutcomeItemMismatch));
        }
        if existing_outcome.ordinal == 0
            || existing_outcome.ordinal != persisted.run.batch_outcome_count
        {
            return Err(reject(
                AlreadySettledOutcomeViolation::OutcomeOrdinalMismatch,
            ));
        }
        let materiality = persisted_outcome_materiality(
            &existing_outcome.outcome,
            input.summary,
            &escalation,
            input.claimed_files,
        )
        .map_err(reject)?;
        if batch.batch.run_id != persisted.run.id {
            return Err(reject(AlreadySettledOutcomeViolation::BatchRunMismatch));
        }
        if batch.batch.status != ExecutionBatchStatus::Active {
            return Err(reject(AlreadySettledOutcomeViolation::BatchStatusMismatch));
        }
        if batch.batch.maker_worker_id != maker.worker_id {
            return Err(reject(AlreadySettledOutcomeViolation::BatchMakerMismatch));
        }
        if batch.batch.settled_count() != persisted.run.batch_outcome_count {
            return Err(reject(AlreadySettledOutcomeViolation::BatchCounterMismatch));
        }
        let batch_index = usize::try_from(existing_outcome.ordinal - 1)
            .map_err(|_| reject(AlreadySettledOutcomeViolation::OutcomeOrdinalMismatch))?;
        if batch.batch.settled_outcome_ids.get(batch_index) != Some(&existing_outcome.id) {
            return Err(reject(
                AlreadySettledOutcomeViolation::BatchMembershipMismatch,
            ));
        }
        let run_outcomes = repository.outcomes(&persisted.run.id)?;
        if u32::try_from(run_outcomes.len()).ok() != Some(persisted.run.outcomes_settled)
            || run_outcomes
                .iter()
                .filter(|outcome| outcome.item_id == input.item_id)
                .count()
                != 1
        {
            return Err(reject(AlreadySettledOutcomeViolation::RunCounterMismatch));
        }
        let execution_state = self.canonical_execution_state_value(&persisted.run.id, None)?;
        let work_packet = json!({
            "kind": "outcome",
            "run_id": persisted.run.id,
            "batch_id": batch_id,
            "run_revision": persisted.revision,
            "batch_outcome_count": persisted.run.batch_outcome_count,
            "transition": "already_settled",
            "disposition": "already_settled",
            "materiality": &materiality,
            "review_gate": null,
            "execution_state": execution_state,
        });
        Ok(OutcomeSettlementTransition::already_settled(
            work_packet,
            materiality,
        ))
    }

    pub(crate) fn settle_feature_run_outcome(
        &self,
        input: OutcomeSettlement<'_>,
    ) -> Result<OutcomeSettlementTransition> {
        let Some(persisted) = self.ensure_outcome_feature_run(input.item_id)? else {
            return Ok(OutcomeSettlementTransition::freshly_recorded(
                json!({"kind": "outcome", "transition": "legacy_unplanned"}),
                input.materiality,
            ));
        };
        if self
            .incompatible_feature_run_hold_value(input.item_id, &persisted)?
            .is_some()
        {
            bail!(
                "feature_run_budget_contract_incompatible:{}",
                persisted.run.id
            );
        }
        let repository = ExecutionRunRepository::new(&self.conn);
        let item = self.get_item(input.item_id)?;
        if repository
            .outcome_for_run_item(&persisted.run.id, input.item_id)?
            .is_some()
        {
            return Err(already_settled_outcome_error(
                &persisted.run.id,
                input.item_id,
                AlreadySettledOutcomeViolation::ExistingOutcomeRequiresRetryAdmission,
            ));
        }
        if matches!(item.status, ItemStatus::Closed | ItemStatus::ClosedPartial) {
            return Err(already_settled_outcome_error(
                &persisted.run.id,
                input.item_id,
                AlreadySettledOutcomeViolation::MissingOutcome,
            ));
        }
        if persisted.run.phase != FeatureRunPhase::Implementation {
            bail!("feature_run_not_accepting_outcomes:{}", persisted.run.id);
        }
        if persisted.run.batch_outcome_count >= DEFAULT_BATCH_OUTCOME_CAP {
            bail!("feature_run_batch_cap_reached:{}", persisted.run.id);
        }
        let maker = persisted
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Maker)
            .ok_or_else(|| anyhow!("feature_run_missing_maker:{}", persisted.run.id))?;
        if maker.worker_id != worker_id() {
            bail!("feature_run_maker_mismatch:{}", persisted.run.id);
        }
        let review_required =
            input.materiality["decision"]["review"].as_str() == Some("independent_high_signal");
        let escalation = admitted_outcome_escalation(input.escalation)?;
        let checkpoint_required = review_required || escalation.is_some();
        let batch_id = persisted
            .run
            .active_batch_id
            .clone()
            .ok_or_else(|| anyhow!("feature_run_missing_active_batch:{}", persisted.run.id))?;
        let batch = repository.batch(&batch_id)?;
        if batch.batch.maker_worker_id != maker.worker_id {
            bail!("feature_run_batch_maker_mismatch:{}", persisted.run.id);
        }
        let ordinal = persisted.run.batch_outcome_count + 1;
        repository.record_outcome(
            &RunOutcomeRecord {
                id: format!("outcome-{}", input.item_id),
                run_id: persisted.run.id.clone(),
                batch_id: batch_id.clone(),
                item_id: input.item_id.to_string(),
                ordinal,
                outcome: json!({
                    "summary": input.summary,
                    "materiality": input.materiality,
                    "escalation": escalation,
                }),
            },
            persisted.revision,
        )?;
        let mut transition = "continue_batch";
        let mut gate = None;
        if checkpoint_required {
            let paused = pause_batch_for_risk_review(&batch.batch)
                .map_err(|violation| anyhow!("batch_pause_rejected:{violation:?}"))?;
            repository.save_batch(&paused, batch.revision)?;
            gate = repository.review_gate_for_scope(
                &persisted.run.id,
                ReviewGateKind::RiskCheckpoint,
                ReviewScopeKind::Outcome,
                input.item_id,
            )?;
            if gate.is_none() {
                let record = ReviewGateRecord {
                    id: short_id("gate"),
                    run_id: persisted.run.id.clone(),
                    scope_kind: ReviewScopeKind::Outcome,
                    scope_id: input.item_id.to_string(),
                    kind: ReviewGateKind::RiskCheckpoint,
                    status: ReviewGateStatus::Pending,
                    required_risk: escalation
                        .as_ref()
                        .map(|value| format!("{:?}", value.reason).to_ascii_lowercase())
                        .or_else(|| Some("protected_risk".to_string())),
                    responsible_maker_id: maker.worker_id.clone(),
                    latest_attempt: 0,
                    source_revision: None,
                };
                repository.create_review_gate(&record)?;
                self.record_event(
                    "review_gate_opened",
                    Some(input.item_id),
                    json!({"gate_id": &record.id, "run_id": &record.run_id, "kind": "risk_checkpoint"}),
                )?;
                gate = Some(record);
            }
            transition = "review_gate";
            self.record_event(
                "maker_paused_for_review",
                Some(input.item_id),
                json!({"run_id": persisted.run.id, "batch_id": batch_id, "gate_id": gate.as_ref().map(|value| &value.id), "batch_outcome_count": ordinal}),
            )?;
        } else if ordinal >= DEFAULT_BATCH_OUTCOME_CAP {
            transition = "batch_cap_reached";
        }
        let (transition, settled_run_revision) = self
            .complete_verified_continuation_after_outcome(
                &persisted,
                &batch_id,
                input.item_id,
                transition,
                checkpoint_required,
            )?;
        let implementation_key = format!("implementation:{}", input.item_id);
        if let Some(reservation) = self
            .load_active_budget_reservation(&persisted.run.id, &implementation_key)?
            .or(self.load_active_budget_reservation(
                &persisted.run.id,
                &format!("repair:{}", persisted.run.id),
            )?)
        {
            self.reconcile_feature_run_budget(
                &reservation,
                &BudgetUsageReport::application(Some(1)),
            )?;
        }
        let execution_state = self.canonical_execution_state_value(
            &persisted.run.id,
            gate.as_ref().map(|gate| gate.id.as_str()),
        )?;
        Ok(OutcomeSettlementTransition::freshly_recorded(
            json!({
                "kind": "outcome",
                "run_id": persisted.run.id,
                "batch_id": batch_id,
                "run_revision": settled_run_revision,
                "batch_outcome_count": ordinal,
                "transition": transition,
                "review_gate": gate,
                "execution_state": execution_state,
            }),
            input.materiality,
        ))
    }

    pub(crate) fn replace_feature_run_maker_value(
        &self,
        plan_id: &str,
        successor_maker_id: &str,
        reason: MakerReplacementReason,
        reference: &str,
        explanation: &str,
    ) -> Result<Value> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository
            .active_feature_run_for_plan(&project.id, plan_id)?
            .ok_or_else(|| anyhow!("feature_run_not_found_for_plan:{plan_id}"))?;
        let current_maker = persisted
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Maker)
            .ok_or_else(|| anyhow!("feature_run_missing_maker:{}", persisted.run.id))?;
        let batch_id = persisted
            .run
            .active_batch_id
            .as_deref()
            .ok_or_else(|| anyhow!("feature_run_missing_active_batch:{}", persisted.run.id))?;
        let batch = repository.batch(batch_id)?;
        let ended = replace_batch_maker(
            &batch.batch,
            Some(MakerReplacement {
                replaced_maker_worker_id: current_maker.worker_id.clone(),
                successor_maker_worker_id: successor_maker_id.to_string(),
                reason,
                reference: reference.to_string(),
                explanation: explanation.to_string(),
            }),
            DEFAULT_BATCH_OUTCOME_CAP,
        )
        .map_err(|violation| anyhow!("maker_replacement_rejected:{violation:?}"))?;
        let next_batch = ExecutionBatch {
            id: short_id("batch"),
            run_id: persisted.run.id.clone(),
            maker_worker_id: successor_maker_id.to_string(),
            status: ExecutionBatchStatus::Active,
            settled_outcome_ids: Vec::new(),
            replacement: None,
        };
        let mut next_run = persisted.run.clone();
        next_run.active_batch_id = Some(next_batch.id.clone());
        next_run.batch_outcome_count = 0;
        next_run.role_owners = vec![RoleOwner {
            role: RunRole::Maker,
            worker_id: successor_maker_id.to_string(),
            lease_generation: self.conn.query_row(
                "SELECT COALESCE(MAX(lease_generation), 0) + 1 FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'maker'",
                [&persisted.run.id],
                |row| row.get::<_, u64>(0),
            )?,
        }];
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT replace_feature_run_maker")?;
        let result = (|| -> Result<Value> {
            repository.save_batch(&ended, batch.revision)?;
            repository.save_feature_run_with_new_batch(
                &next_run,
                persisted.revision,
                &next_batch,
            )?;
            Ok(json!({
                "feature_run": next_run,
                "ended_batch": ended,
                "execution_batch": next_batch,
                "reason": "maker_replaced",
            }))
        })();
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE replace_feature_run_maker; COMMIT")?;
                self.record_event(
                    "maker_replaced",
                    None,
                    json!({"run_id": persisted.run.id, "successor_maker_id": successor_maker_id}),
                )?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO replace_feature_run_maker; RELEASE replace_feature_run_maker; ROLLBACK",
                );
                Err(error)
            }
        }
    }

    pub(crate) fn roll_feature_run_batch_value(
        &self,
        plan_id: &str,
        maker_worker_id: &str,
    ) -> Result<Value> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository
            .active_feature_run_for_plan(&project.id, plan_id)?
            .ok_or_else(|| anyhow!("feature_run_not_found_for_plan:{plan_id}"))?;
        if persisted.run.status != FeatureRunStatus::Active
            || persisted.run.phase != FeatureRunPhase::Implementation
        {
            bail!("same_maker_roll_wrong_phase:{}", persisted.run.id);
        }
        if repository
            .review_gates_for_run(&persisted.run.id, true)?
            .iter()
            .any(|gate| gate.kind == ReviewGateKind::RiskCheckpoint)
        {
            bail!("same_maker_roll_open_material_gate:{}", persisted.run.id);
        }
        let current_maker = persisted
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Maker)
            .ok_or_else(|| anyhow!("feature_run_missing_maker:{}", persisted.run.id))?;
        if current_maker.worker_id != maker_worker_id {
            bail!("same_maker_roll_non_owner:{}", persisted.run.id);
        }
        let batch_id = persisted
            .run
            .active_batch_id
            .as_deref()
            .ok_or_else(|| anyhow!("feature_run_missing_active_batch:{}", persisted.run.id))?;
        let batch = repository.batch(batch_id)?;
        let ended =
            roll_batch_for_same_maker(&batch.batch, maker_worker_id, DEFAULT_BATCH_OUTCOME_CAP)
                .map_err(|violation| anyhow!("same_maker_roll_rejected:{violation:?}"))?;
        let next_batch = ExecutionBatch {
            id: short_id("batch"),
            run_id: persisted.run.id.clone(),
            maker_worker_id: maker_worker_id.to_string(),
            status: ExecutionBatchStatus::Active,
            settled_outcome_ids: Vec::new(),
            replacement: None,
        };
        let mut next_run = persisted.run.clone();
        next_run.active_batch_id = Some(next_batch.id.clone());
        next_run.batch_outcome_count = 0;
        repository.roll_feature_run_batch(
            &ended,
            batch.revision,
            &next_run,
            persisted.revision,
            &next_batch,
        )?;
        self.record_event(
            "same_maker_batch_rolled",
            None,
            json!({
                "run_id": persisted.run.id,
                "ended_batch_id": ended.id,
                "successor_batch_id": next_batch.id,
                "maker_worker_id": maker_worker_id,
            }),
        )?;
        Ok(json!({
            "feature_run": next_run,
            "ended_batch": ended,
            "execution_batch": next_batch,
            "reason": "same_maker_batch_rolled",
        }))
    }

    pub(crate) fn review_gate_pick_value(
        &self,
        plan_id: &str,
        peek: bool,
    ) -> Result<Option<Value>> {
        self.review_gate_pick_value_for_worker(plan_id, peek, &worker_id())
    }

    fn reconcile_final_review_repair_source(
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

    fn review_gate_pick_value_for_worker(
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

#[cfg(test)]
mod tests {
    use super::super::proof::PlanEvidenceAuthority;
    use super::super::repository::execution_run::{
        ReviewSourceBindingRecord, SourceFreezeRecord, SourceFreezeStatus,
    };
    use super::*;
    use crate::evidence::policy::capture_repository_snapshot;
    use crate::storage::ensure_schema;
    use crate::usage_policy::{EscalationSource, ReviewEscalationReason};
    use rusqlite::{Connection, params};
    use std::{fs, path::PathBuf};

    fn test_app_at(root: PathBuf) -> App {
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

    fn test_file_app(root: &std::path::Path, db: &std::path::Path, initialize: bool) -> App {
        let conn = Connection::open(db).expect("file database");
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .expect("busy timeout");
        ensure_schema(&conn).expect("schema");
        if initialize {
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
        }
        App::new(conn, root.to_path_buf(), db.to_path_buf(), true, false)
    }

    fn test_app() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().expect("test root");
        initialize_test_git(root.path());
        let app = test_app_at(root.path().to_path_buf());
        (root, app)
    }

    fn add_outcome(app: &App, id: &str) {
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at) VALUES (?1, 'project-a', ?1, 'outcome', 'picked', 'code', ?2, 'plan-a.md', datetime('now'), datetime('now'))",
                params![id, worker_id()],
            )
            .expect("outcome item");
    }

    fn add_ready_verification(app: &App, id: &str) {
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at) VALUES (?1, 'project-a', ?1, 'verification', 'ready', 'verification', 'plan-a.md', datetime('now'), datetime('now'))",
                [id],
            )
            .expect("verification item");
    }

    fn prepare_fixture_binding(app: &App, binding: Option<(&str, Option<&str>, &str)>) {
        let Some((obligation_id, item_id, slice)) = binding else {
            return;
        };
        let exists: bool = app
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM proof_obligations WHERE id = ?1)",
                [obligation_id],
                |row| row.get(0),
            )
            .expect("fixture binding lookup");
        if exists {
            return;
        }
        let plan_path = app.root.join("plan-a.md");
        fs::write(
            &plan_path,
            crate::planpack::build_plan_body("Plan", "product-plan", slice),
        )
        .expect("fixture build plan");
        let plan_path = plan_path.to_string_lossy().to_string();
        app.conn
            .execute("UPDATE plans SET path = ?1 WHERE id = 'plan-a'", [&plan_path])
            .expect("fixture plan path");
        app.conn
            .execute(
                "UPDATE items SET plan_path = ?1 WHERE project_id = 'project-a'",
                [&plan_path],
            )
            .expect("fixture item paths");
        app.evidence_migration_value(
            json!({
                "schema_version": "planr.evidence.migration.v1",
                "plan_id": "plan-a",
                "obligations": [{
                    "id": obligation_id,
                    "schema_version": "evidence.contract.v1",
                    "criterion_id": format!("criterion-{slice}"),
                    "plan_id": "plan-a",
                    "item_id": item_id,
                    "title": "canonical fixture binding",
                    "binding": true,
                    "observations": [{
                        "id": "obs-fixture-binding",
                        "type": "com.example.ready.status",
                        "subject": "fixture binding",
                        "expected": {"status": "ready"},
                        "target": {"kind": "process", "uri": "local://fixture"},
                        "payload_schema": {"schema_ref": "com.example.ready.status@v1"}
                    }],
                    "fixture_policy": {},
                    "freshness_policy": {},
                    "assurance_policy": {"retry_aggregation": "all_applicable_pass"}
                }]
            }),
            true,
        )
        .expect("fixture Evidence migration");
    }

    fn close_and_bind_fixture(
        app: &App,
        closed_item_id: &str,
        binding: Option<(&str, Option<&str>, &str)>,
    ) {
        prepare_fixture_binding(app, binding);
        if app.get_item(closed_item_id).expect("fixture item").status
            != crate::model::ItemStatus::Closed
        {
            app.close_item_core(closed_item_id, "canonical fixture ordinary outcome closed", false)
                .expect("closed fixture ordinary outcome");
        }
    }

    fn initialize_test_git(root: &std::path::Path) {
        fs::write(root.join("plan-a.md"), "# Plan\n").expect("plan source");
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "tests@planr.local"],
            vec!["config", "user.name", "Planr Tests"],
            vec!["add", "plan-a.md"],
            vec!["commit", "-qm", "fixture"],
        ] {
            assert!(
                std::process::Command::new("git")
                    .current_dir(root)
                    .args(args)
                    .status()
                    .expect("git command")
                    .success()
            );
        }
    }

    fn bind_risk_gate_source(
        app: &App,
        gate_id: &str,
        receipt_lineage: Value,
    ) -> SourceFreezeRecord {
        let repository = ExecutionRunRepository::new(&app.conn);
        let gate = repository.review_gate(gate_id).expect("risk gate");
        let snapshot = capture_repository_snapshot(&app.root).expect("source snapshot");
        let freeze = SourceFreezeRecord {
            id: short_id("freeze"),
            run_id: gate.run_id,
            source_revision: snapshot.source.revision,
            source_digest: snapshot.source.tree_digest.as_str().to_string(),
            status: SourceFreezeStatus::Active,
        };
        repository.freeze_source(&freeze).expect("bound freeze");
        app.conn
            .execute(
                "UPDATE review_gates SET source_revision = ?1 WHERE id = ?2",
                params![freeze.source_revision, gate_id],
            )
            .expect("gate source revision");
        repository
            .create_review_source_binding(&ReviewSourceBindingRecord {
                gate_id: gate_id.to_string(),
                freeze_id: freeze.id.clone(),
                source_revision: freeze.source_revision.clone(),
                source_digest: freeze.source_digest.clone(),
                receipt_lineage,
            })
            .expect("review source binding");
        freeze
    }

    fn bind_risk_gate_to_active_freeze(app: &App, gate_id: &str) -> SourceFreezeRecord {
        let verification_item_id: String = app
            .conn
            .query_row(
                "SELECT id FROM items WHERE work_type = 'verification'
                   ORDER BY created_at, id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("verification item");
        let gate = ExecutionRunRepository::new(&app.conn)
            .review_gate(gate_id)
            .expect("risk gate");
        close_and_bind_fixture(
            app,
            &gate.scope_id,
            Some(("pob-risk-legacy", Some(&verification_item_id), "risk")),
        );
        bind_risk_gate_source(
            app,
            gate_id,
            json!({
                "kind": "product_repair",
                "selective_obligation_ids": ["pob-risk-legacy"]
            }),
        )
    }

    fn bind_risk_gate_without_verification_item(app: &App, gate_id: &str) -> SourceFreezeRecord {
        let gate = ExecutionRunRepository::new(&app.conn)
            .review_gate(gate_id)
            .expect("risk gate");
        close_and_bind_fixture(
            app,
            &gate.scope_id,
            Some(("pob-risk-plan-wide", None, "risk-plan-wide")),
        );
        bind_risk_gate_source(
            app,
            gate_id,
            json!({
                "kind": "product_repair",
                "selective_obligation_ids": []
            }),
        )
    }

    fn materiality(review: bool) -> Value {
        json!({
            "decision": {
                "material": review,
                "review": if review { "independent_high_signal" } else { "none" },
                "reasons": if review { vec!["material_trigger:schema_or_migration".to_string()] } else { Vec::<String>::new() },
            }
        })
    }

    #[test]
    fn accepted_risk_gate_atomically_hands_off_to_verification_and_recovery_is_idempotent() {
        let root = tempfile::tempdir().expect("root");
        initialize_test_git(root.path());
        let app = test_app_at(root.path().to_path_buf());
        add_outcome(&app, "item-risk-final");
        prepare_fixture_binding(
            &app,
            Some(("pob-risk-plan-wide", None, "risk-plan-wide")),
        );
        app.outcome_work_packet("item-risk-final")
            .expect("maker packet");
        let settled = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-risk-final",
                summary: "protected final outcome",
                materiality: &materiality(true),
                escalation: None,
            })
            .expect("risk settlement");
        let gate_id = settled["review_gate"]["id"].as_str().unwrap();
        bind_risk_gate_without_verification_item(&app, gate_id);
        app.review_gate_pick_value_for_worker("plan-a", false, "checker-risk")
            .expect("review pick")
            .expect("review packet");
        let accepted = app
            .complete_review_gate_value(gate_id, ReviewVerdict::Accepted, &[], Some("checker-risk"))
            .expect("accepted review");
        let handoff = &accepted["verification_handoff"];
        assert_eq!(handoff["reason"], "verification_handoff_source_frozen");
        assert_eq!(handoff["work_packet"]["verification_item_id"], Value::Null);
        assert_eq!(accepted["execution_state"]["phase"], "source_frozen");
        assert!(accepted["execution_state"]["owner"].is_null());
        assert_eq!(
            accepted["execution_state"]["execution_batch"]["status"],
            "ended"
        );
        let sealed = ExecutionRunRepository::new(&app.conn)
            .review_source_binding(gate_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            sealed.receipt_lineage,
            json!({
                "kind": "risk_review_acceptance",
                "active_obligation_ids": ["pob-risk-plan-wide"]
            })
        );
        let freeze_id = handoff["work_packet"]["source_freeze"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let repeated = app
            .resume_accepted_risk_verification_handoff_value("plan-a")
            .expect("idempotent recovery")
            .expect("existing handoff");
        assert_eq!(repeated["work_packet"]["source_freeze"]["id"], freeze_id);
        fs::write(root.path().join("plan-a.md"), "# Stale plan\n").expect("stale source");
        let stale = app
            .resume_accepted_risk_verification_handoff_value("plan-a")
            .unwrap_err();
        assert!(stale.to_string().contains("source_freeze_stale:"));
    }

    #[test]
    fn accepted_reopened_risk_gate_reuses_bound_freeze_and_projects_exact_handoff() {
        let root = tempfile::tempdir().expect("root");
        initialize_test_git(root.path());
        let app = test_app_at(root.path().to_path_buf());
        add_outcome(&app, "item-risk-bound");
        add_ready_verification(&app, "item-risk-verification");
        prepare_fixture_binding(
            &app,
            Some(("pob-risk-legacy", Some("item-risk-verification"), "risk")),
        );
        app.outcome_work_packet("item-risk-bound")
            .expect("maker packet");
        let settled = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-risk-bound",
                summary: "protected repaired outcome",
                materiality: &materiality(true),
                escalation: None,
            })
            .expect("risk settlement");
        let gate_id = settled["review_gate"]["id"].as_str().unwrap();
        let bound = bind_risk_gate_to_active_freeze(&app, gate_id);
        app.review_gate_pick_value_for_worker("plan-a", false, "checker-risk")
            .expect("review pick")
            .expect("review packet");
        let accepted = app
            .complete_review_gate_value(gate_id, ReviewVerdict::Accepted, &[], Some("checker-risk"))
            .expect("accepted review");
        assert_eq!(
            accepted["verification_handoff"]["work_packet"]["source_freeze"]["id"],
            bound.id
        );
        assert_eq!(accepted["execution_state"]["phase"], "source_frozen");
        assert!(accepted["execution_state"]["owner"].is_null());
        assert_eq!(
            accepted["execution_state"]["review_source_binding"]["freeze_id"],
            bound.id
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM feature_run_source_freezes WHERE run_id = ?1",
                    [&bound.run_id],
                    |row| row.get::<_, i64>(0),
                )
                .expect("freeze count"),
            1
        );
        let repeated = app
            .resume_accepted_risk_verification_handoff_value("plan-a")
            .expect("idempotent recovery")
            .expect("handoff");
        assert_eq!(repeated["work_packet"]["source_freeze"]["id"], bound.id);
    }

    #[test]
    fn risk_review_finding_repair_refreezes_and_rebinds_changed_source_before_rereview() {
        let root = tempfile::tempdir().expect("root");
        initialize_test_git(root.path());
        let app = test_app_at(root.path().to_path_buf());
        add_outcome(&app, "item-risk-refreeze");
        add_ready_verification(&app, "item-risk-verification");
        app.outcome_work_packet("item-risk-refreeze")
            .expect("maker packet");
        let settled = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-risk-refreeze",
                summary: "protected repaired outcome",
                materiality: &materiality(true),
                escalation: None,
            })
            .expect("risk settlement");
        let gate_id = settled["review_gate"]["id"].as_str().unwrap();
        let old_freeze = bind_risk_gate_to_active_freeze(&app, gate_id);
        app.review_gate_pick_value_for_worker("plan-a", false, "checker-risk")
            .expect("review pick")
            .expect("review packet");
        app.complete_review_gate_value(
            gate_id,
            ReviewVerdict::ChangesRequested,
            &["repair accepted handoff".to_string()],
            Some("checker-risk"),
        )
        .expect("changes requested");
        let finding_id = ExecutionRunRepository::new(&app.conn)
            .findings(gate_id)
            .expect("findings")
            .into_iter()
            .next()
            .expect("finding")
            .id;
        fs::write(root.path().join("plan-a.md"), "# Repaired\n").expect("repair source");
        let resolved = app
            .resolve_review_gate_findings_value(gate_id, std::slice::from_ref(&finding_id))
            .expect("resolve finding");
        assert_eq!(
            resolved["execution_state"]["review_gate"]["status"],
            "pending"
        );
        assert_eq!(
            resolved["execution_state"]["review_source_binding"]["receipt_lineage"]["kind"],
            "risk_review_finding_repair"
        );
        let new_freeze_id = resolved["execution_state"]["review_source_binding"]["freeze_id"]
            .as_str()
            .expect("new freeze");
        assert_ne!(new_freeze_id, old_freeze.id);
        let repository = ExecutionRunRepository::new(&app.conn);
        assert_eq!(
            repository
                .active_source_freeze(&old_freeze.run_id)
                .expect("active freeze")
                .expect("replacement freeze")
                .id,
            new_freeze_id
        );
        assert_eq!(
            repository
                .source_freeze(&old_freeze.id)
                .expect("old freeze")
                .status,
            SourceFreezeStatus::Invalidated
        );
        assert_eq!(resolved["execution_state"]["phase"], "implementation");
        assert_eq!(
            resolved["execution_state"]["owner"]["worker_id"],
            worker_id()
        );
    }

    #[test]
    fn accepted_reopened_risk_gate_rejects_missing_mismatched_and_stale_bound_freezes() {
        for failure in ["missing", "mismatch", "stale"] {
            let root = tempfile::tempdir().expect("root");
            initialize_test_git(root.path());
            let app = test_app_at(root.path().to_path_buf());
            add_outcome(&app, "item-risk-bound-negative");
            add_ready_verification(&app, "item-risk-verification");
            app.outcome_work_packet("item-risk-bound-negative")
                .expect("maker packet");
            let settled = app
                .settle_feature_run_outcome(OutcomeSettlement {
                    item_id: "item-risk-bound-negative",
                    summary: "protected repaired outcome",
                    materiality: &materiality(true),
                    escalation: None,
                })
                .expect("risk settlement");
            let gate_id = settled["review_gate"]["id"].as_str().unwrap();
            let repository = ExecutionRunRepository::new(&app.conn);
            let gate = repository.review_gate(gate_id).expect("gate");
            let snapshot = capture_repository_snapshot(&app.root).expect("snapshot");
            let freeze = SourceFreezeRecord {
                id: short_id("freeze"),
                run_id: gate.run_id.clone(),
                source_revision: snapshot.source.revision.clone(),
                source_digest: snapshot.source.tree_digest.as_str().to_string(),
                status: SourceFreezeStatus::Active,
            };
            repository.freeze_source(&freeze).expect("active freeze");
            app.conn
                .execute(
                    "UPDATE review_gates SET source_revision = ?1 WHERE id = ?2",
                    params![freeze.source_revision, gate_id],
                )
                .expect("gate source");
            repository
                .create_review_source_binding(&ReviewSourceBindingRecord {
                    gate_id: gate_id.to_string(),
                    freeze_id: freeze.id.clone(),
                    source_revision: freeze.source_revision.clone(),
                    source_digest: freeze.source_digest.clone(),
                    receipt_lineage: json!({}),
                })
                .expect("binding");
            if failure == "missing" || failure == "mismatch" {
                app.conn
                    .execute(
                        "UPDATE feature_run_source_freezes SET status = 'invalidated' WHERE id = ?1",
                        [&freeze.id],
                    )
                    .expect("invalidate bound freeze fixture");
            }
            if failure == "mismatch" {
                repository
                    .freeze_source(&SourceFreezeRecord {
                        id: short_id("freeze"),
                        run_id: gate.run_id.clone(),
                        source_revision: freeze.source_revision.clone(),
                        source_digest: freeze.source_digest.clone(),
                        status: SourceFreezeStatus::Active,
                    })
                    .expect("mismatched active freeze");
            }
            if failure == "stale" {
                fs::write(root.path().join("plan-a.md"), "# Changed\n").expect("stale source");
            }
            app.review_gate_pick_value_for_worker("plan-a", false, "checker-risk")
                .expect("review pick")
                .expect("review packet");
            let error = app
                .complete_review_gate_value(
                    gate_id,
                    ReviewVerdict::Accepted,
                    &[],
                    Some("checker-risk"),
                )
                .unwrap_err();
            assert!(
                error.to_string().contains(match failure {
                    "missing" => "review_source_binding_missing_active_freeze:",
                    "mismatch" => "review_source_binding_source_freeze_stale:",
                    _ => "review_source_binding_source_freeze_stale:",
                }),
                "{failure}: {error}"
            );
            assert_eq!(
                repository
                    .review_gate(gate_id)
                    .expect("rolled back gate")
                    .status,
                ReviewGateStatus::Leased
            );
            assert_eq!(
                repository
                    .feature_run(&gate.run_id)
                    .expect("rolled back run")
                    .run
                    .phase,
                FeatureRunPhase::RiskReview
            );
        }
    }

    #[test]
    fn accepted_risk_gate_waits_for_remaining_code_and_rejects_wrong_plan_scope() {
        let root = tempfile::tempdir().expect("root");
        initialize_test_git(root.path());
        let app = test_app_at(root.path().to_path_buf());
        add_outcome(&app, "item-risk-scope");
        add_ready_verification(&app, "item-risk-verification");
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at) VALUES ('item-risk-remaining', 'project-a', 'remaining', 'remaining code', 'ready', 'code', 'plan-a.md', datetime('now'), datetime('now'))",
                [],
            )
            .expect("remaining code");
        prepare_fixture_binding(
            &app,
            Some(("pob-risk-legacy", Some("item-risk-verification"), "risk")),
        );
        app.outcome_work_packet("item-risk-scope")
            .expect("maker packet");
        let settled = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-risk-scope",
                summary: "protected outcome",
                materiality: &materiality(true),
                escalation: None,
            })
            .expect("risk settlement");
        let gate_id = settled["review_gate"]["id"].as_str().unwrap();
        bind_risk_gate_to_active_freeze(&app, gate_id);
        app.review_gate_pick_value_for_worker("plan-a", false, "checker-risk")
            .expect("review pick")
            .expect("review packet");
        let accepted = app
            .complete_review_gate_value(gate_id, ReviewVerdict::Accepted, &[], Some("checker-risk"))
            .expect("accepted review");
        assert!(accepted.get("verification_handoff").is_none());
        assert_eq!(accepted["execution_state"]["phase"], "implementation");

        app.conn
            .execute(
                "UPDATE items SET status = 'cancelled' WHERE id = 'item-risk-remaining'",
                [],
            )
            .expect("settle remaining fixture");
        let recovered = app
            .resume_accepted_risk_verification_handoff_value("plan-a")
            .expect("recovery")
            .expect("handoff");
        assert_eq!(
            recovered["work_packet"]["execution_state"]["phase"],
            "source_frozen"
        );

        let other_root = tempfile::tempdir().expect("other root");
        initialize_test_git(other_root.path());
        let other = test_app_at(other_root.path().to_path_buf());
        add_outcome(&other, "item-risk-wrong-plan");
        add_ready_verification(&other, "item-risk-verification");
        other
            .outcome_work_packet("item-risk-wrong-plan")
            .expect("maker packet");
        let settled = other
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-risk-wrong-plan",
                summary: "protected outcome",
                materiality: &materiality(true),
                escalation: None,
            })
            .expect("risk settlement");
        let gate_id = settled["review_gate"]["id"].as_str().unwrap();
        other
            .review_gate_pick_value_for_worker("plan-a", false, "checker-risk")
            .expect("review pick")
            .expect("review packet");
        other
            .conn
            .execute(
                "UPDATE items SET plan_path = 'wrong-plan.md' WHERE id = 'item-risk-wrong-plan'",
                [],
            )
            .expect("wrong plan fixture");
        let wrong_plan = other
            .complete_review_gate_value(gate_id, ReviewVerdict::Accepted, &[], Some("checker-risk"))
            .unwrap_err();
        assert!(
            wrong_plan
                .to_string()
                .contains("risk_checkpoint_scope_plan_mismatch:")
        );
    }

    #[test]
    fn accepted_risk_handoff_recovery_is_concurrent_and_single_freeze() {
        let root = tempfile::tempdir().expect("root");
        let state = tempfile::tempdir().expect("state");
        initialize_test_git(root.path());
        let db = state.path().join("planr.sqlite");
        let app = test_file_app(root.path(), &db, true);
        add_outcome(&app, "item-risk-concurrent");
        add_ready_verification(&app, "item-risk-verification");
        prepare_fixture_binding(
            &app,
            Some(("pob-risk-legacy", Some("item-risk-verification"), "risk")),
        );
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at) VALUES ('item-risk-remaining', 'project-a', 'remaining', 'remaining code', 'ready', 'code', 'plan-a.md', datetime('now'), datetime('now'))",
                [],
            )
            .expect("remaining code");
        app.outcome_work_packet("item-risk-concurrent")
            .expect("maker packet");
        let settled = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-risk-concurrent",
                summary: "protected outcome",
                materiality: &materiality(true),
                escalation: None,
            })
            .expect("risk settlement");
        let gate_id = settled["review_gate"]["id"].as_str().unwrap();
        bind_risk_gate_to_active_freeze(&app, gate_id);
        app.review_gate_pick_value_for_worker("plan-a", false, "checker-risk")
            .expect("review pick")
            .expect("review packet");
        app.complete_review_gate_value(gate_id, ReviewVerdict::Accepted, &[], Some("checker-risk"))
            .expect("accepted review");
        app.conn
            .execute(
                "UPDATE items SET status = 'cancelled' WHERE id = 'item-risk-remaining'",
                [],
            )
            .expect("settle remaining fixture");
        drop(app);

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let barrier = barrier.clone();
                let root = root.path().to_path_buf();
                let db = db.clone();
                std::thread::spawn(move || {
                    let app = test_file_app(&root, &db, false);
                    barrier.wait();
                    app.resume_accepted_risk_verification_handoff_value("plan-a")
                        .expect("concurrent recovery")
                        .expect("handoff")["work_packet"]["source_freeze"]["id"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
            })
            .collect::<Vec<_>>();
        let freeze_ids = handles
            .into_iter()
            .map(|handle| handle.join().expect("recovery thread"))
            .collect::<Vec<_>>();
        assert_eq!(freeze_ids[0], freeze_ids[1]);
        let app = test_file_app(root.path(), &db, false);
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM feature_run_source_freezes",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .expect("freeze count"),
            1
        );
    }

    #[test]
    fn late_binding_between_nonbinding_preflight_and_refresh_fails_without_mutation() {
        let root = tempfile::tempdir().expect("root");
        let state = tempfile::tempdir().expect("state");
        initialize_test_git(root.path());
        let db = state.path().join("planr.sqlite");
        let app = test_file_app(root.path(), &db, true);
        add_outcome(&app, "item-late-binding");
        app.outcome_work_packet("item-late-binding")
            .expect("maker packet");
        close_and_bind_fixture(&app, "item-late-binding", None);
        let frozen = app
            .freeze_feature_run_source_value("plan-a")
            .expect("freeze")
            .expect("frozen run");
        let run_id = frozen["feature_run"]["id"].as_str().unwrap();
        let original_freeze_id = frozen["source_freeze"]["id"].as_str().unwrap();
        assert_eq!(
            app.plan_evidence_authority("plan-a")
                .expect("nonbinding preflight"),
            PlanEvidenceAuthority::NonBinding
        );

        let migration = test_file_app(root.path(), &db, false);
        close_and_bind_fixture(
            &migration,
            "item-late-binding",
            Some(("pob-late-binding", None, "late-binding")),
        );
        drop(migration);
        fs::write(root.path().join("late-binding-change.txt"), "changed")
            .expect("change source after freeze");

        let error = app
            .refresh_nonbinding_final_review_source_freeze("plan-a", run_id)
            .expect_err("late binding must fail closed");
        assert_eq!(
            error.to_string(),
            "nonbinding_final_review_refresh_evidence_authority_changed:plan-a"
        );
        let repository = ExecutionRunRepository::new(&app.conn);
        assert_eq!(
            repository
                .active_source_freeze(run_id)
                .expect("active freeze")
                .expect("freeze")
                .id,
            original_freeze_id
        );
        assert!(
            repository
                .invalidations(run_id)
                .expect("invalidations")
                .is_empty()
        );
        let gate_error = app
            .ensure_final_product_review_gate_value("plan-a")
            .expect_err("late binding must block final-review admission");
        assert!(
            gate_error
                .to_string()
                == "final_product_review_source_freeze_stale:plan-a"
        );
        assert!(
            repository
                .review_gates_for_run(run_id, false)
                .expect("review gates")
                .is_empty()
        );
    }

    fn seed_incompatible_feature_run(app: &App, run_id: &str) {
        let batch_id = format!("batch-{run_id}");
        app.conn
            .execute_batch("BEGIN IMMEDIATE")
            .expect("begin legacy run seed");
        app.conn
            .execute(
                "INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, active_batch_id) VALUES (?1, 'project-a', 'plan-a', 'active', 'implementation', 'sha256:legacy', ?2)",
                params![run_id, batch_id],
            )
            .expect("legacy run");
        app.conn
            .execute(
                "INSERT INTO execution_batches(id, run_id, maker_worker_id, status) VALUES (?1, ?2, ?3, 'active')",
                params![batch_id, run_id, worker_id()],
            )
            .expect("legacy batch");
        app.conn
            .execute(
                "INSERT INTO feature_run_role_leases(run_id, role, worker_id, lease_generation) VALUES (?1, 'maker', ?2, 1)",
                params![run_id, worker_id()],
            )
            .expect("legacy lease");
        app.conn
            .execute_batch("COMMIT")
            .expect("commit legacy run seed");
    }

    #[test]
    fn restart_application_is_atomic_idempotent_and_rejects_a_healthy_run() {
        let (_root, app) = test_app();
        seed_incompatible_feature_run(&app, "run-incompatible-app");
        let first = app
            .restart_feature_run_value("plan-a", FeatureRunRestartReason::IncompatibleBudget)
            .expect("first restart");
        assert_eq!(first["schema_version"], "planr.feature_run_restart.v1");
        assert_eq!(first["restart"]["disposition"], "retired");
        assert_eq!(first["restart"]["incompatibility"], "missing");
        assert_eq!(
            first["execution_state"]["schema_version"],
            "planr.execution_state.v2"
        );
        assert_eq!(
            first["execution_state"]["feature_run"]["status"],
            "cancelled"
        );
        assert_eq!(
            first["execution_state"]["feature_run"]["terminal_reason"],
            "policy_cancelled"
        );

        let repeated = app
            .restart_feature_run_value("plan-a", FeatureRunRestartReason::IncompatibleBudget)
            .expect("idempotent restart");
        assert_eq!(repeated["restart"]["disposition"], "already_retired");
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE event_type = 'feature_run_incompatible_budget_retired'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("one retirement event"),
            1
        );

        let (_healthy_root, healthy) = test_app();
        add_outcome(&healthy, "item-healthy");
        healthy
            .outcome_work_packet("item-healthy")
            .expect("healthy FeatureRun");
        assert!(
            healthy
                .restart_feature_run_value("plan-a", FeatureRunRestartReason::IncompatibleBudget,)
                .unwrap_err()
                .to_string()
                .contains("RestartBudgetContractCompatible")
        );

        let (_root, stale) = test_app();
        add_outcome(&stale, "item-stale-source");
        let initial = stale
            .outcome_work_packet("item-stale-source")
            .expect("initial ordinary outcome packet");
        let old_run_id = initial["execution_state"]["feature_run"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let repository = ExecutionRunRepository::new(&stale.conn);
        let persisted = repository.feature_run(&old_run_id).expect("active run");
        let snapshot = capture_repository_snapshot(&stale.root).expect("source snapshot");
        let frozen = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::SourceFrozen,
                cause: PhaseTransitionCause::ImplementationSettled,
                reference: snapshot.source.revision.clone(),
                owner: None,
            },
        )
        .expect("malformed source-freeze transition");
        repository
            .save_feature_run(&frozen, persisted.revision)
            .expect("persist malformed frozen run");
        let expected_freeze = SourceFreezeRecord {
            id: short_id("freeze"),
            run_id: old_run_id.clone(),
            source_revision: snapshot.source.revision,
            source_digest: snapshot.source.tree_digest.as_str().to_string(),
            status: SourceFreezeStatus::Active,
        };
        repository
            .freeze_source(&expected_freeze)
            .expect("persist malformed freeze");
        let old_freeze_id = expected_freeze.id.clone();
        let old_freeze = repository
            .source_freeze(&old_freeze_id)
            .expect("old source freeze");
        let old_freeze_bytes = serde_json::to_vec(&old_freeze).expect("serialized old freeze");
        let retired = stale
            .restart_feature_run_value("plan-a", FeatureRunRestartReason::PrematureSourceFreeze)
            .expect("premature source-freeze restart");
        assert_eq!(retired["restart"]["disposition"], "retired");
        assert_eq!(retired["restart"]["facts"]["freeze_id"], old_freeze_id);
        assert_eq!(
            retired["restart"]["facts"]["open_outcome_ids"],
            json!(["item-stale-source"])
        );
        assert_eq!(
            retired["restart"]["facts"]["released_outcome_ids"],
            json!(["item-stale-source"])
        );
        assert_eq!(
            retired["execution_state"]["feature_run"]["status"],
            "cancelled"
        );
        assert_eq!(
            retired["execution_state"]["feature_run"]["phase"],
            "cancelled"
        );
        assert_eq!(
            retired["execution_state"]["feature_run"]["terminal_reason"],
            "policy_cancelled"
        );
        let routed: (String, Option<String>, Option<String>) = stale
            .conn
            .query_row(
                "SELECT status, worker_id, pick_token FROM items WHERE id = 'item-stale-source'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("routed outcome");
        assert_eq!(routed, ("ready".to_string(), None, None));
        assert_eq!(
            serde_json::to_vec(
                &repository
                    .source_freeze(&old_freeze_id)
                    .expect("preserved old freeze")
            )
            .expect("serialized preserved freeze"),
            old_freeze_bytes
        );
        let repeated = stale
            .restart_feature_run_value("plan-a", FeatureRunRestartReason::PrematureSourceFreeze)
            .expect("idempotent premature source-freeze restart");
        assert_eq!(repeated["restart"]["disposition"], "already_retired");
        assert_eq!(stale.conn.query_row("SELECT COUNT(*) FROM events WHERE event_type = 'feature_run_premature_source_freeze_retired'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        let next = stale
            .next_pick_value(None, Some("code"), Some("plan-a"))
            .expect("next ordinary pick");
        assert_eq!(next["work_packet"]["item_id"], "item-stale-source");
        let new_run_id = next["work_packet"]["execution_state"]["feature_run"]["id"]
            .as_str()
            .unwrap();
        assert_ne!(new_run_id, old_run_id);
        stale
            .close_item_core(
                "item-stale-source",
                "normal successor settled before source freeze",
                false,
            )
            .expect("close successor outcome");
        let new_freeze = stale
            .freeze_feature_run_source_value("plan-a")
            .expect("successor readiness freeze")
            .expect("successor run");
        assert_eq!(new_freeze["created"], true);
        assert_ne!(new_freeze["source_freeze"]["id"], old_freeze_id);
        assert_eq!(new_freeze["source_freeze"]["run_id"], new_run_id);
        assert_eq!(
            new_freeze["source_freeze"]["source_revision"],
            old_freeze.source_revision
        );
        assert_eq!(
            new_freeze["source_freeze"]["source_digest"],
            old_freeze.source_digest
        );
        assert_eq!(
            serde_json::to_vec(
                &repository
                    .source_freeze(&old_freeze_id)
                    .expect("old freeze after successor")
            )
            .expect("serialized old freeze after successor"),
            old_freeze_bytes
        );
    }

    #[test]
    fn capped_batch_rolls_to_fourth_outcome_with_same_maker_and_no_review_artifacts() {
        let (_root, app) = test_app();
        add_outcome(&app, "item-a");
        add_outcome(&app, "item-b");
        add_outcome(&app, "item-c");
        add_outcome(&app, "item-d");
        let packet = app.outcome_work_packet("item-a").expect("outcome packet");
        assert_eq!(packet["kind"], "outcome");
        assert_eq!(packet["item_id"], "item-a");
        let first = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-a",
                summary: "first",
                materiality: &materiality(false),
                escalation: None,
            })
            .expect("first settlement");
        let second = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-b",
                summary: "second",
                materiality: &materiality(false),
                escalation: None,
            })
            .expect("second settlement");
        assert_eq!(first["run_id"], second["run_id"]);
        assert_eq!(first["batch_id"], second["batch_id"]);
        assert_eq!(second["batch_outcome_count"], 2);
        assert_eq!(second["transition"], "continue_batch");
        let third = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-c",
                summary: "third",
                materiality: &materiality(false),
                escalation: None,
            })
            .expect("third settlement");
        assert_eq!(third["transition"], "batch_cap_reached");
        let run_id = third["run_id"].as_str().unwrap();
        let before = ExecutionRunRepository::new(&app.conn)
            .feature_run(run_id)
            .expect("run before roll");
        let rolled = app
            .roll_feature_run_batch_value("plan-a", &worker_id())
            .expect("same-maker roll");
        assert_eq!(rolled["reason"], "same_maker_batch_rolled");
        assert_eq!(rolled["ended_batch"]["replacement"], Value::Null);
        assert_eq!(rolled["feature_run"]["batch_outcome_count"], 0);
        assert_eq!(
            rolled["feature_run"]["role_owners"],
            json!(before.run.role_owners)
        );
        let fourth = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-d",
                summary: "fourth",
                materiality: &materiality(false),
                escalation: None,
            })
            .expect("fourth settlement");
        assert_eq!(fourth["transition"], "continue_batch");
        assert_eq!(fourth["batch_outcome_count"], 1);
        assert_ne!(fourth["batch_id"], third["batch_id"]);
        assert_eq!(
            app.conn
                .query_row("SELECT COUNT(*) FROM review_gates", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM items WHERE work_type IN ('review','fix')",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM execution_batches WHERE replacement_reason IS NOT NULL",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn same_maker_roll_fails_closed_for_state_owner_gate_and_double_roll() {
        let (_root, app) = test_app();
        for id in ["guard-a", "guard-b", "guard-c"] {
            add_outcome(&app, id);
        }
        app.outcome_work_packet("guard-a").expect("start run");
        assert!(
            app.roll_feature_run_batch_value("plan-a", &worker_id())
                .unwrap_err()
                .to_string()
                .contains("BatchNotAtCap")
        );
        for id in ["guard-a", "guard-b", "guard-c"] {
            app.settle_feature_run_outcome(OutcomeSettlement {
                item_id: id,
                summary: id,
                materiality: &materiality(false),
                escalation: None,
            })
            .expect("settle to cap");
        }
        let repository = ExecutionRunRepository::new(&app.conn);
        let run = repository
            .active_feature_run_for_plan("project-a", "plan-a")
            .unwrap()
            .unwrap();
        assert!(
            app.roll_feature_run_batch_value("plan-a", "maker-other")
                .unwrap_err()
                .to_string()
                .contains("same_maker_roll_non_owner")
        );
        repository
            .create_review_gate(&ReviewGateRecord {
                id: "gate-roll-guard".into(),
                run_id: run.run.id.clone(),
                scope_kind: ReviewScopeKind::Outcome,
                scope_id: "guard-c".into(),
                kind: ReviewGateKind::RiskCheckpoint,
                status: ReviewGateStatus::Pending,
                required_risk: Some("guard".into()),
                responsible_maker_id: worker_id(),
                latest_attempt: 0,
                source_revision: None,
            })
            .expect("open material gate");
        assert!(
            app.roll_feature_run_batch_value("plan-a", &worker_id())
                .unwrap_err()
                .to_string()
                .contains("same_maker_roll_open_material_gate")
        );
        app.conn
            .execute("DELETE FROM review_gates WHERE id = 'gate-roll-guard'", [])
            .unwrap();
        app.conn
            .execute(
                "UPDATE feature_runs SET phase = 'risk_review' WHERE id = ?1",
                [&run.run.id],
            )
            .unwrap();
        assert!(
            app.roll_feature_run_batch_value("plan-a", &worker_id())
                .is_err(),
            "a non-implementation phase must fail closed even when persisted state is malformed"
        );
        app.conn
            .execute(
                "UPDATE feature_runs SET phase = 'implementation' WHERE id = ?1",
                [&run.run.id],
            )
            .unwrap();
        app.roll_feature_run_batch_value("plan-a", &worker_id())
            .expect("first roll");
        assert!(
            app.roll_feature_run_batch_value("plan-a", &worker_id())
                .unwrap_err()
                .to_string()
                .contains("BatchNotAtCap")
        );
    }

    #[test]
    fn mcp_and_http_completion_inputs_share_canonical_settlement_and_structured_escalation() {
        let (_root, app) = test_app();
        add_outcome(&app, "item-mcp");
        add_outcome(&app, "item-http");
        add_outcome(&app, "item-escalated");
        let empty = Vec::<String>::new();
        let mcp = app
            .settle_surface_completion_value(super::super::flow::SurfaceCompletionInput {
                item_id: "item-mcp",
                summary: "mcp outcome",
                files: &empty,
                commands: &empty,
                tests: &empty,
                source: "mcp",
                profile: None,
                escalation_reason: None,
                escalation_reference: None,
                escalation_explanation: None,
                write_log: true,
            })
            .expect("mcp settlement");
        let http = app
            .settle_surface_completion_value(super::super::flow::SurfaceCompletionInput {
                item_id: "item-http",
                summary: "http outcome",
                files: &empty,
                commands: &empty,
                tests: &empty,
                source: "http",
                profile: None,
                escalation_reason: None,
                escalation_reference: None,
                escalation_explanation: None,
                write_log: true,
            })
            .expect("http settlement");
        assert_eq!(mcp["work_packet"]["run_id"], http["work_packet"]["run_id"]);
        assert_eq!(
            mcp["work_packet"]["batch_id"],
            http["work_packet"]["batch_id"]
        );
        assert_eq!(http["work_packet"]["batch_outcome_count"], 2);
        let rejected =
            app.settle_surface_completion_value(super::super::flow::SurfaceCompletionInput {
                item_id: "item-escalated",
                summary: "escalated outcome",
                files: &empty,
                commands: &empty,
                tests: &empty,
                source: "mcp",
                profile: None,
                escalation_reason: Some("data_integrity_risk"),
                escalation_reference: Some(""),
                escalation_explanation: Some("protect invariant"),
                write_log: true,
            });
        assert!(
            rejected
                .unwrap_err()
                .to_string()
                .contains("MissingReference")
        );
        let escalated = app
            .settle_surface_completion_value(super::super::flow::SurfaceCompletionInput {
                item_id: "item-escalated",
                summary: "escalated outcome",
                files: &empty,
                commands: &empty,
                tests: &empty,
                source: "mcp",
                profile: None,
                escalation_reason: Some("data_integrity_risk"),
                escalation_reference: Some("finding-1"),
                escalation_explanation: Some("protect invariant"),
                write_log: true,
            })
            .expect("structured escalation");
        assert_eq!(escalated["work_packet"]["transition"], "review_gate");
    }

    #[test]
    fn protected_checkpoint_preserves_maker_batch_and_count_across_rereview() {
        let (_root, app) = test_app();
        add_outcome(&app, "item-risk");
        let settlement = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-risk",
                summary: "protected change",
                materiality: &materiality(true),
                escalation: None,
            })
            .expect("protected settlement");
        let gate_id = settlement["review_gate"]["id"]
            .as_str()
            .expect("gate id")
            .to_string();
        let run_id = settlement["run_id"].as_str().unwrap().to_string();
        let batch_id = settlement["batch_id"].as_str().unwrap().to_string();
        app.review_gate_pick_value_for_worker("plan-a", false, "reviewer-a")
            .expect("review pick")
            .expect("gate packet");
        let repository = ExecutionRunRepository::new(&app.conn);
        let in_review = repository.feature_run(&run_id).expect("run in review");
        assert_eq!(in_review.run.phase, FeatureRunPhase::RiskReview);
        assert_eq!(in_review.run.batch_outcome_count, 1);
        assert_eq!(
            in_review.run.active_batch_id.as_deref(),
            Some(batch_id.as_str())
        );
        assert_eq!(
            in_review
                .run
                .role_owners
                .iter()
                .find(|owner| owner.role == RunRole::Maker)
                .unwrap()
                .worker_id,
            worker_id()
        );

        let changed = app
            .complete_review_gate_value(
                &gate_id,
                ReviewVerdict::ChangesRequested,
                &["repair the protected invariant".into()],
                Some("reviewer-a"),
            )
            .expect("changes requested");
        assert_eq!(changed["created_map_items"], json!([]));
        assert_eq!(
            changed["execution_state"]["findings"][0]["owner_worker_id"],
            worker_id()
        );
        assert_eq!(changed["execution_state"]["phase"], "implementation");
        assert_eq!(
            changed["execution_state"]["feature_run"]["batch_outcome_count"],
            1
        );
        assert_eq!(
            repository
                .batch(&batch_id)
                .expect("resumed batch")
                .batch
                .status,
            ExecutionBatchStatus::Active
        );
        let repair_packet = app
            .repair_work_packet_value("plan-a")
            .expect("repair packet")
            .expect("responsible maker repair");
        assert_eq!(repair_packet["work_packet"]["kind"], "outcome");
        assert_eq!(repair_packet["work_packet"]["mode"], "finding_repair");
        let finding_id = changed["execution_state"]["findings"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let resolved = app
            .resolve_review_gate_findings_value(&gate_id, &[finding_id])
            .expect("resolve finding");
        assert_eq!(
            resolved["execution_state"]["review_gate"]["status"],
            "pending"
        );
        app.review_gate_pick_value_for_worker("plan-a", false, "reviewer-b")
            .expect("re-review pick")
            .expect("same gate packet");
        let accepted = app
            .complete_review_gate_value(&gate_id, ReviewVerdict::Accepted, &[], Some("reviewer-b"))
            .expect("accepted re-review");
        assert_eq!(accepted["execution_state"]["review_gate"]["id"], gate_id);
        assert_eq!(
            accepted["execution_state"]["review_attempts"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            app.conn
                .query_row("SELECT COUNT(*) FROM review_gates", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            app.conn
                .query_row("SELECT COUNT(*) FROM items", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn review_override_requires_structured_reference_and_explanation() {
        let (_root, app) = test_app();
        add_outcome(&app, "item-escalation");
        let rejected = app.settle_feature_run_outcome(OutcomeSettlement {
            item_id: "item-escalation",
            summary: "ordinary change",
            materiality: &materiality(false),
            escalation: Some(ReviewEscalation {
                reason: ReviewEscalationReason::DataIntegrityRisk,
                source: EscalationSource::MakerFinding,
                reference: " ".into(),
                explanation: "integrity boundary".into(),
            }),
        });
        assert!(
            rejected
                .unwrap_err()
                .to_string()
                .contains("MissingReference")
        );
        assert_eq!(
            app.conn
                .query_row("SELECT COUNT(*) FROM execution_run_outcomes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let accepted = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-escalation",
                summary: "ordinary change",
                materiality: &materiality(false),
                escalation: Some(ReviewEscalation {
                    reason: ReviewEscalationReason::DataIntegrityRisk,
                    source: EscalationSource::MakerFinding,
                    reference: "finding:integrity".into(),
                    explanation: "integrity boundary".into(),
                }),
            })
            .expect("structured escalation");
        assert_eq!(accepted["transition"], "review_gate");
    }

    #[test]
    fn nonbinding_feature_has_one_current_independent_final_product_gate() {
        let (_root, app) = test_app();
        add_outcome(&app, "item-final");
        add_ready_verification(&app, "item-legacy-verification");
        let persisted = app
            .ensure_outcome_feature_run("item-final")
            .expect("ensure run")
            .expect("run");
        close_and_bind_fixture(
            &app,
            "item-final",
            Some(("pob-superseded-history", None, "superseded")),
        );
        app.conn
            .execute_batch(
                "INSERT INTO proof_obligations(
                   id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                   binding, observation_requirements_json, fixture_policy_json,
                   freshness_policy_json, assurance_policy_json, policy_digest, config_digest,
                   source_digest, supersedes_obligation_id, created_at, retry_aggregation,
                   obligation_shape
                 ) VALUES (
                   'pob-nonbinding-successor', 'project-a', 'plan-a', NULL, 'crit-superseded', 2,
                   'nonbinding successor', 0, '[]', '{}', '{}', '{}',
                   'sha256:0000000000000000000000000000000000000000000000000000000000000000',
                   'sha256:0000000000000000000000000000000000000000000000000000000000000000',
                   NULL, 'pob-superseded-history', datetime('now'), 'latest_applicable_pass',
                   'semantic_v1'
                 );",
            )
            .expect("nonbinding successor history");
        let repository = ExecutionRunRepository::new(&app.conn);
        let batch_id = persisted.run.active_batch_id.as_deref().unwrap();
        let batch = repository.batch(batch_id).expect("batch");
        let mut ended = batch.batch;
        ended.status = ExecutionBatchStatus::Ended;
        repository
            .save_batch(&ended, batch.revision)
            .expect("end batch");
        let snapshot = capture_repository_snapshot(&app.root).expect("source snapshot");
        let frozen = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::SourceFrozen,
                cause: PhaseTransitionCause::ImplementationSettled,
                reference: snapshot.source.revision.clone(),
                owner: None,
            },
        )
        .expect("freeze transition");
        repository
            .save_feature_run(&frozen, persisted.revision)
            .expect("save frozen");
        repository
            .freeze_source(&SourceFreezeRecord {
                id: "freeze-final".into(),
                run_id: frozen.id.clone(),
                source_revision: snapshot.source.revision,
                source_digest: snapshot.source.tree_digest.as_str().to_string(),
                status: SourceFreezeStatus::Active,
            })
            .expect("persist source freeze");
        assert_eq!(
            app.plan_evidence_authority("plan-a")
                .expect("authoritative supersession"),
            PlanEvidenceAuthority::NonBinding
        );
        let first = app
            .ensure_final_product_review_gate_value("plan-a")
            .expect("final gate");
        let second = app
            .ensure_final_product_review_gate_value("plan-a")
            .expect("same final gate");
        assert_eq!(
            first["execution_state"]["review_gate"]["id"],
            second["execution_state"]["review_gate"]["id"]
        );
        assert_eq!(first["created"], true);
        assert_eq!(second["created"], false);
        let gate_id = first["execution_state"]["review_gate"]["id"]
            .as_str()
            .unwrap();
        assert_eq!(
            first["execution_state"]["review_source_binding"]["freeze_id"],
            "freeze-final"
        );
        assert_eq!(
            first["execution_state"]["review_source_binding"]["source_digest"],
            repository
                .source_freeze("freeze-final")
                .unwrap()
                .source_digest
        );
        let packet = app
            .review_gate_pick_value_for_worker("plan-a", false, "reviewer-final")
            .expect("final review pick")
            .expect("packet");
        assert_eq!(packet["work_packet"]["kind"], "review_gate");
        let stale_path = app.root.join("post-review-source-change.txt");
        fs::write(&stale_path, "stale review source").unwrap();
        let stale = app
            .complete_review_gate_value(
                gate_id,
                ReviewVerdict::Accepted,
                &[],
                Some("reviewer-final"),
            )
            .expect_err("source change before close must reject atomically");
        assert!(stale.to_string().contains("source_freeze_stale"));
        fs::remove_file(stale_path).unwrap();
        let changed = app
            .complete_review_gate_value(
                gate_id,
                ReviewVerdict::ChangesRequested,
                &["repair final product finding".into()],
                Some("reviewer-final"),
            )
            .expect("final finding");
        assert_eq!(
            repository.source_freeze("freeze-final").unwrap().status,
            SourceFreezeStatus::Invalidated
        );
        let finding_id = changed["execution_state"]["findings"][0]["id"]
            .as_str()
            .unwrap();
        let rebound = app
            .resolve_review_gate_findings_value(gate_id, &[finding_id.to_string()])
            .expect("resolve, refreeze, and rebind final finding");
        assert_eq!(rebound["execution_state"]["phase"], "source_frozen");
        let refrozen = app
            .freeze_feature_run_source_value("plan-a")
            .expect("read refreeze")
            .expect("refrozen run");
        assert_eq!(refrozen["created"], false);
        assert_eq!(
            rebound["execution_state"]["review_source_binding"]["freeze_id"],
            refrozen["source_freeze"]["id"]
        );
        assert!(
            !rebound["execution_state"]["review_gate"]["source_revision"]
                .as_str()
                .unwrap()
                .starts_with("product_repair:")
        );
        let refrozen_run = repository
            .feature_run(refrozen["feature_run"]["id"].as_str().unwrap())
            .expect("refrozen run record");
        let reverification = apply_phase_transition(
            &refrozen_run.run,
            &PhaseTransition {
                to: FeatureRunPhase::Verification,
                cause: PhaseTransitionCause::VerificationStarted,
                reference: "verification:repaired".into(),
                owner: Some(RoleOwner {
                    role: RunRole::Verifier,
                    worker_id: "verifier-b".into(),
                    lease_generation: 2,
                }),
            },
        )
        .expect("reverification transition");
        repository
            .save_feature_run(&reverification, refrozen_run.revision)
            .expect("save reverification");
        app.review_gate_pick_value_for_worker("plan-a", false, "reviewer-final-b")
            .expect("final re-review pick")
            .expect("re-review packet");
        let accepted = app
            .complete_review_gate_value(
                gate_id,
                ReviewVerdict::Accepted,
                &[],
                Some("reviewer-final-b"),
            )
            .expect("final acceptance");
        assert_eq!(
            accepted["execution_state"]["review_attempts"][0]["reviewer_mode"],
            "independent"
        );
        assert_eq!(accepted["execution_state"]["phase"], "complete");
        let projected = app
            .final_product_review_projection_value("plan-a")
            .expect("canonical projection");
        assert_eq!(projected["current"]["review_gate"]["id"], gate_id);
        assert_eq!(projected["current"]["accepted"], true);
        assert_eq!(
            app.final_product_review_clause_value("plan-a")
                .expect("audit clause")["pass"],
            true
        );
        assert_eq!(
            app.canonical_execution_state_for_plan_value("plan-a")
                .expect("status projection")
                .expect("execution state")["reason_code"],
            "feature_run_complete"
        );
        let after_complete = app
            .ensure_final_product_review_gate_value("plan-a")
            .expect("same terminal gate");
        assert_eq!(
            after_complete["execution_state"]["review_gate"]["id"],
            gate_id
        );
        assert_eq!(after_complete["created"], false);
        assert_eq!(
            repository
                .review_gates_for_plan("project-a", "plan-a", false)
                .expect("gates")
                .into_iter()
                .filter(|gate| gate.kind == ReviewGateKind::FinalProduct)
                .count(),
            1
        );
    }

    #[test]
    fn accepted_risk_legacy_plan_with_authored_verification_item_routes_to_final_review() {
        let root = tempfile::tempdir().expect("root");
        initialize_test_git(root.path());
        let app = test_app_at(root.path().to_path_buf());
        add_outcome(&app, "item-risk-legacy-final");
        add_ready_verification(&app, "item-risk-legacy-verification");
        app.outcome_work_packet("item-risk-legacy-final")
            .expect("maker packet");
        let settled = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-risk-legacy-final",
                summary: "protected legacy outcome",
                materiality: &materiality(true),
                escalation: None,
            })
            .expect("risk settlement");
        let gate_id = settled["review_gate"]["id"].as_str().unwrap();
        close_and_bind_fixture(&app, "item-risk-legacy-final", None);
        bind_risk_gate_source(
            &app,
            gate_id,
            json!({"kind": "product_repair", "selective_obligation_ids": []}),
        );
        app.review_gate_pick_value_for_worker("plan-a", false, "checker-risk-legacy")
            .expect("risk review pick")
            .expect("risk review packet");
        let accepted = app
            .complete_review_gate_value(
                gate_id,
                ReviewVerdict::Accepted,
                &[],
                Some("checker-risk-legacy"),
            )
            .expect("accepted legacy risk review");
        assert_eq!(
            accepted["verification_handoff"]["work_packet"]["kind"],
            "final_review_handoff"
        );
        assert_eq!(
            app.get_item("item-risk-legacy-verification")
                .expect("verification item")
                .status,
            crate::model::ItemStatus::Ready
        );
        let final_gate = app
            .ensure_final_product_review_gate_value("plan-a")
            .expect("legacy final gate");
        assert_eq!(final_gate["created"], true);
    }

    #[test]
    fn product_finding_invalidates_only_affected_evidence_and_routes_last_maker() {
        let (_root, app) = test_app();
        add_outcome(&app, "item-product-finding");
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at)
                 VALUES ('verification-product-finding', 'project-a', 'verify', 'verify', 'running', 'verification', 'verifier-a', 'plan-a.md', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
        app.ensure_outcome_feature_run("item-product-finding")
            .expect("ensure run")
            .expect("feature run");
        close_and_bind_fixture(
            &app,
            "item-product-finding",
            Some(("pob-only", Some("verification-product-finding"), "only")),
        );
        let frozen = app
            .freeze_feature_run_source_value("plan-a")
            .expect("freeze")
            .expect("feature run");
        let run_id = frozen["feature_run"]["id"].as_str().unwrap();
        let freeze_id = frozen["source_freeze"]["id"].as_str().unwrap();
        let repository = ExecutionRunRepository::new(&app.conn);
        let persisted = repository.feature_run(run_id).expect("frozen run");
        let verification = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::Verification,
                cause: PhaseTransitionCause::VerificationStarted,
                reference: format!("source_freeze:{freeze_id}"),
                owner: Some(RoleOwner {
                    role: RunRole::Verifier,
                    worker_id: "verifier-a".into(),
                    lease_generation: 1,
                }),
            },
        )
        .expect("verification transition");
        repository
            .save_feature_run(&verification, persisted.revision)
            .expect("verification save");

        let routed = app
            .route_evidence_product_finding_value(run_id, freeze_id, "pob-only")
            .expect("route finding");
        assert_eq!(routed["classification"], "product_finding");
        assert_eq!(
            routed["selective_replay_obligation_ids"],
            json!(["pob-only"])
        );
        assert_eq!(
            repository.source_freeze(freeze_id).unwrap().status,
            SourceFreezeStatus::Invalidated
        );
        let repair = app
            .repair_work_packet_value("plan-a")
            .expect("repair")
            .expect("packet");
        assert_eq!(repair["work_packet"]["mode"], "product_finding_repair");
        assert_eq!(
            repair["work_packet"]["selective_replay_obligation_ids"],
            json!(["pob-only"])
        );
    }

    #[test]
    fn source_frozen_final_review_repair_retry_refreezes_once() {
        let (root, app) = test_app();
        add_outcome(&app, "item-refreeze-retry");
        app.outcome_work_packet("item-refreeze-retry")
            .expect("feature run");
        let settled = app
            .settle_feature_run_outcome(OutcomeSettlement {
                item_id: "item-refreeze-retry",
                summary: "settled refreeze retry fixture",
                materiality: &materiality(false),
                escalation: None,
            })
            .expect("settle ordinary outcome");
        app.close_item_core(
            "item-refreeze-retry",
            "settled refreeze retry fixture",
            false,
        )
        .expect("close ordinary outcome");
        app.freeze_feature_run_source_value("plan-a")
            .expect("freeze")
            .expect("frozen run");
        let run_id = settled["run_id"].as_str().unwrap();
        let repository = ExecutionRunRepository::new(&app.conn);
        let old_freeze = repository.active_source_freeze(run_id).unwrap().unwrap();
        let gate = ReviewGateRecord {
            id: "gate-refreeze-retry".into(),
            run_id: run_id.into(),
            scope_kind: ReviewScopeKind::Plan,
            scope_id: "plan-a".into(),
            kind: ReviewGateKind::FinalProduct,
            status: ReviewGateStatus::Pending,
            required_risk: None,
            responsible_maker_id: worker_id(),
            latest_attempt: 0,
            source_revision: Some(old_freeze.source_revision.clone()),
        };
        repository.create_review_gate(&gate).unwrap();
        let finding = FindingRecord {
            id: "finding-refreeze-retry".into(),
            gate_id: gate.id.clone(),
            attempt_id: "attempt-refreeze-retry".into(),
            severity: "high".into(),
            target: "plan-a".into(),
            owner_worker_id: worker_id(),
            status: FindingStatus::Resolved,
            invalidated_evidence_ids: Vec::new(),
        };
        repository
            .append_review_attempt(
                &ReviewAttemptRecord {
                    id: finding.attempt_id.clone(),
                    gate_id: gate.id.clone(),
                    attempt_number: 1,
                    reviewer_worker_id: "reviewer-a".into(),
                    reviewer_mode: "independent".into(),
                    verdict: ReviewVerdict::ChangesRequested,
                    source_revision: old_freeze.source_revision.clone(),
                    artifacts: Vec::new(),
                },
                std::slice::from_ref(&finding),
                0,
            )
            .unwrap();
        let gate_before = repository.review_gate(&gate.id).unwrap();
        let findings_before = repository.findings(&gate.id).unwrap();
        fs::write(root.path().join("plan-a.md"), "# Repaired source\n").unwrap();
        let frozen_run = repository.feature_run(run_id).unwrap();
        app.reconcile_final_review_repair_source(&repository, &frozen_run, &gate_before)
            .unwrap();
        let replacement = repository.active_source_freeze(run_id).unwrap().unwrap();
        assert_ne!(replacement.id, old_freeze.id);
        assert_eq!(
            repository.source_freeze(&old_freeze.id).unwrap().status,
            SourceFreezeStatus::Invalidated
        );
        let after = repository.feature_run(run_id).unwrap();
        assert_eq!(after.run.phase, FeatureRunPhase::SourceFrozen);
        assert_eq!(
            after.run.source_revision.as_deref(),
            Some(replacement.source_revision.as_str())
        );
        assert_eq!(repository.review_gate(&gate.id).unwrap(), gate_before);
        assert_eq!(repository.findings(&gate.id).unwrap(), findings_before);
        let freeze_counts = || {
            app.conn.query_row(
            "SELECT COUNT(*), SUM(CASE WHEN status = 'active' THEN 1 ELSE 0 END) FROM feature_run_source_freezes WHERE run_id = ?1",
            [run_id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        ).unwrap()
        };
        assert_eq!(freeze_counts(), (2, 1));
        app.reconcile_final_review_repair_source(&repository, &after, &gate_before)
            .unwrap();
        assert_eq!(
            repository.active_source_freeze(run_id).unwrap().unwrap(),
            replacement
        );
        assert_eq!(repository.feature_run(run_id).unwrap(), after);
        assert_eq!(freeze_counts(), (2, 1));
    }
}
