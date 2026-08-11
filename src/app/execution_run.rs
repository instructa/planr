use super::App;
use super::feature_run_evidence::{BudgetUsageReport, FeatureRunBudgetAdmission};
use super::repository::execution_run::{
    EvidenceInvalidationRecord, ExecutionRunRepository, FindingRecord, FindingStatus,
    PersistedFeatureRun, ReviewAttemptRecord, ReviewGateKind, ReviewGateRecord, ReviewGateStatus,
    ReviewScopeKind, ReviewSourceBindingRecord, ReviewVerdict, RunOutcomeRecord,
    SourceFreezeRecord, SourceFreezeStatus,
};
use crate::canonical_json::sha256_json_digest;
use crate::cli::{RunBatchCommand, RunCommand};
use crate::evidence::coverage::evaluate_plan_coverage;
use crate::evidence::policy::capture_repository_snapshot;
use crate::execution_run::{
    DEFAULT_BATCH_OUTCOME_CAP, ExecutionBatch, ExecutionBatchStatus, FeatureRun, FeatureRunPhase,
    FeatureRunRestartDisposition, FeatureRunRestartReason, FeatureRunRestartRequest,
    FeatureRunStatus, MakerReplacement, MakerReplacementReason, PhaseTransition,
    PhaseTransitionCause, RoleOwner, RunRole, apply_phase_transition, pause_batch_for_risk_review,
    replace_batch_maker, resume_batch_after_risk_review, retire_incompatible_feature_run,
    roll_batch_for_same_maker,
};
use crate::usage_policy::{
    BudgetPhase, FeatureRunBudgetContract, PolicyLoad, ReviewEscalation, ReviewInterruptDecision,
    ReviewInterruptRequest, admit_review_interrupt, feature_run_budget_contract_from_policy,
    load_policy, preview_policy_upgrade,
};
use crate::util::{short_id, worker_id};
use anyhow::{Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Clone, Debug)]
pub(crate) struct OutcomeSettlement<'a> {
    pub(crate) item_id: &'a str,
    pub(crate) summary: &'a str,
    pub(crate) materiality: &'a Value,
    pub(crate) escalation: Option<ReviewEscalation>,
}

