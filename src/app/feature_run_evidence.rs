//! FeatureRun coordination at the Evidence and Usage Policy boundaries.

use super::App;
use super::repository::execution_run::{
    BudgetObservationRecord, EvidenceInvalidationRecord, ExecutionRunRepository,
    PersistedFeatureRun, SourceFreezeRecord, SourceFreezeStatus,
};
use crate::evidence::policy::capture_repository_snapshot;
use crate::execution_run::{
    ExecutionBatch, ExecutionBatchStatus, FeatureRunHoldReason, FeatureRunPhase, PhaseTransition,
    PhaseTransitionCause, RoleOwner, RunRole, apply_phase_transition,
};
use crate::usage_policy::{
    BudgetDimension, BudgetPhase, MeteringMode, PolicyLoad, RiskLevel, TransitionState,
    UsageObservation, assess_budgets, first_exhausted_budget, load_policy,
};
use crate::util::{short_id, worker_id};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct FeatureRunBudgetReservation {
    pub(crate) id: String,
    pub(crate) run_id: String,
    pub(crate) phase: BudgetPhase,
    pub(crate) boundary_key: String,
    pub(crate) owns_wall: bool,
    pub(crate) started_at_unix_ms: u64,
}

pub(crate) enum FeatureRunBudgetAdmission {
    Reserved(FeatureRunBudgetReservation),
    Held(Value),
}

