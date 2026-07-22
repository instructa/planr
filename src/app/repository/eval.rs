#![allow(dead_code)] // Staged storage API; runner/CLI surfaces are downstream map items.

use super::super::App;
use crate::eval_compare::{
    EvalCaseEvidence as CompareCaseEvidence, EvalComparisonPolicy, EvalNumericSample,
    EvalRepetitionOutcome as CompareRepetitionOutcome, EvalRunEvidence, compare_eval_runs,
};
use crate::util::{collect_rows, short_id, worker_id};
use anyhow::{Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub(crate) struct EvalSuiteSnapshotInput {
    pub(crate) digest: String,
    pub(crate) suite_id: String,
    pub(crate) suite_version: String,
    pub(crate) normalized_manifest: Value,
    pub(crate) fixture_digests: Value,
    pub(crate) scorer_identities: Value,
    pub(crate) case_order: Value,
    pub(crate) baseline_selector: Value,
    pub(crate) comparison_policy_digest: String,
    pub(crate) metadata: Value,
    pub(crate) source_path: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalSubjectInput {
    pub(crate) kind: String,
    pub(crate) revision: String,
    pub(crate) path: Option<String>,
    pub(crate) argv: Value,
    pub(crate) label: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalRunInput {
    pub(crate) id: Option<String>,
    pub(crate) suite_digest: String,
    pub(crate) subject: EvalSubjectInput,
    pub(crate) runner_version: String,
    pub(crate) planr_version: String,
    pub(crate) testbed_fingerprint: Value,
    pub(crate) source_state: Value,
    pub(crate) parent_run_id: Option<String>,
    pub(crate) resume_of: Option<String>,
    pub(crate) rescore_of: Option<String>,
    pub(crate) recompute_of: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalCaseResultInput {
    pub(crate) case_id: String,
    pub(crate) scorer_id: String,
    pub(crate) scorer_version: String,
    pub(crate) fixture_digest: String,
    pub(crate) status: String,
    pub(crate) repetition_count: i64,
    pub(crate) warmup_count: i64,
    pub(crate) assertions: Value,
    pub(crate) command: Value,
    pub(crate) reasons: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalSampleInput {
    pub(crate) id: Option<String>,
    pub(crate) attempt: EvalAttemptInput,
    pub(crate) repetition_index: i64,
    pub(crate) warmup: bool,
    pub(crate) seed: i64,
    pub(crate) measure: String,
    pub(crate) value: Value,
    pub(crate) unit: String,
    pub(crate) source: String,
    pub(crate) metering_basis: String,
    pub(crate) basis_source: String,
    pub(crate) basis_confidence: String,
    pub(crate) estimate_provenance: Option<Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalAttemptInput {
    pub(crate) id: Option<String>,
    pub(crate) attempt_index: i64,
    pub(crate) terminal_status: String,
    pub(crate) countable: bool,
    pub(crate) retry_of_attempt_id: Option<String>,
    pub(crate) fallback_of_attempt_id: Option<String>,
    pub(crate) escalation_of_attempt_id: Option<String>,
    pub(crate) resume_of_attempt_id: Option<String>,
    pub(crate) effective_client: Option<String>,
    pub(crate) effective_provider: Option<String>,
    pub(crate) effective_runtime: Option<String>,
    pub(crate) effective_model: Option<String>,
    pub(crate) effective_effort: Option<String>,
    pub(crate) effective_profile_id: Option<String>,
    pub(crate) profile_config_digest: Option<String>,
    pub(crate) runner_harness_version: String,
    pub(crate) route_observation: Option<Value>,
    pub(crate) outcome: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalComparisonInput {
    pub(crate) id: Option<String>,
    pub(crate) baseline_run_id: String,
    pub(crate) candidate_run_id: String,
    pub(crate) policy_digest: String,
    pub(crate) runner_version: String,
    pub(crate) verdict: String,
    pub(crate) reasons: Value,
    pub(crate) gates: Value,
    pub(crate) effect_estimates: Value,
    pub(crate) uncertainty: Value,
    pub(crate) protected_dimensions: Value,
    pub(crate) recompute_of: Option<String>,
    pub(crate) rescore_of: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EvalComparisonProvenance<'a> {
    pub(crate) recompute_of: Option<&'a str>,
    pub(crate) rescore_of: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalReusableCaseEvidence {
    case_id: String,
    completed_repetitions: BTreeSet<usize>,
}

pub(crate) struct EvalReuseCaseKey<'a> {
    pub(crate) case_id: &'a str,
    pub(crate) scorer_id: &'a str,
    pub(crate) scorer_version: &'a str,
}

pub(crate) struct EvalReuseCaseCriteria<'a> {
    pub(crate) suite_digest: &'a str,
    pub(crate) subject: &'a EvalSubjectInput,
    pub(crate) testbed_fingerprint: &'a Value,
    pub(crate) resume_parent_run_id: Option<&'a str>,
    pub(crate) case_id: &'a str,
    pub(crate) scorer_id: &'a str,
    pub(crate) scorer_version: &'a str,
    pub(crate) required_measures: &'a [String],
    pub(crate) required_repetitions: usize,
}

struct EvalRunEvidenceRow {
    run_id: String,
    suite_digest: String,
    subject_kind: String,
    subject_path: Option<String>,
    subject_argv: String,
    testbed_fingerprint: String,
    status: String,
    invalidated_by: Option<String>,
    evidence_unix_seconds: Option<i64>,
}

struct EvalDeclaredSamplingRequirements {
    min_successful_samples: Option<usize>,
    required_repetitions: Option<usize>,
}

impl EvalReusableCaseEvidence {
    pub(crate) fn case_id(&self) -> &str {
        &self.case_id
    }

    pub(crate) fn completed_repetition_count(&self) -> usize {
        self.completed_repetitions.len()
    }
}

impl App {
    pub(crate) fn insert_eval_suite_snapshot(
        &self,
        input: EvalSuiteSnapshotInput,
    ) -> Result<String> {
        let project = self.default_project()?;
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT normalized_manifest FROM eval_suite_snapshots WHERE digest = ?1",
                params![input.digest],
                |row| row.get(0),
            )
            .optional()?;
        let normalized_manifest = input.normalized_manifest.to_string();
        if let Some(existing) = existing {
            if existing != normalized_manifest {
                bail!(
                    "immutable eval suite snapshot conflict for digest {}",
                    input.digest
                );
            }
            return Ok(input.digest);
        }
        self.conn.execute(
            "INSERT INTO eval_suite_snapshots(digest, project_id, suite_id, suite_version, normalized_manifest, fixture_digests, scorer_identities, case_order, baseline_selector, comparison_policy_digest, metadata, source_path, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, datetime('now'))",
            params![
                input.digest,
                project.id,
                input.suite_id,
                input.suite_version,
                normalized_manifest,
                input.fixture_digests.to_string(),
                input.scorer_identities.to_string(),
                input.case_order.to_string(),
                input.baseline_selector.to_string(),
                input.comparison_policy_digest,
                input.metadata.to_string(),
                input.source_path,
            ],
        )?;
        Ok(input.digest)
    }

    pub(crate) fn start_eval_run(&self, input: EvalRunInput) -> Result<String> {
        let project = self.default_project()?;
        let id = input.id.unwrap_or_else(|| short_id("evrun"));
        self.conn.execute(
            "INSERT INTO eval_runs(id, project_id, suite_digest, subject_kind, subject_revision, subject_path, subject_argv, subject_label, runner_version, planr_version, status, created_at, started_at, testbed_fingerprint, source_state, case_counts, parent_run_id, resume_of, rescore_of, recompute_of) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'started', datetime('now'), datetime('now'), ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                id,
                project.id,
                input.suite_digest,
                input.subject.kind,
                input.subject.revision,
                input.subject.path,
                input.subject.argv.to_string(),
                input.subject.label,
                input.runner_version,
                input.planr_version,
                input.testbed_fingerprint.to_string(),
                input.source_state.to_string(),
                empty_case_counts().to_string(),
                input.parent_run_id,
                input.resume_of,
                input.rescore_of,
                input.recompute_of,
            ],
        )?;
        Ok(id)
    }

    pub(crate) fn record_eval_case_result(
        &self,
        run_id: &str,
        case: EvalCaseResultInput,
        samples: Vec<EvalSampleInput>,
    ) -> Result<String> {
        validate_case_status(&case.status)?;
        let tx = self.conn.unchecked_transaction()?;
        let case_result_id = format!("evcase-{}-{}-{}", run_id, case.case_id, case.scorer_id);
        let sample_ids = samples
            .iter()
            .map(|sample| sample.id.clone().unwrap_or_else(|| short_id("evsample")))
            .collect::<Vec<_>>();
        let attempt_ids = samples
            .iter()
            .map(|sample| {
                sample
                    .attempt
                    .id
                    .clone()
                    .unwrap_or_else(|| eval_attempt_id(run_id, &case, sample))
            })
            .collect::<Vec<_>>();
        tx.execute(
            "INSERT INTO eval_case_results(id, run_id, case_id, scorer_id, scorer_version, fixture_digest, status, repetition_count, warmup_count, assertions, command, sample_ids, reasons, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'), datetime('now'))",
            params![
                case_result_id,
                run_id,
                case.case_id,
                case.scorer_id,
                case.scorer_version,
                case.fixture_digest,
                case.status,
                case.repetition_count,
                case.warmup_count,
                case.assertions.to_string(),
                case.command.to_string(),
                json!(sample_ids).to_string(),
                case.reasons.to_string(),
            ],
        )?;
        for (sample, (id, attempt_id)) in samples
            .into_iter()
            .zip(sample_ids.into_iter().zip(attempt_ids))
        {
            validate_sample_source(&sample.source)?;
            validate_attempt_terminal_status(&sample.attempt.terminal_status)?;
            validate_metering_basis(&sample.metering_basis)?;
            validate_basis_confidence(&sample.basis_confidence)?;
            validate_sample_value_for_basis(&sample.value, &sample.metering_basis)?;
            if sample.metering_basis == "unavailable" {
                if sample.basis_confidence != "unavailable" {
                    bail!("unavailable eval samples require unavailable basis_confidence");
                }
                if sample.estimate_provenance.is_some() {
                    bail!("unavailable eval samples must not carry estimate_provenance");
                }
            }
            if sample.metering_basis == "estimated" {
                validate_estimate_provenance(sample.estimate_provenance.as_ref())?;
            }
            ensure_eval_attempt_identity(
                &tx,
                run_id,
                &case,
                &sample,
                &attempt_id,
                &case_result_id,
            )?;
            tx.execute(
                "INSERT INTO eval_samples(id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, seed, measure, value_json, unit, source, attempt_id, attempt_index, metering_basis, basis_source, basis_confidence, estimate_provenance, captured_at) SELECT ?1, run_id, case_id, scorer_id, scorer_version, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, datetime('now') FROM eval_case_results WHERE id = ?15",
                params![
                    id,
                    sample.repetition_index,
                    if sample.warmup { 1 } else { 0 },
                    sample.seed,
                    sample.measure,
                    sample.value.to_string(),
                    sample.unit,
                    sample.source,
                    attempt_id,
                    sample.attempt.attempt_index,
                    sample.metering_basis,
                    sample.basis_source,
                    sample.basis_confidence,
                    sample.estimate_provenance.map(|value| value.to_string()),
                    case_result_id,
                ],
            )?;
        }
        tx.commit()?;
        Ok(case_result_id)
    }

    pub(crate) fn finish_eval_run(&self, run_id: &str, status: &str) -> Result<()> {
        validate_terminal_run_status(status)?;
        let counts = self.eval_case_counts(run_id)?;
        let updated = self.conn.execute(
            "UPDATE eval_runs SET status = ?2, completed_at = datetime('now'), case_counts = ?3 WHERE id = ?1 AND status = 'started'",
            params![run_id, status, counts.to_string()],
        )?;
        if updated != 1 {
            bail!("eval run {run_id} is missing or not in started status");
        }
        Ok(())
    }

    pub(crate) fn invalidate_eval_target(
        &self,
        target_kind: &str,
        target_id: &str,
        reason: &str,
        reason_codes: Value,
        replacement_hint: Option<&str>,
    ) -> Result<String> {
        if !matches!(target_kind, "run" | "comparison") {
            bail!("invalid eval invalidation target kind: {target_kind}");
        }
        let project = self.default_project()?;
        let id = short_id("evinv");
        let tx = self.conn.unchecked_transaction()?;
        if target_kind == "run" {
            let existing: Option<(String, Option<String>)> = tx
                .query_row(
                    "SELECT status, invalidated_by FROM eval_runs WHERE id = ?1 AND project_id = ?2",
                    params![target_id, project.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((status, invalidated_by)) = existing else {
                bail!("eval run {target_id} is missing for invalidation");
            };
            if status == "invalidated" || invalidated_by.is_some() {
                bail!("eval run {target_id} is already invalidated");
            }
        } else {
            let exists: Option<String> = tx
                .query_row(
                    "SELECT id FROM eval_comparisons WHERE id = ?1 AND project_id = ?2",
                    params![target_id, project.id],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                bail!("eval comparison {target_id} is missing for invalidation");
            }
            let prior: i64 = tx.query_row(
                "SELECT COUNT(*) FROM eval_invalidations WHERE target_kind = 'comparison' AND target_id = ?1",
                params![target_id],
                |row| row.get(0),
            )?;
            if prior != 0 {
                bail!("eval comparison {target_id} is already invalidated");
            }
        }
        tx.execute(
            "INSERT INTO eval_invalidations(id, project_id, target_kind, target_id, reason, reason_codes, created_at, created_by, replacement_hint) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), ?7, ?8)",
            params![id, project.id, target_kind, target_id, reason, reason_codes.to_string(), worker_id(), replacement_hint],
        )?;
        if target_kind == "run" {
            let updated = tx.execute(
                "UPDATE eval_runs SET status = 'invalidated', invalidated_by = ?2, completed_at = COALESCE(completed_at, datetime('now')) WHERE id = ?1 AND project_id = ?3 AND status != 'invalidated' AND invalidated_by IS NULL",
                params![target_id, id, project.id],
            )?;
            if updated != 1 {
                bail!("eval run {target_id} could not be invalidated");
            }
        }
        tx.commit()?;
        Ok(id)
    }

    pub(crate) fn create_eval_comparison(&self, input: EvalComparisonInput) -> Result<String> {
        validate_verdict(&input.verdict)?;
        let project = self.default_project()?;
        let id = input.id.unwrap_or_else(|| short_id("evcmp"));
        self.conn.execute(
            "INSERT INTO eval_comparisons(id, project_id, baseline_run_id, candidate_run_id, policy_digest, runner_version, verdict, reasons, gates, effect_estimates, uncertainty, protected_dimensions, recompute_of, rescore_of, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, datetime('now'))",
            params![
                id,
                project.id,
                input.baseline_run_id,
                input.candidate_run_id,
                input.policy_digest,
                input.runner_version,
                input.verdict,
                input.reasons.to_string(),
                input.gates.to_string(),
                input.effect_estimates.to_string(),
                input.uncertainty.to_string(),
                input.protected_dimensions.to_string(),
                input.recompute_of,
                input.rescore_of,
            ],
        )?;
        Ok(id)
    }

    pub(crate) fn compare_stored_eval_runs(
        &self,
        baseline_run_id: &str,
        candidate_run_id: &str,
        policy_digest: &str,
        policy: &EvalComparisonPolicy,
        provenance: EvalComparisonProvenance<'_>,
    ) -> Result<String> {
        let reference_unix_seconds = current_unix_seconds()?;
        self.compare_stored_eval_runs_at(
            baseline_run_id,
            candidate_run_id,
            policy_digest,
            policy,
            reference_unix_seconds,
            provenance,
        )
    }

    pub(crate) fn compare_stored_eval_runs_at(
        &self,
        baseline_run_id: &str,
        candidate_run_id: &str,
        policy_digest: &str,
        policy: &EvalComparisonPolicy,
        reference_unix_seconds: i64,
        provenance: EvalComparisonProvenance<'_>,
    ) -> Result<String> {
        let baseline = self.eval_run_evidence_at(baseline_run_id, reference_unix_seconds)?;
        let candidate = self.eval_run_evidence_at(candidate_run_id, reference_unix_seconds)?;
        self.validate_eval_comparison_provenance(
            baseline_run_id,
            candidate_run_id,
            policy_digest,
            provenance,
        )?;
        let comparison_policy;
        let effective_policy = if provenance.rescore_of.is_some() {
            comparison_policy = Self::rescore_replay_policy(policy);
            &comparison_policy
        } else {
            policy
        };
        let mut result = compare_eval_runs(Some(&baseline), Some(&candidate), effective_policy);
        let baseline_efficiency = self.eval_run_efficiency_summary(baseline_run_id)?;
        let candidate_efficiency = self.eval_run_efficiency_summary(candidate_run_id)?;
        let intervals = self.effort_recommendation_metric_intervals(
            baseline_run_id,
            candidate_run_id,
            effective_policy.bootstrap_iterations,
        )?;
        attach_effort_recommendation_materiality(
            &mut result.effect_estimates,
            &mut result.uncertainty,
            effective_policy,
            &baseline_efficiency,
            &candidate_efficiency,
            &intervals,
        );
        self.create_eval_comparison(EvalComparisonInput {
            id: None,
            baseline_run_id: baseline_run_id.to_string(),
            candidate_run_id: candidate_run_id.to_string(),
            policy_digest: policy_digest.to_string(),
            runner_version: "eval-runner-v1".to_string(),
            verdict: result.verdict,
            reasons: json!(result.reasons),
            gates: result.gates,
            effect_estimates: result.effect_estimates,
            uncertainty: result.uncertainty,
            protected_dimensions: result.protected_dimensions,
            recompute_of: provenance.recompute_of.map(ToOwned::to_owned),
            rescore_of: provenance.rescore_of.map(ToOwned::to_owned),
        })
    }

    fn rescore_replay_policy(policy: &EvalComparisonPolicy) -> EvalComparisonPolicy {
        let mut replay_policy = policy.clone();
        // Rescores replay an existing run, so wall-clock variance should not
        // promote the replay as a better treatment. Regression and protected
        // budget gates still apply under the recorded policy digest.
        replay_policy
            .material_improvement
            .remove("duration_ms_p95_relative");
        replay_policy.material_improvement.remove("duration_ms");
        replay_policy
            .material_improvement
            .remove("wall_time_ms_p95_relative");
        replay_policy.material_improvement.remove("wall_time_ms");
        replay_policy
    }

    fn validate_eval_comparison_provenance(
        &self,
        baseline_run_id: &str,
        candidate_run_id: &str,
        policy_digest: &str,
        provenance: EvalComparisonProvenance<'_>,
    ) -> Result<()> {
        if let Some(recompute_of) = provenance.recompute_of {
            let source = self.eval_comparison_value_by_id(recompute_of)?;
            if source["baseline_run_id"].as_str() != Some(baseline_run_id)
                || source["candidate_run_id"].as_str() != Some(candidate_run_id)
                || source["policy_digest"].as_str() != Some(policy_digest)
            {
                bail!(
                    "eval comparison recompute source {recompute_of} does not match replayed baseline, candidate, and policy"
                );
            }
        }
        if let Some(rescore_of) = provenance.rescore_of {
            self.eval_run_record_value(rescore_of)?;
            if baseline_run_id != rescore_of {
                bail!(
                    "eval comparison rescore source {rescore_of} must be the replayed baseline run"
                );
            }
            let candidate = self.eval_run_record_value(candidate_run_id)?;
            if candidate["rescore_of"].as_str() != Some(rescore_of) {
                bail!(
                    "eval comparison candidate {candidate_run_id} is not a completed rescore of {rescore_of}"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn attach_eval_evidence_ref(
        &self,
        target_kind: &str,
        target_id: &str,
        attachment_kind: &str,
        attachment_id: &str,
        item_id: &str,
    ) -> Result<String> {
        if !matches!(target_kind, "run" | "comparison") {
            bail!("invalid eval evidence target kind: {target_kind}");
        }
        if !matches!(attachment_kind, "log" | "review" | "artifact") {
            bail!("invalid eval evidence attachment kind: {attachment_kind}");
        }
        let project = self.default_project()?;
        let id = format!("eval-{target_kind}:{target_id}");
        self.conn.execute(
            "INSERT INTO eval_evidence_refs(id, project_id, target_kind, target_id, planr_attachment_kind, planr_attachment_id, item_id, closure_authority, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, datetime('now'))",
            params![id, project.id, target_kind, target_id, attachment_kind, attachment_id, item_id],
        )?;
        Ok(id)
    }

    pub(crate) fn reusable_eval_samples(
        &self,
        suite_digest: &str,
        subject: &EvalSubjectInput,
        testbed_fingerprint: &Value,
        resume_parent_run_id: Option<&str>,
        case_key: EvalReuseCaseKey<'_>,
    ) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.repetition_index, s.warmup, s.seed, s.measure, s.value_json, s.unit, s.source, s.attempt_id, s.attempt_index, s.metering_basis, s.basis_source, s.basis_confidence, s.estimate_provenance
             FROM eval_samples s
             JOIN eval_case_results c ON c.run_id = s.run_id AND c.case_id = s.case_id AND c.scorer_id = s.scorer_id AND c.scorer_version = s.scorer_version
             JOIN eval_runs r ON r.id = s.run_id
	             WHERE (
	                   (?7 IS NULL AND r.status = 'success')
	                OR (?7 IS NOT NULL AND r.id = ?7 AND r.status IN ('started','error','inconclusive','success'))
	               )
	               AND r.invalidated_by IS NULL
	               AND r.suite_digest = ?1
	               AND r.subject_kind = ?2
	               AND r.subject_revision = ?3
	               AND ((r.subject_path IS NULL AND ?4 IS NULL) OR r.subject_path = ?4)
	               AND r.subject_argv = ?5
	               AND r.testbed_fingerprint = ?6
	               AND c.status = 'pass'
	               AND c.case_id = ?8
	               AND c.scorer_id = ?9
	               AND c.scorer_version = ?10
	             ORDER BY s.repetition_index, s.warmup, s.measure",
        )?;
        let rows = stmt.query_map(
            params![
                suite_digest,
                subject.kind,
                subject.revision,
                subject.path,
                subject.argv.to_string(),
                testbed_fingerprint.to_string(),
                resume_parent_run_id,
                case_key.case_id,
                case_key.scorer_id,
                case_key.scorer_version,
            ],
            |row| {
                let raw: String = row.get(5)?;
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "repetition_index": row.get::<_, i64>(1)?,
                    "warmup": row.get::<_, i64>(2)? == 1,
                    "seed": row.get::<_, i64>(3)?,
                    "measure": row.get::<_, String>(4)?,
                    "value": serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null),
                    "unit": row.get::<_, String>(6)?,
                    "source": row.get::<_, String>(7)?,
                    "attempt_id": row.get::<_, Option<String>>(8)?,
                    "attempt_index": row.get::<_, i64>(9)?,
                    "metering_basis": row.get::<_, String>(10)?,
                    "basis_source": row.get::<_, String>(11)?,
                    "basis_confidence": row.get::<_, String>(12)?,
                    "estimate_provenance": row.get::<_, Option<String>>(13)?.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()),
                }))
            },
        )?;
        crate::util::collect_rows(rows)
    }

    pub(crate) fn reusable_eval_case_evidence(
        &self,
        criteria: EvalReuseCaseCriteria<'_>,
    ) -> Result<Option<EvalReusableCaseEvidence>> {
        if criteria.required_repetitions == 0 {
            bail!("samples_below_minimum");
        }
        let samples = self.reusable_eval_samples(
            criteria.suite_digest,
            criteria.subject,
            criteria.testbed_fingerprint,
            criteria.resume_parent_run_id,
            EvalReuseCaseKey {
                case_id: criteria.case_id,
                scorer_id: criteria.scorer_id,
                scorer_version: criteria.scorer_version,
            },
        )?;
        let required = criteria
            .required_measures
            .iter()
            .cloned()
            .collect::<BTreeSet<String>>();
        let mut measures_by_repetition = BTreeMap::<usize, BTreeSet<String>>::new();
        for sample in samples {
            if sample
                .get("warmup")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                continue;
            }
            let Some(repetition_index) = sample.get("repetition_index").and_then(Value::as_i64)
            else {
                continue;
            };
            if repetition_index < 0 {
                continue;
            }
            let Some(measure) = sample.get("measure").and_then(Value::as_str) else {
                continue;
            };
            measures_by_repetition
                .entry(repetition_index as usize)
                .or_default()
                .insert(measure.to_string());
        }
        let completed_repetitions = measures_by_repetition
            .into_iter()
            .filter_map(|(repetition, observed)| {
                if required.is_empty() || required.is_subset(&observed) {
                    Some(repetition)
                } else {
                    None
                }
            })
            .take(criteria.required_repetitions)
            .collect::<BTreeSet<_>>();
        if completed_repetitions.len() < criteria.required_repetitions {
            return Ok(None);
        }
        Ok(Some(EvalReusableCaseEvidence {
            case_id: criteria.case_id.to_string(),
            completed_repetitions,
        }))
    }

    fn eval_case_counts(&self, run_id: &str) -> Result<Value> {
        let mut counts = serde_json::Map::new();
        for status in ["pass", "fail", "error", "skipped", "inconclusive"] {
            let count: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM eval_case_results WHERE run_id = ?1 AND status = ?2",
                params![run_id, status],
                |row| row.get(0),
            )?;
            counts.insert(status.to_string(), json!(count));
        }
        Ok(Value::Object(counts))
    }

    fn eval_run_evidence(&self, run_id: &str) -> Result<EvalRunEvidence> {
        self.eval_run_evidence_at(run_id, current_unix_seconds()?)
    }

    fn eval_run_evidence_at(
        &self,
        run_id: &str,
        reference_unix_seconds: i64,
    ) -> Result<EvalRunEvidence> {
        let run = self.conn.query_row(
            "SELECT id, suite_digest, subject_kind, subject_path, subject_argv, testbed_fingerprint, status, invalidated_by, CAST(strftime('%s', COALESCE(completed_at, started_at, created_at)) AS INTEGER) FROM eval_runs WHERE id = ?1",
            params![run_id],
            |row| {
                Ok(EvalRunEvidenceRow {
                    run_id: row.get(0)?,
                    suite_digest: row.get(1)?,
                    subject_kind: row.get(2)?,
                    subject_path: row.get(3)?,
                    subject_argv: row.get(4)?,
                    testbed_fingerprint: row.get(5)?,
                    status: row.get(6)?,
                    invalidated_by: row.get(7)?,
                    evidence_unix_seconds: row.get(8)?,
                })
            },
        )?;
        let declared_sampling = self.eval_declared_sampling_requirements(&run.suite_digest)?;
        let declared_scorer_controls = self.eval_declared_scorer_controls(&run.suite_digest)?;
        let mut case_stmt = self.conn.prepare(
            "SELECT case_id, scorer_id, scorer_version, fixture_digest, status, assertions FROM eval_case_results WHERE run_id = ?1 ORDER BY case_id, scorer_id, scorer_version",
        )?;
        let case_rows = case_stmt.query_map(params![run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        let mut cases = Vec::new();
        for case_row in case_rows {
            let (case_id, scorer_id, scorer_version, fixture_digest, case_status, assertions) =
                case_row?;
            let sampling = declared_sampling.get(&case_id);
            let min_successful_samples =
                sampling.and_then(|requirements| requirements.min_successful_samples);
            let required_repetitions =
                sampling.and_then(|requirements| requirements.required_repetitions);
            let scorer_control = declared_scorer_controls.get(&case_id).cloned();
            let mut sample_stmt = self.conn.prepare(
                "SELECT repetition_index, warmup, seed, measure, value_json FROM eval_samples WHERE run_id = ?1 AND case_id = ?2 AND scorer_id = ?3 AND scorer_version = ?4 ORDER BY repetition_index, warmup, measure",
            )?;
            let sample_rows = sample_stmt.query_map(
                params![run_id, case_id, scorer_id, scorer_version],
                |row| {
                    let raw: String = row.get(4)?;
                    let parsed = serde_json::from_str::<Value>(&raw)
                        .ok()
                        .and_then(|value| value.as_f64());
                    Ok(EvalNumericSample {
                        repetition_index: row.get::<_, i64>(0)? as usize,
                        warmup: row.get::<_, i64>(1)? == 1,
                        seed: row.get(2)?,
                        measure: row.get(3)?,
                        value: parsed.unwrap_or(0.0),
                        valid: parsed.is_some(),
                    })
                },
            )?;
            let repetition_outcomes =
                self.eval_case_repetition_outcomes(run_id, &case_id, &scorer_id, &scorer_version)?;
            cases.push(CompareCaseEvidence {
                case_id,
                scorer_id,
                scorer_version,
                scorer_control,
                fixture_digest,
                status: case_status,
                repetition_outcomes,
                safety_pass: safety_pass_from_assertions(&assertions),
                quality_pass: quality_pass_from_assertions(&assertions),
                min_successful_samples,
                required_repetitions,
                samples: crate::util::collect_rows(sample_rows)?,
            });
        }
        Ok(EvalRunEvidence {
            run_id: run.run_id,
            suite_digest: run.suite_digest,
            subject_kind: run.subject_kind,
            subject_path: run.subject_path,
            subject_argv: run.subject_argv,
            testbed_fingerprint: run.testbed_fingerprint,
            status: run.status,
            invalidated: run.invalidated_by.is_some(),
            age_hours: run
                .evidence_unix_seconds
                .map(|seconds| ((reference_unix_seconds - seconds).max(0)) / 3600)
                .unwrap_or(i64::MAX),
            cases,
        })
    }

    fn eval_case_repetition_outcomes(
        &self,
        run_id: &str,
        case_id: &str,
        scorer_id: &str,
        scorer_version: &str,
    ) -> Result<Vec<CompareRepetitionOutcome>> {
        let mut stmt = self.conn.prepare(
            "SELECT repetition_index, terminal_status, outcome_json
             FROM eval_attempts
             WHERE run_id = ?1 AND case_id = ?2 AND scorer_id = ?3 AND scorer_version = ?4 AND warmup = 0 AND countable = 1
             ORDER BY repetition_index, attempt_index, id",
        )?;
        let rows = stmt.query_map(params![run_id, case_id, scorer_id, scorer_version], |row| {
            let terminal_status: String = row.get(1)?;
            let outcome_json: String = row.get(2)?;
            let outcome_status = serde_json::from_str::<Value>(&outcome_json)
                .ok()
                .and_then(|value| {
                    value
                        .get("status")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .unwrap_or_else(|| normalize_attempt_status(&terminal_status));
            Ok(CompareRepetitionOutcome {
                repetition_index: row.get::<_, i64>(0)? as usize,
                status: outcome_status,
            })
        })?;
        let mut outcomes_by_repetition = BTreeMap::<usize, String>::new();
        for row in rows {
            let outcome = row?;
            outcomes_by_repetition.insert(outcome.repetition_index, outcome.status);
        }
        Ok(outcomes_by_repetition
            .into_iter()
            .map(|(repetition_index, status)| CompareRepetitionOutcome {
                repetition_index,
                status,
            })
            .collect())
    }

    fn eval_declared_sampling_requirements(
        &self,
        suite_digest: &str,
    ) -> Result<BTreeMap<String, EvalDeclaredSamplingRequirements>> {
        let normalized_manifest: Option<String> = self
            .conn
            .query_row(
                "SELECT normalized_manifest FROM eval_suite_snapshots WHERE digest = ?1",
                params![suite_digest],
                |row| row.get(0),
            )
            .optional()?;
        let Some(normalized_manifest) = normalized_manifest else {
            return Ok(BTreeMap::new());
        };
        let Ok(manifest) = serde_json::from_str::<Value>(&normalized_manifest) else {
            return Ok(BTreeMap::new());
        };
        let mut requirements = BTreeMap::new();
        if let Some(cases) = manifest.get("cases").and_then(Value::as_array) {
            for case in cases {
                let Some(case_id) = case.get("case_id").and_then(Value::as_str) else {
                    continue;
                };
                let sampling = case.get("sampling").unwrap_or(&Value::Null);
                let min_successful_samples = sampling
                    .get("min_successful_samples")
                    .and_then(Value::as_u64)
                    .filter(|minimum| *minimum > 0)
                    .map(|minimum| minimum as usize);
                let required_repetitions = sampling
                    .get("repetitions")
                    .and_then(Value::as_u64)
                    .filter(|repetitions| *repetitions > 0)
                    .map(|repetitions| repetitions as usize);
                if min_successful_samples.is_some() || required_repetitions.is_some() {
                    requirements.insert(
                        case_id.to_string(),
                        EvalDeclaredSamplingRequirements {
                            min_successful_samples,
                            required_repetitions,
                        },
                    );
                }
            }
        }
        Ok(requirements)
    }

    fn eval_declared_scorer_controls(
        &self,
        suite_digest: &str,
    ) -> Result<BTreeMap<String, String>> {
        let normalized_manifest: Option<String> = self
            .conn
            .query_row(
                "SELECT normalized_manifest FROM eval_suite_snapshots WHERE digest = ?1",
                params![suite_digest],
                |row| row.get(0),
            )
            .optional()?;
        let Some(normalized_manifest) = normalized_manifest else {
            return Ok(BTreeMap::new());
        };
        let Ok(manifest) = serde_json::from_str::<Value>(&normalized_manifest) else {
            return Ok(BTreeMap::new());
        };
        let mut controls = BTreeMap::new();
        if let Some(cases) = manifest.get("cases").and_then(Value::as_array) {
            for case in cases {
                let Some(case_id) = case.get("case_id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(kind) = case
                    .get("scorer_control")
                    .and_then(|control| control.get("kind"))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                let normalized = if kind == "null" { "no_op" } else { kind };
                controls.insert(case_id.to_string(), normalized.to_string());
            }
        }
        Ok(controls)
    }

    fn effort_recommendation_metric_intervals(
        &self,
        baseline_run_id: &str,
        candidate_run_id: &str,
        iterations: usize,
    ) -> Result<EffortRecommendationIntervals> {
        let baseline = self.effort_cost_cells(baseline_run_id)?;
        let candidate = self.effort_cost_cells(candidate_run_id)?;
        let common_cells = baseline
            .keys()
            .filter(|cell_id| candidate.contains_key(*cell_id))
            .cloned()
            .collect::<Vec<_>>();
        if common_cells.len() < 2 || iterations == 0 {
            return Ok(EffortRecommendationIntervals::default());
        }
        Ok(EffortRecommendationIntervals {
            cost_per_verified_success: paired_cell_metric_per_success_bootstrap_relative_delta(
                &common_cells,
                &baseline,
                &candidate,
                iterations,
                0xc057_5eed,
                |cell| cell.cost_micros,
            ),
            wall_time_per_verified_success: paired_cell_metric_per_success_bootstrap_relative_delta(
                &common_cells,
                &baseline,
                &candidate,
                iterations,
                0x71_5eed,
                |cell| cell.wall_time_ms,
            ),
        })
    }

    fn effort_cost_cells(&self, run_id: &str) -> Result<BTreeMap<String, EffortCostCell>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.case_id, a.repetition_index, a.seed, a.terminal_status, cost.value_json, wall.value_json
             FROM eval_attempts a
             LEFT JOIN eval_samples cost
               ON cost.run_id = a.run_id
              AND cost.attempt_id = a.id
              AND cost.measure = 'cost_micros'
             LEFT JOIN eval_samples wall
               ON wall.run_id = a.run_id
              AND wall.attempt_id = a.id
              AND wall.measure = 'wall_time_ms'
             WHERE a.run_id = ?1 AND a.countable = 1
             ORDER BY a.case_id, a.repetition_index, a.seed, a.attempt_index, a.id",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            let case_id: String = row.get(0)?;
            let repetition_index: i64 = row.get(1)?;
            let seed: i64 = row.get(2)?;
            let terminal_status: String = row.get(3)?;
            let cost_json: Option<String> = row.get(4)?;
            let wall_time_json: Option<String> = row.get(5)?;
            Ok((
                format!("{case_id}:{repetition_index}:{seed}"),
                terminal_status,
                cost_json,
                wall_time_json,
            ))
        })?;
        let mut cells = BTreeMap::<String, EffortCostCell>::new();
        for row in collect_rows(rows)? {
            let (cell_id, terminal_status, cost_json, wall_time_json) = row;
            let cell = cells.entry(cell_id).or_default();
            cell.attempts += 1;
            if terminal_status == "verified_success" {
                cell.verified_successes += 1;
            }
            accumulate_cell_metric(&mut cell.cost_micros, &mut cell.missing_cost, cost_json);
            accumulate_cell_metric(
                &mut cell.wall_time_ms,
                &mut cell.missing_wall_time,
                wall_time_json,
            );
        }
        Ok(cells)
    }
}

fn accumulate_cell_metric(total: &mut Option<f64>, missing: &mut bool, raw: Option<String>) {
    let value = raw
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|value| value.as_f64());
    match (*total, value) {
        (Some(current), Some(value)) => *total = Some(current + value),
        (None, Some(value)) if !*missing => *total = Some(value),
        _ => {
            *total = None;
            *missing = true;
        }
    }
}

#[derive(Debug, Default)]
struct EffortRecommendationIntervals {
    cost_per_verified_success: Option<(f64, f64)>,
    wall_time_per_verified_success: Option<(f64, f64)>,
}

#[derive(Debug, Default)]
struct EffortCostCell {
    attempts: usize,
    verified_successes: usize,
    cost_micros: Option<f64>,
    wall_time_ms: Option<f64>,
    missing_cost: bool,
    missing_wall_time: bool,
}

fn attach_effort_recommendation_materiality(
    effect_estimates: &mut Value,
    uncertainty: &mut Value,
    policy: &EvalComparisonPolicy,
    baseline_efficiency: &Value,
    candidate_efficiency: &Value,
    intervals: &EffortRecommendationIntervals,
) {
    let mut evidences = Vec::new();
    if let Some(evidence) = effort_recommendation_materiality(
        policy,
        baseline_efficiency,
        candidate_efficiency,
        "cost_per_verified_success_micros",
        intervals.cost_per_verified_success,
        selected_cost_materiality_threshold,
    ) {
        if let Some((low, high)) = intervals.cost_per_verified_success {
            if let Some(uncertainty) = uncertainty.as_object_mut() {
                uncertainty.insert(
                    "cost_per_verified_success_micros_p95_ci95".to_string(),
                    relative_interval_value((low, high)),
                );
            }
        }
        evidences.push(evidence);
    }
    if let Some(evidence) = effort_recommendation_materiality(
        policy,
        baseline_efficiency,
        candidate_efficiency,
        "wall_time_per_verified_success_ms",
        intervals.wall_time_per_verified_success,
        selected_wall_time_materiality_threshold,
    ) {
        if let Some((low, high)) = intervals.wall_time_per_verified_success {
            if let Some(uncertainty) = uncertainty.as_object_mut() {
                uncertainty.insert(
                    "wall_time_per_verified_success_ms_p95_ci95".to_string(),
                    relative_interval_value((low, high)),
                );
            }
        }
        evidences.push(evidence);
    }
    let selection = evidences
        .iter()
        .find(|evidence| evidence["materially_improved"].as_bool() == Some(true))
        .cloned()
        .or_else(|| evidences.first().cloned());
    let Some(selection) = selection else {
        return;
    };
    let Some(effect_estimates) = effect_estimates.as_object_mut() else {
        return;
    };
    effect_estimates.insert(
        "effort_recommendation".to_string(),
        json!({
            "selection_metric_materiality": selection,
            "metric_materiality": evidences,
        }),
    );
}

fn effort_recommendation_materiality(
    policy: &EvalComparisonPolicy,
    baseline_efficiency: &Value,
    candidate_efficiency: &Value,
    metric: &str,
    interval: Option<(f64, f64)>,
    threshold_for_policy: fn(&EvalComparisonPolicy) -> Option<f64>,
) -> Option<Value> {
    let threshold = threshold_for_policy(policy)?;
    let baseline = baseline_efficiency[metric]["value"].as_f64()?;
    let candidate = candidate_efficiency[metric]["value"].as_f64()?;
    let relative = relative_delta(candidate, baseline);
    let materially_improved = interval.map(|(_, high)| high <= threshold).unwrap_or(false);
    Some(json!({
        "metric": metric,
        "baseline_value": baseline,
        "candidate_value": candidate,
        "relative": relative_value(relative),
        "threshold": threshold,
        "ci95": interval.map(relative_interval_value).unwrap_or(Value::Null),
        "evidence": if interval.is_some() { "paired_cell_bootstrap_ci95" } else { "unavailable" },
        "materially_improved": materially_improved,
    }))
}

fn selected_cost_materiality_threshold(policy: &EvalComparisonPolicy) -> Option<f64> {
    policy
        .material_improvement
        .get("cost_per_verified_success_micros_p95_relative")
        .or_else(|| {
            policy
                .material_improvement
                .get("cost_per_verified_success_micros")
        })
        .or_else(|| policy.material_improvement.get("cost_micros_p95_relative"))
        .or_else(|| policy.material_improvement.get("cost_micros"))
        .copied()
}

fn selected_wall_time_materiality_threshold(policy: &EvalComparisonPolicy) -> Option<f64> {
    policy
        .material_improvement
        .get("wall_time_per_verified_success_ms_p95_relative")
        .or_else(|| {
            policy
                .material_improvement
                .get("wall_time_per_verified_success_ms")
        })
        .or_else(|| policy.material_improvement.get("wall_time_ms_p95_relative"))
        .or_else(|| policy.material_improvement.get("wall_time_ms"))
        .copied()
}

fn relative_delta(candidate: f64, baseline: f64) -> f64 {
    if baseline == 0.0 {
        if candidate == 0.0 {
            0.0
        } else if candidate > 0.0 {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        }
    } else {
        (candidate - baseline) / baseline
    }
}

fn relative_value(value: f64) -> Value {
    if value.is_infinite() && value.is_sign_positive() {
        json!("unbounded_positive")
    } else if value.is_infinite() && value.is_sign_negative() {
        json!("unbounded_negative")
    } else {
        json!(value)
    }
}

fn relative_interval_value(interval: (f64, f64)) -> Value {
    Value::Array(vec![relative_value(interval.0), relative_value(interval.1)])
}

fn paired_cell_metric_per_success_bootstrap_relative_delta(
    cell_ids: &[String],
    baseline: &BTreeMap<String, EffortCostCell>,
    candidate: &BTreeMap<String, EffortCostCell>,
    iterations: usize,
    seed: u64,
    metric_value: fn(&EffortCostCell) -> Option<f64>,
) -> Option<(f64, f64)> {
    if cell_ids.iter().any(|cell_id| {
        metric_value(&baseline[cell_id]).is_none() || metric_value(&candidate[cell_id]).is_none()
    }) {
        return None;
    }
    Some(
        paired_cell_metric_per_success_bootstrap_relative_delta_unchecked(
            cell_ids,
            baseline,
            candidate,
            iterations,
            seed,
            metric_value,
        ),
    )
}

fn paired_cell_metric_per_success_bootstrap_relative_delta_unchecked(
    cell_ids: &[String],
    baseline: &BTreeMap<String, EffortCostCell>,
    candidate: &BTreeMap<String, EffortCostCell>,
    iterations: usize,
    seed: u64,
    metric_value: fn(&EffortCostCell) -> Option<f64>,
) -> (f64, f64) {
    let mut rng = seed.max(1);
    let mut deltas = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut baseline_total = 0.0;
        let mut candidate_total = 0.0;
        let mut baseline_successes = 0usize;
        let mut candidate_successes = 0usize;
        for _ in 0..cell_ids.len() {
            rng = xorshift64(rng);
            let idx = (rng as usize) % cell_ids.len();
            let cell_id = &cell_ids[idx];
            let baseline_cell = &baseline[cell_id];
            let candidate_cell = &candidate[cell_id];
            baseline_total += metric_value(baseline_cell).unwrap_or(0.0);
            candidate_total += metric_value(candidate_cell).unwrap_or(0.0);
            baseline_successes += baseline_cell.verified_successes;
            candidate_successes += candidate_cell.verified_successes;
        }
        let baseline_value = metric_per_success(baseline_total, baseline_successes);
        let candidate_value = metric_per_success(candidate_total, candidate_successes);
        deltas.push(relative_delta(candidate_value, baseline_value));
    }
    deltas.sort_by(|a, b| a.total_cmp(b));
    let low = ((iterations - 1) as f64 * 0.025).floor() as usize;
    let high = ((iterations - 1) as f64 * 0.975).ceil() as usize;
    (deltas[low], deltas[high])
}

fn metric_per_success(total: f64, successes: usize) -> f64 {
    if successes == 0 {
        f64::INFINITY
    } else {
        total / successes as f64
    }
}

fn xorshift64(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn current_unix_seconds() -> Result<i64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64)
}