impl App {
    fn capture_final_review_source_binding(
        &self,
        gate_id: &str,
        run_id: &str,
        plan_id: &str,
    ) -> Result<ReviewSourceBindingRecord> {
        let repository = ExecutionRunRepository::new(&self.conn);
        let freeze = repository.active_source_freeze(run_id)?.ok_or_else(|| {
            anyhow!("final_product_review_requires_active_source_freeze:{plan_id}")
        })?;
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("final_product_review_source_capture_failed:{error}"))?;
        if snapshot.source.revision != freeze.source_revision
            || snapshot.source.tree_digest.as_str() != freeze.source_digest
        {
            bail!("final_product_review_source_freeze_stale:{plan_id}");
        }
        let project = self.default_project()?;
        let has_binding_obligations = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM proof_obligations WHERE project_id = ?1 AND plan_id = ?2 AND binding = 1)",
            params![project.id, plan_id],
            |row| row.get::<_, bool>(0),
        )?;
        let receipt_lineage = if has_binding_obligations {
            let evaluated_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
            let coverage = evaluate_plan_coverage(&self.conn, &project.id, plan_id, &evaluated_at)
                .map_err(|error| anyhow!("{error}"))?;
            if coverage.status.as_str() != "satisfied" {
                bail!(
                    "final_product_review_requires_satisfied_exact_source_coverage:{plan_id}:{}",
                    coverage.status.as_str()
                );
            }
            coverage.receipt_lineage
        } else {
            json!([])
        };
        Ok(ReviewSourceBindingRecord {
            gate_id: gate_id.to_string(),
            freeze_id: freeze.id,
            source_revision: freeze.source_revision,
            source_digest: freeze.source_digest,
            receipt_lineage,
        })
    }

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
                };
                let value = self.restart_feature_run_value(&args.plan, reason)?;
                self.emit(value, "incompatible feature run retired".to_string())
            }
            RunCommand::ResolveBudgetHold(args) => {
                let value = self.resolve_feature_run_budget_hold_value(&args.plan)?;
                self.emit(value, "feature run budget hold resolved".to_string())
            }
            RunCommand::SettleRepair(args) => {
                let value = self.settle_product_finding_repair_value(
                    &args.plan,
                    &args.invalidation,
                    &args.summary,
                    &args.files,
                    &args.cmd,
                    &args.tests,
                )?;
                self.emit(
                    value,
                    "product finding repair settled and source refrozen".to_string(),
                )
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
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let Some(persisted) = repository.active_feature_run_for_plan(&project.id, plan_id)? else {
            let mut previous = repository
                .latest_incompatible_feature_run_restart(&project.id, plan_id)?
                .ok_or_else(|| anyhow!("feature_run_not_found_for_plan:{plan_id}"))?;
            previous.disposition = FeatureRunRestartDisposition::AlreadyRetired;
            let execution_state =
                self.canonical_execution_state_value(&previous.retired_run.id, None)?;
            return Ok(json!({
                "schema_version": "planr.feature_run_restart.v1",
                "restart": previous,
                "execution_state": execution_state,
            }));
        };
        let compatibility = repository.budget_contract_compatibility(&persisted.run.id)?;
        let request = FeatureRunRestartRequest {
            plan_id: plan_id.to_string(),
            reason,
        };
        let transition =
            retire_incompatible_feature_run(&persisted.run, &request, compatibility)
                .map_err(|violation| anyhow!("feature_run_restart_rejected:{violation:?}"))?;
        repository.retire_incompatible_feature_run(
            &transition,
            persisted.revision,
            &worker_id(),
        )?;
        let execution_state = self.canonical_execution_state_value(&persisted.run.id, None)?;
        Ok(json!({
            "schema_version": "planr.feature_run_restart.v1",
            "restart": transition,
            "execution_state": execution_state,
        }))
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

    pub(crate) fn settle_feature_run_outcome(&self, input: OutcomeSettlement<'_>) -> Result<Value> {
        let Some(persisted) = self.ensure_outcome_feature_run(input.item_id)? else {
            return Ok(json!({"kind": "outcome", "transition": "legacy_unplanned"}));
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
        let escalation = input
            .escalation
            .map(|escalation| {
                match admit_review_interrupt(&ReviewInterruptRequest::StructuredEscalation {
                    escalation: escalation.clone(),
                }) {
                    ReviewInterruptDecision::OpenCheckpoint { escalation } => Ok(escalation),
                    decision => bail!("review_escalation_rejected:{decision:?}"),
                }
            })
            .transpose()?;
        let checkpoint_required = review_required || escalation.is_some();
        let batch_id = persisted
            .run
            .active_batch_id
            .clone()
            .ok_or_else(|| anyhow!("feature_run_missing_active_batch:{}", persisted.run.id))?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let batch = repository.batch(&batch_id)?;
        if batch.batch.maker_worker_id != maker.worker_id {
            bail!("feature_run_batch_maker_mismatch:{}", persisted.run.id);
        }
        let ordinal = persisted.run.batch_outcome_count + 1;
        let next_revision = repository.record_outcome(
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
        Ok(json!({
            "kind": "outcome",
            "run_id": persisted.run.id,
            "batch_id": batch_id,
            "run_revision": next_revision,
            "batch_outcome_count": ordinal,
            "transition": transition,
            "review_gate": gate,
            "execution_state": execution_state,
        }))
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
            if gate.kind == ReviewGateKind::FinalProduct
                && persisted.run.phase == FeatureRunPhase::Verification
                && let Some(active) = repository.active_source_freeze(&gate.run_id)?
            {
                let snapshot = capture_repository_snapshot(&self.root)
                    .map_err(|error| anyhow!("final_review_repair_source_capture:{error}"))?;
                if snapshot.source.revision != active.source_revision
                    || snapshot.source.tree_digest.as_str() != active.source_digest
                {
                    self.reconcile_active_phase_wall(&gate.run_id, BudgetPhase::Repair)?;
                    self.reconcile_active_phase_wall(&gate.run_id, BudgetPhase::Verification)?;
                    let obligation_ids = self
                        .conn
                        .prepare("SELECT id FROM proof_obligations WHERE plan_id = ?1 AND binding = 1 ORDER BY id")?
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
                    let mut refrozen = apply_phase_transition(
                        &persisted.run,
                        &PhaseTransition {
                            to: FeatureRunPhase::SourceFrozen,
                            cause: PhaseTransitionCause::SourceInvalidated,
                            reference: format!("source_freeze:{}", replacement.id),
                            owner: None,
                        },
                    )
                    .map_err(|violation| anyhow!("final_review_repair_refreeze:{violation:?}"))?;
                    refrozen.source_revision = Some(replacement.source_revision);
                    repository.save_feature_run(&refrozen, persisted.revision)?;
                }
            }
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

    pub(crate) fn ensure_final_product_review_gate_value(&self, plan_id: &str) -> Result<Value> {
        let plan = self.get_plan(plan_id)?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let run_id = self
            .canonical_execution_run_id_for_plan(plan_id)?
            .ok_or_else(|| anyhow!("feature_run_not_found_for_plan:{plan_id}"))?;
        if let Some(mut gate) = repository
            .review_gates_for_run(&run_id, false)?
            .into_iter()
            .find(|gate| {
                gate.kind == ReviewGateKind::FinalProduct
                    && gate.scope_kind == ReviewScopeKind::Plan
                    && gate.scope_id == plan_id
            })
        {
            if gate.status == ReviewGateStatus::ChangesRequested
                && !repository
                    .findings(&gate.id)?
                    .iter()
                    .any(|finding| finding.status == FindingStatus::Open)
            {
                let binding = self.capture_final_review_source_binding(
                    &gate.id,
                    &gate.run_id,
                    &gate.scope_id,
                )?;
                self.conn.execute_batch(
                    "BEGIN IMMEDIATE; SAVEPOINT reopen_repaired_final_review_gate",
                )?;
                let reopen = (|| -> Result<()> {
                    repository.rebind_review_gate_source(&binding)?;
                    repository.set_review_gate_status(
                        &gate.id,
                        ReviewGateStatus::ChangesRequested,
                        ReviewGateStatus::Pending,
                    )?;
                    Ok(())
                })();
                match reopen {
                    Ok(()) => self
                        .conn
                        .execute_batch("RELEASE reopen_repaired_final_review_gate; COMMIT")?,
                    Err(error) => {
                        let _ = self.conn.execute_batch("ROLLBACK TO reopen_repaired_final_review_gate; RELEASE reopen_repaired_final_review_gate; ROLLBACK");
                        return Err(error);
                    }
                }
                gate = repository.review_gate(&gate.id)?;
            }
            return Ok(
                json!({"plan": plan, "execution_state": self.canonical_execution_state_value(&gate.run_id, Some(&gate.id))?, "created": false}),
            );
        }
        let run = repository.feature_run(&run_id)?;
        if run.run.phase != FeatureRunPhase::Verification {
            let phase = serde_json::to_value(run.run.phase)?
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            bail!(
                "final_product_review_requires_verification_phase:phase={}: run `planr pick --plan {} --work-type verification --json`",
                phase,
                plan_id
            );
        }
        let verification_item: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT id, status FROM items
                 WHERE project_id = ?1
                   AND plan_path = ?2
                   AND work_type = 'verification'
                 ORDER BY priority DESC, created_at
                 LIMIT 1",
                params![self.default_project()?.id, plan.path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((item_id, status)) = verification_item {
            if status != "closed" {
                bail!("final_product_review_requires_closed_verification_item:{item_id}");
            }
            let evaluated_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
            let project = self.default_project()?;
            let coverage = evaluate_plan_coverage(&self.conn, &project.id, plan_id, &evaluated_at)
                .map_err(|err| anyhow!("{err}"))?;
            if coverage.status.as_str() != "satisfied" {
                bail!(
                    "final_product_review_requires_satisfied_exact_source_coverage:{plan_id}:{}",
                    coverage.status.as_str()
                );
            }
        }
        let responsible_maker_id = run
            .run
            .role_owners
            .iter()
            .find(|owner| owner.role == RunRole::Maker)
            .map(|owner| owner.worker_id.clone())
            .or_else(|| {
                self.conn
                    .query_row(
                        "SELECT worker_id FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'maker' ORDER BY lease_generation DESC LIMIT 1",
                        [&run.run.id],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
            })
            .ok_or_else(|| anyhow!("final_product_review_missing_responsible_maker:{plan_id}"))?;
        let gate_id = short_id("gate");
        let binding = self.capture_final_review_source_binding(&gate_id, &run.run.id, plan_id)?;
        let gate = ReviewGateRecord {
            id: gate_id,
            run_id: run.run.id,
            scope_kind: ReviewScopeKind::Plan,
            scope_id: plan_id.to_string(),
            kind: ReviewGateKind::FinalProduct,
            status: ReviewGateStatus::Pending,
            required_risk: None,
            responsible_maker_id,
            latest_attempt: 0,
            source_revision: Some(binding.source_revision.clone()),
        };
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT create_final_review_gate")?;
        let create_result = (|| -> Result<()> {
            repository.create_review_gate(&gate)?;
            repository.create_review_source_binding(&binding)?;
            Ok(())
        })();
        match create_result {
            Ok(()) => self
                .conn
                .execute_batch("RELEASE create_final_review_gate; COMMIT")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK TO create_final_review_gate; RELEASE create_final_review_gate; ROLLBACK");
                return Err(error);
            }
        }
        self.record_event(
            "final_review_opened",
            None,
            json!({"plan_id": plan_id, "gate_id": gate.id, "run_id": gate.run_id}),
        )?;
        Ok(
            json!({"plan": plan, "execution_state": self.canonical_execution_state_value(&gate.run_id, Some(&gate.id))?, "created": true}),
        )
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
        if let Some(stored) = repository.review_source_binding(gate_id)? {
            let freeze = repository
                .active_source_freeze(&gate.run_id)?
                .ok_or_else(|| anyhow!("review_source_binding_missing_active_freeze:{gate_id}"))?;
            let snapshot = capture_repository_snapshot(&self.root)
                .map_err(|error| anyhow!("review_source_binding_capture_failed:{error}"))?;
            if freeze.id != stored.freeze_id
                || freeze.source_revision != stored.source_revision
                || freeze.source_digest != stored.source_digest
                || snapshot.source.revision != stored.source_revision
                || snapshot.source.tree_digest.as_str() != stored.source_digest
                || gate.source_revision.as_deref() != Some(stored.source_revision.as_str())
            {
                bail!("review_source_binding_source_freeze_stale:{gate_id}");
            }
        } else if gate.kind == ReviewGateKind::FinalProduct {
            bail!("final_product_review_source_binding_missing:{gate_id}");
        }
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
                if let Some(handoff) = self
                    .accepted_risk_verification_handoff_locked(persisted.clone(), &accepted_gate)?
                {
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
    use super::super::repository::execution_run::{SourceFreezeRecord, SourceFreezeStatus};
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

    fn test_app() -> App {
        test_app_at(PathBuf::from("."))
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
        add_ready_verification(&app, "item-risk-verification");
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
        app.review_gate_pick_value_for_worker("plan-a", false, "checker-risk")
            .expect("review pick")
            .expect("review packet");
        let accepted = app
            .complete_review_gate_value(gate_id, ReviewVerdict::Accepted, &[], Some("checker-risk"))
            .expect("accepted review");
        let handoff = &accepted["verification_handoff"];
        assert_eq!(handoff["reason"], "verification_handoff_source_frozen");
        assert_eq!(
            handoff["work_packet"]["verification_item_id"],
            "item-risk-verification"
        );
        assert_eq!(accepted["execution_state"]["phase"], "source_frozen");
        assert!(accepted["execution_state"]["owner"].is_null());
        assert_eq!(
            accepted["execution_state"]["execution_batch"]["status"],
            "ended"
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
        let app = test_app();
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

        let healthy = test_app();
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
    }

    #[test]
    fn capped_batch_rolls_to_fourth_outcome_with_same_maker_and_no_review_artifacts() {
        let app = test_app();
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
        let app = test_app();
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
        let app = test_app();
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
        let app = test_app();
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
        let app = test_app();
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
    fn normal_feature_has_one_current_independent_final_product_gate() {
        let app = test_app();
        add_outcome(&app, "item-final");
        let persisted = app
            .ensure_outcome_feature_run("item-final")
            .expect("ensure run")
            .expect("run");
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
        let revision = repository
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
        let premature = app
            .ensure_final_product_review_gate_value("plan-a")
            .expect_err("source-frozen run must require verification first");
        assert_eq!(
            premature.to_string(),
            "final_product_review_requires_verification_phase:phase=source_frozen: run `planr pick --plan plan-a --work-type verification --json`"
        );
        assert!(
            repository
                .review_gates_for_run(&frozen.id, false)
                .expect("review gates")
                .is_empty(),
            "premature final-review admission must not create a gate"
        );
        let verification = apply_phase_transition(
            &frozen,
            &PhaseTransition {
                to: FeatureRunPhase::Verification,
                cause: PhaseTransitionCause::VerificationStarted,
                reference: "verification:ready".into(),
                owner: Some(RoleOwner {
                    role: RunRole::Verifier,
                    worker_id: "verifier-a".into(),
                    lease_generation: 1,
                }),
            },
        )
        .expect("verification transition");
        repository
            .save_feature_run(&verification, revision)
            .expect("save verification");
        let verification_packet = app
            .verification_work_packet_value("plan-a", true)
            .expect("verification packet")
            .expect("verification ready");
        assert_eq!(verification_packet["work_packet"]["kind"], "verification");

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
    fn product_finding_invalidates_only_affected_evidence_and_routes_last_maker() {
        let app = test_app();
        add_outcome(&app, "item-product-finding");
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at)
                 VALUES ('verification-product-finding', 'project-a', 'verify', 'verify', 'running', 'verification', 'verifier-a', 'plan-a.md', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
        app.conn
            .execute(
                "INSERT INTO proof_obligations(
                   id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                   binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                   assurance_policy_json, retry_aggregation, policy_digest, config_digest,
                   source_digest, created_at, obligation_shape
                 ) VALUES (
                   'pob-only', 'project-a', 'plan-a', 'verification-product-finding', 'crit-only', 1,
                   'only affected', 1, '[]', '{}', '{}', '{}', 'latest_applicable_pass',
                   'sha256:0000000000000000000000000000000000000000000000000000000000000000',
                   'sha256:0000000000000000000000000000000000000000000000000000000000000000',
                   'sha256:0000000000000000000000000000000000000000000000000000000000000000',
                   datetime('now'), 'semantic_v1'
                 )",
                [],
            )
            .unwrap();
        app.ensure_outcome_feature_run("item-product-finding")
            .expect("ensure run")
            .expect("feature run");
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
}
