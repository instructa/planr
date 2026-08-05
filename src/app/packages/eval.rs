use super::super::App;
use super::{
    nullable_str, optional_nullable_array, required_array, required_i64, required_str,
    required_value,
};
use anyhow::{Result, anyhow};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

struct PackagedEvalEvidenceRefContext<'a> {
    items: &'a BTreeMap<&'a str, &'a str>,
    logs: &'a BTreeMap<&'a str, &'a str>,
    runs: &'a BTreeSet<&'a str>,
    comparisons: &'a BTreeSet<&'a str>,
    artifacts: &'a BTreeMap<&'a str, Option<&'a str>>,
}

pub(super) fn packaged_eval_case_count(eval_runs: &[Value]) -> usize {
    eval_runs
        .iter()
        .filter_map(|run| run.get("cases").and_then(Value::as_array))
        .map(Vec::len)
        .sum()
}

pub(super) fn packaged_eval_sample_count(eval_runs: &[Value]) -> usize {
    eval_runs
        .iter()
        .filter_map(|run| run.get("cases").and_then(Value::as_array))
        .flat_map(|cases| cases.iter())
        .filter_map(|case| case.get("samples").and_then(Value::as_array))
        .map(Vec::len)
        .sum()
}

impl App {
    pub(super) fn validate_packaged_eval_identities(&self, data: &Value) -> Result<()> {
        let map = required_value(data, "map", "map")?;
        let items = required_array(map, "items", "map.items")?;
        let logs = optional_nullable_array(data, "logs")?;
        let eval_suite_snapshots = optional_nullable_array(data, "eval_suite_snapshots")?;
        let eval_runs = optional_nullable_array(data, "eval_runs")?;
        let eval_comparisons = optional_nullable_array(data, "eval_comparisons")?;
        let eval_invalidations = optional_nullable_array(data, "eval_invalidations")?;
        let eval_evidence_refs = optional_nullable_array(data, "eval_evidence_refs")?;
        let artifacts = required_array(data, "artifacts", "artifacts")?;
        let mut packaged_suite_snapshots = BTreeMap::new();
        for snapshot in eval_suite_snapshots {
            let digest = required_str(snapshot, "digest", "eval_suite_snapshots[].digest")?;
            insert_packaged_identity(
                "eval suite snapshot",
                digest,
                snapshot.clone(),
                &mut packaged_suite_snapshots,
            )?;
            if let Ok(existing) = self.eval_suite_snapshot_value(digest) {
                assert_identity_match("eval suite snapshot", digest, &existing, snapshot)?;
            }
        }
        let mut packaged_run_values = BTreeMap::new();
        let mut packaged_case_values = BTreeMap::new();
        let mut packaged_attempt_values = BTreeMap::new();
        let mut packaged_sample_values = BTreeMap::new();
        for run in eval_runs {
            let id = required_str(run, "id", "eval_runs[].id")?;
            let suite_digest = required_str(run, "suite_digest", "eval_runs[].suite_digest")?;
            if !packaged_suite_snapshots.contains_key(suite_digest)
                && self.eval_suite_snapshot_value(suite_digest).is_err()
            {
                return Err(anyhow!(
                    "invalid Planr package: eval run {id} suite_digest {suite_digest} is unresolved"
                ));
            }
            insert_packaged_identity("eval run", id, run.clone(), &mut packaged_run_values)?;
            if let Ok(existing) = self.eval_run_record_value(id) {
                assert_identity_match("eval run", id, &existing, run)?;
            }
            for case in optional_nullable_array(run, "cases")? {
                validate_case_sample_ids(case)?;
                let case_id = required_str(case, "id", "eval_runs[].cases[].id")?;
                let case_identity = packaged_case_identity(id, case)?;
                insert_packaged_identity(
                    "eval case result",
                    case_id,
                    case_identity,
                    &mut packaged_case_values,
                )?;
                self.validate_packaged_eval_case_identity(id, case)?;
                for attempt in optional_nullable_array(case, "attempts")? {
                    let attempt_id =
                        required_str(attempt, "id", "eval_runs[].cases[].attempts[].id")?;
                    let attempt_identity = packaged_attempt_identity(id, case, attempt)?;
                    insert_packaged_identity(
                        "eval attempt",
                        attempt_id,
                        attempt_identity,
                        &mut packaged_attempt_values,
                    )?;
                    self.validate_packaged_eval_attempt_identity(id, case, attempt)?;
                }
                for sample in optional_nullable_array(case, "samples")? {
                    let sample_id = required_str(sample, "id", "eval_runs[].cases[].samples[].id")?;
                    let attempt_id = required_str(
                        sample,
                        "attempt_id",
                        "eval_runs[].cases[].samples[].attempt_id",
                    )?;
                    if !packaged_attempt_values.contains_key(attempt_id) {
                        return Err(anyhow!(
                            "invalid Planr package: eval sample {sample_id} attempt_id {attempt_id} is unresolved"
                        ));
                    }
                    validate_sample_attempt_identity(
                        sample_id,
                        sample,
                        packaged_attempt_values.get(attempt_id).unwrap(),
                    )?;
                    validate_packaged_eval_sample_contract(sample)?;
                    let sample_identity = packaged_sample_identity(id, case, sample)?;
                    insert_packaged_identity(
                        "eval sample",
                        sample_id,
                        sample_identity,
                        &mut packaged_sample_values,
                    )?;
                    self.validate_packaged_eval_sample_identity(id, case, sample)?;
                }
            }
        }
        let mut packaged_comparison_values = BTreeMap::new();
        for comparison in eval_comparisons {
            let id = required_str(comparison, "id", "eval_comparisons[].id")?;
            let baseline = required_str(
                comparison,
                "baseline_run_id",
                "eval_comparisons[].baseline_run_id",
            )?;
            let candidate = required_str(
                comparison,
                "candidate_run_id",
                "eval_comparisons[].candidate_run_id",
            )?;
            for run_id in [baseline, candidate] {
                if !packaged_run_values.contains_key(run_id)
                    && self.eval_run_record_value(run_id).is_err()
                {
                    return Err(anyhow!(
                        "invalid Planr package: eval comparison {id} run dependency {run_id} is unresolved"
                    ));
                }
            }
            insert_packaged_identity(
                "eval comparison",
                id,
                comparison.clone(),
                &mut packaged_comparison_values,
            )?;
            if let Ok(existing) = self.eval_comparison_value_by_id(id) {
                assert_identity_match("eval comparison", id, &existing, comparison)?;
            }
        }
        let mut packaged_invalidation_values = BTreeMap::new();
        for invalidation in eval_invalidations {
            let id = required_str(invalidation, "id", "eval_invalidations[].id")?;
            let target_kind = required_str(
                invalidation,
                "target_kind",
                "eval_invalidations[].target_kind",
            )?;
            let target_id =
                required_str(invalidation, "target_id", "eval_invalidations[].target_id")?;
            match target_kind {
                "run" => {
                    if !packaged_run_values.contains_key(target_id)
                        && self.eval_run_record_value(target_id).is_err()
                    {
                        return Err(anyhow!(
                            "invalid Planr package: eval invalidation {id} run target {target_id} is unresolved"
                        ));
                    }
                }
                "comparison" => {
                    if !packaged_comparison_values.contains_key(target_id)
                        && self.eval_comparison_value_by_id(target_id).is_err()
                    {
                        return Err(anyhow!(
                            "invalid Planr package: eval invalidation {id} comparison target {target_id} is unresolved"
                        ));
                    }
                }
                _ => {
                    return Err(anyhow!(
                        "invalid Planr package: eval invalidation {id} target_kind must be run or comparison"
                    ));
                }
            }
            insert_packaged_identity(
                "eval invalidation",
                id,
                invalidation.clone(),
                &mut packaged_invalidation_values,
            )?;
            if let Ok(existing) = self.eval_invalidation_value_by_id(id) {
                assert_identity_match("eval invalidation", id, &existing, invalidation)?;
            }
        }
        let mut packaged_evidence_ref_values = BTreeMap::new();
        for evidence_ref in eval_evidence_refs {
            let id = required_str(evidence_ref, "id", "eval_evidence_refs[].id")?;
            match required_value(
                evidence_ref,
                "closure_authority",
                "eval_evidence_refs[].closure_authority",
            )? {
                Value::Bool(false) => {}
                Value::Bool(true) => {
                    return Err(anyhow!(
                        "invalid Planr package: eval evidence ref {id} closure_authority must be false"
                    ));
                }
                _ => {
                    return Err(anyhow!(
                        "invalid Planr package: eval evidence ref {id} closure_authority must be boolean false"
                    ));
                }
            }
            insert_packaged_identity(
                "eval evidence ref",
                id,
                evidence_ref.clone(),
                &mut packaged_evidence_ref_values,
            )?;
            if let Ok(existing) = self.eval_evidence_ref_record_value(id) {
                assert_identity_match("eval evidence ref", id, &existing, evidence_ref)?;
            }
        }
        let mut packaged_items = BTreeMap::new();
        for item in items {
            packaged_items.insert(
                required_str(item, "id", "map.items[].id")?,
                required_str(item, "work_type", "map.items[].work_type")?,
            );
        }
        let packaged_logs = logs
            .iter()
            .map(|log| {
                Ok((
                    required_str(log, "id", "logs[].id")?,
                    required_str(log, "item_id", "logs[].item_id")?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let packaged_runs = eval_runs
            .iter()
            .map(|run| required_str(run, "id", "eval_runs[].id"))
            .collect::<Result<BTreeSet<_>>>()?;
        let packaged_comparisons = eval_comparisons
            .iter()
            .map(|comparison| required_str(comparison, "id", "eval_comparisons[].id"))
            .collect::<Result<BTreeSet<_>>>()?;
        let mut packaged_artifacts = BTreeMap::new();
        for artifact in artifacts {
            packaged_artifacts.insert(
                required_str(artifact, "id", "artifacts[].id")?,
                nullable_str(artifact, "item_id", "artifacts[].item_id")?,
            );
        }
        let evidence_ref_context = PackagedEvalEvidenceRefContext {
            items: &packaged_items,
            logs: &packaged_logs,
            runs: &packaged_runs,
            comparisons: &packaged_comparisons,
            artifacts: &packaged_artifacts,
        };
        for evidence_ref in eval_evidence_refs {
            self.validate_packaged_eval_evidence_ref(evidence_ref, &evidence_ref_context)?;
        }
        Ok(())
    }

    fn validate_packaged_eval_case_identity(&self, run_id: &str, case: &Value) -> Result<()> {
        let id = required_str(case, "id", "eval_runs[].cases[].id")?;
        let existing = self
            .conn
            .query_row(
                "SELECT id, run_id, case_id, scorer_id, scorer_version, fixture_digest, status, repetition_count, warmup_count, assertions, command, sample_ids, reasons, created_at, updated_at FROM eval_case_results WHERE id = ?1",
                params![id],
                |row| {
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "run_id": row.get::<_, String>(1)?,
                        "case_id": row.get::<_, String>(2)?,
                        "scorer_id": row.get::<_, String>(3)?,
                        "scorer_version": row.get::<_, String>(4)?,
                        "fixture_digest": row.get::<_, String>(5)?,
                        "status": row.get::<_, String>(6)?,
                        "repetition_count": row.get::<_, i64>(7)?,
                        "warmup_count": row.get::<_, i64>(8)?,
                        "assertions": parse_json_package_row(row.get::<_, String>(9)?),
                        "command": parse_json_package_row(row.get::<_, String>(10)?),
                        "sample_ids": parse_json_package_row(row.get::<_, String>(11)?),
                        "reasons": parse_json_package_row(row.get::<_, String>(12)?),
                        "created_at": row.get::<_, String>(13)?,
                        "updated_at": row.get::<_, String>(14)?,
                    }))
                },
            )
            .optional()?;
        if let Some(existing) = existing {
            let expected = json!({
                "id": id,
                "run_id": run_id,
                "case_id": required_str(case, "case_id", "eval_runs[].cases[].case_id")?,
                "scorer_id": required_str(case, "scorer_id", "eval_runs[].cases[].scorer_id")?,
                "scorer_version": required_str(case, "scorer_version", "eval_runs[].cases[].scorer_version")?,
                "fixture_digest": required_str(case, "fixture_digest", "eval_runs[].cases[].fixture_digest")?,
                "status": required_str(case, "status", "eval_runs[].cases[].status")?,
                "repetition_count": required_i64(case, "repetition_count", "eval_runs[].cases[].repetition_count")?,
                "warmup_count": required_i64(case, "warmup_count", "eval_runs[].cases[].warmup_count")?,
                "assertions": required_value(case, "assertions", "eval_runs[].cases[].assertions")?,
                "command": required_value(case, "command", "eval_runs[].cases[].command")?,
                "sample_ids": required_value(case, "sample_ids", "eval_runs[].cases[].sample_ids")?,
                "reasons": required_value(case, "reasons", "eval_runs[].cases[].reasons")?,
                "created_at": required_str(case, "created_at", "eval_runs[].cases[].created_at")?,
                "updated_at": required_str(case, "updated_at", "eval_runs[].cases[].updated_at")?,
            });
            assert_identity_match("eval case result", id, &existing, &expected)?;
        }
        Ok(())
    }

    fn validate_packaged_eval_sample_identity(
        &self,
        run_id: &str,
        case: &Value,
        sample: &Value,
    ) -> Result<()> {
        let id = required_str(sample, "id", "eval_runs[].cases[].samples[].id")?;
        let existing = self
            .conn
            .query_row(
                "SELECT id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, seed, measure, value_json, unit, source, attempt_id, attempt_index, metering_basis, basis_source, basis_confidence, estimate_provenance, captured_at FROM eval_samples WHERE id = ?1",
                params![id],
                |row| {
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "run_id": row.get::<_, String>(1)?,
                        "case_id": row.get::<_, String>(2)?,
                        "scorer_id": row.get::<_, String>(3)?,
                        "scorer_version": row.get::<_, String>(4)?,
                        "repetition_index": row.get::<_, i64>(5)?,
                        "warmup": row.get::<_, i64>(6)? == 1,
                        "seed": row.get::<_, i64>(7)?,
                        "measure": row.get::<_, String>(8)?,
                        "value": parse_json_package_row(row.get::<_, String>(9)?),
                        "unit": row.get::<_, String>(10)?,
                        "source": row.get::<_, String>(11)?,
                        "attempt_id": row.get::<_, Option<String>>(12)?,
                        "attempt_index": row.get::<_, i64>(13)?,
                        "metering_basis": row.get::<_, String>(14)?,
                        "basis_source": row.get::<_, String>(15)?,
                        "basis_confidence": row.get::<_, String>(16)?,
                        "estimate_provenance": row.get::<_, Option<String>>(17)?.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()),
                        "captured_at": row.get::<_, String>(18)?,
                    }))
                },
            )
            .optional()?;
        if let Some(existing) = existing {
            let expected = json!({
                "id": id,
                "run_id": run_id,
                "case_id": required_str(case, "case_id", "eval_runs[].cases[].case_id")?,
                "scorer_id": required_str(case, "scorer_id", "eval_runs[].cases[].scorer_id")?,
                "scorer_version": required_str(case, "scorer_version", "eval_runs[].cases[].scorer_version")?,
                "repetition_index": required_i64(sample, "repetition_index", "eval_runs[].cases[].samples[].repetition_index")?,
                "warmup": required_value(sample, "warmup", "eval_runs[].cases[].samples[].warmup")?.as_bool().unwrap_or(false),
                "seed": required_i64(sample, "seed", "eval_runs[].cases[].samples[].seed")?,
                "measure": required_str(sample, "measure", "eval_runs[].cases[].samples[].measure")?,
                "value": required_value(sample, "value", "eval_runs[].cases[].samples[].value")?,
                "unit": required_str(sample, "unit", "eval_runs[].cases[].samples[].unit")?,
                "source": required_str(sample, "source", "eval_runs[].cases[].samples[].source")?,
                "attempt_id": sample.get("attempt_id").cloned().unwrap_or(Value::Null),
                "attempt_index": required_i64(sample, "attempt_index", "eval_runs[].cases[].samples[].attempt_index")?,
                "metering_basis": required_str(sample, "metering_basis", "eval_runs[].cases[].samples[].metering_basis")?,
                "basis_source": required_str(sample, "basis_source", "eval_runs[].cases[].samples[].basis_source")?,
                "basis_confidence": required_str(sample, "basis_confidence", "eval_runs[].cases[].samples[].basis_confidence")?,
                "estimate_provenance": sample.get("estimate_provenance").cloned().unwrap_or(Value::Null),
                "captured_at": required_str(sample, "captured_at", "eval_runs[].cases[].samples[].captured_at")?,
            });
            assert_identity_match("eval sample", id, &existing, &expected)?;
        }
        Ok(())
    }

    fn validate_packaged_eval_attempt_identity(
        &self,
        run_id: &str,
        case: &Value,
        attempt: &Value,
    ) -> Result<()> {
        let id = required_str(attempt, "id", "eval_runs[].cases[].attempts[].id")?;
        let existing = self
            .conn
            .query_row(
                "SELECT id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, attempt_index, seed, terminal_status, countable, retry_of_attempt_id, fallback_of_attempt_id, escalation_of_attempt_id, resume_of_attempt_id, effective_client, effective_provider, effective_runtime, effective_model, effective_effort, effective_profile_id, profile_config_digest, runner_harness_version, route_observation_json, outcome_json, created_at FROM eval_attempts WHERE id = ?1",
                params![id],
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
                        "outcome": parse_json_package_row(row.get::<_, String>(24)?),
                        "created_at": row.get::<_, String>(25)?,
                    }))
                },
            )
            .optional()?;
        if let Some(existing) = existing {
            let expected = packaged_attempt_identity(run_id, case, attempt)?;
            assert_identity_match("eval attempt", id, &existing, &expected)?;
        }
        Ok(())
    }

    fn validate_packaged_eval_evidence_ref(
        &self,
        evidence_ref: &Value,
        context: &PackagedEvalEvidenceRefContext<'_>,
    ) -> Result<()> {
        let target_kind = required_str(
            evidence_ref,
            "target_kind",
            "eval_evidence_refs[].target_kind",
        )?;
        let target_id = required_str(evidence_ref, "target_id", "eval_evidence_refs[].target_id")?;
        let attachment_kind = required_str(
            evidence_ref,
            "planr_attachment_kind",
            "eval_evidence_refs[].planr_attachment_kind",
        )?;
        let attachment_id = required_str(
            evidence_ref,
            "planr_attachment_id",
            "eval_evidence_refs[].planr_attachment_id",
        )?;
        let item_id = required_str(evidence_ref, "item_id", "eval_evidence_refs[].item_id")?;
        if !context.items.contains_key(item_id) && self.get_item(item_id).is_err() {
            return Err(anyhow!(
                "invalid Planr package: eval evidence ref item {item_id} is unresolved"
            ));
        }
        match target_kind {
            "run" => {
                if !context.runs.contains(target_id)
                    && self.eval_run_record_value(target_id).is_err()
                {
                    return Err(anyhow!(
                        "invalid Planr package: eval evidence ref run target {target_id} is unresolved"
                    ));
                }
            }
            "comparison" => {
                if !context.comparisons.contains(target_id)
                    && self.eval_comparison_value_by_id(target_id).is_err()
                {
                    return Err(anyhow!(
                        "invalid Planr package: eval evidence ref comparison target {target_id} is unresolved"
                    ));
                }
            }
            _ => {
                return Err(anyhow!(
                    "invalid Planr package: eval evidence ref target_kind must be run or comparison"
                ));
            }
        }
        match attachment_kind {
            "log" => {
                let packaged_owner = context.logs.get(attachment_id).copied();
                let existing_owner = if packaged_owner.is_none() {
                    self.get_log(attachment_id)
                        .ok()
                        .and_then(|log| log["item_id"].as_str().map(ToOwned::to_owned))
                } else {
                    None
                };
                let owner = packaged_owner.or(existing_owner.as_deref()).ok_or_else(|| {
                    anyhow!("invalid Planr package: eval evidence ref log attachment {attachment_id} is unresolved")
                })?;
                if owner != item_id {
                    return Err(anyhow!(
                        "invalid Planr package: eval evidence ref log attachment {attachment_id} must belong to item {item_id}"
                    ));
                }
            }
            "artifact" => {
                let packaged_owner = context.artifacts.get(attachment_id).copied().flatten();
                let existing_owner = if packaged_owner.is_none() {
                    self.get_artifact(attachment_id)
                        .ok()
                        .and_then(|artifact| artifact["item_id"].as_str().map(ToOwned::to_owned))
                } else {
                    None
                };
                let owner = packaged_owner.or(existing_owner.as_deref()).ok_or_else(|| {
                    anyhow!(
                        "invalid Planr package: eval evidence ref artifact attachment {attachment_id} is unresolved"
                    )
                })?;
                if owner != item_id {
                    return Err(anyhow!(
                        "invalid Planr package: eval evidence ref artifact attachment {attachment_id} must belong to item {item_id}"
                    ));
                }
            }
            _ => {
                return Err(anyhow!(
                    "invalid Planr package: eval evidence ref attachment kind must be log or artifact"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn import_packaged_eval_run_cases(&self, run: &Value) -> Result<(usize, usize)> {
        let run_id = required_str(run, "id", "eval_runs[].id")?;
        let mut imported_cases = 0usize;
        let mut imported_samples = 0usize;
        for case in optional_nullable_array(run, "cases")? {
            imported_cases += self.conn.execute(
                "INSERT OR IGNORE INTO eval_case_results(id, run_id, case_id, scorer_id, scorer_version, fixture_digest, status, repetition_count, warmup_count, assertions, command, sample_ids, reasons, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    required_str(case, "id", "eval_runs[].cases[].id")?,
                    run_id,
                    required_str(case, "case_id", "eval_runs[].cases[].case_id")?,
                    required_str(case, "scorer_id", "eval_runs[].cases[].scorer_id")?,
                    required_str(case, "scorer_version", "eval_runs[].cases[].scorer_version")?,
                    required_str(case, "fixture_digest", "eval_runs[].cases[].fixture_digest")?,
                    required_str(case, "status", "eval_runs[].cases[].status")?,
                    required_i64(case, "repetition_count", "eval_runs[].cases[].repetition_count")?,
                    required_i64(case, "warmup_count", "eval_runs[].cases[].warmup_count")?,
                    serde_json::to_string(required_value(case, "assertions", "eval_runs[].cases[].assertions")?)?,
                    serde_json::to_string(required_value(case, "command", "eval_runs[].cases[].command")?)?,
                    serde_json::to_string(required_value(case, "sample_ids", "eval_runs[].cases[].sample_ids")?)?,
                    serde_json::to_string(required_value(case, "reasons", "eval_runs[].cases[].reasons")?)?,
                    required_str(case, "created_at", "eval_runs[].cases[].created_at")?,
                    required_str(case, "updated_at", "eval_runs[].cases[].updated_at")?,
                ],
            )?;
            for attempt in optional_nullable_array(case, "attempts")? {
                self.conn.execute(
                    "INSERT OR IGNORE INTO eval_attempts(id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, attempt_index, seed, terminal_status, countable, retry_of_attempt_id, fallback_of_attempt_id, escalation_of_attempt_id, resume_of_attempt_id, effective_client, effective_provider, effective_runtime, effective_model, effective_effort, effective_profile_id, profile_config_digest, runner_harness_version, route_observation_json, outcome_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
                    params![
                        required_str(attempt, "id", "eval_runs[].cases[].attempts[].id")?,
                        run_id,
                        required_str(case, "case_id", "eval_runs[].cases[].case_id")?,
                        required_str(case, "scorer_id", "eval_runs[].cases[].scorer_id")?,
                        required_str(case, "scorer_version", "eval_runs[].cases[].scorer_version")?,
                        required_i64(attempt, "repetition_index", "eval_runs[].cases[].attempts[].repetition_index")?,
                        if required_value(attempt, "warmup", "eval_runs[].cases[].attempts[].warmup")?.as_bool().unwrap_or(false) { 1 } else { 0 },
                        required_i64(attempt, "attempt_index", "eval_runs[].cases[].attempts[].attempt_index")?,
                        required_i64(attempt, "seed", "eval_runs[].cases[].attempts[].seed")?,
                        required_str(attempt, "terminal_status", "eval_runs[].cases[].attempts[].terminal_status")?,
                        if required_value(attempt, "countable", "eval_runs[].cases[].attempts[].countable")?.as_bool().unwrap_or(false) { 1 } else { 0 },
                        nullable_str(attempt, "retry_of_attempt_id", "eval_runs[].cases[].attempts[].retry_of_attempt_id")?,
                        nullable_str(attempt, "fallback_of_attempt_id", "eval_runs[].cases[].attempts[].fallback_of_attempt_id")?,
                        nullable_str(attempt, "escalation_of_attempt_id", "eval_runs[].cases[].attempts[].escalation_of_attempt_id")?,
                        nullable_str(attempt, "resume_of_attempt_id", "eval_runs[].cases[].attempts[].resume_of_attempt_id")?,
                        nullable_str(attempt, "effective_client", "eval_runs[].cases[].attempts[].effective_client")?,
                        nullable_str(attempt, "effective_provider", "eval_runs[].cases[].attempts[].effective_provider")?,
                        nullable_str(attempt, "effective_runtime", "eval_runs[].cases[].attempts[].effective_runtime")?,
                        nullable_str(attempt, "effective_model", "eval_runs[].cases[].attempts[].effective_model")?,
                        nullable_str(attempt, "effective_effort", "eval_runs[].cases[].attempts[].effective_effort")?,
                        nullable_str(attempt, "effective_profile_id", "eval_runs[].cases[].attempts[].effective_profile_id")?,
                        nullable_str(attempt, "profile_config_digest", "eval_runs[].cases[].attempts[].profile_config_digest")?,
                        required_str(attempt, "runner_harness_version", "eval_runs[].cases[].attempts[].runner_harness_version")?,
                        attempt.get("route_observation").map(serde_json::to_string).transpose()?,
                        serde_json::to_string(required_value(attempt, "outcome", "eval_runs[].cases[].attempts[].outcome")?)?,
                        required_str(attempt, "created_at", "eval_runs[].cases[].attempts[].created_at")?,
                    ],
                )?;
            }
            for sample in optional_nullable_array(case, "samples")? {
                validate_packaged_eval_sample_contract(sample)?;
                imported_samples += self.conn.execute(
                    "INSERT OR IGNORE INTO eval_samples(id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, seed, measure, value_json, unit, source, attempt_id, attempt_index, metering_basis, basis_source, basis_confidence, estimate_provenance, captured_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
                    params![
                        required_str(sample, "id", "eval_runs[].cases[].samples[].id")?,
                        run_id,
                        required_str(case, "case_id", "eval_runs[].cases[].case_id")?,
                        required_str(case, "scorer_id", "eval_runs[].cases[].scorer_id")?,
                        required_str(case, "scorer_version", "eval_runs[].cases[].scorer_version")?,
                        required_i64(sample, "repetition_index", "eval_runs[].cases[].samples[].repetition_index")?,
                        if required_value(sample, "warmup", "eval_runs[].cases[].samples[].warmup")?.as_bool().unwrap_or(false) { 1 } else { 0 },
                        required_i64(sample, "seed", "eval_runs[].cases[].samples[].seed")?,
                        required_str(sample, "measure", "eval_runs[].cases[].samples[].measure")?,
                        serde_json::to_string(required_value(sample, "value", "eval_runs[].cases[].samples[].value")?)?,
                        required_str(sample, "unit", "eval_runs[].cases[].samples[].unit")?,
                        required_str(sample, "source", "eval_runs[].cases[].samples[].source")?,
                        required_str(sample, "attempt_id", "eval_runs[].cases[].samples[].attempt_id")?,
                        required_i64(sample, "attempt_index", "eval_runs[].cases[].samples[].attempt_index")?,
                        required_str(sample, "metering_basis", "eval_runs[].cases[].samples[].metering_basis")?,
                        required_str(sample, "basis_source", "eval_runs[].cases[].samples[].basis_source")?,
                        required_str(sample, "basis_confidence", "eval_runs[].cases[].samples[].basis_confidence")?,
                        sample.get("estimate_provenance").map(serde_json::to_string).transpose()?,
                        required_str(sample, "captured_at", "eval_runs[].cases[].samples[].captured_at")?,
                    ],
                )?;
            }
        }
        Ok((imported_cases, imported_samples))
    }

    pub(super) fn import_packaged_eval_suite_snapshots(
        &self,
        snapshots: &[Value],
        project_id: &str,
    ) -> Result<usize> {
        let mut imported = 0usize;
        for snapshot in snapshots {
            imported += self.conn.execute(
                "INSERT OR IGNORE INTO eval_suite_snapshots(digest, project_id, suite_id, suite_version, normalized_manifest, fixture_digests, scorer_identities, case_order, baseline_selector, comparison_policy_digest, metadata, source_path, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    required_str(snapshot, "digest", "eval_suite_snapshots[].digest")?,
                    project_id,
                    required_str(snapshot, "suite_id", "eval_suite_snapshots[].suite_id")?,
                    required_str(snapshot, "suite_version", "eval_suite_snapshots[].suite_version")?,
                    serde_json::to_string(required_value(snapshot, "normalized_manifest", "eval_suite_snapshots[].normalized_manifest")?)?,
                    serde_json::to_string(required_value(snapshot, "fixture_digests", "eval_suite_snapshots[].fixture_digests")?)?,
                    serde_json::to_string(required_value(snapshot, "scorer_identities", "eval_suite_snapshots[].scorer_identities")?)?,
                    serde_json::to_string(required_value(snapshot, "case_order", "eval_suite_snapshots[].case_order")?)?,
                    serde_json::to_string(required_value(snapshot, "baseline_selector", "eval_suite_snapshots[].baseline_selector")?)?,
                    required_str(snapshot, "comparison_policy_digest", "eval_suite_snapshots[].comparison_policy_digest")?,
                    serde_json::to_string(required_value(snapshot, "metadata", "eval_suite_snapshots[].metadata")?)?,
                    nullable_str(snapshot, "source_path", "eval_suite_snapshots[].source_path")?,
                    required_str(snapshot, "created_at", "eval_suite_snapshots[].created_at")?,
                ],
            )?;
        }
        Ok(imported)
    }
}

fn assert_identity_match(kind: &str, id: &str, existing: &Value, packaged: &Value) -> Result<()> {
    if existing != packaged {
        return Err(anyhow!(
            "invalid Planr package: conflicting {kind} id {id}; existing content differs from packaged immutable evidence"
        ));
    }
    Ok(())
}

fn insert_packaged_identity(
    kind: &str,
    id: &str,
    value: Value,
    values: &mut BTreeMap<String, Value>,
) -> Result<()> {
    if let Some(existing) = values.get(id) {
        assert_identity_match(kind, id, existing, &value)?;
        return Ok(());
    }
    values.insert(id.to_string(), value);
    Ok(())
}

fn packaged_case_identity(run_id: &str, case: &Value) -> Result<Value> {
    Ok(json!({
        "id": required_str(case, "id", "eval_runs[].cases[].id")?,
        "run_id": run_id,
        "case_id": required_str(case, "case_id", "eval_runs[].cases[].case_id")?,
        "scorer_id": required_str(case, "scorer_id", "eval_runs[].cases[].scorer_id")?,
        "scorer_version": required_str(case, "scorer_version", "eval_runs[].cases[].scorer_version")?,
        "fixture_digest": required_str(case, "fixture_digest", "eval_runs[].cases[].fixture_digest")?,
        "status": required_str(case, "status", "eval_runs[].cases[].status")?,
        "repetition_count": required_i64(case, "repetition_count", "eval_runs[].cases[].repetition_count")?,
        "warmup_count": required_i64(case, "warmup_count", "eval_runs[].cases[].warmup_count")?,
        "assertions": required_value(case, "assertions", "eval_runs[].cases[].assertions")?,
        "command": required_value(case, "command", "eval_runs[].cases[].command")?,
        "sample_ids": required_value(case, "sample_ids", "eval_runs[].cases[].sample_ids")?,
        "reasons": required_value(case, "reasons", "eval_runs[].cases[].reasons")?,
        "created_at": required_str(case, "created_at", "eval_runs[].cases[].created_at")?,
        "updated_at": required_str(case, "updated_at", "eval_runs[].cases[].updated_at")?,
    }))
}

fn packaged_sample_identity(run_id: &str, case: &Value, sample: &Value) -> Result<Value> {
    Ok(json!({
        "id": required_str(sample, "id", "eval_runs[].cases[].samples[].id")?,
        "run_id": run_id,
        "case_id": required_str(case, "case_id", "eval_runs[].cases[].case_id")?,
        "scorer_id": required_str(case, "scorer_id", "eval_runs[].cases[].scorer_id")?,
        "scorer_version": required_str(case, "scorer_version", "eval_runs[].cases[].scorer_version")?,
        "repetition_index": required_i64(sample, "repetition_index", "eval_runs[].cases[].samples[].repetition_index")?,
        "warmup": required_value(sample, "warmup", "eval_runs[].cases[].samples[].warmup")?.as_bool().unwrap_or(false),
        "seed": required_i64(sample, "seed", "eval_runs[].cases[].samples[].seed")?,
        "measure": required_str(sample, "measure", "eval_runs[].cases[].samples[].measure")?,
        "value": required_value(sample, "value", "eval_runs[].cases[].samples[].value")?,
        "unit": required_str(sample, "unit", "eval_runs[].cases[].samples[].unit")?,
        "source": required_str(sample, "source", "eval_runs[].cases[].samples[].source")?,
        "attempt_id": required_str(sample, "attempt_id", "eval_runs[].cases[].samples[].attempt_id")?,
        "attempt_index": required_i64(sample, "attempt_index", "eval_runs[].cases[].samples[].attempt_index")?,
        "metering_basis": required_str(sample, "metering_basis", "eval_runs[].cases[].samples[].metering_basis")?,
        "basis_source": required_str(sample, "basis_source", "eval_runs[].cases[].samples[].basis_source")?,
        "basis_confidence": required_str(sample, "basis_confidence", "eval_runs[].cases[].samples[].basis_confidence")?,
        "estimate_provenance": sample.get("estimate_provenance").cloned().unwrap_or(Value::Null),
        "captured_at": required_str(sample, "captured_at", "eval_runs[].cases[].samples[].captured_at")?,
    }))
}

fn validate_packaged_eval_sample_contract(sample: &Value) -> Result<()> {
    let basis = required_str(
        sample,
        "metering_basis",
        "eval_runs[].cases[].samples[].metering_basis",
    )?;
    if !matches!(
        basis,
        "actual_trusted" | "estimated" | "mixed" | "unavailable"
    ) {
        return Err(anyhow!("invalid eval metering basis: {basis}"));
    }

    required_str(
        sample,
        "basis_source",
        "eval_runs[].cases[].samples[].basis_source",
    )?;

    let confidence = required_str(
        sample,
        "basis_confidence",
        "eval_runs[].cases[].samples[].basis_confidence",
    )?;
    if !matches!(
        confidence,
        "verified" | "estimated" | "mixed" | "unavailable"
    ) {
        return Err(anyhow!("invalid eval basis confidence: {confidence}"));
    }

    let value = required_value(sample, "value", "eval_runs[].cases[].samples[].value")?;
    if basis == "unavailable" {
        if !value.is_null() {
            return Err(anyhow!("unavailable eval sample values must be null"));
        }
        if confidence != "unavailable" {
            return Err(anyhow!(
                "unavailable eval samples require unavailable basis_confidence"
            ));
        }
        if sample
            .get("estimate_provenance")
            .is_some_and(|provenance| !provenance.is_null())
        {
            return Err(anyhow!(
                "unavailable eval samples must not carry estimate_provenance"
            ));
        }
    } else if value.is_null() {
        return Err(anyhow!(
            "eval sample value must be present unless metering basis is unavailable"
        ));
    }

    if basis == "estimated" {
        validate_packaged_estimate_provenance(sample.get("estimate_provenance"))?;
    }

    Ok(())
}

fn validate_packaged_estimate_provenance(provenance: Option<&Value>) -> Result<()> {
    let Some(provenance) = provenance else {
        return Err(anyhow!(
            "estimated eval samples require estimate_provenance"
        ));
    };
    let Some(object) = provenance.as_object() else {
        return Err(anyhow!(
            "estimated eval samples require canonical estimate_provenance object"
        ));
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
            return Err(anyhow!(
                "estimated eval samples require estimate_provenance.{field}"
            ));
        }
    }
    Ok(())
}

fn packaged_attempt_identity(run_id: &str, case: &Value, attempt: &Value) -> Result<Value> {
    Ok(json!({
        "id": required_str(attempt, "id", "eval_runs[].cases[].attempts[].id")?,
        "run_id": run_id,
        "case_id": required_str(case, "case_id", "eval_runs[].cases[].case_id")?,
        "scorer_id": required_str(case, "scorer_id", "eval_runs[].cases[].scorer_id")?,
        "scorer_version": required_str(case, "scorer_version", "eval_runs[].cases[].scorer_version")?,
        "repetition_index": required_i64(attempt, "repetition_index", "eval_runs[].cases[].attempts[].repetition_index")?,
        "warmup": required_value(attempt, "warmup", "eval_runs[].cases[].attempts[].warmup")?.as_bool().unwrap_or(false),
        "attempt_index": required_i64(attempt, "attempt_index", "eval_runs[].cases[].attempts[].attempt_index")?,
        "seed": required_i64(attempt, "seed", "eval_runs[].cases[].attempts[].seed")?,
        "terminal_status": required_str(attempt, "terminal_status", "eval_runs[].cases[].attempts[].terminal_status")?,
        "countable": required_value(attempt, "countable", "eval_runs[].cases[].attempts[].countable")?.as_bool().unwrap_or(false),
        "retry_of_attempt_id": attempt.get("retry_of_attempt_id").cloned().unwrap_or(Value::Null),
        "fallback_of_attempt_id": attempt.get("fallback_of_attempt_id").cloned().unwrap_or(Value::Null),
        "escalation_of_attempt_id": attempt.get("escalation_of_attempt_id").cloned().unwrap_or(Value::Null),
        "resume_of_attempt_id": attempt.get("resume_of_attempt_id").cloned().unwrap_or(Value::Null),
        "effective_client": attempt.get("effective_client").cloned().unwrap_or(Value::Null),
        "effective_provider": attempt.get("effective_provider").cloned().unwrap_or(Value::Null),
        "effective_runtime": attempt.get("effective_runtime").cloned().unwrap_or(Value::Null),
        "effective_model": attempt.get("effective_model").cloned().unwrap_or(Value::Null),
        "effective_effort": attempt.get("effective_effort").cloned().unwrap_or(Value::Null),
        "effective_profile_id": attempt.get("effective_profile_id").cloned().unwrap_or(Value::Null),
        "profile_config_digest": attempt.get("profile_config_digest").cloned().unwrap_or(Value::Null),
        "runner_harness_version": required_str(attempt, "runner_harness_version", "eval_runs[].cases[].attempts[].runner_harness_version")?,
        "route_observation": attempt.get("route_observation").cloned().unwrap_or(Value::Null),
        "outcome": required_value(attempt, "outcome", "eval_runs[].cases[].attempts[].outcome")?,
        "created_at": required_str(attempt, "created_at", "eval_runs[].cases[].attempts[].created_at")?,
    }))
}

fn validate_case_sample_ids(case: &Value) -> Result<()> {
    let case_id = required_str(case, "id", "eval_runs[].cases[].id")?;
    let declared = required_value(case, "sample_ids", "eval_runs[].cases[].sample_ids")?
        .as_array()
        .ok_or_else(|| {
            anyhow!("invalid Planr package: eval case result {case_id} sample_ids must be an array")
        })?
        .iter()
        .map(|value| {
            value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                anyhow!(
                    "invalid Planr package: eval case result {case_id} sample_ids must contain strings"
                )
            })
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let actual = optional_nullable_array(case, "samples")?
        .iter()
        .map(|sample| {
            required_str(sample, "id", "eval_runs[].cases[].samples[].id").map(ToOwned::to_owned)
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if declared != actual {
        return Err(anyhow!(
            "invalid Planr package: eval case result {case_id} sample_ids must match nested samples"
        ));
    }
    Ok(())
}

fn validate_sample_attempt_identity(
    sample_id: &str,
    sample: &Value,
    attempt: &Value,
) -> Result<()> {
    let sample_identity = json!({
        "repetition_index": required_i64(sample, "repetition_index", "eval_runs[].cases[].samples[].repetition_index")?,
        "warmup": required_value(sample, "warmup", "eval_runs[].cases[].samples[].warmup")?.as_bool().unwrap_or(false),
        "seed": required_i64(sample, "seed", "eval_runs[].cases[].samples[].seed")?,
        "attempt_index": sample.get("attempt_index").and_then(Value::as_i64).unwrap_or(0),
    });
    let attempt_identity = json!({
        "repetition_index": required_i64(attempt, "repetition_index", "eval_runs[].cases[].attempts[].repetition_index")?,
        "warmup": required_value(attempt, "warmup", "eval_runs[].cases[].attempts[].warmup")?.as_bool().unwrap_or(false),
        "seed": required_i64(attempt, "seed", "eval_runs[].cases[].attempts[].seed")?,
        "attempt_index": required_i64(attempt, "attempt_index", "eval_runs[].cases[].attempts[].attempt_index")?,
    });
    if sample_identity != attempt_identity {
        return Err(anyhow!(
            "invalid Planr package: eval sample {sample_id} identity does not match referenced attempt"
        ));
    }
    Ok(())
}

fn parse_json_package_row(raw: String) -> Value {
    serde_json::from_str(&raw).unwrap_or(Value::Null)
}