fn empty_case_counts() -> Value {
    json!({"pass": 0, "fail": 0, "error": 0, "skipped": 0, "inconclusive": 0})
}

fn validate_run_status(status: &str) -> Result<()> {
    if matches!(
        status,
        "started" | "success" | "error" | "inconclusive" | "invalidated"
    ) {
        Ok(())
    } else {
        Err(anyhow!("invalid eval run status: {status}"))
    }
}

fn validate_terminal_run_status(status: &str) -> Result<()> {
    if matches!(status, "success" | "error" | "inconclusive") {
        Ok(())
    } else {
        Err(anyhow!("invalid terminal eval run status: {status}"))
    }
}

fn validate_case_status(status: &str) -> Result<()> {
    if matches!(
        status,
        "pass" | "fail" | "error" | "skipped" | "inconclusive"
    ) {
        Ok(())
    } else {
        Err(anyhow!("invalid eval case status: {status}"))
    }
}

fn validate_attempt_terminal_status(status: &str) -> Result<()> {
    if matches!(
        status,
        "verified_success"
            | "pass"
            | "failed"
            | "fail"
            | "error"
            | "skipped"
            | "inconclusive"
            | "stopped"
    ) {
        Ok(())
    } else {
        Err(anyhow!("invalid eval attempt terminal status: {status}"))
    }
}

