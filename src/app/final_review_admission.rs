use super::App;
use super::repository::execution_run::{
    ExecutionRunRepository, FindingStatus, ReviewGateKind, ReviewGateRecord, ReviewGateStatus,
    ReviewScopeKind, ReviewSourceBindingRecord,
};
use crate::evidence::coverage::evaluate_plan_coverage;
use crate::evidence::policy::capture_repository_snapshot;
use crate::execution_run::{FeatureRunPhase, RunRole};
use crate::util::short_id;
use anyhow::{Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

impl App {
    pub(crate) fn capture_final_review_source_binding(
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
        let receipt_lineage = if self.plan_has_active_binding_obligations(plan_id)? {
            let project = self.default_project()?;
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
                self.conn.execute_batch(
                    "BEGIN IMMEDIATE; SAVEPOINT reopen_repaired_final_review_gate",
                )?;
                let reopen = (|| -> Result<()> {
                    let binding = self.capture_final_review_source_binding(
                        &gate.id,
                        &gate.run_id,
                        &gate.scope_id,
                    )?;
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
            return Ok(json!({
                "plan": plan,
                "execution_state": self.canonical_execution_state_value(&gate.run_id, Some(&gate.id))?,
                "created": false,
            }));
        }

        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT create_final_review_gate")?;
        let create_result = (|| -> Result<Value> {
            let repository = ExecutionRunRepository::new(&self.conn);
            if let Some(gate) = repository
                .review_gates_for_run(&run_id, false)?
                .into_iter()
                .find(|gate| {
                    gate.kind == ReviewGateKind::FinalProduct
                        && gate.scope_kind == ReviewScopeKind::Plan
                        && gate.scope_id == plan_id
                })
            {
                return Ok(json!({
                    "plan": plan,
                    "execution_state": self.canonical_execution_state_value(&gate.run_id, Some(&gate.id))?,
                    "created": false,
                }));
            }
            let run = repository.feature_run(&run_id)?;
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
            let verification_closed = verification_item
                .as_ref()
                .is_some_and(|(_, status)| status == "closed");
            let active_binding = self.plan_has_active_binding_obligations(plan_id)?;
            let legacy_nonbinding = !active_binding;
            if run.run.phase != FeatureRunPhase::Verification
                && !(run.run.phase == FeatureRunPhase::SourceFrozen
                    && (verification_closed || legacy_nonbinding))
            {
                bail!(
                    "final_product_review_requires_verification_phase:phase={}: run `planr pick --plan {} --work-type verification --json`",
                    serde_json::to_string(&run.run.phase)?.trim_matches('"'),
                    plan_id
                );
            }
            if legacy_nonbinding && run.run.phase == FeatureRunPhase::SourceFrozen {
                self.refresh_legacy_final_review_source_freeze(plan_id, &run.run.id)?;
            }
            if active_binding
                && let Some((item_id, status)) = verification_item
                && status != "closed"
            {
                bail!("final_product_review_requires_closed_verification_item:{item_id}");
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
                .ok_or_else(|| {
                    anyhow!("final_product_review_missing_responsible_maker:{plan_id}")
                })?;
            let gate_id = short_id("gate");
            let binding =
                self.capture_final_review_source_binding(&gate_id, &run.run.id, plan_id)?;
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
            repository.create_review_gate(&gate)?;
            repository.create_review_source_binding(&binding)?;
            self.record_event(
                "final_review_opened",
                None,
                json!({"plan_id": plan_id, "gate_id": gate.id, "run_id": gate.run_id}),
            )?;
            Ok(json!({
                "plan": plan,
                "execution_state": self.canonical_execution_state_value(&gate.run_id, Some(&gate.id))?,
                "created": true,
            }))
        })();
        match create_result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE create_final_review_gate; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO create_final_review_gate; RELEASE create_final_review_gate; ROLLBACK",
                );
                Err(error)
            }
        }
    }
}