pub(crate) const HUMAN_PHASE_WALL_ALLOWANCE_SECONDS: u64 = 5;

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

    // Keep all reservation dimensions explicit at the transaction boundary; callers must not
    // silently inherit wall ownership, projected usage, or provenance defaults.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_feature_run_budget(
        &self,
        persisted: &PersistedFeatureRun,
        phase: BudgetPhase,
        boundary_key: &str,
        projected_wall_seconds: Option<u64>,
        projected_tool_calls: Option<u64>,
        owns_wall: bool,
        provenance: &str,
    ) -> Result<FeatureRunBudgetAdmission> {
        if boundary_key.trim().is_empty() || provenance.trim().is_empty() {
            bail!("budget reservation boundary and provenance must be non-empty");
        }
        if projected_wall_seconds == Some(0) || projected_tool_calls == Some(0) {
            bail!("budget reservation projections must be positive when observed");
        }
        let owns_transaction = self.conn.is_autocommit();
        self.conn.execute_batch(if owns_transaction {
            "BEGIN IMMEDIATE"
        } else {
            "SAVEPOINT admit_feature_run_budget"
        })?;
        let result = (|| -> Result<FeatureRunBudgetAdmission> {
            if let Some(existing) =
                self.load_active_budget_reservation(&persisted.run.id, boundary_key)?
            {
                if let Some(hold) = self.feature_run_budget_hold_projected(
                    persisted,
                    phase,
                    projected_wall_seconds,
                    projected_tool_calls,
                )? {
                    return Ok(FeatureRunBudgetAdmission::Held(hold));
                }
                return Ok(FeatureRunBudgetAdmission::Reserved(existing));
            }
            if let Some(hold) = self.feature_run_budget_hold_projected(
                persisted,
                phase,
                projected_wall_seconds,
                projected_tool_calls,
            )? {
                return Ok(FeatureRunBudgetAdmission::Held(hold));
            }
            let reservation = FeatureRunBudgetReservation {
                id: short_id("budget-reservation"),
                run_id: persisted.run.id.clone(),
                phase,
                boundary_key: boundary_key.to_string(),
                owns_wall,
                started_at_unix_ms: unix_time_ms()?,
            };
            self.conn.execute(
                "INSERT INTO feature_run_budget_reservations(id, run_id, phase, boundary_key, status, reserved_wall_seconds, reserved_tokens, reserved_tool_calls, owns_wall, started_at_unix_ms, provenance) VALUES (?1, ?2, ?3, ?4, 'active', ?5, NULL, ?6, ?7, ?8, ?9)",
                rusqlite::params![reservation.id, reservation.run_id, budget_phase_name(phase), reservation.boundary_key, projected_wall_seconds, projected_tool_calls, owns_wall, reservation.started_at_unix_ms, provenance],
            )?;
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
        actual_tool_calls: Option<u64>,
    ) -> Result<()> {
        let owns_transaction = self.conn.is_autocommit();
        self.conn.execute_batch(if owns_transaction {
            "BEGIN IMMEDIATE"
        } else {
            "SAVEPOINT reconcile_feature_run_budget"
        })?;
        let result = (|| -> Result<()> {
            let status: String = self.conn.query_row(
                "SELECT status FROM feature_run_budget_reservations WHERE id = ?1 AND run_id = ?2",
                rusqlite::params![reservation.id, reservation.run_id],
                |row| row.get(0),
            )?;
            if status == "reconciled" {
                return Ok(());
            }
            if status != "active" {
                bail!("budget_reservation_not_active:{}", reservation.id);
            }
            let actual_wall_seconds = if reservation.owns_wall {
                Some(
                    (unix_time_ms()?
                        .saturating_sub(reservation.started_at_unix_ms)
                        .saturating_add(999)
                        / 1000)
                        .max(1),
                )
            } else {
                None
            };
            let repository = ExecutionRunRepository::new(&self.conn);
            let observations = repository.budget_observations(&reservation.run_id)?;
            let maximum = |get: fn(&BudgetObservationRecord) -> Option<u64>| {
                observations.iter().filter_map(get).max().unwrap_or(0)
            };
            let provenance: String = self.conn.query_row(
                "SELECT provenance FROM feature_run_budget_reservations WHERE id = ?1",
                [&reservation.id],
                |row| row.get(0),
            )?;
            repository.record_budget_observation(&BudgetObservationRecord {
                id: short_id("budget"),
                run_id: reservation.run_id.clone(),
                phase: reservation.phase,
                metering: MeteringMode::Trusted,
                wall_seconds: actual_wall_seconds
                    .map(|value| maximum(|observation| observation.wall_seconds) + value),
                tokens: None,
                tool_calls: actual_tool_calls
                    .map(|value| maximum(|observation| observation.tool_calls) + value),
                credits_micros: None,
                provenance: format!("{provenance};tokens=unavailable"),
            })?;
            self.conn.execute(
                "UPDATE feature_run_budget_reservations SET status = 'reconciled', finished_at = datetime('now') WHERE id = ?1 AND status = 'active'",
                [&reservation.id],
            )?;
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
        self.conn.execute(
            "UPDATE feature_run_budget_reservations SET status = 'released', finished_at = datetime('now') WHERE id = ?1 AND run_id = ?2 AND status = 'active'",
            rusqlite::params![reservation.id, reservation.run_id],
        )?;
        Ok(())
    }

    pub(crate) fn load_active_budget_reservation(
        &self,
        run_id: &str,
        boundary_key: &str,
    ) -> Result<Option<FeatureRunBudgetReservation>> {
        let mut statement = self.conn.prepare(
            "SELECT id, phase, owns_wall, started_at_unix_ms FROM feature_run_budget_reservations WHERE run_id = ?1 AND boundary_key = ?2 AND status = 'active'",
        )?;
        let mut rows = statement.query(rusqlite::params![run_id, boundary_key])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(FeatureRunBudgetReservation {
            id: row.get(0)?,
            run_id: run_id.to_string(),
            phase: parse_budget_phase_name(&row.get::<_, String>(1)?)?,
            boundary_key: boundary_key.to_string(),
            owns_wall: row.get(2)?,
            started_at_unix_ms: row.get(3)?,
        }))
    }

    pub(crate) fn reconcile_active_phase_wall(
        &self,
        run_id: &str,
        phase: BudgetPhase,
    ) -> Result<()> {
        let mut statement = self.conn.prepare(
            "SELECT id, boundary_key, started_at_unix_ms FROM feature_run_budget_reservations WHERE run_id = ?1 AND phase = ?2 AND status = 'active' AND owns_wall = 1 ORDER BY id",
        )?;
        let rows =
            statement.query_map(rusqlite::params![run_id, budget_phase_name(phase)], |row| {
                Ok(FeatureRunBudgetReservation {
                    id: row.get(0)?,
                    run_id: run_id.to_string(),
                    phase,
                    boundary_key: row.get(1)?,
                    owns_wall: true,
                    started_at_unix_ms: row.get(2)?,
                })
            })?;
        let reservations = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        for reservation in reservations {
            self.reconcile_feature_run_budget(&reservation, Some(1))?;
        }
        Ok(())
    }

    pub(crate) fn continue_review_budget(
        &self,
        run: &PersistedFeatureRun,
        gate_id: &str,
    ) -> Result<Option<Value>> {
        match self.admit_feature_run_budget(
            run,
            BudgetPhase::Review,
            &format!("review:{gate_id}"),
            Some(HUMAN_PHASE_WALL_ALLOWANCE_SECONDS),
            None,
            true,
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
            && run.run.phase == FeatureRunPhase::Implementation
        {
            let invalidations = repository.invalidations(&run.run.id)?;
            if let Some(latest) = invalidations.last() {
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
                match self.admit_feature_run_budget(
                    &run,
                    BudgetPhase::Repair,
                    &format!("repair:{}", run.run.id),
                    Some(HUMAN_PHASE_WALL_ALLOWANCE_SECONDS),
                    Some(1),
                    true,
                    "repair.dispatch",
                )? {
                    FeatureRunBudgetAdmission::Held(hold) => return Ok(Some(hold)),
                    FeatureRunBudgetAdmission::Reserved(_) => {}
                }
                let replay_obligation_ids = latest
                    .affected_evidence_ids
                    .first()
                    .cloned()
                    .into_iter()
                    .collect::<Vec<_>>();
                return Ok(Some(json!({
                    "work_packet": {"kind": "outcome", "mode": "product_finding_repair", "execution_state": self.canonical_execution_state_value(&run.run.id, None)?,
                        "responsible_maker_id": maker_worker_id, "invalidation": latest,
                        "selective_replay_obligation_ids": replay_obligation_ids},
                    "remaining": self.progress_value()?,
                })));
            }
        }
        let Some(gate) = repository.repair_review_gate_for_plan(&project.id, plan_id)? else {
            return Ok(None);
        };
        if gate.responsible_maker_id != worker_id() {
            return Ok(None);
        }
        let persisted = repository.feature_run(&gate.run_id)?;
        match self.admit_feature_run_budget(
            &persisted,
            BudgetPhase::Repair,
            &format!("repair:{}", persisted.run.id),
            Some(HUMAN_PHASE_WALL_ALLOWANCE_SECONDS),
            Some(1),
            true,
            "repair.dispatch",
        )? {
            FeatureRunBudgetAdmission::Held(hold) => return Ok(Some(hold)),
            FeatureRunBudgetAdmission::Reserved(_) => {}
        }
        Ok(Some(
            json!({"work_packet": {"kind": "outcome", "mode": "finding_repair", "execution_state": self.canonical_execution_state_value(&persisted.run.id, Some(&gate.id))?}, "remaining": self.progress_value()?}),
        ))
    }

    pub(crate) fn verification_work_packet_value(
        &self,
        plan_id: &str,
        peek: bool,
    ) -> Result<Option<Value>> {
        let project = self.default_project()?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let Some(run) = repository.active_feature_run_for_plan(&project.id, plan_id)? else {
            return Ok(None);
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
                return Ok(Some(
                    json!({"work_packet": {"kind": "verification", "execution_state": self.canonical_execution_state_value(&run.run.id, None)?,
                    "source_freeze": freeze, "verification_lease": {"worker_id": verifier_worker_id, "generation": lease_generation}},
                    "peek": true, "remaining": self.progress_value()?}),
                ));
            }
            let reservation = match self.admit_feature_run_budget(
                &run,
                BudgetPhase::Verification,
                &format!("verification:{}", run.run.id),
                Some(HUMAN_PHASE_WALL_ALLOWANCE_SECONDS),
                Some(1),
                true,
                "verification.dispatch",
            )? {
                FeatureRunBudgetAdmission::Held(hold) => return Ok(Some(hold)),
                FeatureRunBudgetAdmission::Reserved(reservation) => reservation,
            };
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
            if let Err(error) = repository.save_feature_run(&verification, run.revision) {
                self.release_feature_run_budget(&reservation)?;
                return Err(error);
            }
            return Ok(Some(
                json!({"work_packet": {"kind": "verification", "execution_state": self.canonical_execution_state_value(&verification.id, None)?,
                "source_freeze": freeze, "verification_lease": {"worker_id": verifier_worker_id, "generation": lease_generation}},
                "peek": false, "remaining": self.progress_value()?}),
            ));
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
                Some(HUMAN_PHASE_WALL_ALLOWANCE_SECONDS),
                Some(1),
                true,
                "verification.dispatch",
            )? {
                FeatureRunBudgetAdmission::Held(hold) => return Ok(Some(hold)),
                FeatureRunBudgetAdmission::Reserved(_) => {}
            }
        }
        Ok(Some(
            json!({"work_packet": {"kind": "verification", "execution_state": self.canonical_execution_state_value(&run.run.id, None)?,
            "verifier_worker_id": verifier.worker_id, "verification_lease": {"worker_id": verifier.worker_id, "generation": verifier.lease_generation},
            "source_freeze": repository.active_source_freeze(&run.run.id)?}, "peek": peek, "remaining": self.progress_value()?}),
        ))
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
        if let Some(hold) = self.feature_run_budget_hold(&persisted, BudgetPhase::Implementation)? {
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
        let repository = ExecutionRunRepository::new(&self.conn);
        let persisted = repository.feature_run(run_id)?;
        if persisted.run.phase != FeatureRunPhase::Verification {
            bail!("product_finding_requires_verification:{run_id}");
        }
        let maker_worker_id = self.conn.query_row(
            "SELECT worker_id FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'maker' ORDER BY lease_generation DESC LIMIT 1",
            [run_id],
            |row| row.get::<_, String>(0),
        )?;
        let mut affected_evidence_ids = vec![obligation_id.to_string()];
        let mut receipt_statement = self.conn.prepare(
            "SELECT id FROM evidence_receipts WHERE project_id = ?1 AND obligation_id = ?2 AND receipt_status = 'trusted' ORDER BY created_at, id",
        )?;
        let receipt_rows = receipt_statement.query_map(
            rusqlite::params![persisted.project_id, obligation_id],
            |row| row.get::<_, String>(0),
        )?;
        affected_evidence_ids.extend(crate::util::collect_rows(receipt_rows)?);
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
                reference: format!("evidence:{obligation_id}"),
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
            Ok(json!({
                "classification": "product_finding",
                "responsible_maker_id": maker_worker_id,
                "invalidation": invalidation,
                "selective_replay_obligation_ids": [obligation_id],
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
        self.feature_run_budget_hold_projected(persisted, phase, None, None)
    }

    fn feature_run_budget_hold_projected(
        &self,
        persisted: &PersistedFeatureRun,
        phase: BudgetPhase,
        projected_wall_seconds: Option<u64>,
        projected_tool_calls: Option<u64>,
    ) -> Result<Option<Value>> {
        let PolicyLoad::Loaded(policy) = load_policy(&self.root) else {
            return Ok(None);
        };
        let observations =
            ExecutionRunRepository::new(&self.conn).budget_observations(&persisted.run.id)?;
        let trusted = observations
            .iter()
            .filter(|value| value.metering == MeteringMode::Trusted)
            .collect::<Vec<_>>();
        let maximum = |get: fn(&BudgetObservationRecord) -> Option<u64>| {
            trusted.iter().filter_map(|value| get(value)).max()
        };
        let now_ms = unix_time_ms()?;
        let mut statement = self.conn.prepare(
            "SELECT reserved_wall_seconds, reserved_tool_calls, owns_wall, started_at_unix_ms FROM feature_run_budget_reservations WHERE run_id = ?1 AND status = 'active'",
        )?;
        let active = statement.query_map([&persisted.run.id], |row| {
            Ok((
                row.get::<_, Option<u64>>(0)?,
                row.get::<_, Option<u64>>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, u64>(3)?,
            ))
        })?;
        let mut reserved_wall = 0_u64;
        let mut reserved_tools = 0_u64;
        for row in active {
            let (wall, tools, owns_wall, started_at_ms) = row?;
            let elapsed = owns_wall
                .then(|| (now_ms.saturating_sub(started_at_ms).saturating_add(999) / 1000).max(1));
            reserved_wall += wall.unwrap_or(0).max(elapsed.unwrap_or(0));
            reserved_tools += tools.unwrap_or(0);
        }
        let state = TransitionState {
            current_depth: 0,
            projected_active_agents: 1,
            projected_parallel_readers: 0,
            projected_parallel_writers: 0,
            attempts_started: 0,
            same_route_retries: 0,
            availability_fallbacks: 0,
            quality_escalations: 0,
            quota_downgrades: 0,
            elapsed_seconds: maximum(|value| value.wall_seconds).unwrap_or(0)
                + reserved_wall
                + projected_wall_seconds.unwrap_or(0),
            usage: UsageObservation {
                metering: if trusted.is_empty()
                    && reserved_tools + projected_tool_calls.unwrap_or(0) == 0
                {
                    MeteringMode::Unavailable
                } else {
                    MeteringMode::Trusted
                },
                tool_calls: maximum(|value| value.tool_calls)
                    .map(|value| value + reserved_tools + projected_tool_calls.unwrap_or(0))
                    .or_else(|| {
                        let projected = reserved_tools + projected_tool_calls.unwrap_or(0);
                        (projected > 0).then_some(projected)
                    }),
                tokens: maximum(|value| value.tokens),
                credits_micros: maximum(|value| value.credits_micros),
            },
            risk: RiskLevel::Moderate,
            material: true,
            budget_phase: phase,
            pending_safety_stop: None,
        };
        let wall_available = maximum(|value| value.wall_seconds).is_some()
            || reserved_wall > 0
            || projected_wall_seconds.is_some();
        let mut assessments = assess_budgets(&policy, &state);
        if !wall_available
            && let Some(wall) = assessments
                .iter_mut()
                .find(|value| value.dimension == BudgetDimension::WallTime)
        {
            wall.metering = MeteringMode::Unavailable;
            wall.observed = None;
            wall.exhausted = false;
        }
        let Some((reason, message)) = first_exhausted_budget(&assessments) else {
            return Ok(None);
        };
        self.reconcile_active_phase_wall(&persisted.run.id, phase)?;
        let held = apply_phase_transition(
            &persisted.run,
            &PhaseTransition {
                to: FeatureRunPhase::Held,
                cause: PhaseTransitionCause::BudgetHold,
                reference: format!("budget:{reason:?}"),
                owner: None,
            },
        )
        .map_err(|violation| anyhow!("budget_hold_transition:{violation:?}"))?;
        ExecutionRunRepository::new(&self.conn).save_feature_run(&held, persisted.revision)?;
        Ok(Some(json!({
            "work_packet": {"kind": "hold", "classification": "budget", "reason": reason, "message": message, "budget": assessments, "execution_state": self.canonical_execution_state_value(&held.id, None)?},
            "remaining": self.progress_value()?,
        })))
    }
}

fn unix_time_ms() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| anyhow!("system clock before unix epoch: {error}"))?
        .as_millis()
        .try_into()
        .map_err(|_| anyhow!("system clock millisecond value overflow"))
}

fn budget_phase_name(phase: BudgetPhase) -> &'static str {
    match phase {
        BudgetPhase::Implementation => "implementation",
        BudgetPhase::Verification => "verification",
        BudgetPhase::Review => "review",
        BudgetPhase::Repair => "repair",
    }
}

fn parse_budget_phase_name(value: &str) -> Result<BudgetPhase> {
    match value {
        "implementation" => Ok(BudgetPhase::Implementation),
        "verification" => Ok(BudgetPhase::Verification),
        "review" => Ok(BudgetPhase::Review),
        "repair" => Ok(BudgetPhase::Repair),
        _ => bail!("invalid budget reservation phase:{value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::ensure_schema;
    use rusqlite::{Connection, params};
    use std::path::{Path, PathBuf};

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
        app.conn
            .execute(
                "INSERT INTO items(id, project_id, title, description, status, work_type, worker_id, plan_path, created_at, updated_at) VALUES (?1, 'project-a', ?1, 'outcome', 'picked', 'code', ?2, 'plan-a.md', datetime('now'), datetime('now'))",
                params![id, worker_id()],
            )
            .expect("outcome item");
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

    fn age_reservation(app: &App, run_id: &str, boundary_key: &str, seconds: u64) {
        assert_eq!(
            app.conn
                .execute(
                    "UPDATE feature_run_budget_reservations SET started_at_unix_ms = started_at_unix_ms - ?1 WHERE run_id = ?2 AND boundary_key = ?3 AND status = 'active'",
                    rusqlite::params![seconds * 1000, run_id, boundary_key],
                )
                .unwrap(),
            1
        );
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
            "args": ["-c", "printf ready"],
            "working_directory": ".",
            "timeout_ms": 5000,
            "stdout_limit_bytes": 4096,
            "stderr_limit_bytes": 4096,
            "payload_schema": payload_schema
        });
        let adapter_digest = crate::canonical_json::sha256_json_digest(&json!({
            "schema_version": "planr.process_adapter.binding.v1",
            "execution_contract": execution,
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
            "repeatability": "repeatable",
            "independence": "repository-owned test adapter",
            "blind_spots": [],
            "availability_probe": {"kind": "process", "execution": execution}
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

    fn verification_fixture() -> (tempfile::TempDir, App, String, String) {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        initialize_git(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-phase");
        let run = app
            .ensure_outcome_feature_run("item-phase")
            .unwrap()
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
        let packet = app
            .verification_work_packet_value("plan-a", false)
            .unwrap()
            .unwrap();
        assert_eq!(packet["work_packet"]["kind"], "verification");
        (root, app, run.run.id, freeze_id)
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
    fn projected_next_action_holds_at_n_minus_one_for_every_phase_and_evidence_producer() {
        for (phase, threshold, boundary) in [
            (BudgetPhase::Implementation, 60, "implementation.dispatch"),
            (BudgetPhase::Verification, 80, "verification.dispatch"),
            (BudgetPhase::Review, 90, "review.dispatch"),
            (BudgetPhase::Repair, 100, "repair.dispatch"),
            (BudgetPhase::Verification, 80, "evidence.process_adapter"),
            (BudgetPhase::Verification, 80, "evidence.host_capture"),
        ] {
            for dimension in ["wall", "tool"] {
                let root = tempfile::tempdir().unwrap();
                write_budget_policy(root.path());
                let app = test_app(root.path().to_path_buf());
                add_outcome(&app, &format!("item-{boundary}-{dimension}"));
                let persisted = app
                    .ensure_outcome_feature_run(&format!("item-{boundary}-{dimension}"))
                    .expect("ensure run")
                    .expect("run");
                ExecutionRunRepository::new(&app.conn)
                    .record_budget_observation(&BudgetObservationRecord {
                        id: short_id("budget"),
                        run_id: persisted.run.id.clone(),
                        phase,
                        metering: MeteringMode::Trusted,
                        wall_seconds: (dimension == "wall").then_some(threshold - 1),
                        tokens: None,
                        tool_calls: (dimension == "tool").then_some(threshold - 1),
                        credits_micros: None,
                        provenance: format!("test.{boundary}.{dimension};tokens=unavailable"),
                    })
                    .expect("N-1 observation");
                let hold = match app
                    .admit_feature_run_budget(
                        &persisted,
                        phase,
                        &format!("{boundary}:{dimension}"),
                        (dimension == "wall").then_some(1),
                        (dimension == "tool").then_some(1),
                        !boundary.starts_with("evidence."),
                        boundary,
                    )
                    .expect("projected admission")
                {
                    FeatureRunBudgetAdmission::Held(hold) => hold,
                    FeatureRunBudgetAdmission::Reserved(_) => {
                        panic!("next {dimension} action crossed the {phase:?} reserve")
                    }
                };
                let reason = if phase == BudgetPhase::Repair {
                    if dimension == "wall" {
                        "wall_time_budget_exhausted"
                    } else {
                        "tool_call_budget_exhausted"
                    }
                } else {
                    "required_phase_reserve_protected"
                };
                assert_eq!(hold["work_packet"]["reason"], reason);
                let token = hold["work_packet"]["budget"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|value| value["dimension"] == "tokens")
                    .unwrap();
                assert_eq!(token["metering"], "unavailable");
                assert_eq!(token["observed"], Value::Null);
                if dimension == "tool" {
                    let wall = hold["work_packet"]["budget"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|value| value["dimension"] == "wall_time")
                        .unwrap();
                    assert_eq!(wall["metering"], "unavailable");
                    assert_eq!(wall["observed"], Value::Null);
                }
            }
        }
    }

    #[test]
    fn reservation_release_and_reconcile_are_durable_and_never_claim_unobserved_tokens() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-reservation");
        let persisted = app
            .ensure_outcome_feature_run("item-reservation")
            .unwrap()
            .unwrap();
        let no_usage_hold = app
            .feature_run_budget_hold(&persisted, BudgetPhase::Implementation)
            .unwrap();
        assert!(no_usage_hold.is_none());
        let assessments = app
            .feature_run_budget_hold_projected(&persisted, BudgetPhase::Implementation, None, None)
            .unwrap();
        assert!(assessments.is_none());
        let failed = match app
            .admit_feature_run_budget(
                &persisted,
                BudgetPhase::Implementation,
                "failed-dispatch",
                Some(1),
                Some(1),
                true,
                "test.failed_dispatch",
            )
            .unwrap()
        {
            FeatureRunBudgetAdmission::Reserved(value) => value,
            FeatureRunBudgetAdmission::Held(_) => panic!("unexpected hold"),
        };
        app.release_feature_run_budget(&failed).unwrap();
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT COUNT(*) FROM feature_run_budget_reservations WHERE status = 'active'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );
        let successful = match app
            .admit_feature_run_budget(
                &persisted,
                BudgetPhase::Implementation,
                "successful-dispatch",
                Some(1),
                Some(1),
                true,
                "test.successful_dispatch",
            )
            .unwrap()
        {
            FeatureRunBudgetAdmission::Reserved(value) => value,
            FeatureRunBudgetAdmission::Held(_) => panic!("released capacity was not restored"),
        };
        app.conn
            .execute(
                "UPDATE feature_run_budget_reservations SET started_at_unix_ms = started_at_unix_ms - 1500 WHERE id = ?1",
                [&successful.id],
            )
            .unwrap();
        let mut successful = successful;
        successful.started_at_unix_ms -= 1500;
        app.reconcile_feature_run_budget(&successful, Some(1))
            .unwrap();
        let observation = ExecutionRunRepository::new(&app.conn)
            .budget_observations(&persisted.run.id)
            .unwrap()
            .pop()
            .unwrap();
        assert!(observation.wall_seconds.unwrap() >= 2);
        assert_eq!(observation.tool_calls, Some(1));
        assert_eq!(observation.tokens, None);
        assert!(observation.provenance.contains("tokens=unavailable"));
    }

    #[test]
    fn aged_human_phase_reservations_stop_all_four_real_entry_points() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-implementation");
        let first = app.outcome_work_packet("item-implementation").unwrap();
        let run_id = first["execution_state"]["feature_run"]["id"]
            .as_str()
            .unwrap();
        age_reservation(&app, run_id, "implementation:item-implementation", 56);
        assert_eq!(
            app.outcome_work_packet("item-implementation").unwrap()["kind"],
            "hold"
        );

        let (_root, app, run_id, _) = verification_fixture();
        age_reservation(&app, &run_id, &format!("verification:{run_id}"), 76);
        assert_eq!(
            app.verification_work_packet_value("plan-a", false)
                .unwrap()
                .unwrap()["work_packet"]["kind"],
            "hold"
        );

        let (_root, app, run_id, freeze_id) = verification_fixture();
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = ?1 WHERE run_id = ?2 AND role = 'maker'",
                rusqlite::params![worker_id(), run_id],
            )
            .unwrap();
        app.route_evidence_product_finding_value(&run_id, &freeze_id, "pob-aged")
            .unwrap();
        app.repair_work_packet_value("plan-a").unwrap().unwrap();
        age_reservation(&app, &run_id, &format!("repair:{run_id}"), 96);
        assert_eq!(
            app.repair_work_packet_value("plan-a").unwrap().unwrap()["work_packet"]["kind"],
            "hold"
        );

        let (_root, app, run_id, _) = verification_fixture();
        let gate = app
            .ensure_final_product_review_gate_value("plan-a")
            .unwrap();
        let gate_id = gate["execution_state"]["review_gate"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        app.review_gate_pick_value("plan-a", false)
            .unwrap()
            .unwrap();
        age_reservation(&app, &run_id, &format!("review:{gate_id}"), 86);
        assert_eq!(
            app.complete_review_gate_value(
                &gate_id,
                super::super::repository::execution_run::ReviewVerdict::Accepted,
                &[],
                None,
            )
            .unwrap()["work_packet"]["kind"],
            "hold"
        );
    }

    #[test]
    fn product_finding_closes_verification_and_refreeze_closes_repair_without_overlap() {
        let (_root, app, run_id, freeze_id) = verification_fixture();
        app.conn
            .execute(
                "UPDATE feature_run_role_leases SET worker_id = ?1 WHERE run_id = ?2 AND role = 'maker'",
                rusqlite::params![worker_id(), run_id],
            )
            .unwrap();
        app.route_evidence_product_finding_value(&run_id, &freeze_id, "pob-lifecycle")
            .unwrap();
        assert!(
            app.load_active_budget_reservation(&run_id, &format!("verification:{run_id}"))
                .unwrap()
                .is_none()
        );
        app.repair_work_packet_value("plan-a").unwrap().unwrap();
        app.settle_feature_run_outcome(super::super::execution_run::OutcomeSettlement {
            item_id: "item-phase",
            summary: "repair complete",
            materiality: &json!({"decision": {"review": "none"}}),
            escalation: None,
        })
        .unwrap();
        assert!(
            app.load_active_budget_reservation(&run_id, &format!("repair:{run_id}"))
                .unwrap()
                .is_none()
        );
        let refrozen = app
            .freeze_feature_run_source_value("plan-a")
            .unwrap()
            .unwrap();
        assert_eq!(refrozen["feature_run"]["phase"], "source_frozen");
        let observations = ExecutionRunRepository::new(&app.conn)
            .budget_observations(&run_id)
            .unwrap();
        assert_eq!(
            observations
                .iter()
                .filter(|observation| observation.wall_seconds.is_some())
                .count(),
            2,
            "verification and repair phase intervals are the only wall owners"
        );
    }

    #[test]
    fn successful_nested_evidence_counts_wall_once_under_the_phase_owner() {
        let root = tempfile::tempdir().unwrap();
        write_budget_policy(root.path());
        let app = test_app(root.path().to_path_buf());
        add_outcome(&app, "item-wall-owner");
        let run = app
            .ensure_outcome_feature_run("item-wall-owner")
            .unwrap()
            .unwrap();
        let phase = match app
            .admit_feature_run_budget(
                &run,
                BudgetPhase::Verification,
                "verification:wall-owner",
                Some(5),
                Some(1),
                true,
                "verification.dispatch",
            )
            .unwrap()
        {
            FeatureRunBudgetAdmission::Reserved(value) => value,
            FeatureRunBudgetAdmission::Held(_) => panic!("phase unexpectedly held"),
        };
        let evidence = match app
            .admit_feature_run_budget(
                &run,
                BudgetPhase::Verification,
                "evidence.process:wall-owner",
                Some(10),
                Some(1),
                false,
                "evidence.process_adapter",
            )
            .unwrap()
        {
            FeatureRunBudgetAdmission::Reserved(value) => value,
            FeatureRunBudgetAdmission::Held(_) => panic!("Evidence unexpectedly held"),
        };
        age_reservation(&app, &run.run.id, &phase.boundary_key, 2);
        age_reservation(&app, &run.run.id, &evidence.boundary_key, 2);
        let mut phase = phase;
        phase.started_at_unix_ms -= 2000;
        let mut evidence = evidence;
        evidence.started_at_unix_ms -= 2000;
        app.reconcile_feature_run_budget(&evidence, Some(1))
            .unwrap();
        app.reconcile_feature_run_budget(&phase, Some(1)).unwrap();
        let observations = ExecutionRunRepository::new(&app.conn)
            .budget_observations(&run.run.id)
            .unwrap();
        assert_eq!(
            observations
                .iter()
                .filter(|observation| observation.wall_seconds.is_some())
                .count(),
            1
        );
        assert_eq!(
            observations
                .iter()
                .filter_map(|observation| observation.tool_calls)
                .max(),
            Some(2)
        );
    }
}