fn normalize_attempt_status(status: &str) -> String {
    match status {
        "verified_success" | "pass" => "pass",
        "failed" | "fail" => "fail",
        "error" => "error",
        "skipped" => "skipped",
        "stopped" => "inconclusive",
        _ => "inconclusive",
    }
    .to_string()
}

fn validate_sample_source(source: &str) -> Result<()> {
    if matches!(source, "process" | "planr_event" | "scorer" | "metering") {
        Ok(())
    } else {
        Err(anyhow!("invalid eval sample source: {source}"))
    }
}

fn validate_metering_basis(basis: &str) -> Result<()> {
    if matches!(
        basis,
        "actual_trusted" | "estimated" | "mixed" | "unavailable"
    ) {
        Ok(())
    } else {
        Err(anyhow!("invalid eval metering basis: {basis}"))
    }
}

fn validate_basis_confidence(confidence: &str) -> Result<()> {
    if matches!(
        confidence,
        "verified" | "estimated" | "mixed" | "unavailable"
    ) {
        Ok(())
    } else {
        Err(anyhow!("invalid eval basis confidence: {confidence}"))
    }
}

fn validate_sample_value_for_basis(value: &Value, basis: &str) -> Result<()> {
    if basis == "unavailable" {
        if value.is_null() {
            Ok(())
        } else {
            Err(anyhow!("unavailable eval sample values must be null"))
        }
    } else if value.is_null() {
        Err(anyhow!(
            "eval sample value must be present unless metering basis is unavailable"
        ))
    } else if value.as_f64().is_none_or(|value| !value.is_finite()) {
        Err(anyhow!(
            "eval sample value must be a finite number unless metering basis is unavailable"
        ))
    } else {
        Ok(())
    }
}

fn validate_estimate_provenance(provenance: Option<&Value>) -> Result<()> {
    let Some(provenance) = provenance else {
        bail!("estimated eval samples require estimate_provenance");
    };
    let Some(object) = provenance.as_object() else {
        bail!("estimated eval samples require canonical estimate_provenance object");
    };
    for field in [
        "pricing_reference_id",
        "pricing_reference_version",
        "currency",
        "effective_at",
    ] {
        if object
            .get(field)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            bail!("estimated eval samples require estimate_provenance.{field}");
        }
    }
    Ok(())
}

#[cfg(test)]
fn canonical_estimate_provenance() -> Value {
    json!({
        "pricing_reference_id": "openai-prices",
        "pricing_reference_version": "2026-07-01",
        "currency": "USD",
        "effective_at": "2026-07-01T00:00:00Z"
    })
}

fn ensure_eval_attempt_identity(
    tx: &rusqlite::Transaction<'_>,
    run_id: &str,
    case: &EvalCaseResultInput,
    sample: &EvalSampleInput,
    attempt_id: &str,
    case_result_id: &str,
) -> Result<()> {
    let expected = eval_attempt_identity_value(run_id, case, sample, attempt_id);
    let existing = tx
        .query_row(
            "SELECT id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, attempt_index, seed, terminal_status, countable, retry_of_attempt_id, fallback_of_attempt_id, escalation_of_attempt_id, resume_of_attempt_id, effective_client, effective_provider, effective_runtime, effective_model, effective_effort, effective_profile_id, profile_config_digest, runner_harness_version, route_observation_json, outcome_json FROM eval_attempts WHERE id = ?1",
            params![attempt_id],
            |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "run_id": row.get::<_, String>(1)?,
                    "case_id": row.get::<_, String>(2)?,
                    "scorer_id": row.get::<_, String>(3)?,
                    "scorer_version": row.get::<_, String>(4)?,
                    "repetition_index": row.get::<_, i64>(5)?,
                    "warmup": row.get::<_, i64>(6)? == 1,
                    "attempt_index": row.get::<_, i64>(7)?,
                    "seed": row.get::<_, i64>(8)?,
                    "terminal_status": row.get::<_, String>(9)?,
                    "countable": row.get::<_, i64>(10)? != 0,
                    "retry_of_attempt_id": row.get::<_, Option<String>>(11)?,
                    "fallback_of_attempt_id": row.get::<_, Option<String>>(12)?,
                    "escalation_of_attempt_id": row.get::<_, Option<String>>(13)?,
                    "resume_of_attempt_id": row.get::<_, Option<String>>(14)?,
                    "effective_client": row.get::<_, Option<String>>(15)?,
                    "effective_provider": row.get::<_, Option<String>>(16)?,
                    "effective_runtime": row.get::<_, Option<String>>(17)?,
                    "effective_model": row.get::<_, Option<String>>(18)?,
                    "effective_effort": row.get::<_, Option<String>>(19)?,
                    "effective_profile_id": row.get::<_, Option<String>>(20)?,
                    "profile_config_digest": row.get::<_, Option<String>>(21)?,
                    "runner_harness_version": row.get::<_, String>(22)?,
                    "route_observation": row.get::<_, Option<String>>(23)?.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()),
                    "outcome": serde_json::from_str::<Value>(&row.get::<_, String>(24)?).unwrap_or(Value::Null),
                }))
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        if existing != expected {
            bail!("attempt_identity_conflict");
        }
        return Ok(());
    }
    tx.execute(
        "INSERT INTO eval_attempts(id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, attempt_index, seed, terminal_status, countable, retry_of_attempt_id, fallback_of_attempt_id, escalation_of_attempt_id, resume_of_attempt_id, effective_client, effective_provider, effective_runtime, effective_model, effective_effort, effective_profile_id, profile_config_digest, runner_harness_version, route_observation_json, outcome_json, created_at)
         SELECT ?1, run_id, case_id, scorer_id, scorer_version, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, datetime('now') FROM eval_case_results WHERE id = ?22",
        params![
            attempt_id,
            sample.repetition_index,
            if sample.warmup { 1 } else { 0 },
            sample.attempt.attempt_index,
            sample.seed,
            sample.attempt.terminal_status,
            if sample.attempt.countable { 1 } else { 0 },
            sample.attempt.retry_of_attempt_id,
            sample.attempt.fallback_of_attempt_id,
            sample.attempt.escalation_of_attempt_id,
            sample.attempt.resume_of_attempt_id,
            sample.attempt.effective_client,
            sample.attempt.effective_provider,
            sample.attempt.effective_runtime,
            sample.attempt.effective_model,
            sample.attempt.effective_effort,
            sample.attempt.effective_profile_id,
            sample.attempt.profile_config_digest,
            sample.attempt.runner_harness_version,
            sample.attempt.route_observation.as_ref().map(Value::to_string),
            sample.attempt.outcome.to_string(),
            case_result_id,
        ],
    )?;
    Ok(())
}

fn eval_attempt_identity_value(
    run_id: &str,
    case: &EvalCaseResultInput,
    sample: &EvalSampleInput,
    attempt_id: &str,
) -> Value {
    json!({
        "id": attempt_id,
        "run_id": run_id,
        "case_id": case.case_id,
        "scorer_id": case.scorer_id,
        "scorer_version": case.scorer_version,
        "repetition_index": sample.repetition_index,
        "warmup": sample.warmup,
        "attempt_index": sample.attempt.attempt_index,
        "seed": sample.seed,
        "terminal_status": sample.attempt.terminal_status,
        "countable": sample.attempt.countable,
        "retry_of_attempt_id": sample.attempt.retry_of_attempt_id,
        "fallback_of_attempt_id": sample.attempt.fallback_of_attempt_id,
        "escalation_of_attempt_id": sample.attempt.escalation_of_attempt_id,
        "resume_of_attempt_id": sample.attempt.resume_of_attempt_id,
        "effective_client": sample.attempt.effective_client,
        "effective_provider": sample.attempt.effective_provider,
        "effective_runtime": sample.attempt.effective_runtime,
        "effective_model": sample.attempt.effective_model,
        "effective_effort": sample.attempt.effective_effort,
        "effective_profile_id": sample.attempt.effective_profile_id,
        "profile_config_digest": sample.attempt.profile_config_digest,
        "runner_harness_version": sample.attempt.runner_harness_version,
        "route_observation": sample.attempt.route_observation,
        "outcome": sample.attempt.outcome,
    })
}

fn eval_attempt_id(run_id: &str, case: &EvalCaseResultInput, sample: &EvalSampleInput) -> String {
    format!(
        "evattempt-{run_id}-{}-{}-{}-{}-{}-{}",
        case.case_id,
        case.scorer_id,
        case.scorer_version,
        sample.repetition_index,
        if sample.warmup { 1 } else { 0 },
        sample.attempt.attempt_index
    )
}

fn validate_verdict(verdict: &str) -> Result<()> {
    if matches!(
        verdict,
        "improved" | "regressed" | "no_material_difference" | "insufficient_evidence"
    ) {
        Ok(())
    } else {
        Err(anyhow!("invalid eval comparison verdict: {verdict}"))
    }
}

fn safety_pass_from_assertions(raw_assertions: &str) -> Option<bool> {
    let assertions = serde_json::from_str::<Value>(raw_assertions).ok()?;
    match assertions {
        Value::Array(items) => items.iter().find_map(safety_pass_from_assertion),
        Value::Object(_) => safety_pass_from_assertion(&assertions),
        _ => None,
    }
}

fn safety_pass_from_assertion(assertion: &Value) -> Option<bool> {
    if let Some(safety_pass) = assertion.get("safety_pass").and_then(Value::as_bool) {
        return Some(safety_pass);
    }
    if assertion.get("kind").and_then(Value::as_str) == Some("safety_pass") {
        return match assertion.get("status").and_then(Value::as_str) {
            Some("pass") => Some(true),
            Some("fail") => Some(false),
            _ => None,
        };
    }
    None
}

fn quality_pass_from_assertions(raw_assertions: &str) -> Option<bool> {
    let assertions = serde_json::from_str::<Value>(raw_assertions).ok()?;
    match assertions {
        Value::Array(items) => items.iter().find_map(quality_pass_from_assertion),
        Value::Object(_) => quality_pass_from_assertion(&assertions),
        _ => None,
    }
}

fn quality_pass_from_assertion(assertion: &Value) -> Option<bool> {
    if let Some(quality_pass) = assertion.get("quality_pass").and_then(Value::as_bool) {
        return Some(quality_pass);
    }
    if assertion.get("kind").and_then(Value::as_str) == Some("quality_pass") {
        return match assertion.get("status").and_then(Value::as_str) {
            Some("pass") => Some(true),
            Some("fail") => Some(false),
            _ => None,
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::ensure_schema;
    use rusqlite::Connection;
    use std::path::PathBuf;

    fn test_app() -> App {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let app = App::new(
            conn,
            PathBuf::from("."),
            PathBuf::from(":memory:"),
            true,
            false,
        );
        app.conn
            .execute(
                "INSERT INTO projects(id, name, root_path, status, created_at, updated_at) VALUES ('p-test', 'Test', '.', 'active', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
        app
    }

    fn test_app_at(db_path: PathBuf) -> App {
        let conn = Connection::open(&db_path).unwrap();
        ensure_schema(&conn).unwrap();
        let app = App::new(conn, PathBuf::from("."), db_path, true, false);
        app.conn
            .execute(
                "INSERT OR IGNORE INTO projects(id, name, root_path, status, created_at, updated_at) VALUES ('p-test', 'Test', '.', 'active', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
        app
    }

    fn suite_input(digest: &str) -> EvalSuiteSnapshotInput {
        EvalSuiteSnapshotInput {
            digest: digest.to_string(),
            suite_id: "suite".to_string(),
            suite_version: "1.0.0".to_string(),
            normalized_manifest: json!({"suite_id": "suite"}),
            fixture_digests: json!({"fixture": "sha256:abc"}),
            scorer_identities: json!(["scorer@1.0.0"]),
            case_order: json!(["case-a"]),
            baseline_selector: json!({"kind": "explicit_run_id", "run_id": "baseline"}),
            comparison_policy_digest: "sha256:policy".to_string(),
            metadata: json!({}),
            source_path: Some("eval/suite.json".to_string()),
        }
    }

    fn run_input(suite_digest: &str, id: &str, revision: &str, testbed: Value) -> EvalRunInput {
        EvalRunInput {
            id: Some(id.to_string()),
            suite_digest: suite_digest.to_string(),
            subject: EvalSubjectInput {
                kind: "planr_cli".to_string(),
                revision: revision.to_string(),
                path: Some(".".to_string()),
                argv: json!(["planr"]),
                label: None,
            },
            runner_version: "eval-runner-v1".to_string(),
            planr_version: "1.5.2".to_string(),
            testbed_fingerprint: testbed,
            source_state: json!({"commit": "abc"}),
            parent_run_id: None,
            resume_of: None,
            rescore_of: None,
            recompute_of: None,
        }
    }

    fn run_input_with_subject(
        suite_digest: &str,
        id: &str,
        revision: &str,
        testbed: Value,
        subject: EvalSubjectInput,
    ) -> EvalRunInput {
        EvalRunInput {
            subject,
            ..run_input(suite_digest, id, revision, testbed)
        }
    }

    fn passing_case() -> EvalCaseResultInput {
        EvalCaseResultInput {
            case_id: "case-a".to_string(),
            scorer_id: "scorer".to_string(),
            scorer_version: "1.0.0".to_string(),
            fixture_digest: "sha256:abc".to_string(),
            status: "pass".to_string(),
            repetition_count: 1,
            warmup_count: 0,
            assertions: json!([]),
            command: json!({"argv": ["planr"], "exit_code": 0}),
            reasons: json!([]),
        }
    }

    fn sample(value: i64) -> EvalSampleInput {
        sample_at(value, 0, false, "duration_ms")
    }

    fn sample_at(
        value: i64,
        repetition_index: i64,
        warmup: bool,
        measure: &str,
    ) -> EvalSampleInput {
        EvalSampleInput {
            id: Some(format!(
                "evsample-{value}-{repetition_index}-{}-{measure}",
                if warmup { "warmup" } else { "sample" }
            )),
            attempt: EvalAttemptInput {
                id: None,
                attempt_index: 0,
                terminal_status: "pass".to_string(),
                countable: !warmup,
                retry_of_attempt_id: None,
                fallback_of_attempt_id: None,
                escalation_of_attempt_id: None,
                resume_of_attempt_id: None,
                effective_client: Some("codex".to_string()),
                effective_provider: Some("openai".to_string()),
                effective_runtime: Some("native".to_string()),
                effective_model: Some("gpt-test".to_string()),
                effective_effort: Some("medium".to_string()),
                effective_profile_id: Some("test-profile".to_string()),
                profile_config_digest: Some("sha256:profile".to_string()),
                runner_harness_version: "eval-runner-v1".to_string(),
                route_observation: None,
                outcome: json!({"status": "pass"}),
            },
            repetition_index,
            warmup,
            seed: 1,
            measure: measure.to_string(),
            value: json!(value),
            unit: if measure == "tool_calls" {
                "count"
            } else {
                "ms"
            }
            .to_string(),
            source: "process".to_string(),
            metering_basis: "actual_trusted".to_string(),
            basis_source: "process".to_string(),
            basis_confidence: "verified".to_string(),
            estimate_provenance: None,
        }
    }

    #[test]
    fn eval_schema_migrates_existing_database_and_enforces_constraints() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        assert!(
            app.insert_eval_suite_snapshot(EvalSuiteSnapshotInput {
                normalized_manifest: json!({"suite_id": "mutated"}),
                ..suite_input("sha256:suite")
            })
            .is_err(),
            "same digest with different manifest must not overwrite history"
        );
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-a",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        app.record_eval_case_result(&run, passing_case(), vec![sample(10)])
            .unwrap();
        assert!(
            app.record_eval_case_result(&run, passing_case(), vec![sample(11)])
                .is_err(),
            "case identity is unique per run/scorer"
        );
    }

    #[test]
    fn eval_schema_upgrade_preserves_existing_project_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE projects(
              id TEXT PRIMARY KEY,
              name TEXT NOT NULL,
              root_path TEXT NOT NULL,
              status TEXT NOT NULL DEFAULT 'active',
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
            VALUES ('p-old', 'Old Project', '.', 'active', datetime('now'), datetime('now'));",
        )
        .unwrap();
        ensure_schema(&conn).unwrap();
        let project_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE id = 'p-old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let eval_tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('eval_suite_snapshots','eval_runs','eval_case_results','eval_attempts','eval_samples','eval_comparisons','eval_invalidations','eval_evidence_refs')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(project_count, 1);
        assert_eq!(eval_tables, 8);
    }

    #[test]
    fn eval_schema_upgrade_rebuilds_old_basis_confidence_constraint() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE eval_samples(
              id TEXT PRIMARY KEY,
              run_id TEXT NOT NULL,
              case_id TEXT NOT NULL,
              scorer_id TEXT NOT NULL,
              scorer_version TEXT NOT NULL,
              repetition_index INTEGER NOT NULL CHECK(repetition_index >= 0),
              warmup INTEGER NOT NULL CHECK(warmup IN (0, 1)),
              seed INTEGER NOT NULL,
              measure TEXT NOT NULL,
              value_json TEXT NOT NULL,
              unit TEXT NOT NULL,
              source TEXT NOT NULL CHECK(source IN ('process','planr_event','scorer','metering')),
              attempt_id TEXT NOT NULL,
              attempt_index INTEGER NOT NULL DEFAULT 0 CHECK(attempt_index >= 0),
              metering_basis TEXT NOT NULL DEFAULT 'actual_trusted' CHECK(metering_basis IN ('actual_trusted','estimated','mixed','unavailable')),
              basis_source TEXT NOT NULL DEFAULT 'process',
              basis_confidence TEXT NOT NULL DEFAULT 'actual_trusted' CHECK(basis_confidence IN ('actual_trusted','estimated','mixed','unavailable')),
              estimate_provenance TEXT,
              captured_at TEXT NOT NULL,
              UNIQUE(run_id, case_id, scorer_id, scorer_version, attempt_id, measure)
            );
            INSERT INTO eval_samples(
              id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, seed,
              measure, value_json, unit, source, attempt_id, attempt_index, metering_basis,
              basis_source, basis_confidence, estimate_provenance, captured_at
            ) VALUES (
              'old-sample', 'run-a', 'case-a', 'scorer', '1.0.0', 0, 0, 1,
              'wall_time_ms', '10', 'ms', 'metering', 'attempt-a', 0, 'actual_trusted',
              'metering', 'actual_trusted', NULL, '2026-07-21T00:00:00Z'
            );",
        )
        .unwrap();

        ensure_schema(&conn).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();

        let migrated_confidence: String = conn
            .query_row(
                "SELECT basis_confidence FROM eval_samples WHERE id = 'old-sample'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migrated_confidence, "verified");
        conn.execute(
            "INSERT INTO eval_samples(
              id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, seed,
              measure, value_json, unit, source, attempt_id, attempt_index, metering_basis,
              basis_source, basis_confidence, estimate_provenance, captured_at
            ) VALUES (
              'verified-sample', 'run-a', 'case-a', 'scorer', '1.0.0', 0, 0, 1,
              'duration_ms', '10', 'ms', 'process', 'attempt-a', 0, 'actual_trusted',
              'process', 'verified', NULL, '2026-07-21T00:00:01Z'
            )",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO eval_samples(
                  id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, seed,
                  measure, value_json, unit, source, attempt_id, attempt_index, metering_basis,
                  basis_source, basis_confidence, estimate_provenance, captured_at
                ) VALUES (
                  'rejected-sample', 'run-a', 'case-a', 'scorer', '1.0.0', 0, 0, 1,
                  'tool_calls', '1', 'count', 'metering', 'attempt-a', 0, 'actual_trusted',
                  'metering', 'actual_trusted', NULL, '2026-07-21T00:00:02Z'
                )",
                [],
            )
            .is_err(),
            "old actual_trusted basis_confidence must no longer be accepted"
        );
    }

    #[test]
    fn eval_attempt_lineage_and_unavailable_metering_are_persisted() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-attempts",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let mut failed = sample_at(250, 0, false, "wall_time_ms");
        failed.id = Some("evsample-failed-wall".to_string());
        failed.attempt.id = Some("evattempt-failed".to_string());
        failed.attempt.terminal_status = "fail".to_string();
        failed.attempt.outcome = json!({"exit_code": 1});
        let mut retry = sample_at(90, 0, false, "wall_time_ms");
        retry.id = Some("evsample-retry-wall".to_string());
        retry.attempt.id = Some("evattempt-retry".to_string());
        retry.attempt.attempt_index = 1;
        retry.attempt.retry_of_attempt_id = Some("evattempt-failed".to_string());
        retry.attempt.terminal_status = "pass".to_string();
        let mut unavailable = sample_at(0, 0, false, "cost_micros");
        unavailable.id = Some("evsample-retry-cost".to_string());
        unavailable.attempt.id = Some("evattempt-retry".to_string());
        unavailable.attempt.attempt_index = 1;
        unavailable.attempt.retry_of_attempt_id = Some("evattempt-failed".to_string());
        unavailable.value = Value::Null;
        unavailable.source = "metering".to_string();
        unavailable.unit = "micros".to_string();
        unavailable.metering_basis = "unavailable".to_string();
        unavailable.basis_source = "route_observation".to_string();
        unavailable.basis_confidence = "unavailable".to_string();
        app.record_eval_case_result(&run, passing_case(), vec![failed, retry, unavailable])
            .unwrap();

        let attempt_count: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM eval_attempts WHERE run_id = ?1 AND countable = 1",
                params![run],
                |row| row.get(0),
            )
            .unwrap();
        let retry_parent: String = app
            .conn
            .query_row(
                "SELECT retry_of_attempt_id FROM eval_attempts WHERE id = 'evattempt-retry'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let cost_value: String = app
            .conn
            .query_row(
                "SELECT value_json FROM eval_samples WHERE id = 'evsample-retry-cost'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(attempt_count, 2);
        assert_eq!(retry_parent, "evattempt-failed");
        assert_eq!(cost_value, "null");
    }

    #[test]
    fn eval_efficiency_summary_is_failure_inclusive_and_mixed_basis() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-efficiency-mixed",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let mut failed_wall = sample_at(100, 0, false, "wall_time_ms");
        failed_wall.id = Some("evsample-eff-failed-wall".to_string());
        failed_wall.attempt.id = Some("evattempt-eff-failed".to_string());
        failed_wall.attempt.terminal_status = "failed".to_string();
        failed_wall.attempt.outcome = json!({"exit_code": 1});
        let mut failed_cost = sample_at(300, 0, false, "cost_micros");
        failed_cost.id = Some("evsample-eff-failed-cost".to_string());
        failed_cost.attempt.id = Some("evattempt-eff-failed".to_string());
        failed_cost.attempt.terminal_status = "failed".to_string();
        failed_cost.attempt.outcome = json!({"exit_code": 1});
        failed_cost.unit = "micros".to_string();
        failed_cost.source = "metering".to_string();

        let mut retry_wall = sample_at(50, 0, false, "wall_time_ms");
        retry_wall.id = Some("evsample-eff-retry-wall".to_string());
        retry_wall.attempt.id = Some("evattempt-eff-retry".to_string());
        retry_wall.attempt.attempt_index = 1;
        retry_wall.attempt.retry_of_attempt_id = Some("evattempt-eff-failed".to_string());
        retry_wall.attempt.terminal_status = "verified_success".to_string();
        let mut retry_cost = sample_at(700, 0, false, "cost_micros");
        retry_cost.id = Some("evsample-eff-retry-cost".to_string());
        retry_cost.attempt.id = Some("evattempt-eff-retry".to_string());
        retry_cost.attempt.attempt_index = 1;
        retry_cost.attempt.retry_of_attempt_id = Some("evattempt-eff-failed".to_string());
        retry_cost.attempt.terminal_status = "verified_success".to_string();
        retry_cost.unit = "micros".to_string();
        retry_cost.source = "metering".to_string();
        retry_cost.metering_basis = "estimated".to_string();
        retry_cost.basis_source = "pricing_reference".to_string();
        retry_cost.basis_confidence = "estimated".to_string();
        retry_cost.estimate_provenance = Some(canonical_estimate_provenance());

        app.record_eval_case_result(
            &run,
            passing_case(),
            vec![failed_wall, failed_cost, retry_wall, retry_cost],
        )
        .unwrap();
        app.finish_eval_run(&run, "success").unwrap();

        let shown = app.eval_run_record_value(&run).unwrap();
        let summary = &shown["efficiency_summary"];
        assert_eq!(summary["countable_attempts"], json!(2));
        assert_eq!(summary["verified_successes"], json!(1));
        assert_eq!(
            summary["matched_denominator"]["retry_policy"]["retries"],
            json!(1)
        );
        assert_eq!(summary["total_cost_micros"]["value"], json!(1000));
        assert_eq!(summary["total_cost_micros"]["basis"], "mixed");
        assert_eq!(summary["cost_per_attempt_micros"]["value"], json!(500));
        assert_eq!(
            summary["cost_per_verified_success_micros"]["value"],
            json!(1000)
        );
        assert_eq!(
            summary["cost_per_verified_success_micros"]["estimate_provenance"],
            json!(["openai-prices@2026-07-01"])
        );
        assert_eq!(summary["total_wall_time_ms"]["value"], json!(150));
        assert_eq!(
            summary["wall_time_per_verified_success_ms"]["value"],
            json!(150)
        );
        assert_eq!(summary["sample_count"], json!(4));
        assert_eq!(summary["reasons"], json!([]));
    }

    #[test]
    fn eval_efficiency_summary_keeps_zero_success_and_unavailable_cost_null() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-efficiency-unavailable",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let mut failed_wall = sample_at(80, 0, false, "wall_time_ms");
        failed_wall.id = Some("evsample-eff-zero-wall".to_string());
        failed_wall.attempt.id = Some("evattempt-eff-zero".to_string());
        failed_wall.attempt.terminal_status = "failed".to_string();
        let mut failed_cost = sample_at(0, 0, false, "cost_micros");
        failed_cost.id = Some("evsample-eff-zero-cost".to_string());
        failed_cost.attempt.id = Some("evattempt-eff-zero".to_string());
        failed_cost.attempt.terminal_status = "failed".to_string();
        failed_cost.value = Value::Null;
        failed_cost.unit = "micros".to_string();
        failed_cost.source = "metering".to_string();
        failed_cost.metering_basis = "unavailable".to_string();
        failed_cost.basis_source = "provider_unavailable".to_string();
        failed_cost.basis_confidence = "unavailable".to_string();

        app.record_eval_case_result(&run, passing_case(), vec![failed_wall, failed_cost])
            .unwrap();
        app.finish_eval_run(&run, "success").unwrap();

        let shown = app.eval_run_record_value(&run).unwrap();
        let summary = &shown["efficiency_summary"];
        assert_eq!(summary["countable_attempts"], json!(1));
        assert_eq!(summary["verified_successes"], json!(0));
        assert_eq!(
            summary["cost_per_verified_success_micros"]["value"],
            Value::Null
        );
        assert_eq!(
            summary["cost_per_verified_success_micros"]["state"],
            "unavailable"
        );
        assert_eq!(
            summary["cost_per_verified_success_micros"]["basis"],
            "unavailable"
        );
        assert_eq!(
            summary["reasons"],
            json!([
                "samples_unavailable",
                "samples_below_minimum",
                "zero_verified_successes"
            ])
        );
    }

    #[test]
    fn eval_efficiency_summary_counts_only_explicit_verified_success() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-efficiency-pass-is-not-verified",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let mut wall = sample_at(80, 0, false, "wall_time_ms");
        wall.id = Some("evsample-eff-pass-wall".to_string());
        wall.attempt.id = Some("evattempt-eff-pass".to_string());
        wall.attempt.terminal_status = "pass".to_string();
        let mut cost = sample_at(200, 0, false, "cost_micros");
        cost.id = Some("evsample-eff-pass-cost".to_string());
        cost.attempt.id = Some("evattempt-eff-pass".to_string());
        cost.attempt.terminal_status = "pass".to_string();
        cost.unit = "micros".to_string();
        cost.source = "metering".to_string();

        app.record_eval_case_result(&run, passing_case(), vec![wall, cost])
            .unwrap();
        app.finish_eval_run(&run, "success").unwrap();

        let shown = app.eval_run_record_value(&run).unwrap();
        let summary = &shown["efficiency_summary"];
        assert_eq!(summary["countable_attempts"], json!(1));
        assert_eq!(summary["verified_successes"], json!(0));
        assert_eq!(
            summary["cost_per_verified_success_micros"]["value"],
            Value::Null
        );
        assert!(
            summary["reasons"]
                .as_array()
                .unwrap()
                .contains(&json!("zero_verified_successes"))
        );
    }

    #[test]
    fn eval_efficiency_summary_includes_countable_warmup_attempts() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-efficiency-countable-warmup",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let mut warmup_wall = sample_at(40, 0, true, "wall_time_ms");
        warmup_wall.id = Some("evsample-eff-warmup-wall".to_string());
        warmup_wall.attempt.id = Some("evattempt-eff-warmup".to_string());
        warmup_wall.attempt.terminal_status = "verified_success".to_string();
        warmup_wall.attempt.countable = true;
        let mut warmup_cost = sample_at(100, 0, true, "cost_micros");
        warmup_cost.id = Some("evsample-eff-warmup-cost".to_string());
        warmup_cost.attempt.id = Some("evattempt-eff-warmup".to_string());
        warmup_cost.attempt.terminal_status = "verified_success".to_string();
        warmup_cost.attempt.countable = true;
        warmup_cost.unit = "micros".to_string();
        warmup_cost.source = "metering".to_string();

        let mut measured_wall = sample_at(60, 0, false, "wall_time_ms");
        measured_wall.id = Some("evsample-eff-measured-wall".to_string());
        measured_wall.attempt.id = Some("evattempt-eff-measured".to_string());
        measured_wall.attempt.terminal_status = "failed".to_string();
        let mut measured_cost = sample_at(300, 0, false, "cost_micros");
        measured_cost.id = Some("evsample-eff-measured-cost".to_string());
        measured_cost.attempt.id = Some("evattempt-eff-measured".to_string());
        measured_cost.attempt.terminal_status = "failed".to_string();
        measured_cost.unit = "micros".to_string();
        measured_cost.source = "metering".to_string();

        app.record_eval_case_result(
            &run,
            passing_case(),
            vec![warmup_wall, warmup_cost, measured_wall, measured_cost],
        )
        .unwrap();
        app.finish_eval_run(&run, "success").unwrap();

        let shown = app.eval_run_record_value(&run).unwrap();
        let summary = &shown["efficiency_summary"];
        assert_eq!(summary["countable_attempts"], json!(2));
        assert_eq!(summary["verified_successes"], json!(1));
        assert_eq!(summary["sample_count"], json!(4));
        assert_eq!(summary["basis_mix"], json!(["actual_trusted"]));
        assert_eq!(summary["total_cost_micros"]["value"], json!(400));
        assert_eq!(
            summary["cost_per_verified_success_micros"]["value"],
            json!(400)
        );
        assert_eq!(
            summary["wall_time_per_verified_success_ms"]["value"],
            json!(100)
        );
    }

    #[test]
    fn eval_efficiency_summary_recomputes_stably_from_storage() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-efficiency-stable-recompute",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let mut wall = sample_at(50, 0, false, "wall_time_ms");
        wall.id = Some("evsample-eff-stable-wall".to_string());
        wall.attempt.id = Some("evattempt-eff-stable".to_string());
        wall.attempt.terminal_status = "verified_success".to_string();
        let mut cost = sample_at(125, 0, false, "cost_micros");
        cost.id = Some("evsample-eff-stable-cost".to_string());
        cost.attempt.id = Some("evattempt-eff-stable".to_string());
        cost.attempt.terminal_status = "verified_success".to_string();
        cost.unit = "micros".to_string();
        cost.source = "metering".to_string();

        app.record_eval_case_result(&run, passing_case(), vec![wall, cost])
            .unwrap();
        app.finish_eval_run(&run, "success").unwrap();

        let first = app.eval_run_record_value(&run).unwrap();
        let second = app.eval_run_record_value(&run).unwrap();
        assert_eq!(first["efficiency_summary"], second["efficiency_summary"]);
    }

    #[test]
    fn eval_attempt_identity_conflicts_rollback_case_result() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-attempt-conflict",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let mut wall = sample_at(120, 0, false, "wall_time_ms");
        wall.id = Some("evsample-conflict-wall".to_string());
        wall.attempt.id = Some("evattempt-conflict".to_string());
        wall.attempt.effective_model = Some("gpt-a".to_string());
        let mut cost = sample_at(7, 0, false, "cost_micros");
        cost.id = Some("evsample-conflict-cost".to_string());
        cost.attempt.id = Some("evattempt-conflict".to_string());
        cost.attempt.effective_model = Some("gpt-b".to_string());

        let err = app
            .record_eval_case_result(&run, passing_case(), vec![wall, cost])
            .unwrap_err();
        assert!(
            err.to_string().contains("attempt_identity_conflict"),
            "unexpected error: {err}"
        );
        let case_results: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM eval_case_results WHERE run_id = ?1",
                params![run],
                |row| row.get(0),
            )
            .unwrap();
        let samples: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM eval_samples WHERE run_id = ?1",
                params![run],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(case_results, 0);
        assert_eq!(samples, 0);
    }

    #[test]
    fn eval_samples_reject_unavailable_values_as_zero_and_estimates_without_provenance() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-metering-invalid",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let mut unavailable_zero = sample_at(0, 0, false, "cost_micros");
        unavailable_zero.metering_basis = "unavailable".to_string();
        unavailable_zero.basis_confidence = "unavailable".to_string();
        unavailable_zero.source = "metering".to_string();
        assert!(
            app.record_eval_case_result(&run, passing_case(), vec![unavailable_zero])
                .is_err(),
            "unavailable metering must not be coerced to zero"
        );

        let retry_run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-estimate-invalid",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let mut estimated = sample_at(10, 0, false, "cost_micros");
        estimated.metering_basis = "estimated".to_string();
        estimated.basis_confidence = "estimated".to_string();
        estimated.source = "metering".to_string();
        assert!(
            app.record_eval_case_result(&retry_run, passing_case(), vec![estimated])
                .is_err(),
            "estimated metering requires versioned provenance"
        );

        let malformed_run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-estimate-malformed",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let mut malformed = sample_at(10, 0, false, "cost_micros");
        malformed.metering_basis = "estimated".to_string();
        malformed.basis_confidence = "estimated".to_string();
        malformed.source = "metering".to_string();
        malformed.estimate_provenance = Some(json!({
            "method": "fixture-rate-card",
            "version": "live-oracle-v1",
            "rate_micros_per_attempt": 10
        }));
        let error = app
            .record_eval_case_result(&malformed_run, passing_case(), vec![malformed])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("estimate_provenance.pricing_reference_id"),
            "malformed estimated provenance must be rejected, got {error}"
        );
    }

    #[test]
    fn eval_run_lifecycle_invalidation_rescore_and_comparison_are_append_only() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-base",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-cand",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        app.record_eval_case_result(&baseline, passing_case(), vec![sample(10)])
            .unwrap();
        app.record_eval_case_result(&candidate, passing_case(), vec![sample(9)])
            .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();
        let comparison = app
            .create_eval_comparison(EvalComparisonInput {
                id: Some("evcmp-a".to_string()),
                baseline_run_id: baseline.clone(),
                candidate_run_id: candidate.clone(),
                policy_digest: "sha256:policy".to_string(),
                runner_version: "eval-runner-v1".to_string(),
                verdict: "improved".to_string(),
                reasons: json!(["candidate_improved"]),
                gates: json!([]),
                effect_estimates: json!({"duration_ms": -1}),
                uncertainty: json!({}),
                protected_dimensions: json!({}),
                recompute_of: None,
                rescore_of: None,
            })
            .unwrap();
        let invalidation = app
            .invalidate_eval_target(
                "run",
                &baseline,
                "stale baseline",
                json!(["evidence_stale"]),
                Some("recapture"),
            )
            .unwrap();
        let rescore = app
            .start_eval_run(EvalRunInput {
                id: Some("evrun-rescore".to_string()),
                rescore_of: Some(candidate.clone()),
                ..run_input("sha256:suite", "ignored", "rev-a", json!({"os": "x"}))
            })
            .unwrap();
        let status: String = app
            .conn
            .query_row(
                "SELECT status FROM eval_runs WHERE id = ?1",
                params![baseline],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "invalidated");
        assert!(!invalidation.is_empty());
        assert_eq!(comparison, "evcmp-a");
        assert_eq!(rescore, "evrun-rescore");
    }

    #[test]
    fn rescore_comparison_ignores_runtime_improvement_from_replay_variance() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-rescore-policy-base",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let faster_candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-rescore-policy-fast",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let rescore = app
            .start_eval_run(EvalRunInput {
                id: Some("evrun-rescore-policy-replay".to_string()),
                rescore_of: Some(baseline.clone()),
                ..run_input("sha256:suite", "ignored", "rev-a", json!({"os": "x"}))
            })
            .unwrap();
        let mut case = passing_case();
        case.repetition_count = 3;
        app.record_eval_case_result(
            &baseline,
            case.clone(),
            vec![
                sample_at(100, 0, false, "duration_ms"),
                sample_at(101, 1, false, "duration_ms"),
                sample_at(99, 2, false, "duration_ms"),
            ],
        )
        .unwrap();
        app.record_eval_case_result(
            &faster_candidate,
            case.clone(),
            vec![
                sample_at(70, 0, false, "duration_ms"),
                sample_at(71, 1, false, "duration_ms"),
                sample_at(69, 2, false, "duration_ms"),
            ],
        )
        .unwrap();
        app.record_eval_case_result(
            &rescore,
            case,
            vec![
                sample_at(80, 0, false, "duration_ms"),
                sample_at(81, 1, false, "duration_ms"),
                sample_at(79, 2, false, "duration_ms"),
            ],
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&faster_candidate, "success").unwrap();
        app.finish_eval_run(&rescore, "success").unwrap();

        let normal = app
            .compare_stored_eval_runs(
                &baseline,
                &faster_candidate,
                "default",
                &EvalComparisonPolicy::default(),
                EvalComparisonProvenance {
                    recompute_of: None,
                    rescore_of: None,
                },
            )
            .unwrap();
        let replay = app
            .compare_stored_eval_runs(
                &baseline,
                &rescore,
                "default",
                &EvalComparisonPolicy::default(),
                EvalComparisonProvenance {
                    recompute_of: None,
                    rescore_of: Some(&baseline),
                },
            )
            .unwrap();

        let normal = app.eval_comparison_value_by_id(&normal).unwrap();
        let replay = app.eval_comparison_value_by_id(&replay).unwrap();
        assert_eq!(normal["verdict"], "improved");
        assert_eq!(normal["reasons"], json!(["candidate_improved"]));
        assert_eq!(replay["verdict"], "no_material_difference");
        assert_eq!(replay["reasons"], json!(["no_material_effect"]));
    }

    #[test]
    fn rescore_comparison_keeps_regression_and_protected_policy_gates() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-rescore-regression-base",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let rescore = app
            .start_eval_run(EvalRunInput {
                id: Some("evrun-rescore-regression-replay".to_string()),
                rescore_of: Some(baseline.clone()),
                ..run_input("sha256:suite", "ignored", "rev-a", json!({"os": "x"}))
            })
            .unwrap();
        let mut case = passing_case();
        case.repetition_count = 3;
        app.record_eval_case_result(
            &baseline,
            case.clone(),
            vec![
                sample_at(100, 0, false, "duration_ms"),
                sample_at(101, 1, false, "duration_ms"),
                sample_at(99, 2, false, "duration_ms"),
                sample_at(10, 0, false, "tool_calls"),
                sample_at(10, 1, false, "tool_calls"),
                sample_at(10, 2, false, "tool_calls"),
            ],
        )
        .unwrap();
        app.record_eval_case_result(
            &rescore,
            case,
            vec![
                sample_at(125, 0, false, "duration_ms"),
                sample_at(126, 1, false, "duration_ms"),
                sample_at(124, 2, false, "duration_ms"),
                sample_at(20, 0, false, "tool_calls"),
                sample_at(20, 1, false, "tool_calls"),
                sample_at(20, 2, false, "tool_calls"),
            ],
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&rescore, "success").unwrap();

        let policy = EvalComparisonPolicy {
            protected_relative_max: BTreeMap::from([("tool_calls_relative_max".to_string(), 0.50)]),
            ..EvalComparisonPolicy::default()
        };
        let replay = app
            .compare_stored_eval_runs(
                &baseline,
                &rescore,
                "sha256:rescore-regression-policy",
                &policy,
                EvalComparisonProvenance {
                    recompute_of: None,
                    rescore_of: Some(&baseline),
                },
            )
            .unwrap();

        let replay = app.eval_comparison_value_by_id(&replay).unwrap();
        assert_eq!(replay["policy_digest"], "sha256:rescore-regression-policy");
        assert_eq!(replay["verdict"], "regressed");
        assert_eq!(
            replay["reasons"],
            json!([
                "performance_regressed",
                "protected_dimension_regressed",
                "cost_regressed"
            ])
        );
        assert_eq!(
            replay["protected_dimensions"]["tool_calls_relative"],
            json!(1.0)
        );
    }

    #[test]
    fn eval_run_terminal_status_is_write_once() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-terminal",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        app.record_eval_case_result(&run, passing_case(), vec![sample(10)])
            .unwrap();
        app.finish_eval_run(&run, "success").unwrap();

        assert!(
            app.finish_eval_run(&run, "error").is_err(),
            "terminal runs must not be rewritten"
        );
        assert!(
            app.finish_eval_run(&run, "started").is_err(),
            "started is not a terminal close status"
        );
        assert!(
            app.finish_eval_run("evrun-missing", "success").is_err(),
            "missing runs must not silently finish"
        );
        let status: String = app
            .conn
            .query_row(
                "SELECT status FROM eval_runs WHERE id = ?1",
                params![run],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "success");
    }

    #[test]
    fn eval_invalidation_is_referential_transactional_and_unique() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-inv-base",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-inv-cand",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        app.record_eval_case_result(&baseline, passing_case(), vec![sample(41)])
            .unwrap();
        app.record_eval_case_result(&candidate, passing_case(), vec![sample(42)])
            .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();
        let comparison = app
            .create_eval_comparison(EvalComparisonInput {
                id: Some("evcmp-inv".to_string()),
                baseline_run_id: baseline.clone(),
                candidate_run_id: candidate,
                policy_digest: "sha256:policy".to_string(),
                runner_version: "eval-runner-v1".to_string(),
                verdict: "improved".to_string(),
                reasons: json!([]),
                gates: json!([]),
                effect_estimates: json!({}),
                uncertainty: json!({}),
                protected_dimensions: json!({}),
                recompute_of: None,
                rescore_of: None,
            })
            .unwrap();

        assert!(
            app.invalidate_eval_target("run", "evrun-missing", "missing", json!(["missing"]), None)
                .is_err()
        );
        assert!(
            app.invalidate_eval_target(
                "comparison",
                "evcmp-missing",
                "missing",
                json!(["missing"]),
                None
            )
            .is_err()
        );
        let count_after_missing: i64 = app
            .conn
            .query_row("SELECT COUNT(*) FROM eval_invalidations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count_after_missing, 0, "failed invalidations roll back");

        app.invalidate_eval_target(
            "run",
            &baseline,
            "stale baseline",
            json!(["evidence_stale"]),
            None,
        )
        .unwrap();
        assert!(
            app.invalidate_eval_target(
                "run",
                &baseline,
                "stale again",
                json!(["evidence_stale"]),
                None
            )
            .is_err(),
            "run invalidation is unique"
        );

        app.invalidate_eval_target(
            "comparison",
            &comparison,
            "policy changed",
            json!(["policy_changed"]),
            None,
        )
        .unwrap();
        assert!(
            app.invalidate_eval_target(
                "comparison",
                &comparison,
                "policy changed again",
                json!(["policy_changed"]),
                None
            )
            .is_err(),
            "comparison invalidation is unique"
        );
    }

    #[test]
    fn eval_case_sample_write_rolls_back_on_partial_failure() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-rollback",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();

        let mut invalid_sample = sample(50);
        invalid_sample.source = "unknown-source".to_string();
        assert!(
            app.record_eval_case_result(&run, passing_case(), vec![invalid_sample])
                .is_err()
        );
        let case_count: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM eval_case_results WHERE run_id = ?1",
                params![run],
                |row| row.get(0),
            )
            .unwrap();
        let sample_count: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM eval_samples WHERE run_id = ?1",
                params![run],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(case_count, 0);
        assert_eq!(sample_count, 0);
    }

    #[test]
    fn eval_resume_reuses_only_identity_compatible_successful_samples() {
        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        app.insert_eval_suite_snapshot(suite_input("sha256:other-suite"))
            .unwrap();
        let testbed = json!({"os": "x", "arch": "a"});
        let good = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-good",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        app.record_eval_case_result(&good, passing_case(), vec![sample(10)])
            .unwrap();
        app.finish_eval_run(&good, "success").unwrap();

        let wrong_revision = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-wrong-rev",
                "rev-b",
                testbed.clone(),
            ))
            .unwrap();
        app.record_eval_case_result(&wrong_revision, passing_case(), vec![sample(20)])
            .unwrap();
        app.finish_eval_run(&wrong_revision, "success").unwrap();

        let wrong_suite = app
            .start_eval_run(run_input(
                "sha256:other-suite",
                "evrun-wrong-suite",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        app.record_eval_case_result(&wrong_suite, passing_case(), vec![sample(30)])
            .unwrap();
        app.finish_eval_run(&wrong_suite, "success").unwrap();

        let wrong_kind_subject = EvalSubjectInput {
            kind: "other_runner".to_string(),
            revision: "rev-a".to_string(),
            path: Some(".".to_string()),
            argv: json!(["planr"]),
            label: None,
        };
        let wrong_kind = app
            .start_eval_run(run_input_with_subject(
                "sha256:suite",
                "evrun-wrong-kind",
                "rev-a",
                testbed.clone(),
                wrong_kind_subject,
            ))
            .unwrap();
        app.record_eval_case_result(&wrong_kind, passing_case(), vec![sample(40)])
            .unwrap();
        app.finish_eval_run(&wrong_kind, "success").unwrap();

        let wrong_path_subject = EvalSubjectInput {
            kind: "planr_cli".to_string(),
            revision: "rev-a".to_string(),
            path: Some("./other".to_string()),
            argv: json!(["planr"]),
            label: None,
        };
        let wrong_path = app
            .start_eval_run(run_input_with_subject(
                "sha256:suite",
                "evrun-wrong-path",
                "rev-a",
                testbed.clone(),
                wrong_path_subject,
            ))
            .unwrap();
        app.record_eval_case_result(&wrong_path, passing_case(), vec![sample(50)])
            .unwrap();
        app.finish_eval_run(&wrong_path, "success").unwrap();

        let wrong_argv_subject = EvalSubjectInput {
            kind: "planr_cli".to_string(),
            revision: "rev-a".to_string(),
            path: Some(".".to_string()),
            argv: json!(["planr", "--other"]),
            label: None,
        };
        let wrong_argv = app
            .start_eval_run(run_input_with_subject(
                "sha256:suite",
                "evrun-wrong-argv",
                "rev-a",
                testbed.clone(),
                wrong_argv_subject,
            ))
            .unwrap();
        app.record_eval_case_result(&wrong_argv, passing_case(), vec![sample(60)])
            .unwrap();
        app.finish_eval_run(&wrong_argv, "success").unwrap();

        let subject = EvalSubjectInput {
            kind: "planr_cli".to_string(),
            revision: "rev-a".to_string(),
            path: Some(".".to_string()),
            argv: json!(["planr"]),
            label: None,
        };

        let reusable = app
            .reusable_eval_samples(
                "sha256:suite",
                &subject,
                &testbed,
                None,
                EvalReuseCaseKey {
                    case_id: "case-a",
                    scorer_id: "scorer",
                    scorer_version: "1.0.0",
                },
            )
            .unwrap();
        assert_eq!(reusable.len(), 1);
        assert_eq!(reusable[0]["value"], json!(10));
    }

    #[test]
    fn eval_file_backed_reopen_preserves_evidence_and_resume_lineage() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("planr.sqlite");

        {
            let app = test_app_at(db_path.clone());
            app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
                .unwrap();
            let interrupted = app
                .start_eval_run(run_input(
                    "sha256:suite",
                    "evrun-interrupted",
                    "rev-a",
                    json!({"os": "x"}),
                ))
                .unwrap();
            app.record_eval_case_result(&interrupted, passing_case(), vec![sample(70)])
                .unwrap();
        }

        {
            let app = test_app_at(db_path.clone());
            let status: String = app
                .conn
                .query_row(
                    "SELECT status FROM eval_runs WHERE id = 'evrun-interrupted'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(status, "started");

            let resumed = app
                .start_eval_run(EvalRunInput {
                    id: Some("evrun-resumed".to_string()),
                    resume_of: Some("evrun-interrupted".to_string()),
                    ..run_input("sha256:suite", "ignored", "rev-a", json!({"os": "x"}))
                })
                .unwrap();
            app.record_eval_case_result(&resumed, passing_case(), vec![sample(71)])
                .unwrap();
            app.finish_eval_run(&resumed, "success").unwrap();
            app.attach_eval_evidence_ref("run", &resumed, "log", "log-resumed", "item-a")
                .unwrap();
        }

        {
            let app = test_app_at(db_path);
            let lineage: String = app
                .conn
                .query_row(
                    "SELECT resume_of FROM eval_runs WHERE id = 'evrun-resumed'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let evidence: i64 = app
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM eval_evidence_refs WHERE target_id = 'evrun-resumed' AND closure_authority = 0",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let samples: i64 = app
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM eval_samples WHERE run_id IN ('evrun-interrupted', 'evrun-resumed')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(lineage, "evrun-interrupted");
            assert_eq!(evidence, 1);
            assert_eq!(samples, 2);
        }
    }

    #[test]
    fn eval_resume_token_counts_distinct_non_warmup_repetitions_with_measure_coverage() {
        use crate::eval_runner::{
            EvalAssertion, EvalFixture, EvalRunOptions, EvalRunnerCase, EvalRunnerManifest,
            EvalRunnerSafety, EvalRunnerSubject, EvalSampling, run_eval_manifest,
        };

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let run = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-resume-token",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let mut case = passing_case();
        case.repetition_count = 2;
        case.warmup_count = 1;
        app.record_eval_case_result(
            &run,
            case,
            vec![
                sample_at(1, 0, true, "duration_ms"),
                sample_at(2, 0, true, "tool_calls"),
                sample_at(3, 0, false, "duration_ms"),
                sample_at(4, 0, false, "tool_calls"),
                sample_at(5, 1, false, "duration_ms"),
            ],
        )
        .unwrap();
        app.finish_eval_run(&run, "success").unwrap();
        let subject = EvalSubjectInput {
            kind: "planr_cli".to_string(),
            revision: "rev-a".to_string(),
            path: Some(".".to_string()),
            argv: json!(["planr"]),
            label: None,
        };
        let required_measures = vec!["duration_ms".to_string(), "tool_calls".to_string()];
        assert!(
            app.reusable_eval_case_evidence(EvalReuseCaseCriteria {
                suite_digest: "sha256:suite",
                subject: &subject,
                testbed_fingerprint: &testbed,
                resume_parent_run_id: None,
                case_id: "case-a",
                scorer_id: "scorer",
                scorer_version: "1.0.0",
                required_measures: &required_measures,
                required_repetitions: 2,
            })
            .unwrap()
            .is_none(),
            "warmups, duplicate measures from one repetition, and under-covered repetitions cannot skip"
        );
        let enough_for_one = app
            .reusable_eval_case_evidence(EvalReuseCaseCriteria {
                suite_digest: "sha256:suite",
                subject: &subject,
                testbed_fingerprint: &testbed,
                resume_parent_run_id: None,
                case_id: "case-a",
                scorer_id: "scorer",
                scorer_version: "1.0.0",
                required_measures: &required_measures,
                required_repetitions: 1,
            })
            .unwrap()
            .expect("one fully covered repetition is reusable for one required repetition");
        assert_eq!(enough_for_one.completed_repetition_count(), 1);

        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("fixture.txt"), b"fixture").unwrap();
        let manifest = EvalRunnerManifest {
            schema_version: "eval.suite.v1".to_string(),
            suite_id: "suite".to_string(),
            scorers: Vec::new(),
            fixtures: vec![EvalFixture {
                id: "fixture".to_string(),
                path: "fixture.txt".to_string(),
                digest: "sha256:f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d"
                    .to_string(),
            }],
            cases: vec![EvalRunnerCase {
                case_id: "case-a".to_string(),
                fixture_ids: vec!["fixture".to_string()],
                scorer_id: "scorer".to_string(),
                scorer_version: "1.0.0".to_string(),
                scorer_control: None,
                subject: EvalRunnerSubject {
                    kind: "planr_cli".to_string(),
                    argv: vec!["printf".to_string(), "ok".to_string()],
                },
                attempt_transitions: Vec::new(),
                assertions: vec![EvalAssertion::ExitCode(0)],
                measures: required_measures,
                sampling: EvalSampling {
                    repetitions: 1,
                    warmups: 0,
                    seed: 0,
                    max_attempts: 1,
                },
                timeout_ms: 1_000,
                output_limit_bytes: 1_024,
            }],
            safety: EvalRunnerSafety {
                allow_shell: false,
                max_concurrency: 1,
                allow_environment_capture: false,
            },
        };
        let skipped = run_eval_manifest(
            root.path(),
            &manifest,
            EvalRunOptions {
                reusable_cases: vec![enough_for_one],
                ..EvalRunOptions::default()
            },
        )
        .unwrap();
        assert!(skipped.cases.is_empty());
    }

    #[test]
    fn eval_resume_from_interrupted_parent_skips_completed_case_and_runs_incomplete_case() {
        use crate::eval_runner::{
            EvalAssertion, EvalFixture, EvalRunOptions, EvalRunnerCase, EvalRunnerManifest,
            EvalRunnerSafety, EvalRunnerSubject, EvalSampling, run_eval_manifest,
        };

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let parent = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-parent-interrupted",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let mut completed_case = passing_case();
        completed_case.case_id = "case-a".to_string();
        app.record_eval_case_result(
            &parent,
            completed_case,
            vec![sample_at(10, 0, false, "duration_ms")],
        )
        .unwrap();

        let subject = EvalSubjectInput {
            kind: "planr_cli".to_string(),
            revision: "rev-a".to_string(),
            path: Some(".".to_string()),
            argv: json!(["planr"]),
            label: None,
        };
        let required_measures = vec!["duration_ms".to_string()];
        assert!(
            app.reusable_eval_case_evidence(EvalReuseCaseCriteria {
                suite_digest: "sha256:suite",
                subject: &subject,
                testbed_fingerprint: &testbed,
                resume_parent_run_id: None,
                case_id: "case-a",
                scorer_id: "scorer",
                scorer_version: "1.0.0",
                required_measures: &required_measures,
                required_repetitions: 1,
            })
            .unwrap()
            .is_none(),
            "ordinary reuse must not consume non-terminal parent evidence without declared lineage"
        );
        let completed_parent_case = app
            .reusable_eval_case_evidence(EvalReuseCaseCriteria {
                suite_digest: "sha256:suite",
                subject: &subject,
                testbed_fingerprint: &testbed,
                resume_parent_run_id: Some(&parent),
                case_id: "case-a",
                scorer_id: "scorer",
                scorer_version: "1.0.0",
                required_measures: &required_measures,
                required_repetitions: 1,
            })
            .unwrap()
            .expect("declared resume parent exposes its completed passing case");

        let resumed = app
            .start_eval_run(EvalRunInput {
                id: Some("evrun-child-resume".to_string()),
                resume_of: Some(parent.clone()),
                ..run_input("sha256:suite", "ignored", "rev-a", testbed.clone())
            })
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("fixture.txt"), b"fixture").unwrap();
        let make_case = |case_id: &str, output: &str| EvalRunnerCase {
            case_id: case_id.to_string(),
            fixture_ids: vec!["fixture".to_string()],
            scorer_id: "scorer".to_string(),
            scorer_version: "1.0.0".to_string(),
            scorer_control: None,
            subject: EvalRunnerSubject {
                kind: "planr_cli".to_string(),
                argv: vec!["printf".to_string(), output.to_string()],
            },
            attempt_transitions: Vec::new(),
            assertions: vec![EvalAssertion::ExitCode(0)],
            measures: required_measures.clone(),
            sampling: EvalSampling {
                repetitions: 1,
                warmups: 0,
                seed: 0,
                max_attempts: 1,
            },
            timeout_ms: 1_000,
            output_limit_bytes: 1_024,
        };
        let manifest = EvalRunnerManifest {
            schema_version: "eval.suite.v1".to_string(),
            suite_id: "suite".to_string(),
            scorers: Vec::new(),
            fixtures: vec![EvalFixture {
                id: "fixture".to_string(),
                path: "fixture.txt".to_string(),
                digest: "sha256:f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d"
                    .to_string(),
            }],
            cases: vec![make_case("case-a", "skipped"), make_case("case-b", "ran")],
            safety: EvalRunnerSafety {
                allow_shell: false,
                max_concurrency: 1,
                allow_environment_capture: false,
            },
        };
        let runner_output = run_eval_manifest(
            root.path(),
            &manifest,
            EvalRunOptions {
                reusable_cases: vec![completed_parent_case],
                ..EvalRunOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            runner_output
                .cases
                .iter()
                .map(|case| case.case_id.as_str())
                .collect::<Vec<_>>(),
            vec!["case-b"],
            "resume must skip only the parent-completed case and run the incomplete case"
        );

        let mut resumed_case = passing_case();
        resumed_case.case_id = "case-b".to_string();
        app.record_eval_case_result(
            &resumed,
            resumed_case,
            vec![sample_at(20, 0, false, "duration_ms")],
        )
        .unwrap();
        app.finish_eval_run(&resumed, "success").unwrap();
        let parent_status: String = app
            .conn
            .query_row(
                "SELECT status FROM eval_runs WHERE id = ?1",
                params![parent],
                |row| row.get(0),
            )
            .unwrap();
        let child_parent: String = app
            .conn
            .query_row(
                "SELECT resume_of FROM eval_runs WHERE id = ?1",
                params![resumed],
                |row| row.get(0),
            )
            .unwrap();
        let parent_case_count: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM eval_case_results WHERE run_id = 'evrun-parent-interrupted' AND case_id = 'case-a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let resumed_case_count: i64 = app
            .conn
            .query_row(
                "SELECT COUNT(*) FROM eval_case_results WHERE run_id = 'evrun-child-resume' AND case_id = 'case-b'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(parent_status, "started");
        assert_eq!(child_parent, "evrun-parent-interrupted");
        assert_eq!(parent_case_count, 1);
        assert_eq!(resumed_case_count, 1);
    }

    #[test]
    fn eval_stored_raw_evidence_reproduces_comparison_verdict() {
        use crate::eval_compare::{EvalComparisonPolicy, compare_eval_runs};

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-compare-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-compare-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();
        let mut baseline_case = passing_case();
        baseline_case.repetition_count = 5;
        app.record_eval_case_result(
            &baseline,
            baseline_case,
            vec![
                sample_at(100, 0, false, "duration_ms"),
                sample_at(101, 1, false, "duration_ms"),
                sample_at(99, 2, false, "duration_ms"),
                sample_at(102, 3, false, "duration_ms"),
                sample_at(100, 4, false, "duration_ms"),
            ],
        )
        .unwrap();
        let mut candidate_case = passing_case();
        candidate_case.repetition_count = 5;
        app.record_eval_case_result(
            &candidate,
            candidate_case,
            vec![
                sample_at(80, 0, false, "duration_ms"),
                sample_at(81, 1, false, "duration_ms"),
                sample_at(79, 2, false, "duration_ms"),
                sample_at(82, 3, false, "duration_ms"),
                sample_at(80, 4, false, "duration_ms"),
            ],
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();

        let policy = EvalComparisonPolicy::default();
        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let (stored_verdict, stored_reasons): (String, String) = app
            .conn
            .query_row(
                "SELECT verdict, reasons FROM eval_comparisons WHERE id = ?1",
                params![comparison_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let replayed = compare_eval_runs(
            Some(&app.eval_run_evidence(&baseline).unwrap()),
            Some(&app.eval_run_evidence(&candidate).unwrap()),
            &policy,
        );
        assert_eq!(stored_verdict, replayed.verdict);
        assert_eq!(
            serde_json::from_str::<Value>(&stored_reasons).unwrap(),
            json!(replayed.reasons)
        );
        assert_eq!(stored_verdict, "improved");
    }

    #[test]
    fn eval_stored_quality_evidence_reproduces_wilson_verdict() {
        use crate::eval_compare::{
            EvalCaseEvidence as DirectCaseEvidence, EvalComparisonPolicy,
            EvalNumericSample as DirectNumericSample,
            EvalRepetitionOutcome as DirectRepetitionOutcome, EvalRunEvidence as DirectRunEvidence,
            compare_eval_runs,
        };

        fn direct_quality_run(run_id: &str, quality_passes: &[bool]) -> DirectRunEvidence {
            DirectRunEvidence {
                run_id: run_id.to_string(),
                suite_digest: "sha256:suite".to_string(),
                subject_kind: "planr_cli".to_string(),
                subject_path: Some(".".to_string()),
                subject_argv: "[\"planr\"]".to_string(),
                testbed_fingerprint: "{\"os\":\"x\"}".to_string(),
                status: "success".to_string(),
                invalidated: false,
                age_hours: 0,
                cases: quality_passes
                    .iter()
                    .enumerate()
                    .map(|(index, quality_pass)| DirectCaseEvidence {
                        case_id: format!("case-{index:02}"),
                        scorer_id: "scorer".to_string(),
                        scorer_version: "1.0.0".to_string(),
                        scorer_control: None,
                        fixture_digest: "sha256:abc".to_string(),
                        status: "pass".to_string(),
                        repetition_outcomes: vec![DirectRepetitionOutcome {
                            repetition_index: 0,
                            status: "pass".to_string(),
                        }],
                        safety_pass: None,
                        quality_pass: Some(*quality_pass),
                        min_successful_samples: Some(1),
                        required_repetitions: Some(1),
                        samples: vec![DirectNumericSample {
                            repetition_index: 0,
                            warmup: false,
                            seed: 1,
                            measure: "duration_ms".to_string(),
                            value: 100.0,
                            valid: true,
                        }],
                    })
                    .collect(),
            }
        }

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-quality-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-quality-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();
        let baseline_quality = vec![true; 20];
        let mut candidate_quality = vec![true; 20];
        for quality_pass in candidate_quality.iter_mut().take(8) {
            *quality_pass = false;
        }
        for (run_id, quality_values) in [
            (&baseline, baseline_quality.as_slice()),
            (&candidate, candidate_quality.as_slice()),
        ] {
            for (index, quality_pass) in quality_values.iter().enumerate() {
                let mut case = passing_case();
                case.case_id = format!("case-{index:02}");
                case.assertions = json!([{
                    "kind": "quality_pass",
                    "status": if *quality_pass { "pass" } else { "fail" }
                }]);
                let mut sample = sample_at(100, 0, false, "duration_ms");
                sample.id = None;
                app.record_eval_case_result(run_id, case, vec![sample])
                    .unwrap();
            }
            app.finish_eval_run(run_id, "success").unwrap();
        }

        let policy = EvalComparisonPolicy::default();
        let direct = compare_eval_runs(
            Some(&direct_quality_run(&baseline, &baseline_quality)),
            Some(&direct_quality_run(&candidate, &candidate_quality)),
            &policy,
        );
        assert_eq!(direct.verdict, "regressed");
        assert_eq!(direct.reasons, vec!["quality_regressed"]);

        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:quality-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let (stored_verdict, stored_reasons, stored_uncertainty): (String, String, String) = app
            .conn
            .query_row(
                "SELECT verdict, reasons, uncertainty FROM eval_comparisons WHERE id = ?1",
                params![comparison_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(stored_verdict, direct.verdict);
        assert_eq!(
            serde_json::from_str::<Value>(&stored_reasons).unwrap(),
            json!(direct.reasons)
        );
        assert_eq!(
            serde_json::from_str::<Value>(&stored_uncertainty).unwrap(),
            serde_json::from_str::<Value>(&direct.uncertainty.to_string()).unwrap()
        );
    }

    #[test]
    fn eval_stored_safety_evidence_hard_gate_reproduces_regression() {
        use crate::eval_compare::{EvalComparisonPolicy, compare_eval_runs};

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-safety-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-safety-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();
        let mut baseline_case = passing_case();
        baseline_case.repetition_count = 5;
        baseline_case.assertions = json!([{
            "kind": "safety_pass",
            "status": "pass"
        }]);
        app.record_eval_case_result(
            &baseline,
            baseline_case,
            vec![
                sample_at(100, 0, false, "duration_ms"),
                sample_at(101, 1, false, "duration_ms"),
                sample_at(99, 2, false, "duration_ms"),
                sample_at(102, 3, false, "duration_ms"),
                sample_at(100, 4, false, "duration_ms"),
            ],
        )
        .unwrap();
        let mut candidate_case = passing_case();
        candidate_case.repetition_count = 5;
        candidate_case.assertions = json!([{
            "kind": "safety_pass",
            "status": "fail"
        }]);
        app.record_eval_case_result(
            &candidate,
            candidate_case,
            vec![
                sample_at(80, 0, false, "duration_ms"),
                sample_at(81, 1, false, "duration_ms"),
                sample_at(79, 2, false, "duration_ms"),
                sample_at(82, 3, false, "duration_ms"),
                sample_at(80, 4, false, "duration_ms"),
            ],
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();

        let policy = EvalComparisonPolicy::default();
        let direct = compare_eval_runs(
            Some(&app.eval_run_evidence(&baseline).unwrap()),
            Some(&app.eval_run_evidence(&candidate).unwrap()),
            &policy,
        );
        assert_eq!(direct.verdict, "regressed");
        assert_eq!(direct.reasons, vec!["safety_regressed"]);
        assert_eq!(direct.effect_estimates, json!({}));

        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:safety-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let (stored_verdict, stored_reasons, stored_gates, stored_effects): (
            String,
            String,
            String,
            String,
        ) = app
            .conn
            .query_row(
                "SELECT verdict, reasons, gates, effect_estimates FROM eval_comparisons WHERE id = ?1",
                params![comparison_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(stored_verdict, direct.verdict);
        assert_eq!(
            serde_json::from_str::<Value>(&stored_reasons).unwrap(),
            json!(direct.reasons)
        );
        assert_eq!(
            serde_json::from_str::<Value>(&stored_effects).unwrap(),
            json!({})
        );
        let gates = serde_json::from_str::<Value>(&stored_gates).unwrap();
        let safety_gate = gates
            .as_array()
            .unwrap()
            .iter()
            .find(|gate| gate["gate"] == "safety")
            .expect("stored safety gate is recorded");
        assert_eq!(safety_gate["status"], "fail");
        assert_eq!(safety_gate["reasons"], json!(["safety_regressed"]));
    }

    #[test]
    fn eval_stored_zero_baseline_tool_calls_regresses_protected_cost_gate() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-zero-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-zero-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();
        let unchanged_candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-zero-cand-unchanged",
                "rev-c",
                json!({"os": "x"}),
            ))
            .unwrap();
        let mut baseline_case = passing_case();
        baseline_case.repetition_count = 3;
        app.record_eval_case_result(
            &baseline,
            baseline_case,
            vec![
                sample_at(0, 0, false, "tool_calls"),
                sample_at(0, 1, false, "tool_calls"),
                sample_at(0, 2, false, "tool_calls"),
            ],
        )
        .unwrap();
        let mut unchanged_case = passing_case();
        unchanged_case.repetition_count = 3;
        let mut unchanged_samples = vec![
            sample_at(0, 0, false, "tool_calls"),
            sample_at(0, 1, false, "tool_calls"),
            sample_at(0, 2, false, "tool_calls"),
        ];
        for sample in &mut unchanged_samples {
            sample.id = None;
        }
        app.record_eval_case_result(&unchanged_candidate, unchanged_case, unchanged_samples)
            .unwrap();
        let mut candidate_case = passing_case();
        candidate_case.repetition_count = 3;
        app.record_eval_case_result(
            &candidate,
            candidate_case,
            vec![
                sample_at(1, 0, false, "tool_calls"),
                sample_at(1, 1, false, "tool_calls"),
                sample_at(1, 2, false, "tool_calls"),
            ],
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&unchanged_candidate, "success")
            .unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();

        let policy = EvalComparisonPolicy {
            material_improvement: BTreeMap::new(),
            material_regression: BTreeMap::new(),
            protected_relative_max: BTreeMap::from([("tool_calls_relative_max".to_string(), 2.0)]),
            ..EvalComparisonPolicy::default()
        };
        let unchanged_comparison = app
            .compare_stored_eval_runs(
                &baseline,
                &unchanged_candidate,
                "sha256:zero-unchanged-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let (unchanged_verdict, unchanged_reasons, unchanged_effects, unchanged_uncertainty): (
            String,
            String,
            String,
            String,
        ) = app
            .conn
            .query_row(
                "SELECT verdict, reasons, effect_estimates, uncertainty FROM eval_comparisons WHERE id = ?1",
                params![unchanged_comparison],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(unchanged_verdict, "no_material_difference");
        assert_eq!(
            serde_json::from_str::<Value>(&unchanged_reasons).unwrap(),
            json!(["no_material_effect"])
        );
        assert_eq!(
            serde_json::from_str::<Value>(&unchanged_effects).unwrap()["tool_calls_p95_relative"],
            json!(0.0)
        );
        assert_eq!(
            serde_json::from_str::<Value>(&unchanged_uncertainty).unwrap()["tool_calls_p95_ci95"],
            json!([0.0, 0.0])
        );

        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:zero-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let (verdict, reasons, effect_estimates, uncertainty, protected): (
            String,
            String,
            String,
            String,
            String,
        ) = app
            .conn
            .query_row(
                "SELECT verdict, reasons, effect_estimates, uncertainty, protected_dimensions FROM eval_comparisons WHERE id = ?1",
                params![comparison_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(verdict, "regressed");
        assert_eq!(
            serde_json::from_str::<Value>(&reasons).unwrap(),
            json!(["protected_dimension_regressed", "cost_regressed"])
        );
        assert_eq!(
            serde_json::from_str::<Value>(&effect_estimates).unwrap()["tool_calls_p95_relative"],
            json!("unbounded_positive")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&uncertainty).unwrap()["tool_calls_p95_ci95"],
            json!(["unbounded_positive", "unbounded_positive"])
        );
        assert_eq!(
            serde_json::from_str::<Value>(&protected).unwrap()["tool_calls_relative"],
            json!("unbounded_positive")
        );
    }

    #[test]
    fn eval_stored_comparison_uses_manifest_minimum_not_recorded_repetitions() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        let mut suite = suite_input("sha256:min-suite");
        suite.normalized_manifest = json!({
            "suite_id": "suite",
            "cases": [
                {
                    "case_id": "case-a",
                    "sampling": {
                        "repetitions": 1,
                        "min_successful_samples": 4
                    }
                }
            ]
        });
        app.insert_eval_suite_snapshot(suite).unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:min-suite",
                "evrun-min-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:min-suite",
                "evrun-min-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();
        let mut baseline_case = passing_case();
        baseline_case.repetition_count = 1;
        app.record_eval_case_result(
            &baseline,
            baseline_case,
            vec![sample_at(100, 0, false, "duration_ms")],
        )
        .unwrap();
        let mut candidate_case = passing_case();
        candidate_case.repetition_count = 1;
        app.record_eval_case_result(
            &candidate,
            candidate_case,
            vec![sample_at(80, 0, false, "duration_ms")],
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();

        let policy = EvalComparisonPolicy {
            min_samples: 1,
            ..EvalComparisonPolicy::default()
        };
        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:min-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let (verdict, reasons, effect_estimates): (String, String, String) = app
            .conn
            .query_row(
                "SELECT verdict, reasons, effect_estimates FROM eval_comparisons WHERE id = ?1",
                params![comparison_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(verdict, "insufficient_evidence");
        assert_eq!(
            serde_json::from_str::<Value>(&reasons).unwrap(),
            json!(["samples_below_minimum"])
        );
        assert_eq!(
            serde_json::from_str::<Value>(&effect_estimates).unwrap(),
            json!({})
        );
    }

    #[test]
    fn eval_stored_comparison_blocks_unexpected_no_op_scorer_control_success() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        let mut suite = suite_input("sha256:control-suite");
        suite.normalized_manifest = json!({
            "suite_id": "suite",
            "cases": [
                {
                    "case_id": "oracle",
                    "scorer_control": {"kind": "oracle", "expect": "pass"},
                    "sampling": {"min_successful_samples": 3}
                },
                {
                    "case_id": "no-op",
                    "scorer_control": {"kind": "no_op", "expect": "fail"},
                    "sampling": {"min_successful_samples": 3}
                }
            ]
        });
        app.insert_eval_suite_snapshot(suite).unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:control-suite",
                "evrun-control-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:control-suite",
                "evrun-control-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();
        for run_id in [&baseline, &candidate] {
            let mut oracle = passing_case();
            oracle.case_id = "oracle".to_string();
            let mut oracle_samples = [100, 101, 99]
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    let mut sample = sample_at(value, index as i64, false, "duration_ms");
                    sample.id = None;
                    sample
                })
                .collect::<Vec<_>>();
            app.record_eval_case_result(run_id, oracle, std::mem::take(&mut oracle_samples))
                .unwrap();

            let mut no_op = passing_case();
            no_op.case_id = "no-op".to_string();
            no_op.status = if run_id == &baseline { "fail" } else { "pass" }.to_string();
            let no_op_samples = [110, 111, 109]
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    let mut sample = sample_at(value, index as i64, false, "duration_ms");
                    sample.id = None;
                    sample
                })
                .collect::<Vec<_>>();
            app.record_eval_case_result(run_id, no_op, no_op_samples)
                .unwrap();
            app.finish_eval_run(run_id, "success").unwrap();
        }

        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:control-policy",
                &EvalComparisonPolicy::default(),
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let (verdict, reasons): (String, String) = app
            .conn
            .query_row(
                "SELECT verdict, reasons FROM eval_comparisons WHERE id = ?1",
                params![comparison_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(verdict, "insufficient_evidence");
        assert_eq!(
            serde_json::from_str::<Value>(&reasons).unwrap(),
            json!([
                "scorer_control_nondeterministic",
                "scorer_control_unexpected_no_op_success"
            ])
        );
    }

    #[test]
    fn eval_stored_comparison_blocks_mixed_no_op_control_repetition_outcomes() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        let mut suite = suite_input("sha256:mixed-control-suite");
        suite.normalized_manifest = json!({
            "suite_id": "suite",
            "cases": [
                {
                    "case_id": "oracle",
                    "scorer_control": {"kind": "oracle", "expect": "pass"},
                    "sampling": {"min_successful_samples": 2}
                },
                {
                    "case_id": "no-op",
                    "scorer_control": {"kind": "no_op", "expect": "fail"},
                    "sampling": {"min_successful_samples": 2}
                }
            ]
        });
        app.insert_eval_suite_snapshot(suite).unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:mixed-control-suite",
                "evrun-mixed-control-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:mixed-control-suite",
                "evrun-mixed-control-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();
        let mixed_samples = |run_id: &str, case_id: &str, statuses: [&str; 2]| {
            statuses
                .into_iter()
                .enumerate()
                .map(|(index, status)| {
                    let mut sample =
                        sample_at(110 + index as i64, index as i64, false, "duration_ms");
                    sample.id = Some(format!("evsample-{run_id}-{case_id}-mixed-{index}"));
                    sample.attempt.terminal_status = status.to_string();
                    sample.attempt.outcome = json!({"status": status});
                    sample
                })
                .collect::<Vec<_>>()
        };
        for (run_id, statuses) in [
            (&baseline, ["pass", "error"]),
            (&candidate, ["fail", "inconclusive"]),
        ] {
            let mut oracle = passing_case();
            oracle.case_id = "oracle".to_string();
            oracle.repetition_count = 2;
            app.record_eval_case_result(
                run_id,
                oracle,
                mixed_samples(run_id, "oracle", ["pass", "pass"]),
            )
            .unwrap();

            let mut no_op = passing_case();
            no_op.case_id = "no-op".to_string();
            no_op.status = "fail".to_string();
            no_op.repetition_count = 2;
            no_op.reasons = json!(["missing_output"]);
            app.record_eval_case_result(run_id, no_op, mixed_samples(run_id, "no-op", statuses))
                .unwrap();
            app.finish_eval_run(run_id, "success").unwrap();
        }

        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:mixed-control-policy",
                &EvalComparisonPolicy::default(),
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let (verdict, reasons): (String, String) = app
            .conn
            .query_row(
                "SELECT verdict, reasons FROM eval_comparisons WHERE id = ?1",
                params![comparison_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(verdict, "insufficient_evidence");
        assert_eq!(
            serde_json::from_str::<Value>(&reasons).unwrap(),
            json!([
                "scorer_control_nondeterministic",
                "scorer_control_unexpected_no_op_success"
            ])
        );
    }

    #[test]
    fn eval_stored_comparison_requires_frozen_control_repetitions_and_status_agreement() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        let mut suite = suite_input("sha256:frozen-control-suite");
        suite.normalized_manifest = json!({
            "suite_id": "suite",
            "cases": [
                {
                    "case_id": "oracle",
                    "scorer_control": {"kind": "oracle", "expect": "pass"},
                    "sampling": {"repetitions": 5, "min_successful_samples": 2}
                },
                {
                    "case_id": "no-op",
                    "scorer_control": {"kind": "no_op", "expect": "fail"},
                    "sampling": {"repetitions": 5, "min_successful_samples": 2}
                }
            ]
        });
        app.insert_eval_suite_snapshot(suite).unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:frozen-control-suite",
                "evrun-frozen-control-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:frozen-control-suite",
                "evrun-frozen-control-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();
        let samples_with_status = |run_id: &str, case_id: &str, indices: &[i64], status: &str| {
            indices
                .iter()
                .enumerate()
                .map(|(offset, repetition_index)| {
                    let mut sample =
                        sample_at(120 + offset as i64, *repetition_index, false, "duration_ms");
                    sample.id = Some(format!(
                        "evsample-{run_id}-{case_id}-{status}-{repetition_index}"
                    ));
                    sample.attempt.terminal_status = status.to_string();
                    sample.attempt.outcome = json!({"status": status});
                    sample
                })
                .collect::<Vec<_>>()
        };
        for run_id in [&baseline, &candidate] {
            let mut oracle = passing_case();
            oracle.case_id = "oracle".to_string();
            oracle.repetition_count = 2;
            app.record_eval_case_result(
                run_id,
                oracle,
                samples_with_status(run_id, "oracle", &[3, 4], "pass"),
            )
            .unwrap();

            let mut no_op = passing_case();
            no_op.case_id = "no-op".to_string();
            no_op.status = "fail".to_string();
            no_op.repetition_count = 5;
            no_op.reasons = json!(["missing_output"]);
            app.record_eval_case_result(
                run_id,
                no_op,
                samples_with_status(run_id, "no-op", &[0, 1, 2, 3, 4], "error"),
            )
            .unwrap();
            app.finish_eval_run(run_id, "success").unwrap();
        }

        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:frozen-control-policy",
                &EvalComparisonPolicy::default(),
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let (verdict, reasons): (String, String) = app
            .conn
            .query_row(
                "SELECT verdict, reasons FROM eval_comparisons WHERE id = ?1",
                params![comparison_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(verdict, "insufficient_evidence");
        assert_eq!(
            serde_json::from_str::<Value>(&reasons).unwrap(),
            json!(["scorer_control_nondeterministic", "samples_below_minimum"])
        );
    }

    #[test]
    fn eval_stored_comparison_detects_stale_and_non_numeric_evidence() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-stale-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-stale-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();
        let mut baseline_case = passing_case();
        baseline_case.repetition_count = 3;
        app.record_eval_case_result(
            &baseline,
            baseline_case,
            vec![
                sample_at(100, 0, false, "duration_ms"),
                sample_at(101, 1, false, "duration_ms"),
                sample_at(99, 2, false, "duration_ms"),
            ],
        )
        .unwrap();
        let mut candidate_case = passing_case();
        candidate_case.repetition_count = 3;
        app.record_eval_case_result(
            &candidate,
            candidate_case,
            vec![
                sample_at(80, 0, false, "duration_ms"),
                sample_at(81, 1, false, "duration_ms"),
                sample_at(79, 2, false, "duration_ms"),
            ],
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();
        app.conn
            .execute(
                "UPDATE eval_runs SET completed_at = datetime('now', '-240 hours') WHERE id = ?1",
                params![baseline],
            )
            .unwrap();
        let policy = EvalComparisonPolicy::default();
        let comparison_id = app
            .compare_stored_eval_runs_at(
                &baseline,
                &candidate,
                "sha256:stale-policy",
                &policy,
                current_unix_seconds().unwrap(),
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let (verdict, reasons): (String, String) = app
            .conn
            .query_row(
                "SELECT verdict, reasons FROM eval_comparisons WHERE id = ?1",
                params![comparison_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(verdict, "insufficient_evidence");
        assert_eq!(
            serde_json::from_str::<Value>(&reasons).unwrap(),
            json!(["evidence_stale"])
        );

        app.conn
            .execute(
                "UPDATE eval_runs SET completed_at = datetime('now') WHERE id = ?1",
                params![baseline],
            )
            .unwrap();
        app.conn
            .execute(
                "UPDATE eval_samples SET value_json = '\"not-a-number\"' WHERE run_id IN (?1, ?2)",
                params![baseline, candidate],
            )
            .unwrap();
        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:nonnumeric-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let (verdict, reasons): (String, String) = app
            .conn
            .query_row(
                "SELECT verdict, reasons FROM eval_comparisons WHERE id = ?1",
                params![comparison_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(verdict, "insufficient_evidence");
        assert_eq!(
            serde_json::from_str::<Value>(&reasons).unwrap(),
            json!(["samples_below_minimum"])
        );
    }

    #[test]
    fn eval_effort_recommendation_blocks_cheaper_non_material_treatment() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-medium",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-low",
                "rev-b",
                testbed,
            ))
            .unwrap();

        app.record_eval_case_result(
            &baseline,
            passing_case_with_repetitions(3),
            effort_samples(&baseline, "medium", 100, 90),
        )
        .unwrap();
        app.record_eval_case_result(
            &candidate,
            passing_case_with_repetitions(3),
            effort_samples(&candidate, "low", 80, 90),
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();

        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:effort-policy",
                &EvalComparisonPolicy::default(),
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let comparison = app.eval_comparison_value_by_id(&comparison_id).unwrap();
        let recommendation = &comparison["effort_recommendation"];
        assert_eq!(comparison["verdict"], json!("no_material_difference"));
        assert_eq!(recommendation["kind"], json!("derived_pareto_view"));
        assert_eq!(recommendation["recommendation"], json!("no_recommendation"));
        assert_eq!(recommendation["promotion_allowed"], json!(false));
        assert_eq!(
            recommendation["baseline"]["effective_treatment"]["effort"],
            json!("medium")
        );
        assert_eq!(
            recommendation["candidate"]["effective_treatment"]["effort"],
            json!("low")
        );
        assert_eq!(
            recommendation["baseline"]["matched_denominator"]["identity"],
            json!("case_id,repetition_index,seed")
        );
        assert_eq!(
            recommendation["baseline"]["matched_denominator"]["cell_ids"],
            json!(["case-a:0:1", "case-a:1:1", "case-a:2:1"])
        );
        assert_eq!(recommendation["reasons"], json!(["no_material_effect"]));
    }

    #[test]
    fn eval_effort_recommendation_ranks_materially_cheaper_treatment() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-material-medium",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-material-low",
                "rev-b",
                testbed,
            ))
            .unwrap();

        app.record_eval_case_result(
            &baseline,
            passing_case_with_repetitions(3),
            effort_samples(&baseline, "medium", 100, 90),
        )
        .unwrap();
        app.record_eval_case_result(
            &candidate,
            passing_case_with_repetitions(3),
            effort_samples(&candidate, "low", 80, 90),
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();

        let policy = EvalComparisonPolicy {
            material_improvement: BTreeMap::from([("cost_micros_p95_relative".to_string(), -0.10)]),
            material_regression: BTreeMap::from([("cost_micros_p95_relative".to_string(), 0.10)]),
            ..EvalComparisonPolicy::default()
        };
        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:effort-material-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let comparison = app.eval_comparison_value_by_id(&comparison_id).unwrap();
        let recommendation = &comparison["effort_recommendation"];
        assert_eq!(comparison["verdict"], json!("improved"));
        assert_eq!(comparison["reasons"], json!(["candidate_improved"]));
        assert_eq!(
            comparison["effect_estimates"]["material_improvement_measures"],
            json!(["cost_micros"])
        );
        assert_eq!(recommendation["recommendation"], json!("prefer_candidate"));
        assert_eq!(recommendation["promotion_allowed"], json!(true));
        assert_eq!(recommendation["reasons"], json!([]));
    }

    #[test]
    fn eval_effort_recommendation_blocks_raw_cost_material_but_selected_metric_non_material() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-selected-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-selected-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();

        app.record_eval_case_result(
            &baseline,
            passing_case_with_repetitions(3),
            effort_samples(&baseline, "medium", 100, 90),
        )
        .unwrap();
        let mut candidate_samples = effort_samples(&candidate, "low", 65, 90);
        for sample in &mut candidate_samples {
            if sample.repetition_index == 2 {
                sample.attempt.terminal_status = "fail".to_string();
            }
        }
        app.record_eval_case_result(
            &candidate,
            passing_case_with_repetitions(3),
            candidate_samples,
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();

        let policy = EvalComparisonPolicy {
            material_improvement: BTreeMap::from([("cost_micros_p95_relative".to_string(), -0.10)]),
            material_regression: BTreeMap::from([("cost_micros_p95_relative".to_string(), 0.10)]),
            ..EvalComparisonPolicy::default()
        };
        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:effort-selected-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let comparison = app.eval_comparison_value_by_id(&comparison_id).unwrap();
        let recommendation = &comparison["effort_recommendation"];
        assert_eq!(comparison["verdict"], json!("improved"));
        assert_eq!(
            comparison["effect_estimates"]["material_improvement_measures"],
            json!(["cost_micros"])
        );
        assert_eq!(
            comparison["effect_estimates"]["effort_recommendation"]["selection_metric_materiality"]
                ["materially_improved"],
            json!(false)
        );
        assert_eq!(
            recommendation["candidate"]["cost_per_verified_success_micros"]["value"],
            json!(97.5)
        );
        assert_eq!(recommendation["recommendation"], json!("no_recommendation"));
        assert_eq!(recommendation["promotion_allowed"], json!(false));
        assert_eq!(recommendation["reasons"], json!(["no_material_effect"]));
    }

    #[test]
    fn eval_effort_recommendation_blocks_uncertain_selected_metric_materiality() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-uncertain-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-uncertain-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();

        app.record_eval_case_result(
            &baseline,
            passing_case_with_repetitions(10),
            effort_samples_with_repetitions(&baseline, "medium", 100, 90, 10),
        )
        .unwrap();
        let mut candidate_samples = effort_samples_with_repetitions(&candidate, "low", 40, 90, 10);
        for sample in &mut candidate_samples {
            if sample.repetition_index >= 5 {
                sample.attempt.terminal_status = "fail".to_string();
            }
        }
        app.record_eval_case_result(
            &candidate,
            passing_case_with_repetitions(10),
            candidate_samples,
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();

        let policy = EvalComparisonPolicy {
            material_improvement: BTreeMap::from([
                ("cost_micros_p95_relative".to_string(), -0.10),
                (
                    "cost_per_verified_success_micros_p95_relative".to_string(),
                    -0.10,
                ),
            ]),
            material_regression: BTreeMap::from([("cost_micros_p95_relative".to_string(), 0.10)]),
            ..EvalComparisonPolicy::default()
        };
        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:effort-uncertain-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let comparison = app.eval_comparison_value_by_id(&comparison_id).unwrap();
        let materiality = &comparison["effect_estimates"]["effort_recommendation"]["selection_metric_materiality"];
        let recommendation = &comparison["effort_recommendation"];

        assert_eq!(comparison["verdict"], json!("improved"));
        assert_eq!(
            comparison["effect_estimates"]["material_improvement_measures"],
            json!(["cost_micros"])
        );
        assert_eq!(
            recommendation["candidate"]["cost_per_verified_success_micros"]["value"],
            json!(80)
        );
        assert_eq!(materiality["relative"], json!(-0.2));
        assert_eq!(materiality["evidence"], json!("paired_cell_bootstrap_ci95"));
        assert_eq!(materiality["materially_improved"], json!(false));
        assert_eq!(
            comparison["uncertainty"]["cost_per_verified_success_micros_p95_ci95"],
            materiality["ci95"]
        );
        assert_eq!(recommendation["recommendation"], json!("no_recommendation"));
        assert_eq!(recommendation["promotion_allowed"], json!(false));
        assert_eq!(recommendation["reasons"], json!(["no_material_effect"]));
    }

    #[test]
    fn eval_effort_recommendation_prefers_duration_improved_cost_non_material() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-duration-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-duration-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();

        app.record_eval_case_result(
            &baseline,
            passing_case_with_repetitions(5),
            effort_samples_with_repetitions(&baseline, "medium", 100, 100, 5),
        )
        .unwrap();
        app.record_eval_case_result(
            &candidate,
            passing_case_with_repetitions(5),
            effort_samples_with_repetitions(&candidate, "low", 99, 50, 5),
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();

        let policy = EvalComparisonPolicy {
            material_improvement: BTreeMap::from([(
                "wall_time_ms_p95_relative".to_string(),
                -0.10,
            )]),
            material_regression: BTreeMap::from([("wall_time_ms_p95_relative".to_string(), 0.10)]),
            ..EvalComparisonPolicy::default()
        };
        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:effort-duration-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let comparison = app.eval_comparison_value_by_id(&comparison_id).unwrap();
        let recommendation = &comparison["effort_recommendation"];
        let materiality = &comparison["effect_estimates"]["effort_recommendation"]["selection_metric_materiality"];
        assert_eq!(comparison["verdict"], json!("improved"));
        assert_eq!(comparison["reasons"], json!(["candidate_improved"]));
        assert_eq!(
            comparison["effect_estimates"]["material_improvement_measures"],
            json!(["wall_time_ms"])
        );
        assert_eq!(
            materiality["metric"],
            json!("wall_time_per_verified_success_ms")
        );
        assert_eq!(materiality["relative"], json!(-0.5));
        assert_eq!(materiality["threshold"], json!(-0.1));
        assert_eq!(materiality["evidence"], json!("paired_cell_bootstrap_ci95"));
        assert_eq!(materiality["materially_improved"], json!(true));
        assert_eq!(
            comparison["uncertainty"]["wall_time_per_verified_success_ms_p95_ci95"],
            materiality["ci95"]
        );
        assert_eq!(
            recommendation["selection_metric"],
            json!("wall_time_per_verified_success_ms")
        );
        assert_eq!(recommendation["recommendation"], json!("prefer_candidate"));
        assert_eq!(recommendation["promotion_allowed"], json!(true));
        assert_eq!(recommendation["reasons"], json!([]));
    }

    #[test]
    fn eval_effort_recommendation_blocks_non_material_duration_improvement() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-duration-small-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-duration-small-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();

        app.record_eval_case_result(
            &baseline,
            passing_case_with_repetitions(5),
            effort_samples_with_repetitions(&baseline, "medium", 100, 100, 5),
        )
        .unwrap();
        app.record_eval_case_result(
            &candidate,
            passing_case_with_repetitions(5),
            effort_samples_with_repetitions(&candidate, "low", 99, 95, 5),
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();

        let policy = EvalComparisonPolicy {
            material_improvement: BTreeMap::from([(
                "wall_time_ms_p95_relative".to_string(),
                -0.10,
            )]),
            material_regression: BTreeMap::from([("wall_time_ms_p95_relative".to_string(), 0.10)]),
            ..EvalComparisonPolicy::default()
        };
        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:effort-duration-small-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let comparison = app.eval_comparison_value_by_id(&comparison_id).unwrap();
        let recommendation = &comparison["effort_recommendation"];
        let materiality = &comparison["effect_estimates"]["effort_recommendation"]["selection_metric_materiality"];

        assert_eq!(
            materiality["metric"],
            json!("wall_time_per_verified_success_ms")
        );
        assert_eq!(materiality["relative"], json!(-0.05));
        assert_eq!(materiality["threshold"], json!(-0.1));
        assert_eq!(materiality["evidence"], json!("paired_cell_bootstrap_ci95"));
        assert_eq!(materiality["materially_improved"], json!(false));
        assert_eq!(
            comparison["uncertainty"]["wall_time_per_verified_success_ms_p95_ci95"],
            materiality["ci95"]
        );
        assert_eq!(recommendation["recommendation"], json!("no_recommendation"));
        assert_eq!(recommendation["promotion_allowed"], json!(false));
        assert_eq!(recommendation["reasons"], json!(["no_material_effect"]));
    }

    #[test]
    fn eval_effort_recommendation_blocks_unmatched_extra_candidate_cell() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let testbed = json!({"os": "x"});
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-cell-base",
                "rev-a",
                testbed.clone(),
            ))
            .unwrap();
        let candidate = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-cell-cand",
                "rev-b",
                testbed,
            ))
            .unwrap();

        app.record_eval_case_result(
            &baseline,
            passing_case_with_repetitions(3),
            effort_samples(&baseline, "medium", 100, 90),
        )
        .unwrap();
        let mut candidate_case = passing_case_with_repetitions(4);
        candidate_case.case_id = "case-a".to_string();
        app.record_eval_case_result(
            &candidate,
            candidate_case,
            effort_samples_with_repetitions(&candidate, "low", 80, 90, 4),
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();
        app.finish_eval_run(&candidate, "success").unwrap();

        let policy = EvalComparisonPolicy {
            material_improvement: BTreeMap::from([("cost_micros_p95_relative".to_string(), -0.10)]),
            material_regression: BTreeMap::from([("cost_micros_p95_relative".to_string(), 0.10)]),
            ..EvalComparisonPolicy::default()
        };
        let comparison_id = app
            .compare_stored_eval_runs(
                &baseline,
                &candidate,
                "sha256:effort-cell-policy",
                &policy,
                EvalComparisonProvenance::default(),
            )
            .unwrap();
        let comparison = app.eval_comparison_value_by_id(&comparison_id).unwrap();
        let recommendation = &comparison["effort_recommendation"];
        assert_eq!(comparison["verdict"], json!("improved"));
        assert_eq!(recommendation["recommendation"], json!("no_recommendation"));
        assert_eq!(recommendation["promotion_allowed"], json!(false));
        assert_eq!(
            recommendation["candidate"]["matched_denominator"]["cell_ids"],
            json!(["case-a:0:1", "case-a:1:1", "case-a:2:1", "case-a:3:1"])
        );
        assert!(
            recommendation["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason.as_str() == Some("testbed_incompatible"))
        );
    }

    #[test]
    fn eval_effort_recommendation_blocks_wrong_weak_incompatible_and_mixed_evidence() {
        use crate::eval_compare::EvalComparisonPolicy;

        let app = test_app();
        app.insert_eval_suite_snapshot(suite_input("sha256:suite"))
            .unwrap();
        let baseline = app
            .start_eval_run(run_input(
                "sha256:suite",
                "evrun-effort-high",
                "rev-a",
                json!({"os": "x"}),
            ))
            .unwrap();
        app.record_eval_case_result(
            &baseline,
            passing_case_with_repetitions(3),
            effort_samples(&baseline, "high", 120, 100),
        )
        .unwrap();
        app.finish_eval_run(&baseline, "success").unwrap();

        let wrong = stored_effort_run(
            &app,
            "evrun-effort-wrong",
            "low",
            20,
            80,
            "fail",
            json!({"os": "x"}),
        );
        let thin = stored_effort_run(
            &app,
            "evrun-effort-thin",
            "low",
            20,
            80,
            "pass",
            json!({"os": "x"}),
        );
        app.conn
            .execute(
                "DELETE FROM eval_samples WHERE run_id = ?1 AND repetition_index > 0",
                params![thin],
            )
            .unwrap();
        let incompatible = stored_effort_run(
            &app,
            "evrun-effort-incompatible",
            "low",
            20,
            80,
            "pass",
            json!({"os": "other"}),
        );
        let mixed = stored_effort_run(
            &app,
            "evrun-effort-mixed",
            "low",
            20,
            80,
            "pass",
            json!({"os": "x"}),
        );
        app.conn
            .execute(
                "UPDATE eval_attempts SET effective_effort = 'medium' WHERE run_id = ?1 AND repetition_index = 1",
                params![mixed],
            )
            .unwrap();

        for (run_id, expected_reason) in [
            (wrong.as_str(), "correctness_regressed"),
            (thin.as_str(), "samples_below_minimum"),
            (incompatible.as_str(), "testbed_incompatible"),
            (mixed.as_str(), "testbed_incompatible"),
        ] {
            let comparison_id = app
                .compare_stored_eval_runs(
                    &baseline,
                    run_id,
                    &format!("sha256:effort-{run_id}"),
                    &EvalComparisonPolicy::default(),
                    EvalComparisonProvenance::default(),
                )
                .unwrap();
            let comparison = app.eval_comparison_value_by_id(&comparison_id).unwrap();
            assert_eq!(
                comparison["effort_recommendation"]["recommendation"],
                json!("no_recommendation"),
                "{run_id} should not be recommended"
            );
            assert_eq!(
                comparison["effort_recommendation"]["promotion_allowed"],
                json!(false)
            );
            assert!(
                comparison["effort_recommendation"]["reasons"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|reason| reason.as_str() == Some(expected_reason)),
                "{run_id} should retain {expected_reason}: {}",
                comparison["effort_recommendation"]["reasons"]
            );
        }
    }

    fn passing_case_with_repetitions(repetitions: i64) -> EvalCaseResultInput {
        EvalCaseResultInput {
            repetition_count: repetitions,
            ..passing_case()
        }
    }

    fn stored_effort_run(
        app: &App,
        id: &str,
        effort: &str,
        cost: i64,
        duration: i64,
        status: &str,
        testbed: Value,
    ) -> String {
        let run = app
            .start_eval_run(run_input("sha256:suite", id, id, testbed))
            .unwrap();
        app.record_eval_case_result(
            &run,
            EvalCaseResultInput {
                status: status.to_string(),
                ..passing_case_with_repetitions(3)
            },
            effort_samples(&run, effort, cost, duration),
        )
        .unwrap();
        app.finish_eval_run(&run, "success").unwrap();
        run
    }

    fn effort_samples(
        run_id: &str,
        effort: &str,
        cost: i64,
        duration: i64,
    ) -> Vec<EvalSampleInput> {
        effort_samples_with_repetitions(run_id, effort, cost, duration, 3)
    }

    fn effort_samples_with_repetitions(
        run_id: &str,
        effort: &str,
        cost: i64,
        duration: i64,
        repetitions: i64,
    ) -> Vec<EvalSampleInput> {
        let mut samples = Vec::new();
        for repetition in 0..repetitions {
            let attempt_id = format!("evattempt-{run_id}-{effort}-{repetition}");
            let mut wall = sample_at(duration, repetition, false, "wall_time_ms");
            wall.id = Some(format!("evsample-{run_id}-{effort}-{repetition}-wall"));
            wall.attempt.id = Some(attempt_id.clone());
            wall.attempt.terminal_status = "verified_success".to_string();
            wall.attempt.effective_effort = Some(effort.to_string());
            wall.attempt.effective_profile_id = Some(format!("eval-{effort}"));
            wall.attempt.profile_config_digest = Some(format!("sha256:profile-{effort}"));
            let mut cost_sample = sample_at(cost, repetition, false, "cost_micros");
            cost_sample.id = Some(format!("evsample-{run_id}-{effort}-{repetition}-cost"));
            cost_sample.attempt.id = Some(attempt_id);
            cost_sample.attempt.terminal_status = "verified_success".to_string();
            cost_sample.attempt.effective_effort = Some(effort.to_string());
            cost_sample.attempt.effective_profile_id = Some(format!("eval-{effort}"));
            cost_sample.attempt.profile_config_digest = Some(format!("sha256:profile-{effort}"));
            cost_sample.unit = "micros".to_string();
            cost_sample.source = "metering".to_string();
            cost_sample.basis_source = "metering".to_string();
            samples.push(wall);
            samples.push(cost_sample);
        }
        samples
    }
}
