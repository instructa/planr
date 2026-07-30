use super::App;
use crate::canonical_json::{canonical_json_bytes, sha256_json_digest_without_top_level_field};
use crate::cli::{
    EvalCommand, EvalEvidenceAttachmentKind, EvalEvidenceTargetKind, EvalInvalidationTargetKind,
    EvalShowKind,
};
use crate::eval_compare::EvalComparisonPolicy;
use crate::eval_runner::{
    EvalCommandEvidence, EvalRunOptions, EvalRunnerCase, EvalRunnerManifest,
    eval_runner_manifest_from_value, run_eval_manifest, validate_eval_manifest,
};
use crate::route_audit::{EnforcementState, parse_route_observation};
use crate::util::collect_rows;
use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

use super::repository::eval::{
    EvalAttemptInput, EvalCaseResultInput, EvalComparisonProvenance, EvalRunInput, EvalSampleInput,
    EvalSubjectInput, EvalSuiteSnapshotInput,
};

#[derive(Debug)]
pub(crate) struct EvalCliExit {
    code: i32,
    message: String,
    emitted: bool,
}

impl EvalCliExit {
    fn new(code: i32, message: impl Into<String>, emitted: bool) -> Self {
        Self {
            code,
            message: message.into(),
            emitted,
        }
    }

    pub(crate) fn code(&self) -> i32 {
        self.code
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn emitted(&self) -> bool {
        self.emitted
    }
}

impl std::fmt::Display for EvalCliExit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EvalCliExit {}

impl App {
    pub(crate) fn eval(&self, command: EvalCommand) -> Result<()> {
        let (command_name, result, human): (&'static str, Result<Value>, String) = match command {
            EvalCommand::SuiteCheck(args) => {
                let result = read_json_file(&args.input).and_then(|input| {
                    self.eval_suite_check_value(input, Some(args.input.display().to_string()))
                });
                let human = result
                    .as_ref()
                    .ok()
                    .and_then(|value| value["suite"]["digest"].as_str())
                    .map(|digest| format!("eval suite {digest} checked"))
                    .unwrap_or_else(|| "eval suite check failed".to_string());
                ("eval.suite.check", result, human)
            }
            EvalCommand::Run(args) => {
                let result = read_json_file(&args.input)
                    .and_then(|input| self.eval_run_value_from_input(input));
                let human = result
                    .as_ref()
                    .ok()
                    .map(|value| {
                        let id = value["run"]["id"].as_str().unwrap_or_default();
                        let status = value["run"]["status"].as_str().unwrap_or_default();
                        format!("eval run {id} {status}")
                    })
                    .unwrap_or_else(|| "eval run failed".to_string());
                ("eval.run", result, human)
            }
            EvalCommand::Show(args) => {
                let kind = eval_show_kind(args.kind);
                let id = args.id;
                let result = self.eval_show_value(kind, &id);
                ("eval.show", result, format!("eval {kind} {id}"))
            }
            EvalCommand::Compare(args) => {
                let result = self.eval_compare_value(
                    &args.baseline_run_id,
                    &args.candidate_run_id,
                    &args.policy_digest,
                    args.recompute_of.as_deref(),
                    args.rescore_of.as_deref(),
                );
                let human = result
                    .as_ref()
                    .ok()
                    .map(|value| {
                        let id = value["comparison"]["id"].as_str().unwrap_or_default();
                        let verdict = value["verdict"].as_str().unwrap_or_default();
                        format!("eval comparison {id}: {verdict}")
                    })
                    .unwrap_or_else(|| "eval comparison failed".to_string());
                ("eval.compare", result, human)
            }
            EvalCommand::Gate(args) => {
                let result = self.eval_gate_value(&args.comparison_id);
                let human = result
                    .as_ref()
                    .ok()
                    .map(|value| {
                        let verdict = value["verdict"].as_str().unwrap_or_default();
                        let blocker = value["blocker"].as_str().unwrap_or_default();
                        format!(
                            "eval gate {verdict}{}",
                            if blocker.is_empty() {
                                String::new()
                            } else {
                                format!(": {blocker}")
                            }
                        )
                    })
                    .unwrap_or_else(|| "eval gate failed".to_string());
                ("eval.gate", result, human)
            }
            EvalCommand::Invalidate(args) => {
                let target_kind = match args.target_kind {
                    EvalInvalidationTargetKind::Run => "run",
                    EvalInvalidationTargetKind::Comparison => "comparison",
                };
                let result = self.eval_invalidate_value(
                    target_kind,
                    &args.target_id,
                    &args.reason,
                    json!(args.reason_codes),
                    args.replacement_hint.as_deref(),
                );
                let human = result
                    .as_ref()
                    .ok()
                    .and_then(|value| value["invalidation"]["id"].as_str())
                    .map(|id| format!("eval invalidation {id} recorded"))
                    .unwrap_or_else(|| "eval invalidation failed".to_string());
                ("eval.invalidate", result, human)
            }
            EvalCommand::Rescore(args) => {
                let result = self.eval_rescore_value(&args.run_id, args.id);
                let human = result
                    .as_ref()
                    .ok()
                    .and_then(|value| value["run"]["id"].as_str())
                    .map(|id| format!("eval rescore run {id} started"))
                    .unwrap_or_else(|| "eval rescore failed".to_string());
                ("eval.rescore", result, human)
            }
            EvalCommand::EvidenceRef(args) => {
                let target_kind = match args.target_kind {
                    EvalEvidenceTargetKind::Run => "run",
                    EvalEvidenceTargetKind::Comparison => "comparison",
                };
                let attachment_kind = match args.attachment_kind {
                    EvalEvidenceAttachmentKind::Log => "log",
                    EvalEvidenceAttachmentKind::Review => "review",
                    EvalEvidenceAttachmentKind::Artifact => "artifact",
                };
                let result = self.eval_evidence_ref_value(
                    target_kind,
                    &args.target_id,
                    attachment_kind,
                    &args.attachment_id,
                    &args.item,
                );
                let human = result
                    .as_ref()
                    .ok()
                    .and_then(|value| value["evidence_ref"]["id"].as_str())
                    .map(|id| format!("eval evidence ref {id} attached"))
                    .unwrap_or_else(|| "eval evidence ref failed".to_string());
                ("eval.evidence.ref", result, human)
            }
        };
        self.emit_eval_result(command_name, result, human)
    }

    fn emit_eval_result(
        &self,
        command: &'static str,
        result: Result<Value>,
        human: String,
    ) -> Result<()> {
        match result {
            Ok(object) => {
                let envelope = eval_success_envelope(command, object);
                let exit_code = eval_envelope_exit_code(&envelope);
                self.emit(envelope, human)?;
                if exit_code == 0 {
                    Ok(())
                } else {
                    Err(
                        EvalCliExit::new(exit_code, format!("eval {command} did not pass"), true)
                            .into(),
                    )
                }
            }
            Err(error) => {
                let message = error.to_string();
                let envelope = eval_error_envelope(command, &message);
                let exit_code = eval_error_exit_code(&message);
                if self.json {
                    crate::util::print_json(&envelope)?;
                    Err(EvalCliExit::new(exit_code, message, true).into())
                } else {
                    Err(EvalCliExit::new(exit_code, message, false).into())
                }
            }
        }
    }

    pub(crate) fn eval_suite_check_value(
        &self,
        input: Value,
        source_path: Option<String>,
    ) -> Result<Value> {
        let manifest = input
            .get("normalized_manifest")
            .cloned()
            .unwrap_or_else(|| input.clone());
        let digest = string_field(&input, "digest")
            .or_else(|| string_field(&manifest, "digest"))
            .ok_or_else(|| anyhow!("missing required eval suite field: digest"))?;
        validate_sha256_digest_format(&digest)?;
        if let Some(input_digest) = string_field(&input, "digest") {
            if input_digest != digest {
                bail!("eval suite digest mismatch: expected {digest}, got {input_digest}");
            }
        }
        if let Some(manifest_digest) = string_field(&manifest, "digest") {
            if manifest_digest != digest {
                bail!("eval suite digest mismatch: expected {digest}, got {manifest_digest}");
            }
        }
        let computed_digest = sha256_json_digest_without_top_level_digest(&manifest)?;
        if !eval_manifest_digest_matches(&digest, &computed_digest, &manifest)? {
            bail!("eval suite digest mismatch: expected {digest}, computed {computed_digest}");
        }
        let canonical_manifest = canonical_eval_suite_snapshot_value(&manifest, &digest);
        validate_scorer_control_admission(&canonical_manifest)?;
        let suite_id = string_field(&input, "suite_id")
            .or_else(|| string_field(&canonical_manifest, "suite_id"))
            .unwrap_or_else(|| "default".to_string());
        let suite_version = string_field(&input, "suite_version")
            .or_else(|| string_field(&canonical_manifest, "suite_version"))
            .unwrap_or_else(|| "v1".to_string());
        let policy_digest = string_field(&input, "comparison_policy_digest")
            .or_else(|| string_field(&canonical_manifest, "comparison_policy_digest"))
            .unwrap_or_else(|| "default".to_string());
        self.insert_eval_suite_snapshot(EvalSuiteSnapshotInput {
            digest: digest.clone(),
            suite_id,
            suite_version,
            normalized_manifest: canonical_manifest,
            fixture_digests: input
                .get("fixture_digests")
                .cloned()
                .unwrap_or_else(|| json!({})),
            scorer_identities: input
                .get("scorer_identities")
                .cloned()
                .unwrap_or_else(|| json!([])),
            case_order: input
                .get("case_order")
                .cloned()
                .unwrap_or_else(|| json!([])),
            baseline_selector: input
                .get("baseline_selector")
                .cloned()
                .unwrap_or_else(|| json!({})),
            comparison_policy_digest: policy_digest,
            metadata: input.get("metadata").cloned().unwrap_or_else(|| json!({})),
            source_path,
        })?;
        Ok(json!({
            "suite": self.eval_suite_snapshot_value(&digest)?,
            "verdict": "valid",
            "blocker": null,
            "deeper_reads": [
                format!("planr eval show suite {digest} --json")
            ]
        }))
    }

    pub(crate) fn eval_run_value_from_input(&self, input: Value) -> Result<Value> {
        let run_input = eval_run_input(&input)?;
        if input.get("runner_manifest").is_some() {
            return self.eval_run_value_from_runner_input(input, run_input);
        }
        self.eval_run_value_from_supplied_evidence(input, run_input)
    }

    fn eval_run_value_from_supplied_evidence(
        &self,
        input: Value,
        run_input: EvalRunInput,
    ) -> Result<Value> {
        let stored_manifest = self.stored_eval_suite_manifest(&run_input.suite_digest)?;
        validate_sha256_manifest_digest(&run_input.suite_digest, &stored_manifest)?;
        let validated_cases = validate_supplied_evidence_cases(&input, &stored_manifest)?;
        let existing_run = run_input
            .id
            .as_deref()
            .and_then(|id| self.eval_run_record_value(id).ok());
        let run_id = if let Some(existing) = existing_run {
            if existing["status"].as_str() != Some("started") {
                bail!(
                    "eval run {} already exists with terminal status {}",
                    existing["id"].as_str().unwrap_or("<unknown>"),
                    existing["status"].as_str().unwrap_or("<unknown>")
                );
            }
            ensure_eval_run_input_matches_started_run(&run_input, &existing)?;
            existing["id"]
                .as_str()
                .ok_or_else(|| anyhow!("stored eval run id is missing"))?
                .to_string()
        } else {
            self.start_eval_run(run_input)?
        };
        for (case, samples) in validated_cases {
            self.record_eval_case_result(&run_id, case, samples)?;
        }
        if let Some(status) = input.get("status").and_then(Value::as_str) {
            if status != "started" {
                self.finish_eval_run(&run_id, status)?;
            }
        }
        Ok(json!({
            "run": self.eval_run_record_value(&run_id)?,
            "verdict": self.eval_run_record_value(&run_id)?["status"].clone(),
            "blocker": null,
            "deeper_reads": [
                format!("planr eval show run {run_id} --json")
            ]
        }))
    }

    fn eval_run_value_from_runner_input(
        &self,
        input: Value,
        run_input: EvalRunInput,
    ) -> Result<Value> {
        let manifest_value = input
            .get("runner_manifest")
            .ok_or_else(|| anyhow!("missing required eval field: runner_manifest"))?;
        self.ensure_runner_manifest_matches_suite(&run_input.suite_digest, manifest_value)?;
        let manifest = eval_runner_manifest_from_value(manifest_value)?;
        let repo_root = self.resolve_eval_repo_root(
            string_field(&input, "repo_root")
                .unwrap_or_else(|| ".".to_string())
                .as_str(),
        )?;
        validate_eval_manifest(&repo_root, &manifest, &EvalRunOptions::default())?;
        let interrupt_after_cases = input
            .get("interrupt_after_cases")
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        let existing_run = run_input
            .id
            .as_deref()
            .and_then(|id| self.eval_run_record_value(id).ok());
        let run_id = if let Some(existing) = existing_run {
            if existing["status"].as_str() != Some("started") {
                bail!(
                    "eval run {} already exists with terminal status {}",
                    existing["id"].as_str().unwrap_or("<unknown>"),
                    existing["status"].as_str().unwrap_or("<unknown>")
                );
            }
            ensure_eval_run_input_matches_started_run(&run_input, &existing)?;
            existing["id"]
                .as_str()
                .ok_or_else(|| anyhow!("stored eval run id is missing"))?
                .to_string()
        } else {
            self.start_eval_run(run_input.clone())?
        };
        let reusable_cases = reusable_cases_for_runner_manifest(
            self,
            &run_input,
            &manifest,
            run_input.resume_of.as_deref(),
        )?;
        let mut executed_manifest = manifest.clone();
        if let Some(limit) = interrupt_after_cases {
            executed_manifest
                .cases
                .truncate(limit.min(executed_manifest.cases.len()));
        }
        let output = run_eval_manifest(
            &repo_root,
            &executed_manifest,
            EvalRunOptions {
                reusable_cases,
                ..EvalRunOptions::default()
            },
        )?;
        for case in &output.cases {
            let Some(manifest_case) = manifest
                .cases
                .iter()
                .find(|candidate| candidate.case_id == case.case_id)
            else {
                bail!("runner output case {} is not in manifest", case.case_id);
            };
            self.record_eval_case_result(
                &run_id,
                runner_case_input(manifest_case, case, &output.fixture_digests),
                runner_sample_inputs(&run_id, case),
            )?;
        }
        let status = if interrupt_after_cases.is_some_and(|limit| limit < manifest.cases.len()) {
            "inconclusive"
        } else {
            runner_run_status(&output.cases)
        };
        self.finish_eval_run(&run_id, status)?;
        let run = self.eval_run_record_value(&run_id)?;
        Ok(json!({
            "run": run,
            "verdict": status,
            "runner": {
                "suite_id": output.suite_id,
                "executed_case_ids": output.cases.iter().map(|case| case.case_id.clone()).collect::<Vec<_>>(),
                "skipped_case_ids": manifest.cases.iter().filter(|case| !output.cases.iter().any(|executed| executed.case_id == case.case_id)).map(|case| case.case_id.clone()).collect::<Vec<_>>(),
            },
            "blocker": null,
            "deeper_reads": [
                format!("planr eval show run {run_id} --json")
            ]
        }))
    }

    fn ensure_runner_manifest_matches_suite(
        &self,
        suite_digest: &str,
        manifest_value: &Value,
    ) -> Result<()> {
        let stored_manifest = self.stored_eval_suite_manifest(suite_digest)?;
        let stored_scope = runner_binding_scope(&stored_manifest);
        let input_scope = runner_binding_scope(manifest_value);
        let stored_bytes = canonical_json_bytes(&stored_scope)?;
        let input_bytes = canonical_json_bytes(&input_scope)?;
        if stored_bytes != input_bytes {
            bail!(
                "eval runner manifest does not match frozen suite snapshot {suite_digest} (stored {}, input {})",
                sha256_prefixed_bytes(&stored_bytes),
                sha256_prefixed_bytes(&input_bytes)
            );
        }
        if stored_manifest
            .get("digest")
            .and_then(Value::as_str)
            .is_some_and(|stored| stored != suite_digest)
        {
            bail!("eval suite snapshot digest field does not match {suite_digest}");
        }
        validate_sha256_manifest_digest(suite_digest, &stored_manifest)?;
        Ok(())
    }

    fn stored_eval_suite_manifest(&self, suite_digest: &str) -> Result<Value> {
        let raw: String = self
            .conn
            .query_row(
                "SELECT normalized_manifest FROM eval_suite_snapshots WHERE digest = ?1",
                params![suite_digest],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("eval suite {suite_digest} not found"))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("parsing normalized manifest for eval suite {suite_digest}"))
    }

    fn resolve_eval_repo_root(&self, repo_root: &str) -> Result<PathBuf> {
        let requested = Path::new(repo_root);
        if repo_root.trim().is_empty() {
            bail!("eval repo_root must not be empty");
        }
        if requested.is_absolute() {
            bail!("eval repo_root must be relative to the Planr project root");
        }
        if repo_root.starts_with('~') {
            bail!("eval repo_root must not use home-directory expansion");
        }
        if requested
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            bail!("eval repo_root must not traverse outside the Planr project root");
        }
        let project_root = self
            .root
            .canonicalize()
            .context("canonicalizing Planr project root")?;
        let canonical = project_root
            .join(requested)
            .canonicalize()
            .with_context(|| format!("canonicalizing eval repo_root {repo_root}"))?;
        if !canonical.starts_with(&project_root) {
            bail!("eval repo_root escapes the Planr project root");
        }
        if !canonical.is_dir() {
            bail!("eval repo_root must resolve to a directory");
        }
        Ok(canonical)
    }

    pub(crate) fn eval_show_value(&self, kind: &str, id: &str) -> Result<Value> {
        let value = match kind {
            "suite" => json!({"suite": self.eval_suite_snapshot_value(id)?}),
            "run" => json!({"run": self.eval_run_record_value(id)?}),
            "comparison" => {
                let comparison = self.eval_comparison_value_by_id(id)?;
                json!({
                    "comparison": comparison,
                    "verdict": comparison["verdict"],
                    "blocker": comparison["reasons"].as_array().and_then(|values| values.first()).cloned().unwrap_or(Value::Null),
                    "efficiency_summary": comparison["efficiency_summary"],
                    "deeper_reads": [format!("planr eval gate {id} --json")]
                })
            }
            "invalidation" => json!({"invalidation": self.eval_invalidation_value_by_id(id)?}),
            _ => bail!("invalid eval show kind: {kind}"),
        };
        Ok(value)
    }

    pub(crate) fn eval_compare_value(
        &self,
        baseline_run_id: &str,
        candidate_run_id: &str,
        policy_digest: &str,
        recompute_of: Option<&str>,
        rescore_of: Option<&str>,
    ) -> Result<Value> {
        let baseline_run = self.eval_run_record_value(baseline_run_id)?;
        self.eval_run_record_value(candidate_run_id)?;
        let suite_digest = required_string(&baseline_run, "suite_digest")?;
        let suite_manifest = self.stored_eval_suite_manifest(&suite_digest)?;
        let comparison_policy = eval_comparison_policy_from_manifest(&suite_manifest);
        let id = self.compare_stored_eval_runs(
            baseline_run_id,
            candidate_run_id,
            policy_digest,
            &comparison_policy,
            EvalComparisonProvenance {
                recompute_of,
                rescore_of,
            },
        )?;
        let comparison = self.eval_comparison_value_by_id(&id)?;
        Ok(json!({
            "comparison": comparison,
            "verdict": comparison["verdict"],
            "blocker": comparison["reasons"].as_array().and_then(|values| values.first()).cloned().unwrap_or(Value::Null),
            "effects": comparison["effect_estimates"],
            "uncertainty": comparison["uncertainty"],
            "efficiency_summary": comparison["efficiency_summary"],
            "effort_recommendation": comparison["effort_recommendation"],
            "coverage": comparison["gates"],
            "freshness": comparison["gates"],
            "deeper_reads": [
                format!("planr eval show comparison {id} --json"),
                format!("planr eval gate {id} --json")
            ]
        }))
    }

    pub(crate) fn eval_gate_value(&self, comparison_id: &str) -> Result<Value> {
        let comparison = self.eval_comparison_value_by_id(comparison_id)?;
        let verdict = comparison["verdict"]
            .as_str()
            .unwrap_or("insufficient_evidence");
        let ok = matches!(verdict, "improved" | "no_material_difference");
        Ok(json!({
            "ok": ok,
            "verdict": verdict,
            "blocker": if ok { Value::Null } else { comparison["reasons"].as_array().and_then(|values| values.first()).cloned().unwrap_or(json!("blocked")) },
            "comparison": comparison,
            "effects": comparison["effect_estimates"],
            "uncertainty": comparison["uncertainty"],
            "efficiency_summary": comparison["efficiency_summary"],
            "effort_recommendation": comparison["effort_recommendation"],
            "coverage": comparison["gates"],
            "freshness": comparison["gates"],
            "deeper_reads": [
                format!("planr eval show comparison {comparison_id} --json"),
                format!("planr eval compare {} {} --policy-digest {} --json", comparison["baseline_run_id"].as_str().unwrap_or("<baseline>"), comparison["candidate_run_id"].as_str().unwrap_or("<candidate>"), comparison["policy_digest"].as_str().unwrap_or("default"))
            ]
        }))
    }

    pub(crate) fn eval_invalidate_value(
        &self,
        target_kind: &str,
        target_id: &str,
        reason: &str,
        reason_codes: Value,
        replacement_hint: Option<&str>,
    ) -> Result<Value> {
        let id = self.invalidate_eval_target(
            target_kind,
            target_id,
            reason,
            reason_codes,
            replacement_hint,
        )?;
        Ok(json!({
            "invalidation": self.eval_invalidation_value_by_id(&id)?,
            "verdict": "invalidated",
            "blocker": reason,
            "deeper_reads": [
                format!("planr eval show invalidation {id} --json"),
                format!("planr eval show {target_kind} {target_id} --json")
            ]
        }))
    }

    pub(crate) fn eval_rescore_value(&self, run_id: &str, id: Option<String>) -> Result<Value> {
        let source = self.eval_run_record_value(run_id)?;
        let input = json!({
            "id": id,
            "suite_digest": source["suite_digest"],
            "subject": {
                "kind": source["subject_kind"],
                "revision": source["subject_revision"],
                "path": source["subject_path"],
                "argv": source["subject_argv"],
                "label": source["subject_label"]
            },
            "runner_version": source["runner_version"],
            "planr_version": source["planr_version"],
            "testbed_fingerprint": source["testbed_fingerprint"],
            "source_state": source["source_state"],
            "rescore_of": run_id
        });
        let id = self.start_eval_run(eval_run_input(&input)?)?;
        Ok(json!({
            "run": self.eval_run_record_value(&id)?,
            "verdict": "started",
            "blocker": null,
            "deeper_reads": [
                format!("planr eval show run {id} --json")
            ]
        }))
    }

    pub(crate) fn eval_evidence_ref_value(
        &self,
        target_kind: &str,
        target_id: &str,
        attachment_kind: &str,
        attachment_id: &str,
        item_id: &str,
    ) -> Result<Value> {
        self.get_item(item_id)?;
        match target_kind {
            "run" => {
                self.eval_run_record_value(target_id)?;
            }
            "comparison" => {
                self.eval_comparison_value_by_id(target_id)?;
            }
            _ => bail!("invalid eval evidence target kind: {target_kind}"),
        }
        match attachment_kind {
            "log" => {
                let log = self.get_log(attachment_id)?;
                if log["item_id"].as_str() != Some(item_id) {
                    bail!("eval evidence log attachment must belong to item {item_id}");
                }
            }
            "review" => {
                let review = self.get_item(attachment_id)?;
                if review.work_type != "review" {
                    bail!("eval evidence review attachment must be a review item");
                }
                let target = self.review_target(attachment_id)?.ok_or_else(|| {
                    anyhow!("eval evidence review attachment must review item {item_id}")
                })?;
                if target.id != item_id {
                    bail!("eval evidence review attachment must review item {item_id}");
                }
            }
            "artifact" => {
                let artifact = self.get_artifact(attachment_id)?;
                if artifact["item_id"].as_str() != Some(item_id) {
                    bail!("eval evidence artifact attachment must belong to item {item_id}");
                }
            }
            _ => bail!("invalid eval evidence attachment kind: {attachment_kind}"),
        }
        let id = self.attach_eval_evidence_ref(
            target_kind,
            target_id,
            attachment_kind,
            attachment_id,
            item_id,
        )?;
        Ok(json!({
            "evidence_ref": self.eval_evidence_ref_record_value(&id)?,
            "deeper_reads": [
                format!("planr eval show {target_kind} {target_id} --json"),
                format!("planr trace item {item_id} --json")
            ]
        }))
    }

    pub(crate) fn eval_suite_snapshot_value(&self, digest: &str) -> Result<Value> {
        self.conn
            .query_row(
                "SELECT digest, suite_id, suite_version, normalized_manifest, fixture_digests, scorer_identities, case_order, baseline_selector, comparison_policy_digest, metadata, source_path, created_at FROM eval_suite_snapshots WHERE digest = ?1",
                params![digest],
                |row| {
                    Ok(json!({
                        "digest": row.get::<_, String>(0)?,
                        "suite_id": row.get::<_, String>(1)?,
                        "suite_version": row.get::<_, String>(2)?,
                        "normalized_manifest": parse_json_row(row.get::<_, String>(3)?),
                        "fixture_digests": parse_json_row(row.get::<_, String>(4)?),
                        "scorer_identities": parse_json_row(row.get::<_, String>(5)?),
                        "case_order": parse_json_row(row.get::<_, String>(6)?),
                        "baseline_selector": parse_json_row(row.get::<_, String>(7)?),
                        "comparison_policy_digest": row.get::<_, String>(8)?,
                        "metadata": parse_json_row(row.get::<_, String>(9)?),
                        "source_path": row.get::<_, Option<String>>(10)?,
                        "created_at": row.get::<_, String>(11)?,
                    }))
                },
            )
            .optional()?
            .ok_or_else(|| anyhow!("eval suite {digest} not found"))
    }

    pub(crate) fn eval_suite_snapshot_values(&self) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare("SELECT digest FROM eval_suite_snapshots ORDER BY created_at, digest")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let digests = collect_rows(rows)?;
        digests
            .iter()
            .map(|digest| self.eval_suite_snapshot_value(digest))
            .collect()
    }

    pub(crate) fn eval_run_record_value(&self, id: &str) -> Result<Value> {
        let mut run = self
            .conn
            .query_row(
                "SELECT id, suite_digest, subject_kind, subject_revision, subject_path, subject_argv, subject_label, runner_version, planr_version, status, created_at, started_at, completed_at, testbed_fingerprint, source_state, case_counts, parent_run_id, resume_of, rescore_of, recompute_of, invalidated_by, aggregate_summary FROM eval_runs WHERE id = ?1",
                params![id],
                |row| {
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "suite_digest": row.get::<_, String>(1)?,
                        "subject_kind": row.get::<_, String>(2)?,
                        "subject_revision": row.get::<_, String>(3)?,
                        "subject_path": row.get::<_, Option<String>>(4)?,
                        "subject_argv": parse_json_row(row.get::<_, String>(5)?),
                        "subject_label": row.get::<_, Option<String>>(6)?,
                        "runner_version": row.get::<_, String>(7)?,
                        "planr_version": row.get::<_, String>(8)?,
                        "status": row.get::<_, String>(9)?,
                        "created_at": row.get::<_, String>(10)?,
                        "started_at": row.get::<_, Option<String>>(11)?,
                        "completed_at": row.get::<_, Option<String>>(12)?,
                        "testbed_fingerprint": parse_json_row(row.get::<_, String>(13)?),
                        "source_state": parse_json_row(row.get::<_, String>(14)?),
                        "case_counts": parse_json_row(row.get::<_, String>(15)?),
                        "parent_run_id": row.get::<_, Option<String>>(16)?,
                        "resume_of": row.get::<_, Option<String>>(17)?,
                        "rescore_of": row.get::<_, Option<String>>(18)?,
                        "recompute_of": row.get::<_, Option<String>>(19)?,
                        "invalidated_by": row.get::<_, Option<String>>(20)?,
                        "aggregate_summary": parse_json_row(row.get::<_, String>(21)?),
                    }))
                },
            )
            .optional()?
            .ok_or_else(|| anyhow!("eval run {id} not found"))?;
        run["cases"] = json!(self.eval_case_values(id)?);
        run["sample_metering"] = json!(self.eval_run_sample_metering_values(id)?);
        run["efficiency_summary"] = self.eval_run_efficiency_summary(id)?;
        Ok(run)
    }

    pub(crate) fn eval_run_record_values(&self) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM eval_runs ORDER BY created_at, id")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let ids = collect_rows(rows)?;
        ids.iter()
            .map(|id| self.eval_run_record_value(id))
            .collect()
    }

    pub(crate) fn eval_comparison_value_by_id(&self, id: &str) -> Result<Value> {
        let mut comparison = self.conn
            .query_row(
                "SELECT id, baseline_run_id, candidate_run_id, policy_digest, runner_version, verdict, reasons, gates, effect_estimates, uncertainty, protected_dimensions, recompute_of, rescore_of, created_at FROM eval_comparisons WHERE id = ?1",
                params![id],
                |row| {
                    let candidate_run_id = row.get::<_, String>(2)?;
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "baseline_run_id": row.get::<_, String>(1)?,
                        "candidate_run_id": candidate_run_id,
                        "policy_digest": row.get::<_, String>(3)?,
                        "runner_version": row.get::<_, String>(4)?,
                        "verdict": row.get::<_, String>(5)?,
                        "reasons": parse_json_row(row.get::<_, String>(6)?),
                        "gates": parse_json_row(row.get::<_, String>(7)?),
                        "effect_estimates": parse_json_row(row.get::<_, String>(8)?),
                        "uncertainty": parse_json_row(row.get::<_, String>(9)?),
                        "protected_dimensions": parse_json_row(row.get::<_, String>(10)?),
                        "recompute_of": row.get::<_, Option<String>>(11)?,
                        "rescore_of": row.get::<_, Option<String>>(12)?,
                        "created_at": row.get::<_, String>(13)?,
                    }))
                },
            )
            .optional()?
            .ok_or_else(|| anyhow!("eval comparison {id} not found"))?;
        let candidate_run_id = comparison["candidate_run_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let baseline_run_id = comparison["baseline_run_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let baseline_efficiency = self.eval_run_efficiency_summary(&baseline_run_id)?;
        let candidate_efficiency = self.eval_run_efficiency_summary(&candidate_run_id)?;
        comparison["baseline_efficiency_summary"] = baseline_efficiency.clone();
        comparison["candidate_efficiency_summary"] = candidate_efficiency.clone();
        comparison["efficiency_summary"] = candidate_efficiency.clone();
        comparison["effort_recommendation"] = self.eval_effort_recommendation(
            &comparison,
            &baseline_run_id,
            &candidate_run_id,
            &baseline_efficiency,
            &candidate_efficiency,
        )?;
        Ok(comparison)
    }

    pub(crate) fn eval_comparison_values(&self) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM eval_comparisons ORDER BY created_at, id")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let ids = collect_rows(rows)?;
        ids.iter()
            .map(|id| self.eval_comparison_value_by_id(id))
            .collect()
    }

    pub(crate) fn eval_invalidation_value_by_id(&self, id: &str) -> Result<Value> {
        self.conn
            .query_row(
                "SELECT id, target_kind, target_id, reason, reason_codes, created_at, created_by, replacement_hint FROM eval_invalidations WHERE id = ?1",
                params![id],
                |row| {
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "target_kind": row.get::<_, String>(1)?,
                        "target_id": row.get::<_, String>(2)?,
                        "reason": row.get::<_, String>(3)?,
                        "reason_codes": parse_json_row(row.get::<_, String>(4)?),
                        "created_at": row.get::<_, String>(5)?,
                        "created_by": row.get::<_, String>(6)?,
                        "replacement_hint": row.get::<_, Option<String>>(7)?,
                    }))
                },
            )
            .optional()?
            .ok_or_else(|| anyhow!("eval invalidation {id} not found"))
    }

    pub(crate) fn eval_invalidation_values(&self) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM eval_invalidations ORDER BY created_at, id")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let ids = collect_rows(rows)?;
        ids.iter()
            .map(|id| self.eval_invalidation_value_by_id(id))
            .collect()
    }

    pub(crate) fn eval_evidence_ref_record_value(&self, id: &str) -> Result<Value> {
        self.conn
            .query_row(
                "SELECT id, target_kind, target_id, planr_attachment_kind, planr_attachment_id, item_id, created_at, closure_authority FROM eval_evidence_refs WHERE id = ?1",
                params![id],
                |row| {
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "evidence_ref": row.get::<_, String>(0)?,
                        "target_kind": row.get::<_, String>(1)?,
                        "target_id": row.get::<_, String>(2)?,
                        "planr_attachment_kind": row.get::<_, String>(3)?,
                        "planr_attachment_id": row.get::<_, String>(4)?,
                        "item_id": row.get::<_, String>(5)?,
                        "created_at": row.get::<_, String>(6)?,
                        "closure_authority": row.get::<_, i64>(7)? != 0,
                    }))
                },
            )
            .optional()?
            .ok_or_else(|| anyhow!("eval evidence ref {id} not found"))
    }

    pub(crate) fn eval_evidence_ref_values(&self) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, target_kind, target_id, planr_attachment_kind, planr_attachment_id, item_id, created_at, closure_authority FROM eval_evidence_refs ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "evidence_ref": row.get::<_, String>(0)?,
                "target_kind": row.get::<_, String>(1)?,
                "target_id": row.get::<_, String>(2)?,
                "planr_attachment_kind": row.get::<_, String>(3)?,
                "planr_attachment_id": row.get::<_, String>(4)?,
                "item_id": row.get::<_, String>(5)?,
                "created_at": row.get::<_, String>(6)?,
                "closure_authority": row.get::<_, i64>(7)? != 0,
            }))
        })?;
        collect_rows(rows)
    }

    fn eval_case_values(&self, run_id: &str) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, scorer_id, scorer_version, fixture_digest, status, repetition_count, warmup_count, assertions, command, sample_ids, reasons, created_at, updated_at FROM eval_case_results WHERE run_id = ?1 ORDER BY case_id, scorer_id, scorer_version",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            let case_id = row.get::<_, String>(1)?;
            let scorer_id = row.get::<_, String>(2)?;
            let scorer_version = row.get::<_, String>(3)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "case_id": case_id,
                "scorer_id": scorer_id,
                "scorer_version": scorer_version,
                "fixture_digest": row.get::<_, String>(4)?,
                "status": row.get::<_, String>(5)?,
                "repetition_count": row.get::<_, i64>(6)?,
                "warmup_count": row.get::<_, i64>(7)?,
                "assertions": parse_json_row(row.get::<_, String>(8)?),
                "command": parse_json_row(row.get::<_, String>(9)?),
                "sample_ids": parse_json_row(row.get::<_, String>(10)?),
                "reasons": parse_json_row(row.get::<_, String>(11)?),
                "created_at": row.get::<_, String>(12)?,
                "updated_at": row.get::<_, String>(13)?,
            }))
        })?;
        let mut cases = collect_rows(rows)?;
        for case in &mut cases {
            let case_id = case["case_id"].as_str().unwrap_or_default().to_string();
            let scorer_id = case["scorer_id"].as_str().unwrap_or_default().to_string();
            let scorer_version = case["scorer_version"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            case["attempts"] =
                json!(self.eval_attempt_values(run_id, &case_id, &scorer_id, &scorer_version)?);
            case["samples"] =
                json!(self.eval_sample_values(run_id, &case_id, &scorer_id, &scorer_version)?);
        }
        Ok(cases)
    }

    fn eval_attempt_values(
        &self,
        run_id: &str,
        case_id: &str,
        scorer_id: &str,
        scorer_version: &str,
    ) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repetition_index, warmup, attempt_index, seed, terminal_status, countable, retry_of_attempt_id, fallback_of_attempt_id, escalation_of_attempt_id, resume_of_attempt_id, effective_client, effective_provider, effective_runtime, effective_model, effective_effort, effective_profile_id, profile_config_digest, runner_harness_version, route_observation_json, outcome_json, created_at FROM eval_attempts WHERE run_id = ?1 AND case_id = ?2 AND scorer_id = ?3 AND scorer_version = ?4 ORDER BY repetition_index, warmup, attempt_index, id",
        )?;
        let rows = stmt.query_map(params![run_id, case_id, scorer_id, scorer_version], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "repetition_index": row.get::<_, i64>(1)?,
                "warmup": row.get::<_, i64>(2)? == 1,
                "attempt_index": row.get::<_, i64>(3)?,
                "seed": row.get::<_, i64>(4)?,
                "terminal_status": row.get::<_, String>(5)?,
                "countable": row.get::<_, i64>(6)? != 0,
                "retry_of_attempt_id": row.get::<_, Option<String>>(7)?,
                "fallback_of_attempt_id": row.get::<_, Option<String>>(8)?,
                "escalation_of_attempt_id": row.get::<_, Option<String>>(9)?,
                "resume_of_attempt_id": row.get::<_, Option<String>>(10)?,
                "effective_client": row.get::<_, Option<String>>(11)?,
                "effective_provider": row.get::<_, Option<String>>(12)?,
                "effective_runtime": row.get::<_, Option<String>>(13)?,
                "effective_model": row.get::<_, Option<String>>(14)?,
                "effective_effort": row.get::<_, Option<String>>(15)?,
                "effective_profile_id": row.get::<_, Option<String>>(16)?,
                "profile_config_digest": row.get::<_, Option<String>>(17)?,
                "runner_harness_version": row.get::<_, String>(18)?,
                "route_observation": row.get::<_, Option<String>>(19)?.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()),
                "outcome": parse_json_row(row.get::<_, String>(20)?),
                "created_at": row.get::<_, String>(21)?,
            }))
        })?;
        let mut attempts = collect_rows(rows)?;
        for attempt in &mut attempts {
            attempt["route_observation_validation"] = eval_route_observation_validation(attempt);
        }
        Ok(attempts)
    }

    fn eval_sample_values(
        &self,
        run_id: &str,
        case_id: &str,
        scorer_id: &str,
        scorer_version: &str,
    ) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repetition_index, warmup, seed, measure, value_json, unit, source, attempt_id, attempt_index, metering_basis, basis_source, basis_confidence, estimate_provenance, captured_at FROM eval_samples WHERE run_id = ?1 AND case_id = ?2 AND scorer_id = ?3 AND scorer_version = ?4 ORDER BY repetition_index, warmup, attempt_index, measure, id",
        )?;
        let rows = stmt.query_map(params![run_id, case_id, scorer_id, scorer_version], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "repetition_index": row.get::<_, i64>(1)?,
                "warmup": row.get::<_, i64>(2)? == 1,
                "seed": row.get::<_, i64>(3)?,
                "measure": row.get::<_, String>(4)?,
                "value": parse_json_row(row.get::<_, String>(5)?),
                "unit": row.get::<_, String>(6)?,
                "source": row.get::<_, String>(7)?,
                "attempt_id": row.get::<_, Option<String>>(8)?,
                "attempt_index": row.get::<_, i64>(9)?,
                "metering_basis": row.get::<_, String>(10)?,
                "basis_source": row.get::<_, String>(11)?,
                "basis_confidence": row.get::<_, String>(12)?,
                "estimate_provenance": row.get::<_, Option<String>>(13)?.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()),
                "captured_at": row.get::<_, String>(14)?,
            }))
        })?;
        collect_rows(rows)
    }

    fn eval_run_sample_metering_values(&self, run_id: &str) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT attempt_id, measure, metering_basis, basis_source, basis_confidence, estimate_provenance
             FROM eval_samples
             WHERE run_id = ?1
             ORDER BY case_id, scorer_id, scorer_version, repetition_index, warmup, attempt_index, measure, id",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok(json!({
                "attempt_id": row.get::<_, Option<String>>(0)?,
                "measure": row.get::<_, String>(1)?,
                "metering_basis": row.get::<_, String>(2)?,
                "basis_source": row.get::<_, String>(3)?,
                "basis_confidence": row.get::<_, String>(4)?,
                "estimate_provenance": row.get::<_, Option<String>>(5)?.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()),
            }))
        })?;
        collect_rows(rows)
    }

    pub(crate) fn eval_run_efficiency_summary(&self, run_id: &str) -> Result<Value> {
        let attempts = self.eval_efficiency_attempts(run_id)?;
        let cost = self.eval_efficiency_metric(run_id, &attempts, "cost_micros", "micros")?;
        let wall = self.eval_efficiency_metric(run_id, &attempts, "wall_time_ms", "ms")?;
        let countable_attempts = attempts.len();
        let verified_successes = attempts
            .iter()
            .filter(|attempt| attempt.terminal_status == "verified_success")
            .count();
        let mut reasons = Vec::new();
        append_json_reasons(&mut reasons, &cost["reasons"]);
        append_json_reasons(&mut reasons, &wall["reasons"]);
        if verified_successes == 0 && countable_attempts > 0 {
            push_reason_string(&mut reasons, "zero_verified_successes");
        }
        if countable_attempts > 0 && countable_attempts < 2 {
            push_reason_string(&mut reasons, "samples_below_minimum");
        }
        let retry_policy = json!({
            "retries": attempts.iter().filter(|attempt| attempt.retry_of_attempt_id.is_some()).count(),
            "fallbacks": attempts.iter().filter(|attempt| attempt.fallback_of_attempt_id.is_some()).count(),
            "escalations": attempts.iter().filter(|attempt| attempt.escalation_of_attempt_id.is_some()).count(),
            "resumes": attempts.iter().filter(|attempt| attempt.resume_of_attempt_id.is_some()).count(),
        });
        Ok(json!({
            "countable_attempts": countable_attempts,
            "verified_successes": verified_successes,
            "solve_rate": if countable_attempts == 0 { Value::Null } else { json!(verified_successes as f64 / countable_attempts as f64) },
            "matched_denominator": {
                "cells": attempts.iter().map(|attempt| format!("{}:{}:{}", attempt.case_id, attempt.repetition_index, attempt.seed)).collect::<BTreeSet<_>>().len(),
                "cell_ids": attempt_cell_set(&attempts).into_iter().collect::<Vec<_>>(),
                "identity": "case_id,repetition_index,seed",
                "retry_policy": retry_policy,
            },
            "total_cost_micros": cost["total"].clone(),
            "cost_per_attempt_micros": cost["per_attempt"].clone(),
            "cost_per_verified_success_micros": cost["per_verified_success"].clone(),
            "total_wall_time_ms": wall["total"].clone(),
            "wall_time_per_attempt_ms": wall["per_attempt"].clone(),
            "wall_time_per_verified_success_ms": wall["per_verified_success"].clone(),
            "sample_count": self.eval_efficiency_sample_count(run_id, &attempts)?,
            "basis_mix": self.eval_efficiency_basis_mix(run_id, &attempts)?,
            "confidence_intervals": {},
            "reasons": reasons,
        }))
    }

    fn eval_effort_recommendation(
        &self,
        comparison: &Value,
        baseline_run_id: &str,
        candidate_run_id: &str,
        baseline_efficiency: &Value,
        candidate_efficiency: &Value,
    ) -> Result<Value> {
        let baseline_identity = self.eval_effective_treatment_identity(baseline_run_id)?;
        let candidate_identity = self.eval_effective_treatment_identity(candidate_run_id)?;
        let mut reasons = non_promotable_json_reasons(&comparison["reasons"]);
        append_json_reasons(&mut reasons, &baseline_identity["reasons"]);
        append_json_reasons(&mut reasons, &candidate_identity["reasons"]);
        append_json_reasons(&mut reasons, &baseline_efficiency["reasons"]);
        append_json_reasons(&mut reasons, &candidate_efficiency["reasons"]);

        let materially_improved = comparison["verdict"].as_str() == Some("improved")
            && has_json_reason(&comparison["reasons"], "candidate_improved");
        let selected_metric_materiality = selected_metric_materiality(comparison);
        let selected_metric_materiality_value = selected_metric_materiality.unwrap_or(&Value::Null);
        let selection_metric = selected_metric_materiality_value["metric"]
            .as_str()
            .unwrap_or("cost_per_verified_success_micros");
        let selected_metric_materially_improved =
            selected_metric_materiality_value["materially_improved"].as_bool() == Some(true);
        if !materially_improved || !selected_metric_materially_improved {
            push_reason_string(&mut reasons, "no_material_effect");
        }
        if !matched_denominator_cell_sets_match(baseline_efficiency, candidate_efficiency) {
            push_reason_string(&mut reasons, "testbed_incompatible");
        }
        let better_candidate = if selected_metric_materiality.is_some() {
            match (
                selected_metric_materiality_value["baseline_value"].as_f64(),
                selected_metric_materiality_value["candidate_value"].as_f64(),
            ) {
                (Some(baseline), Some(candidate)) => candidate < baseline,
                _ => {
                    push_reason_string(&mut reasons, "samples_unavailable");
                    false
                }
            }
        } else {
            false
        };
        if !better_candidate
            && selected_metric_materiality_value["baseline_value"]
                .as_f64()
                .is_some()
            && selected_metric_materiality_value["candidate_value"]
                .as_f64()
                .is_some()
        {
            push_reason_string(&mut reasons, "no_material_effect");
        }
        let recommendation = if materially_improved
            && selected_metric_materially_improved
            && better_candidate
            && reasons.is_empty()
        {
            "prefer_candidate"
        } else {
            "no_recommendation"
        };
        Ok(json!({
            "kind": "derived_pareto_view",
            "recommendation": recommendation,
            "promotion_allowed": recommendation == "prefer_candidate",
            "selection_metric": selection_metric,
            "baseline": {
                "run_id": baseline_run_id,
                "rank": if recommendation == "prefer_candidate" { 2 } else { 1 },
                "effective_treatment": baseline_identity["identity"].clone(),
                "cost_per_verified_success_micros": baseline_efficiency["cost_per_verified_success_micros"].clone(),
                "wall_time_per_verified_success_ms": baseline_efficiency["wall_time_per_verified_success_ms"].clone(),
                "matched_denominator": baseline_efficiency["matched_denominator"].clone(),
            },
            "candidate": {
                "run_id": candidate_run_id,
                "rank": if recommendation == "prefer_candidate" { 1 } else { 2 },
                "effective_treatment": candidate_identity["identity"].clone(),
                "cost_per_verified_success_micros": candidate_efficiency["cost_per_verified_success_micros"].clone(),
                "wall_time_per_verified_success_ms": candidate_efficiency["wall_time_per_verified_success_ms"].clone(),
                "matched_denominator": candidate_efficiency["matched_denominator"].clone(),
            },
            "basis": {
                "verdict": comparison["verdict"].clone(),
                "testbed_comparison": "baseline and candidate compatibility remains governed by the stored comparison gates",
                "matched_cells": "case_id,repetition_index,seed",
                "automatic_routing": false,
            },
            "reasons": reasons,
        }))
    }

    fn eval_effective_treatment_identity(&self, run_id: &str) -> Result<Value> {
        let mut stmt = self.conn.prepare(
            "SELECT effective_client, effective_provider, effective_runtime, effective_model, effective_effort, effective_profile_id, profile_config_digest
             FROM eval_attempts
             WHERE run_id = ?1 AND countable = 1
             ORDER BY case_id, repetition_index, seed, attempt_index, id",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;
        let identities = collect_rows(rows)?
            .into_iter()
            .map(
                |(client, provider, runtime, model, effort, profile_id, profile_config_digest)| {
                    json!({
                        "client": client,
                        "provider": provider,
                        "runtime": runtime,
                        "model": model,
                        "effort": effort,
                        "profile_id": profile_id,
                        "profile_config_digest": profile_config_digest,
                    })
                },
            )
            .collect::<Vec<_>>();
        let mut reasons = Vec::new();
        if identities.is_empty() {
            push_reason_string(&mut reasons, "samples_below_minimum");
            return Ok(json!({"identity": Value::Null, "reasons": reasons}));
        }
        if identities.iter().any(|identity| {
            [
                "client",
                "provider",
                "runtime",
                "model",
                "effort",
                "profile_config_digest",
            ]
            .iter()
            .any(|field| identity[*field].is_null())
        }) {
            push_reason_string(&mut reasons, "samples_unavailable");
        }
        let unique = identities
            .into_iter()
            .map(|identity| serde_json::to_string(&identity).unwrap_or_else(|_| "null".to_string()))
            .collect::<BTreeSet<_>>();
        if unique.len() > 1 {
            push_reason_string(&mut reasons, "testbed_incompatible");
        }
        let identity = unique
            .into_iter()
            .next()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .unwrap_or(Value::Null);
        Ok(json!({
            "identity": identity,
            "reasons": reasons,
        }))
    }

    fn eval_efficiency_attempts(&self, run_id: &str) -> Result<Vec<EfficiencyAttempt>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, repetition_index, seed, terminal_status, retry_of_attempt_id, fallback_of_attempt_id, escalation_of_attempt_id, resume_of_attempt_id
             FROM eval_attempts
             WHERE run_id = ?1 AND countable = 1
             ORDER BY case_id, repetition_index, seed, attempt_index, id",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok(EfficiencyAttempt {
                id: row.get(0)?,
                case_id: row.get(1)?,
                repetition_index: row.get(2)?,
                seed: row.get(3)?,
                terminal_status: row.get(4)?,
                retry_of_attempt_id: row.get(5)?,
                fallback_of_attempt_id: row.get(6)?,
                escalation_of_attempt_id: row.get(7)?,
                resume_of_attempt_id: row.get(8)?,
            })
        })?;
        collect_rows(rows)
    }

    fn eval_efficiency_metric(
        &self,
        run_id: &str,
        attempts: &[EfficiencyAttempt],
        measure: &str,
        unit: &str,
    ) -> Result<Value> {
        let mut stmt = self.conn.prepare(
            "SELECT attempt_id, value_json, metering_basis, estimate_provenance
             FROM eval_samples
             WHERE run_id = ?1 AND measure = ?2
             ORDER BY attempt_index, id",
        )?;
        let rows = stmt.query_map(params![run_id, measure], |row| {
            let raw: String = row.get(1)?;
            Ok((
                row.get::<_, String>(0)?,
                parse_json_row(raw),
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let attempt_ids = attempt_id_set(attempts);
        let samples = collect_rows(rows)?
            .into_iter()
            .filter(|(attempt_id, _, _, _)| attempt_ids.contains(attempt_id))
            .map(|(attempt_id, value, basis, estimate_provenance)| {
                (attempt_id, (value, basis, estimate_provenance))
            })
            .collect::<BTreeMap<String, (Value, String, Option<String>)>>();
        let mut total = 0.0;
        let mut observed = 0usize;
        let mut has_unavailable = false;
        let mut bases = BTreeSet::new();
        let mut provenance = BTreeSet::new();
        let mut reasons = Vec::new();
        for attempt in attempts {
            let Some((value, basis, estimate_provenance)) = samples.get(&attempt.id) else {
                has_unavailable = true;
                push_reason_string(&mut reasons, "samples_unavailable");
                continue;
            };
            if basis == "unavailable" {
                has_unavailable = true;
                push_reason_string(&mut reasons, "samples_unavailable");
            } else {
                bases.insert(basis.clone());
                if let Some(raw) = estimate_provenance {
                    if let Ok(value) = serde_json::from_str::<Value>(raw) {
                        if let Some(label) = estimate_provenance_label(&value) {
                            provenance.insert(label);
                        }
                    }
                }
            }
            if let Some(value) = value.as_f64() {
                total += value;
                observed += 1;
            }
        }
        if !attempts.is_empty() && attempts.len() < 2 {
            push_reason_string(&mut reasons, "samples_below_minimum");
        }
        let verified_successes = attempts
            .iter()
            .filter(|attempt| attempt.terminal_status == "verified_success")
            .count();
        if verified_successes == 0 && !attempts.is_empty() {
            push_reason_string(&mut reasons, "zero_verified_successes");
        }
        let total_available =
            !attempts.is_empty() && !has_unavailable && observed == attempts.len();
        let basis = metric_basis(
            &bases,
            has_unavailable || !total_available || verified_successes == 0,
        );
        let total_value = if total_available {
            number_value(total)
        } else {
            Value::Null
        };
        let per_attempt_value = if total_available && !attempts.is_empty() {
            number_value(total / attempts.len() as f64)
        } else {
            Value::Null
        };
        let per_success_value = if total_available && verified_successes > 0 {
            number_value(total / verified_successes as f64)
        } else {
            Value::Null
        };
        let estimate_provenance = Value::Array(provenance.into_iter().map(Value::String).collect());
        Ok(json!({
            "total": {
                "value": total_value,
                "unit": unit,
                "basis": basis,
                "estimate_provenance": estimate_provenance,
                "reasons": reasons,
            },
            "per_attempt": {
                "value": per_attempt_value,
                "unit": unit,
                "basis": basis,
                "state": if per_attempt_value.is_null() { "unavailable" } else { "available" },
                "estimate_provenance": estimate_provenance,
                "reasons": reasons,
            },
            "per_verified_success": {
                "value": per_success_value,
                "unit": unit,
                "basis": basis,
                "state": if per_success_value.is_null() { "unavailable" } else { "available" },
                "estimate_provenance": estimate_provenance,
                "reasons": reasons,
            },
            "reasons": reasons,
        }))
    }

    fn eval_efficiency_sample_count(
        &self,
        run_id: &str,
        attempts: &[EfficiencyAttempt],
    ) -> Result<i64> {
        let attempt_ids = attempt_id_set(attempts);
        let mut stmt = self.conn.prepare(
            "SELECT attempt_id FROM eval_samples WHERE run_id = ?1 AND measure IN ('cost_micros','wall_time_ms')",
        )?;
        let rows = stmt.query_map(params![run_id], |row| row.get::<_, String>(0))?;
        Ok(collect_rows(rows)?
            .into_iter()
            .filter(|attempt_id| attempt_ids.contains(attempt_id))
            .count() as i64)
    }

    fn eval_efficiency_basis_mix(
        &self,
        run_id: &str,
        attempts: &[EfficiencyAttempt],
    ) -> Result<Value> {
        let attempt_ids = attempt_id_set(attempts);
        let mut stmt = self.conn.prepare(
            "SELECT attempt_id, metering_basis FROM eval_samples WHERE run_id = ?1 AND measure IN ('cost_micros','wall_time_ms') ORDER BY metering_basis",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let bases = collect_rows(rows)?
            .into_iter()
            .filter_map(|(attempt_id, basis)| attempt_ids.contains(&attempt_id).then_some(basis))
            .collect::<BTreeSet<_>>();
        Ok(Value::Array(bases.into_iter().map(Value::String).collect()))
    }
}

fn eval_comparison_policy_from_manifest(manifest: &Value) -> EvalComparisonPolicy {
    let mut policy = EvalComparisonPolicy::default();
    let Some(value) = manifest.get("comparison_policy") else {
        return policy;
    };
    if let Some(number) = value.get("required_case_coverage").and_then(Value::as_f64) {
        policy.required_case_coverage = number;
    }
    if let Some(hours) = value.get("freshness_max_age_hours").and_then(Value::as_i64) {
        policy.freshness_max_age_hours = hours;
    }
    if let Some(number) = value
        .get("quality_non_inferiority_margin")
        .and_then(Value::as_f64)
    {
        policy.quality_non_inferiority_margin = number;
    }
    if let Some(number) = value.get("variance_cv_max").and_then(Value::as_f64) {
        policy.variance_cv_max = number;
    }
    if let Some(iterations) = value.get("bootstrap_iterations").and_then(Value::as_u64) {
        policy.bootstrap_iterations = iterations as usize;
    }
    if let Some(min_samples) = value.get("min_samples").and_then(Value::as_u64) {
        policy.min_samples = min_samples as usize;
    } else if let Some(min_samples) = manifest
        .get("cases")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|case| {
            case.get("sampling")?
                .get("min_successful_samples")?
                .as_u64()
        })
        .max()
    {
        policy.min_samples = min_samples as usize;
    }
    if value.get("material_improvement").is_some() {
        policy.material_improvement = comparison_thresholds(value, "material_improvement");
    }
    if value.get("material_regression").is_some() {
        policy.material_regression = comparison_thresholds(value, "material_regression");
    }
    if value.get("protected_relative_max").is_some() {
        policy.protected_relative_max = comparison_thresholds(value, "protected_relative_max");
    }
    policy
}

fn comparison_thresholds(policy: &Value, field: &str) -> BTreeMap<String, f64> {
    policy
        .get(field)
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| {
            let threshold = value.as_f64()?;
            let canonical = key
                .strip_suffix("_relative")
                .map(|measure| format!("{measure}_p95_relative"))
                .unwrap_or_else(|| key.clone());
            Some((canonical, threshold))
        })
        .collect()
}

#[derive(Debug)]
struct EfficiencyAttempt {
    id: String,
    case_id: String,
    repetition_index: i64,
    seed: i64,
    terminal_status: String,
    retry_of_attempt_id: Option<String>,
    fallback_of_attempt_id: Option<String>,
    escalation_of_attempt_id: Option<String>,
    resume_of_attempt_id: Option<String>,
}

fn attempt_id_set(attempts: &[EfficiencyAttempt]) -> BTreeSet<String> {
    attempts.iter().map(|attempt| attempt.id.clone()).collect()
}

fn attempt_cell_set(attempts: &[EfficiencyAttempt]) -> BTreeSet<String> {
    attempts
        .iter()
        .map(|attempt| {
            format!(
                "{}:{}:{}",
                attempt.case_id, attempt.repetition_index, attempt.seed
            )
        })
        .collect()
}

fn append_json_reasons(reasons: &mut Vec<String>, value: &Value) {
    if let Some(items) = value.as_array() {
        for item in items {
            if let Some(reason) = item.as_str() {
                push_reason_string(reasons, reason);
            }
        }
    }
}

fn non_promotable_json_reasons(value: &Value) -> Vec<String> {
    let mut reasons = Vec::new();
    if let Some(items) = value.as_array() {
        for item in items {
            if let Some(reason) = item.as_str() {
                if !matches!(reason, "candidate_improved" | "no_material_effect") {
                    push_reason_string(&mut reasons, reason);
                }
            }
        }
    }
    reasons
}

fn has_json_reason(value: &Value, expected: &str) -> bool {
    value
        .as_array()
        .map(|items| items.iter().any(|item| item.as_str() == Some(expected)))
        .unwrap_or(false)
}

fn matched_denominator_cell_sets_match(baseline: &Value, candidate: &Value) -> bool {
    json_string_set(&baseline["matched_denominator"]["cell_ids"])
        == json_string_set(&candidate["matched_denominator"]["cell_ids"])
}

fn selected_metric_materiality(comparison: &Value) -> Option<&Value> {
    let materiality =
        &comparison["effect_estimates"]["effort_recommendation"]["selection_metric_materiality"];
    materiality.as_object().map(|_| materiality)
}

fn json_string_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn push_reason_string(reasons: &mut Vec<String>, reason: &str) {
    if !reasons.iter().any(|existing| existing == reason) {
        reasons.push(reason.to_string());
    }
}

fn metric_basis(bases: &BTreeSet<String>, unavailable: bool) -> String {
    if unavailable || bases.is_empty() {
        "unavailable".to_string()
    } else if bases.len() == 1 {
        bases
            .iter()
            .next()
            .cloned()
            .unwrap_or_else(|| "unavailable".to_string())
    } else {
        "mixed".to_string()
    }
}

fn number_value(value: f64) -> Value {
    if (value.fract()).abs() < f64::EPSILON {
        json!(value as i64)
    } else {
        json!(value)
    }
}

fn estimate_provenance_label(value: &Value) -> Option<String> {
    let id = value.get("pricing_reference_id").and_then(Value::as_str)?;
    let version = value
        .get("pricing_reference_version")
        .and_then(Value::as_str)?;
    Some(format!("{id}@{version}"))
}

fn eval_show_kind(kind: EvalShowKind) -> &'static str {
    match kind {
        EvalShowKind::Suite => "suite",
        EvalShowKind::Run => "run",
        EvalShowKind::Comparison => "comparison",
        EvalShowKind::Invalidation => "invalidation",
    }
}

fn read_json_file(path: &std::path::Path) -> Result<Value> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

fn ensure_eval_run_input_matches_started_run(input: &EvalRunInput, existing: &Value) -> Result<()> {
    let expected_id = input.id.as_deref().unwrap_or("<generated>");
    for (field, matches) in [
        (
            "suite_digest",
            existing["suite_digest"].as_str() == Some(input.suite_digest.as_str()),
        ),
        (
            "subject_kind",
            existing["subject_kind"].as_str() == Some(input.subject.kind.as_str()),
        ),
        (
            "subject_revision",
            existing["subject_revision"].as_str() == Some(input.subject.revision.as_str()),
        ),
        (
            "runner_version",
            existing["runner_version"].as_str() == Some(input.runner_version.as_str()),
        ),
        (
            "planr_version",
            existing["planr_version"].as_str() == Some(input.planr_version.as_str()),
        ),
    ] {
        if !matches {
            bail!("eval run {expected_id} already exists with different {field}");
        }
    }
    if existing["subject_path"].as_str() != input.subject.path.as_deref() {
        bail!("eval run {expected_id} already exists with different subject_path");
    }
    if existing["subject_label"].as_str() != input.subject.label.as_deref() {
        bail!("eval run {expected_id} already exists with different subject_label");
    }
    if existing["subject_argv"] != input.subject.argv {
        bail!("eval run {expected_id} already exists with different subject_argv");
    }
    if existing["testbed_fingerprint"] != input.testbed_fingerprint {
        bail!("eval run {expected_id} already exists with different testbed_fingerprint");
    }
    if existing["source_state"] != input.source_state {
        bail!("eval run {expected_id} already exists with different source_state");
    }
    for (field, expected) in [
        ("parent_run_id", input.parent_run_id.as_deref()),
        ("resume_of", input.resume_of.as_deref()),
        ("rescore_of", input.rescore_of.as_deref()),
        ("recompute_of", input.recompute_of.as_deref()),
    ] {
        if existing[field].as_str() != expected {
            bail!("eval run {expected_id} already exists with different {field}");
        }
    }
    Ok(())
}

fn reusable_cases_for_runner_manifest(
    app: &App,
    run_input: &EvalRunInput,
    manifest: &EvalRunnerManifest,
    resume_parent_run_id: Option<&str>,
) -> Result<Vec<super::repository::eval::EvalReusableCaseEvidence>> {
    let Some(parent_run_id) = resume_parent_run_id else {
        return Ok(Vec::new());
    };
    let mut reusable = Vec::new();
    for case in &manifest.cases {
        if let Some(evidence) =
            app.reusable_eval_case_evidence(super::repository::eval::EvalReuseCaseCriteria {
                suite_digest: &run_input.suite_digest,
                subject: &run_input.subject,
                testbed_fingerprint: &run_input.testbed_fingerprint,
                resume_parent_run_id: Some(parent_run_id),
                case_id: &case.case_id,
                scorer_id: &case.scorer_id,
                scorer_version: &case.scorer_version,
                required_measures: &case.measures,
                required_repetitions: case.sampling.repetitions,
            })?
        {
            reusable.push(evidence);
        }
    }
    Ok(reusable)
}

#[derive(Debug, Clone)]
struct SuiteCaseContract {
    scorer_id: String,
    scorer_version: String,
    fixture_digest: String,
    scorer_control: Option<String>,
    required_repetitions: usize,
    measures: BTreeSet<String>,
}

fn validate_supplied_evidence_cases(
    input: &Value,
    stored_manifest: &Value,
) -> Result<Vec<(EvalCaseResultInput, Vec<EvalSampleInput>)>> {
    let suite_cases = supplied_case_contracts(stored_manifest)?;
    let mut seen_cases = BTreeSet::new();
    let mut validated = Vec::new();
    for raw_case in input
        .get("cases")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let case_input = eval_case_input(raw_case)?;
        let Some(contract) = suite_cases.get(&case_input.case_id) else {
            bail!(
                "invalid eval supplied case {} is not in frozen suite",
                case_input.case_id
            );
        };
        if !seen_cases.insert(case_input.case_id.clone()) {
            bail!(
                "invalid eval supplied duplicate case {}",
                case_input.case_id
            );
        }
        if case_input.scorer_id != contract.scorer_id
            || case_input.scorer_version != contract.scorer_version
        {
            bail!(
                "invalid eval supplied scorer identity for case {}: expected {}@{}, got {}@{}",
                case_input.case_id,
                contract.scorer_id,
                contract.scorer_version,
                case_input.scorer_id,
                case_input.scorer_version
            );
        }
        if case_input.fixture_digest != contract.fixture_digest {
            bail!(
                "invalid eval supplied fixture digest for case {}: expected {}, got {}",
                case_input.case_id,
                contract.fixture_digest,
                case_input.fixture_digest
            );
        }
        let samples = eval_samples_input(raw_case.get("samples").and_then(Value::as_array))?;
        let sample_count = samples.iter().filter(|sample| !sample.warmup).count() as i64;
        let warmup_count = samples.iter().filter(|sample| sample.warmup).count() as i64;
        if case_input.repetition_count != sample_count || case_input.warmup_count != warmup_count {
            bail!(
                "invalid eval supplied sampling shape for case {}: declared repetitions/warmups {}/{}, samples {}/{}",
                case_input.case_id,
                case_input.repetition_count,
                case_input.warmup_count,
                sample_count,
                warmup_count
            );
        }
        validate_supplied_control_samples(&case_input, &samples, contract)?;
        for sample in &samples {
            if !contract.measures.is_empty() && !contract.measures.contains(&sample.measure) {
                bail!(
                    "invalid eval supplied sample measure {} for case {}",
                    sample.measure,
                    case_input.case_id
                );
            }
        }
        validated.push((case_input, samples));
    }
    Ok(validated)
}

fn supplied_case_contracts(stored_manifest: &Value) -> Result<BTreeMap<String, SuiteCaseContract>> {
    let fixture_digests = stored_manifest
        .get("fixtures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|fixture| {
            let id = required_string(fixture, "id")?;
            let digest = required_string(fixture, "digest")?;
            validate_sha256_digest_format(&digest)?;
            Ok((id, digest))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut cases = BTreeMap::new();
    for value in stored_manifest
        .get("cases")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let case_id = required_string(value, "case_id")?;
        if cases.contains_key(&case_id) {
            bail!("invalid eval suite duplicate case {case_id}");
        }
        let fixture_ids = string_array_field(value, "fixture_ids")
            .or_else(|| string_field(value, "fixture_id").map(|fixture_id| vec![fixture_id]))
            .unwrap_or_default();
        let fixture_digest = fixture_ids
            .iter()
            .map(|fixture_id| {
                fixture_digests.get(fixture_id).cloned().ok_or_else(|| {
                    anyhow!("invalid eval suite case {case_id} references fixture {fixture_id}")
                })
            })
            .collect::<Result<Vec<_>>>()?
            .join(",");
        let (scorer_id, scorer_version) = suite_case_scorer_identity(value);
        let scorer_control = value
            .get("scorer_control")
            .and_then(|control| control.get("kind"))
            .and_then(Value::as_str)
            .map(|kind| {
                if kind == "null" {
                    "no_op".to_string()
                } else {
                    kind.to_string()
                }
            });
        let required_repetitions = value
            .get("sampling")
            .and_then(|sampling| sampling.get("repetitions"))
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize;
        let measures = string_array_field(value, "measures")
            .unwrap_or_default()
            .into_iter()
            .collect::<BTreeSet<_>>();
        cases.insert(
            case_id,
            SuiteCaseContract {
                scorer_id,
                scorer_version,
                fixture_digest,
                scorer_control,
                required_repetitions,
                measures,
            },
        );
    }
    Ok(cases)
}

fn validate_supplied_control_samples(
    case_input: &EvalCaseResultInput,
    samples: &[EvalSampleInput],
    contract: &SuiteCaseContract,
) -> Result<()> {
    let Some(control) = contract.scorer_control.as_deref() else {
        return Ok(());
    };
    let required_repetitions = contract.required_repetitions.max(2);
    if case_input.repetition_count < required_repetitions as i64 {
        bail!("samples_below_minimum: {}", case_input.case_id);
    }

    let mut repetitions = BTreeSet::new();
    let mut sample_cells = BTreeSet::new();
    let mut statuses_by_repetition = BTreeMap::<i64, BTreeSet<String>>::new();
    for sample in samples.iter().filter(|sample| !sample.warmup) {
        if sample.repetition_index < 0 {
            bail!("samples_below_minimum: {}", case_input.case_id);
        }
        if !sample_cells.insert((sample.repetition_index, sample.measure.clone())) {
            bail!("scorer_control_nondeterministic: {}", case_input.case_id);
        }
        repetitions.insert(sample.repetition_index);
        statuses_by_repetition
            .entry(sample.repetition_index)
            .or_default()
            .insert(sample.attempt.terminal_status.clone());
    }
    let expected_repetitions = (0..required_repetitions as i64).collect::<BTreeSet<_>>();
    if repetitions != expected_repetitions {
        bail!("samples_below_minimum: {}", case_input.case_id);
    }
    for statuses in statuses_by_repetition.values() {
        if statuses.len() != 1 {
            bail!("scorer_control_nondeterministic: {}", case_input.case_id);
        }
        let status = statuses.iter().next().expect("status exists");
        if status != &case_input.status {
            bail!("scorer_control_nondeterministic: {}", case_input.case_id);
        }
    }
    if control == "oracle" && case_input.status != "pass" {
        bail!("scorer_control_nondeterministic: {}", case_input.case_id);
    }
    if matches!(control, "known_bad" | "no_op") && case_input.status == "pass" {
        bail!("scorer_control_nondeterministic: {}", case_input.case_id);
    }
    Ok(())
}

fn suite_case_scorer_identity(value: &Value) -> (String, String) {
    let scorer_id = string_field(value, "scorer_id")
        .or_else(|| {
            value
                .get("scorer_ids")
                .and_then(Value::as_array)
                .and_then(|values| values.first())
                .and_then(Value::as_str)
                .and_then(|identity| identity.split_once('@').map(|(id, _)| id.to_string()))
        })
        .unwrap_or_else(|| "default".to_string());
    let scorer_version = string_field(value, "scorer_version")
        .or_else(|| {
            value
                .get("scorer_ids")
                .and_then(Value::as_array)
                .and_then(|values| values.first())
                .and_then(Value::as_str)
                .and_then(|identity| {
                    identity
                        .split_once('@')
                        .map(|(_, version)| version.to_string())
                })
        })
        .unwrap_or_else(|| "v1".to_string());
    (scorer_id, scorer_version)
}

fn runner_case_input(
    manifest_case: &EvalRunnerCase,
    case: &crate::eval_runner::EvalCaseEvidence,
    fixture_digests: &std::collections::HashMap<String, String>,
) -> EvalCaseResultInput {
    let sample_count = case.samples.iter().filter(|sample| !sample.warmup).count();
    let warmup_count = case.samples.iter().filter(|sample| sample.warmup).count();
    let fixture_digest = manifest_case
        .fixture_ids
        .iter()
        .filter_map(|fixture_id| fixture_digests.get(fixture_id))
        .cloned()
        .collect::<Vec<_>>()
        .join(",");
    EvalCaseResultInput {
        case_id: case.case_id.clone(),
        scorer_id: manifest_case.scorer_id.clone(),
        scorer_version: manifest_case.scorer_version.clone(),
        fixture_digest,
        status: case.status.clone(),
        repetition_count: sample_count as i64,
        warmup_count: warmup_count as i64,
        assertions: json!([
            {"kind": "safety_pass", "status": if case.status == "pass" { "pass" } else { "fail" }},
            {"kind": "quality_pass", "status": if case.status == "pass" { "pass" } else { "fail" }}
        ]),
        command: json!({
            "samples": case.samples.iter().map(|sample| command_evidence_value(&sample.command)).collect::<Vec<_>>()
        }),
        reasons: json!(case.reasons),
    }
}

fn runner_sample_inputs(
    run_id: &str,
    case: &crate::eval_runner::EvalCaseEvidence,
) -> Vec<EvalSampleInput> {
    case.samples
        .iter()
        .flat_map(|sample| {
            let outcome = runner_sample_outcome(case, sample);
            let attempt_id_for_index = |index: usize| {
                let phase = if sample.warmup { "warmup" } else { "measured" };
                format!(
                    "evattempt-{}-{}-{}-{}-{}-{}",
                    run_id, case.case_id, phase, sample.repetition_index, sample.seed, index
                )
            };
            let attempt_id = attempt_id_for_index(sample.attempt_index);
            let attempt = EvalAttemptInput {
                id: Some(attempt_id.clone()),
                attempt_index: sample.attempt_index as i64,
                terminal_status: if sample.command.timed_out || sample.command.interrupted {
                    "error".to_string()
                } else if sample.command.exit_code == Some(0) {
                    "pass".to_string()
                } else {
                    "fail".to_string()
                },
                countable: !sample.warmup,
                retry_of_attempt_id: sample.retry_of_attempt_index.map(attempt_id_for_index),
                fallback_of_attempt_id: sample.fallback_of_attempt_index.map(attempt_id_for_index),
                escalation_of_attempt_id: sample
                    .escalation_of_attempt_index
                    .map(attempt_id_for_index),
                resume_of_attempt_id: None,
                effective_client: Some("planr".to_string()),
                effective_provider: Some("local_process".to_string()),
                effective_runtime: Some("bounded-command".to_string()),
                effective_model: Some("external-subject".to_string()),
                effective_effort: Some("not-applicable".to_string()),
                effective_profile_id: Some("eval-runner-local-process".to_string()),
                profile_config_digest: Some("builtin:eval-runner-local-process-v1".to_string()),
                runner_harness_version: "eval-runner-v1".to_string(),
                route_observation: Some(local_process_route_observation(sample.duration_ms)),
                outcome,
            };
            let base = |measure: &str, value: Value, unit: &str, source: &str, basis: &str| {
                let basis_confidence = if basis == "actual_trusted" {
                    "verified"
                } else {
                    basis
                };
                EvalSampleInput {
                    id: None,
                    attempt: attempt.clone(),
                    repetition_index: sample.repetition_index as i64,
                    warmup: sample.warmup,
                    seed: sample.seed as i64,
                    measure: measure.to_string(),
                    value,
                    unit: unit.to_string(),
                    source: source.to_string(),
                    metering_basis: basis.to_string(),
                    basis_source: source.to_string(),
                    basis_confidence: basis_confidence.to_string(),
                    estimate_provenance: None,
                }
            };
            vec![
                base(
                    "duration_ms",
                    json!(sample.duration_ms),
                    "ms",
                    "process",
                    "actual_trusted",
                ),
                base(
                    "wall_time_ms",
                    json!(sample.duration_ms),
                    "ms",
                    "metering",
                    "actual_trusted",
                ),
                base(
                    "tool_calls",
                    Value::Null,
                    "count",
                    "metering",
                    "unavailable",
                ),
                base(
                    "input_tokens",
                    Value::Null,
                    "tokens",
                    "metering",
                    "unavailable",
                ),
                base(
                    "output_tokens",
                    Value::Null,
                    "tokens",
                    "metering",
                    "unavailable",
                ),
                base(
                    "total_tokens",
                    Value::Null,
                    "tokens",
                    "metering",
                    "unavailable",
                ),
                base(
                    "credits_micros",
                    Value::Null,
                    "micros",
                    "metering",
                    "unavailable",
                ),
                base(
                    "cost_micros",
                    Value::Null,
                    "micros",
                    "metering",
                    "unavailable",
                ),
            ]
        })
        .collect()
}

fn runner_sample_outcome(
    case: &crate::eval_runner::EvalCaseEvidence,
    sample: &crate::eval_runner::EvalSampleEvidence,
) -> Value {
    let mut outcome = command_evidence_value(&sample.command);
    if let Some(object) = outcome.as_object_mut() {
        if let Some(repetition) = case
            .repetition_outcomes
            .iter()
            .find(|outcome| outcome.repetition_index == sample.repetition_index)
        {
            object.insert("status".to_string(), json!(repetition.status));
            object.insert("reasons".to_string(), json!(repetition.reasons));
        }
    }
    outcome
}

fn command_evidence_value(command: &EvalCommandEvidence) -> Value {
    json!({
        "status": command_status(command),
        "argv": command.argv,
        "exit_code": command.exit_code,
        "timed_out": command.timed_out,
        "interrupted": command.interrupted,
        "stdout_digest": command.stdout_digest,
        "stderr_digest": command.stderr_digest,
    })
}

fn command_status(command: &EvalCommandEvidence) -> &'static str {
    if command.timed_out || command.interrupted {
        "error"
    } else if command.exit_code == Some(0) {
        "pass"
    } else {
        "fail"
    }
}

fn runner_run_status(cases: &[crate::eval_runner::EvalCaseEvidence]) -> &'static str {
    if cases
        .iter()
        .any(|case| matches!(case.status.as_str(), "error"))
    {
        "error"
    } else if cases
        .iter()
        .any(|case| matches!(case.status.as_str(), "inconclusive"))
    {
        "inconclusive"
    } else {
        "success"
    }
}

fn string_field(value: &Value, name: &str) -> Option<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn string_array_field(value: &Value, name: &str) -> Option<Vec<String>> {
    value.get(name).and_then(Value::as_array).map(|values| {
        values
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn required_string(value: &Value, name: &str) -> Result<String> {
    string_field(value, name).ok_or_else(|| anyhow!("missing required eval field: {name}"))
}

fn eval_run_input(value: &Value) -> Result<EvalRunInput> {
    let subject = value
        .get("subject")
        .ok_or_else(|| anyhow!("missing required eval field: subject"))?;
    Ok(EvalRunInput {
        id: string_field(value, "id"),
        suite_digest: required_string(value, "suite_digest")?,
        subject: EvalSubjectInput {
            kind: required_string(subject, "kind")?,
            revision: required_string(subject, "revision")?,
            path: string_field(subject, "path"),
            argv: subject.get("argv").cloned().unwrap_or_else(|| json!([])),
            label: string_field(subject, "label"),
        },
        runner_version: string_field(value, "runner_version")
            .unwrap_or_else(|| "eval-runner-v1".to_string()),
        planr_version: string_field(value, "planr_version")
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        testbed_fingerprint: value
            .get("testbed_fingerprint")
            .cloned()
            .unwrap_or_else(|| json!({})),
        source_state: value
            .get("source_state")
            .cloned()
            .unwrap_or_else(|| json!({})),
        parent_run_id: string_field(value, "parent_run_id"),
        resume_of: string_field(value, "resume_of"),
        rescore_of: string_field(value, "rescore_of"),
        recompute_of: string_field(value, "recompute_of"),
    })
}

fn validate_sha256_manifest_digest(expected: &str, manifest: &Value) -> Result<()> {
    validate_sha256_digest_format(expected)?;
    if manifest
        .get("digest")
        .and_then(Value::as_str)
        .is_some_and(|declared| declared != expected)
    {
        bail!("eval suite snapshot digest field does not match {expected}");
    }
    let actual = sha256_json_digest_without_top_level_digest(manifest)?;
    if !eval_manifest_digest_matches(expected, &actual, manifest)? {
        bail!("eval suite digest mismatch: expected {expected}, computed {actual}");
    }
    Ok(())
}

fn eval_manifest_digest_matches(expected: &str, actual: &str, manifest: &Value) -> Result<bool> {
    if expected == actual {
        return Ok(true);
    }
    Ok(expected == legacy_sha256_json_digest_without_top_level_digest(manifest)?)
}

fn legacy_sha256_json_digest_without_top_level_digest(value: &Value) -> Result<String> {
    let mut canonical = legacy_sorted_json_value(value);
    if let Some(object) = canonical.as_object_mut() {
        object.remove("digest");
    }
    let bytes =
        serde_json::to_vec(&canonical).context("canonicalizing legacy eval suite manifest")?;
    Ok(sha256_prefixed_bytes(&bytes))
}

fn legacy_sorted_json_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(legacy_sorted_json_value).collect()),
        Value::Object(object) => {
            let mut entries = object
                .iter()
                .map(|(key, value)| (key.clone(), legacy_sorted_json_value(value)))
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut sorted = serde_json::Map::new();
            for (key, value) in entries {
                sorted.insert(key, value);
            }
            Value::Object(sorted)
        }
        scalar => scalar.clone(),
    }
}

fn validate_sha256_digest_format(digest: &str) -> Result<()> {
    let Some(hex) = digest.strip_prefix("sha256:") else {
        bail!("invalid eval suite digest {digest}: expected sha256:<64 hex>");
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid eval suite digest {digest}: expected sha256:<64 hex>");
    }
    Ok(())
}

fn sha256_json_digest_without_top_level_digest(value: &Value) -> Result<String> {
    let value = normalized_eval_suite_digest_value(value);
    sha256_json_digest_without_top_level_field(&value, "digest")
        .context("canonicalizing eval suite manifest")
}

fn canonical_eval_suite_snapshot_value(value: &Value, digest: &str) -> Value {
    let mut normalized = normalized_eval_suite_digest_value(value);
    if let Some(object) = normalized.as_object_mut() {
        object.insert("digest".to_string(), Value::String(digest.to_string()));
    }
    normalized
}

fn normalized_eval_suite_digest_value(value: &Value) -> Value {
    let mut normalized = value.clone();
    let Some(object) = normalized.as_object_mut() else {
        return normalized;
    };
    object.remove("digest");
    sort_object_array_by_string_field(object, "fixtures", "id");
    sort_scorers_by_id_version(object);
    sort_object_array_by_string_field(object, "cases", "case_id");
    if let Some(cases) = object.get_mut("cases").and_then(Value::as_array_mut) {
        for case in cases {
            let Some(case_object) = case.as_object_mut() else {
                continue;
            };
            sort_string_array_field(case_object, "fixture_ids");
            sort_string_array_field(case_object, "scorer_ids");
            sort_string_array_field(case_object, "measures");
        }
    }
    normalized
}

fn sort_scorers_by_id_version(object: &mut serde_json::Map<String, Value>) {
    if let Some(values) = object.get_mut("scorers").and_then(Value::as_array_mut) {
        values.sort_by(|left, right| {
            scorer_id_version_key(left)
                .as_str()
                .cmp(scorer_id_version_key(right).as_str())
        });
    }
}

fn scorer_id_version_key(value: &Value) -> String {
    let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("{id}@{version}")
}

fn sort_object_array_by_string_field(
    object: &mut serde_json::Map<String, Value>,
    array_field: &str,
    key_field: &str,
) {
    if let Some(values) = object.get_mut(array_field).and_then(Value::as_array_mut) {
        values.sort_by(|left, right| {
            let left_key = left
                .get(key_field)
                .and_then(Value::as_str)
                .unwrap_or_default();
            let right_key = right
                .get(key_field)
                .and_then(Value::as_str)
                .unwrap_or_default();
            left_key.cmp(right_key)
        });
    }
}

fn sort_string_array_field(object: &mut serde_json::Map<String, Value>, field: &str) {
    if let Some(values) = object.get_mut(field).and_then(Value::as_array_mut) {
        values.sort_by(|left, right| {
            let left = left.as_str().unwrap_or_default();
            let right = right.as_str().unwrap_or_default();
            left.cmp(right)
        });
    }
}

fn sha256_prefixed_bytes(bytes: &[u8]) -> String {
    crate::canonical_json::sha256_prefixed_bytes(bytes)
}

fn runner_binding_scope(value: &Value) -> Value {
    normalized_eval_suite_digest_value(&json!({
        "schema_version": value.get("schema_version").cloned().unwrap_or(Value::Null),
        "suite_id": value.get("suite_id").cloned().unwrap_or(Value::Null),
        "suite_version": value.get("suite_version").cloned().unwrap_or(Value::Null),
        "fixtures": value.get("fixtures").cloned().unwrap_or(Value::Null),
        "scorers": value.get("scorers").cloned().unwrap_or(Value::Null),
        "cases": value.get("cases").cloned().unwrap_or(Value::Null),
        "safety": value.get("safety").cloned().unwrap_or(Value::Null),
    }))
}

fn validate_scorer_control_admission(manifest: &Value) -> Result<()> {
    let Some(cases) = manifest.get("cases").and_then(Value::as_array) else {
        return Ok(());
    };
    let scorer_identities = manifest
        .get("scorers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|scorer| {
            Some(format!(
                "{}@{}",
                scorer.get("id")?.as_str()?,
                scorer.get("version")?.as_str()?
            ))
        })
        .collect::<BTreeSet<_>>();
    let mut controls_by_scorer = BTreeMap::<String, BTreeSet<String>>::new();
    for case in cases {
        let Some(control) = case.get("scorer_control") else {
            continue;
        };
        let case_id = case
            .get("case_id")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let kind = control
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("scorer_control_missing_kind: {case_id}"))?;
        let kind = if kind == "null" { "no_op" } else { kind };
        if !matches!(kind, "oracle" | "known_bad" | "no_op") {
            bail!("unsupported eval scorer control kind: {kind}");
        }
        let expect = control
            .get("expect")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("scorer_control_missing_expectation: {case_id}"))?;
        if !matches!(expect, "pass" | "fail") {
            bail!("unsupported eval scorer control expectation: {expect}");
        }
        if (kind == "oracle" && expect != "pass") || (kind != "oracle" && expect != "fail") {
            bail!("scorer_control_incorrect_polarity: {case_id}");
        }
        let scorer_ids = case
            .get("scorer_ids")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                let id = case
                    .get("scorer_id")
                    .and_then(Value::as_str)
                    .unwrap_or("default");
                let version = case
                    .get("scorer_version")
                    .and_then(Value::as_str)
                    .unwrap_or("v1");
                vec![format!("{id}@{version}")]
            });
        let sampling = case.get("sampling").unwrap_or(&Value::Null);
        if sampling
            .get("repetitions")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            < 2
        {
            bail!("scorer_control_nondeterministic: {case_id}");
        }
        if sampling
            .get("max_attempts")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            != 1
        {
            bail!("scorer_control_nondeterministic: {case_id}");
        }
        for scorer in scorer_ids {
            if !scorer_identities.is_empty() && !scorer_identities.contains(&scorer) {
                bail!("dangling_reference: scorer {scorer}");
            }
            let kinds = controls_by_scorer.entry(scorer).or_default();
            if !kinds.insert(kind.to_string()) {
                bail!("duplicate_identity: scorer control {kind}");
            }
        }
    }
    for (scorer, controls) in controls_by_scorer {
        if !controls.contains("oracle") {
            bail!("scorer_control_missing_oracle: {scorer}");
        }
        if !controls.contains("known_bad") && !controls.contains("no_op") {
            bail!("scorer_control_missing_negative: {scorer}");
        }
    }
    Ok(())
}

fn eval_case_input(value: &Value) -> Result<EvalCaseResultInput> {
    let case = value.get("case").unwrap_or(value);
    Ok(EvalCaseResultInput {
        case_id: required_string(case, "case_id")?,
        scorer_id: string_field(case, "scorer_id").unwrap_or_else(|| "default".to_string()),
        scorer_version: string_field(case, "scorer_version").unwrap_or_else(|| "v1".to_string()),
        fixture_digest: string_field(case, "fixture_digest")
            .unwrap_or_else(|| "unknown".to_string()),
        status: string_field(case, "status").unwrap_or_else(|| "pass".to_string()),
        repetition_count: case
            .get("repetition_count")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        warmup_count: case
            .get("warmup_count")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        assertions: case.get("assertions").cloned().unwrap_or_else(|| json!([])),
        command: case.get("command").cloned().unwrap_or_else(|| json!({})),
        reasons: case.get("reasons").cloned().unwrap_or_else(|| json!([])),
    })
}

fn eval_samples_input(samples: Option<&Vec<Value>>) -> Result<Vec<EvalSampleInput>> {
    samples
        .into_iter()
        .flatten()
        .map(|sample| {
            let metering_basis_was_supplied = sample.get("metering_basis").is_some();
            let metering_basis =
                string_field(sample, "metering_basis").unwrap_or_else(|| "unavailable".to_string());
            let value = sample.get("value").cloned().unwrap_or(Value::Null);
            let sample_label = string_field(sample, "id")
                .or_else(|| string_field(sample, "measure"))
                .unwrap_or_else(|| "<unknown>".to_string());
            if !metering_basis_was_supplied && !value.is_null() {
                bail!(
                    "invalid eval supplied sample {sample_label} has value but missing metering_basis"
                );
            }
            if metering_basis == "unavailable" && !value.is_null() {
                bail!(
                    "invalid eval supplied sample {sample_label} with unavailable metering_basis requires null value"
                );
            }
            if metering_basis != "unavailable" && value.is_null() {
                bail!(
                    "invalid eval supplied sample {sample_label} with metering_basis {metering_basis} requires value"
                );
            }
            let basis_confidence = string_field(sample, "basis_confidence").unwrap_or_else(|| {
                if metering_basis == "unavailable" {
                    "unavailable".to_string()
                } else {
                    "verified".to_string()
                }
            });
            Ok(EvalSampleInput {
                id: string_field(sample, "id"),
                attempt: eval_attempt_input(sample)?,
                repetition_index: sample
                    .get("repetition_index")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                warmup: sample
                    .get("warmup")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                seed: sample.get("seed").and_then(Value::as_i64).unwrap_or(0),
                measure: string_field(sample, "measure")
                    .unwrap_or_else(|| "duration_ms_p95".to_string()),
                value,
                unit: string_field(sample, "unit").unwrap_or_else(|| "ms".to_string()),
                source: string_field(sample, "source").unwrap_or_else(|| "process".to_string()),
                metering_basis,
                basis_source: string_field(sample, "basis_source")
                    .unwrap_or_else(|| "process".to_string()),
                basis_confidence,
                estimate_provenance: sample.get("estimate_provenance").cloned(),
            })
        })
        .collect()
}

fn eval_attempt_input(sample: &Value) -> Result<EvalAttemptInput> {
    let attempt = sample.get("attempt").unwrap_or(&Value::Null);
    let route_observation = attempt
        .get("route_observation")
        .cloned()
        .map(parse_route_observation)
        .transpose()?
        .map(serde_json::to_value)
        .transpose()?;
    Ok(EvalAttemptInput {
        id: string_field(attempt, "id").or_else(|| string_field(sample, "attempt_id")),
        attempt_index: attempt
            .get("attempt_index")
            .or_else(|| sample.get("attempt_index"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        terminal_status: string_field(attempt, "terminal_status")
            .unwrap_or_else(|| "inconclusive".to_string()),
        countable: attempt
            .get("countable")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| {
                !sample
                    .get("warmup")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            }),
        retry_of_attempt_id: string_field(attempt, "retry_of_attempt_id"),
        fallback_of_attempt_id: string_field(attempt, "fallback_of_attempt_id"),
        escalation_of_attempt_id: string_field(attempt, "escalation_of_attempt_id"),
        resume_of_attempt_id: string_field(attempt, "resume_of_attempt_id"),
        effective_client: string_field(attempt, "effective_client"),
        effective_provider: string_field(attempt, "effective_provider"),
        effective_runtime: string_field(attempt, "effective_runtime"),
        effective_model: string_field(attempt, "effective_model"),
        effective_effort: string_field(attempt, "effective_effort"),
        effective_profile_id: string_field(attempt, "effective_profile_id"),
        profile_config_digest: string_field(attempt, "profile_config_digest"),
        runner_harness_version: string_field(attempt, "runner_harness_version")
            .unwrap_or_else(|| "supplied-evidence-v1".to_string()),
        route_observation,
        outcome: attempt.get("outcome").cloned().unwrap_or_else(|| json!({})),
    })
}

fn eval_route_observation_validation(attempt: &Value) -> Value {
    let Some(raw) = attempt
        .get("route_observation")
        .filter(|value| !value.is_null())
    else {
        return json!({"status": "unavailable", "source": "planr.route_audit.v1"});
    };
    let Ok(observation) = parse_route_observation(raw.clone()) else {
        return json!({"status": "invalid", "source": "planr.route_audit.v1"});
    };
    let effective = &observation.effective;
    let dimensions_verified = [
        effective.agent_type.enforcement,
        effective.model.enforcement,
        effective.effort.enforcement,
        effective.context_fork.enforcement,
    ]
    .into_iter()
    .all(|enforcement| enforcement == EnforcementState::Verified)
        && [
            effective.provider.as_ref(),
            effective.runtime.as_ref(),
            effective.profile_config_digest.as_ref(),
            effective.runner_harness_version.as_ref(),
        ]
        .into_iter()
        .all(|dimension| {
            dimension.is_some_and(|dimension| {
                dimension.enforcement == EnforcementState::Verified
                    && dimension
                        .value
                        .as_ref()
                        .is_some_and(|value| !value.is_empty())
            })
        });
    let identity_matches = [
        (
            effective.client.as_deref(),
            attempt["effective_client"].as_str(),
        ),
        (
            effective.profile.as_deref(),
            attempt["effective_profile_id"].as_str(),
        ),
        (
            effective.model.value.as_deref(),
            attempt["effective_model"].as_str(),
        ),
        (
            effective.effort.value.as_deref(),
            attempt["effective_effort"].as_str(),
        ),
        (
            effective
                .provider
                .as_ref()
                .and_then(|field| field.value.as_deref()),
            attempt["effective_provider"].as_str(),
        ),
        (
            effective
                .runtime
                .as_ref()
                .and_then(|field| field.value.as_deref()),
            attempt["effective_runtime"].as_str(),
        ),
        (
            effective
                .profile_config_digest
                .as_ref()
                .and_then(|field| field.value.as_deref()),
            attempt["profile_config_digest"].as_str(),
        ),
        (
            effective
                .runner_harness_version
                .as_ref()
                .and_then(|field| field.value.as_deref()),
            attempt["runner_harness_version"].as_str(),
        ),
    ]
    .into_iter()
    .all(|(observed, recorded)| observed.is_some() && observed == recorded);
    json!({
        "status": if dimensions_verified && identity_matches { "verified" } else { "invalid" },
        "source": "planr.route_audit.v1",
        "effective": {
            "client": effective.client,
            "profile_id": effective.profile,
            "agent_type": effective.agent_type.value,
            "provider": effective.provider.as_ref().and_then(|field| field.value.as_ref()),
            "runtime": effective.runtime.as_ref().and_then(|field| field.value.as_ref()),
            "model": effective.model.value,
            "effort": effective.effort.value,
            "profile_config_digest": effective.profile_config_digest.as_ref().and_then(|field| field.value.as_ref()),
            "runner_harness_version": effective.runner_harness_version.as_ref().and_then(|field| field.value.as_ref()),
        }
    })
}

fn local_process_route_observation(duration_ms: u128) -> Value {
    let stage = |enforcement: &str, evidence: &str| {
        json!({
            "profile": "eval-runner-local-process",
            "client": "planr",
            "provider": {"value": "local_process", "enforcement": enforcement, "evidence": evidence},
            "runtime": {"value": "bounded-command", "enforcement": enforcement, "evidence": evidence},
            "profile_config_digest": {"value": "builtin:eval-runner-local-process-v1", "enforcement": enforcement, "evidence": evidence},
            "runner_harness_version": {"value": "eval-runner-v1", "enforcement": enforcement, "evidence": evidence},
            "agent_type": {"value": "bounded-command", "enforcement": enforcement, "evidence": evidence},
            "model": {"value": "external-subject", "enforcement": enforcement, "evidence": evidence},
            "effort": {"value": "not-applicable", "enforcement": enforcement, "evidence": evidence},
            "context_fork": {"value": {"mode": "none"}, "enforcement": enforcement, "evidence": evidence}
        })
    };
    json!({
        "requested": stage("requested_only", "policy"),
        "resolved": stage("verified", "binding"),
        "effective": stage("verified", "process_exit"),
        "transition": {"kind": "initial", "reason": "allowlisted process runner completed", "evidence": ["process_exit"]},
        "policy": {"id": "eval-runner-local-process", "version": "v1"},
        "binding": {"id": "bounded-command", "version": "v1"},
        "metering": {
            "wall_time_seconds": {"value": duration_ms.div_ceil(1000).min(u64::MAX as u128) as u64, "confidence": "trusted"},
            "tool_calls": {"confidence": "unavailable"},
            "tokens": {"confidence": "unavailable"},
            "credits_micros": {"confidence": "unavailable"}
        }
    })
}

fn parse_json_row(raw: String) -> Value {
    serde_json::from_str(&raw).unwrap_or(Value::Null)
}

pub(crate) fn eval_success_envelope(command: &str, object: Value) -> Value {
    let reasons = eval_reasons(&object);
    let ok = eval_object_ok(command, &object);
    json!({
        "ok": ok,
        "command": command,
        "object": object,
        "warnings": [],
        "reasons": reasons,
        "error": null
    })
}

pub(crate) fn eval_error_envelope(command: &str, message: &str) -> Value {
    let reasons = eval_error_reasons(message);
    let field = eval_error_field(message);
    json!({
        "ok": false,
        "command": command,
        "object": null,
        "warnings": [],
        "reasons": reasons,
        "error": {
            "code": eval_error_code(message),
            "message": message,
            "reasons": reasons,
            "field": field
        }
    })
}

pub(crate) fn eval_envelope_exit_code(envelope: &Value) -> i32 {
    if envelope.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return 0;
    }
    if envelope.get("error").is_some_and(|error| !error.is_null()) {
        let message = envelope["error"]["message"].as_str().unwrap_or_default();
        return eval_error_exit_code(message);
    }
    match envelope["object"].get("verdict").and_then(Value::as_str) {
        Some("insufficient_evidence") => 2,
        Some("regressed") => 1,
        _ => 1,
    }
}

pub(crate) fn eval_error_exit_code(message: &str) -> i32 {
    if is_eval_invalid_input(message) { 3 } else { 4 }
}

pub(crate) fn eval_error_code(message: &str) -> &'static str {
    if is_eval_invalid_input(message) {
        "invalid_input"
    } else {
        "infrastructure_error"
    }
}

fn eval_object_ok(command: &str, object: &Value) -> bool {
    if command == "eval.compare" || command == "eval.gate" {
        match object.get("verdict").and_then(Value::as_str) {
            Some("regressed" | "insufficient_evidence") => false,
            _ => object.get("ok").and_then(Value::as_bool).unwrap_or(true),
        }
    } else {
        true
    }
}

fn eval_reasons(object: &Value) -> Value {
    if let Some(reasons) = object.get("reasons").and_then(Value::as_array) {
        return Value::Array(reasons.clone());
    }
    if let Some(reasons) = object
        .get("comparison")
        .and_then(|comparison| comparison.get("reasons"))
        .and_then(Value::as_array)
    {
        return Value::Array(reasons.clone());
    }
    match object.get("blocker") {
        Some(Value::String(blocker)) if !blocker.is_empty() => json!([blocker]),
        Some(blocker) if !blocker.is_null() => json!([blocker]),
        _ => json!([]),
    }
}

fn is_eval_invalid_input(message: &str) -> bool {
    message.contains("missing required eval")
        || message.contains("missing required MCP argument")
        || message.contains("eval evidence")
        || message.contains("invalid eval")
        || message.contains("invalid terminal eval")
        || message.contains("invalidation target kind")
        || message.contains("invalid eval show kind")
        || message.contains("immutable eval suite snapshot conflict")
        || message.contains("eval runner manifest does not match frozen suite")
        || message.contains("eval suite digest mismatch")
        || message.contains("invalid eval suite digest")
        || message.contains("eval fixture")
        || message.contains("eval suite snapshot digest field does not match")
        || message.contains("eval repo_root")
        || message.contains("parse ")
}

fn eval_error_reasons(message: &str) -> Value {
    let reason = if message.contains("missing required") {
        "missing_required_field"
    } else if message.contains("invalid") {
        "invalid_value"
    } else if message.contains("parse ") {
        "parse_error"
    } else if message.contains("not found") {
        "not_found"
    } else {
        "infrastructure_error"
    };
    json!([reason])
}

fn eval_error_field(message: &str) -> Value {
    let field = message
        .strip_prefix("missing required eval field: ")
        .or_else(|| message.strip_prefix("missing required eval suite field: "))
        .or_else(|| message.strip_prefix("missing required MCP argument: "))
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .map(ToOwned::to_owned);
    json!(field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval_compare::{
        EvalCaseEvidence as ComparisonCaseEvidence, EvalNumericSample,
        EvalRunEvidence as ComparisonRunEvidence, compare_eval_runs,
    };
    use crate::eval_runner::EvalRunOptions;
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

    fn comparison_run(id: &str, measure: &str, value: f64) -> ComparisonRunEvidence {
        ComparisonRunEvidence {
            run_id: id.to_string(),
            suite_digest: "suite".to_string(),
            subject_kind: "local_authenticated_agent".to_string(),
            subject_path: None,
            subject_argv: "[\"maintainer-eval\",\"lean-skills-dogfood\"]".to_string(),
            testbed_fingerprint: "matched".to_string(),
            status: "success".to_string(),
            invalidated: false,
            age_hours: 0,
            cases: ["goal", "loop", "graph"]
                .into_iter()
                .map(|case_id| ComparisonCaseEvidence {
                    case_id: case_id.to_string(),
                    scorer_id: "invariant-scorer".to_string(),
                    scorer_version: "1.0.0".to_string(),
                    scorer_control: None,
                    fixture_digest: "fixture".to_string(),
                    status: "pass".to_string(),
                    repetition_outcomes: Vec::new(),
                    safety_pass: Some(true),
                    quality_pass: Some(true),
                    min_successful_samples: Some(3),
                    required_repetitions: Some(3),
                    samples: (0..3)
                        .map(|repetition_index| EvalNumericSample {
                            repetition_index,
                            warmup: false,
                            seed: repetition_index as i64,
                            measure: measure.to_string(),
                            value,
                            valid: true,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn suite_digest_is_stable_for_authoring_order() {
        let ordered = json!({
            "schema_version": "eval.suite.v1",
            "suite_id": "ordering-suite",
            "suite_version": "v1",
            "fixtures": [
                {"id": "a-fixture", "path": "a.json", "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
                {"id": "b-fixture", "path": "b.json", "digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
            ],
            "scorers": [
                {"id": "a-score", "version": "v1", "kind": "test"},
                {"id": "b-score", "version": "v1", "kind": "test"}
            ],
            "cases": [
                {
                    "case_id": "a-case",
                    "fixture_ids": ["a-fixture", "b-fixture"],
                    "scorer_ids": ["a-score@v1", "b-score@v1"],
                    "measures": ["cost_micros", "duration_ms"],
                    "sampling": {"repetitions": 1}
                },
                {
                    "case_id": "b-case",
                    "fixture_ids": ["b-fixture", "a-fixture"],
                    "scorer_ids": ["b-score@v1", "a-score@v1"],
                    "measures": ["duration_ms", "cost_micros"],
                    "sampling": {"repetitions": 1}
                }
            ],
            "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
        });
        let reordered = json!({
            "schema_version": "eval.suite.v1",
            "suite_id": "ordering-suite",
            "suite_version": "v1",
            "fixtures": [
                {"id": "b-fixture", "path": "b.json", "digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
                {"id": "a-fixture", "path": "a.json", "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
            ],
            "scorers": [
                {"id": "b-score", "version": "v1", "kind": "test"},
                {"id": "a-score", "version": "v1", "kind": "test"}
            ],
            "cases": [
                {
                    "case_id": "b-case",
                    "fixture_ids": ["a-fixture", "b-fixture"],
                    "scorer_ids": ["a-score@v1", "b-score@v1"],
                    "measures": ["cost_micros", "duration_ms"],
                    "sampling": {"repetitions": 1}
                },
                {
                    "case_id": "a-case",
                    "fixture_ids": ["b-fixture", "a-fixture"],
                    "scorer_ids": ["b-score@v1", "a-score@v1"],
                    "measures": ["duration_ms", "cost_micros"],
                    "sampling": {"repetitions": 1}
                }
            ],
            "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
        });

        assert_eq!(
            sha256_json_digest_without_top_level_digest(&ordered).unwrap(),
            sha256_json_digest_without_top_level_digest(&reordered).unwrap()
        );
    }

    #[test]
    fn suite_digest_sorts_same_id_scorers_by_version() {
        let ordered = json!({
            "schema_version": "eval.suite.v1",
            "suite_id": "versioned-scorer-ordering-suite",
            "suite_version": "v1",
            "fixtures": [
                {"id": "fixture", "path": "fixture.json", "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
            ],
            "scorers": [
                {"id": "score", "version": "v1", "kind": "test", "config": {"weight": 1}},
                {"id": "score", "version": "v2", "kind": "test", "config": {"weight": 2}}
            ],
            "cases": [{
                "case_id": "case",
                "fixture_ids": ["fixture"],
                "scorer_ids": ["score@v2", "score@v1"],
                "measures": ["duration_ms"],
                "sampling": {"repetitions": 1}
            }],
            "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
        });
        let reordered = json!({
            "schema_version": "eval.suite.v1",
            "suite_id": "versioned-scorer-ordering-suite",
            "suite_version": "v1",
            "fixtures": [
                {"id": "fixture", "path": "fixture.json", "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
            ],
            "scorers": [
                {"id": "score", "version": "v2", "kind": "test", "config": {"weight": 2}},
                {"id": "score", "version": "v1", "kind": "test", "config": {"weight": 1}}
            ],
            "cases": [{
                "case_id": "case",
                "fixture_ids": ["fixture"],
                "scorer_ids": ["score@v1", "score@v2"],
                "measures": ["duration_ms"],
                "sampling": {"repetitions": 1}
            }],
            "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
        });

        assert_eq!(
            sha256_json_digest_without_top_level_digest(&ordered).unwrap(),
            sha256_json_digest_without_top_level_digest(&reordered).unwrap()
        );
    }

    #[test]
    fn manifest_policy_drives_material_efficiency_verdicts() {
        let manifest = json!({
            "cases": [{
                "sampling": { "min_successful_samples": 3 }
            }],
            "comparison_policy": {
                "required_case_coverage": 1.0,
                "material_improvement": {
                    "total_tokens_relative": -0.1,
                    "wall_time_ms_relative": -0.1,
                    "cost_per_verified_success_micros_relative": -0.1
                },
                "material_regression": {
                    "total_tokens_relative": 0.1,
                    "wall_time_ms_relative": 0.1,
                    "cost_per_verified_success_micros_relative": 0.1
                }
            }
        });
        let policy = eval_comparison_policy_from_manifest(&manifest);

        for measure in [
            "total_tokens",
            "wall_time_ms",
            "cost_per_verified_success_micros",
        ] {
            let baseline = comparison_run("baseline", measure, 100.0);
            let improved = comparison_run("improved", measure, 80.0);
            let unchanged = comparison_run("unchanged", measure, 100.0);
            let regressed = comparison_run("regressed", measure, 120.0);

            assert_eq!(
                compare_eval_runs(Some(&baseline), Some(&improved), &policy).verdict,
                "improved",
                "{measure} must use the immutable suite threshold"
            );
            assert_eq!(
                compare_eval_runs(Some(&baseline), Some(&unchanged), &policy).verdict,
                "no_material_difference",
                "{measure} no-effect evidence must not claim improvement"
            );
            assert_eq!(
                compare_eval_runs(Some(&baseline), Some(&regressed), &policy).verdict,
                "regressed",
                "{measure} regression must remain blocked"
            );
        }
    }

    #[test]
    fn suites_without_comparison_policy_keep_default_behavior() {
        let policy = eval_comparison_policy_from_manifest(&json!({}));
        let default = EvalComparisonPolicy::default();
        assert_eq!(
            policy.required_case_coverage,
            default.required_case_coverage
        );
        assert_eq!(
            policy.freshness_max_age_hours,
            default.freshness_max_age_hours
        );
        assert_eq!(policy.min_samples, default.min_samples);
        assert_eq!(policy.material_improvement, default.material_improvement);
        assert_eq!(policy.material_regression, default.material_regression);
    }

    fn supplied_control_manifest() -> Value {
        json!({
            "schema_version": "eval.suite.v1",
            "suite_id": "suite",
            "suite_version": "1.0.0",
            "fixtures": [{
                "id": "fixture",
                "digest": "sha256:339966ca2904a4e86569a7f407874b6a421a910edea972a2bee1b949ea3c1112"
            }],
            "cases": [
                {
                    "case_id": "oracle",
                    "fixture_ids": ["fixture"],
                    "scorer_id": "scorer",
                    "scorer_version": "1.0.0",
                    "scorer_control": {"kind": "oracle", "expect": "pass"},
                    "measures": ["duration_ms"],
                    "sampling": {"repetitions": 2}
                },
                {
                    "case_id": "no-op",
                    "fixture_ids": ["fixture"],
                    "scorer_id": "scorer",
                    "scorer_version": "1.0.0",
                    "scorer_control": {"kind": "no_op", "expect": "fail"},
                    "measures": ["duration_ms"],
                    "sampling": {"repetitions": 2}
                }
            ]
        })
    }

    fn supplied_control_case(case_id: &str, status: &str, repetitions: &[i64]) -> Value {
        json!({
            "case": {
                "case_id": case_id,
                "scorer_id": "scorer",
                "scorer_version": "1.0.0",
                "fixture_digest": "sha256:339966ca2904a4e86569a7f407874b6a421a910edea972a2bee1b949ea3c1112",
                "status": status,
                "repetition_count": repetitions.len()
            },
            "samples": repetitions
                .iter()
                .map(|index| json!({
                    "repetition_index": index,
                    "seed": index,
                    "measure": "duration_ms",
                    "value": 10.0,
                    "unit": "ms",
                    "source": "process",
                    "metering_basis": "actual_trusted",
                    "basis_source": "process",
                    "basis_confidence": "verified",
                    "attempt": {
                        "terminal_status": status,
                        "countable": true,
                        "outcome": {"status": status}
                    }
                }))
                .collect::<Vec<_>>()
        })
    }

    fn verified_eval_route_observation() -> Value {
        let stage = |enforcement: &str, evidence: &str| {
            json!({
                "profile": "observed-profile",
                "client": "codex",
                "provider": {"value": "openai", "enforcement": enforcement, "evidence": evidence},
                "runtime": {"value": "codex-cli", "enforcement": enforcement, "evidence": evidence},
                "profile_config_digest": {"value": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "enforcement": enforcement, "evidence": evidence},
                "runner_harness_version": {"value": "release-eval-v1", "enforcement": enforcement, "evidence": evidence},
                "agent_type": {"value": "codex-worker", "enforcement": enforcement, "evidence": evidence},
                "model": {"value": "observed-model", "enforcement": enforcement, "evidence": evidence},
                "effort": {"value": "high", "enforcement": enforcement, "evidence": evidence},
                "context_fork": {"value": {"mode": "none"}, "enforcement": enforcement, "evidence": evidence}
            })
        };
        json!({
            "requested": stage("requested_only", "policy"),
            "resolved": stage("verified", "binding"),
            "effective": stage("verified", "host_report"),
            "transition": {"kind": "initial", "reason": "host reported effective route", "evidence": ["host_report"]},
            "policy": {"id": "route-policy", "version": "v1"},
            "binding": {"id": "route-binding", "version": "v1"},
            "metering": {
                "wall_time_seconds": {"value": 1, "confidence": "trusted"},
                "tool_calls": {"confidence": "unavailable"},
                "tokens": {"confidence": "unavailable"},
                "credits_micros": {"confidence": "unavailable"}
            }
        })
    }

    fn observed_attempt(observation: Value) -> Value {
        json!({
            "effective_client": "codex",
            "effective_provider": "openai",
            "effective_runtime": "codex-cli",
            "effective_model": "observed-model",
            "effective_effort": "high",
            "effective_profile_id": "observed-profile",
            "profile_config_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "runner_harness_version": "release-eval-v1",
            "route_observation": observation
        })
    }

    #[test]
    fn supplied_eval_route_provenance_uses_canonical_route_audit_contract() {
        let attempt = observed_attempt(verified_eval_route_observation());
        let parsed = eval_attempt_input(&json!({"attempt": attempt.clone()})).unwrap();
        assert!(parsed.route_observation.is_some());
        assert_eq!(
            eval_route_observation_validation(&attempt)["status"],
            "verified"
        );

        let mut requested_only = verified_eval_route_observation();
        requested_only["effective"]["model"]["enforcement"] = json!("requested_only");
        assert!(eval_attempt_input(&json!({"attempt": observed_attempt(requested_only)})).is_err());

        let mut policy_only = verified_eval_route_observation();
        policy_only["effective"]["model"]["evidence"] = json!("policy");
        assert!(eval_attempt_input(&json!({"attempt": observed_attempt(policy_only)})).is_err());

        let mut invalid_confidence = verified_eval_route_observation();
        invalid_confidence["metering"]["wall_time_seconds"]["confidence"] = json!("actual_trusted");
        assert!(
            eval_attempt_input(&json!({"attempt": observed_attempt(invalid_confidence)})).is_err()
        );

        let ad_hoc =
            json!({"source": "host_report", "effective_treatment": {"confidence": "verified"}});
        assert!(eval_attempt_input(&json!({"attempt": observed_attempt(ad_hoc)})).is_err());

        for field in [
            "provider",
            "runtime",
            "profile_config_digest",
            "runner_harness_version",
        ] {
            let mut incomplete = verified_eval_route_observation();
            incomplete["effective"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert_eq!(
                eval_route_observation_validation(&observed_attempt(incomplete))["status"],
                "invalid",
                "canonical effective {field} evidence is release-required"
            );
        }

        for field in [
            "effective_client",
            "effective_provider",
            "effective_runtime",
            "effective_model",
            "effective_effort",
            "effective_profile_id",
            "profile_config_digest",
            "runner_harness_version",
        ] {
            let mut fabricated = observed_attempt(verified_eval_route_observation());
            fabricated[field] = json!(format!("fabricated-{field}"));
            assert_eq!(
                eval_route_observation_validation(&fabricated)["status"],
                "invalid",
                "{field} must be bound to canonical route evidence"
            );
        }
    }

    #[test]
    fn supplied_control_evidence_rejects_zero_or_one_repetition() {
        for repetitions in [Vec::<i64>::new(), vec![0]] {
            let error = validate_supplied_evidence_cases(
                &json!({"cases": [supplied_control_case("oracle", "pass", &repetitions)]}),
                &supplied_control_manifest(),
            )
            .unwrap_err();

            assert!(error.to_string().contains("samples_below_minimum"));
        }
    }

    #[test]
    fn supplied_control_evidence_rejects_duplicate_or_status_mismatched_repetitions() {
        let duplicate = validate_supplied_evidence_cases(
            &json!({"cases": [supplied_control_case("oracle", "pass", &[0, 0])]}),
            &supplied_control_manifest(),
        )
        .unwrap_err();
        assert!(
            duplicate
                .to_string()
                .contains("scorer_control_nondeterministic")
        );

        let mut mismatched = supplied_control_case("no-op", "fail", &[0, 1]);
        mismatched["samples"][0]["attempt"]["terminal_status"] = json!("pass");
        let mismatch = validate_supplied_evidence_cases(
            &json!({"cases": [mismatched]}),
            &supplied_control_manifest(),
        )
        .unwrap_err();
        assert!(
            mismatch
                .to_string()
                .contains("scorer_control_nondeterministic")
        );
    }

    #[test]
    fn supplied_eval_samples_without_metering_do_not_default_to_trusted_zero() {
        let samples = eval_samples_input(Some(&vec![json!({
            "id": "evsample-unknown-cost",
            "attempt_id": "evattempt-unknown",
            "measure": "cost_micros",
            "unit": "micros",
            "source": "metering"
        })]))
        .unwrap();

        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].metering_basis, "unavailable");
        assert_eq!(samples[0].value, Value::Null);
        assert_eq!(samples[0].basis_confidence, "unavailable");
        assert_eq!(samples[0].attempt.terminal_status, "inconclusive");
    }

    #[test]
    fn supplied_eval_samples_with_value_require_metering_basis() {
        let error = eval_samples_input(Some(&vec![json!({
            "id": "evsample-duration",
            "attempt_id": "evattempt-duration",
            "measure": "duration_ms",
            "value": 80.0,
            "unit": "ms",
            "source": "process"
        })]))
        .unwrap_err();

        assert!(error.to_string().contains("missing metering_basis"));
    }

    #[test]
    fn supplied_eval_samples_without_metering_persist_as_unavailable() {
        let app = test_app();
        let manifest = json!({
            "schema_version": "eval.suite.v1",
            "suite_id": "suite",
            "suite_version": "1.0.0",
            "fixtures": [{
                "id": "fixture",
                "digest": "sha256:339966ca2904a4e86569a7f407874b6a421a910edea972a2bee1b949ea3c1112"
            }],
            "cases": [{
                "case_id": "case-a",
                "fixture_ids": ["fixture"],
                "scorer_id": "scorer",
                "scorer_version": "1.0.0",
                "measures": ["cost_micros"]
            }]
        });
        let digest = sha256_json_digest_without_top_level_digest(&manifest).unwrap();
        app.eval_suite_check_value(
            json!({
                "digest": digest,
                "normalized_manifest": manifest
            }),
            None,
        )
        .unwrap();

        let output = app
            .eval_run_value_from_input(json!({
                "id": "evrun-supplied-unavailable",
                "suite_digest": digest,
                "subject": {
                    "kind": "planr_cli",
                    "revision": "rev-a",
                    "argv": ["planr", "--version"]
                },
                "status": "success",
                "cases": [{
                    "case_id": "case-a",
                    "scorer_id": "scorer",
                    "scorer_version": "1.0.0",
                    "fixture_digest": "sha256:339966ca2904a4e86569a7f407874b6a421a910edea972a2bee1b949ea3c1112",
                    "status": "pass",
                    "repetition_count": 1,
                    "warmup_count": 0,
                    "samples": [{
                        "id": "evsample-unknown-cost",
                        "attempt_id": "evattempt-unknown",
                        "measure": "cost_micros",
                        "unit": "micros",
                        "source": "metering"
                    }]
                }]
            }))
            .unwrap();

        let sample = &output["run"]["cases"][0]["samples"][0];
        assert_eq!(sample["id"], "evsample-unknown-cost");
        assert_eq!(sample["metering_basis"], "unavailable");
        assert_eq!(sample["value"], Value::Null);
        assert_eq!(sample["basis_confidence"], "unavailable");
        assert_eq!(sample["estimate_provenance"], Value::Null);
    }

    #[test]
    fn runner_samples_project_route_treatment_and_metering_confidence() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("fixture.txt"),
            b"fixture:basic-project:v1\n",
        )
        .unwrap();
        let manifest = eval_runner_manifest_from_value(&json!({
            "schema_version": "eval.suite.v1",
            "suite_id": "suite",
            "fixtures": [{
                "id": "fixture",
                "path": "fixture.txt",
                "digest": "sha256:339966ca2904a4e86569a7f407874b6a421a910edea972a2bee1b949ea3c1112"
            }],
            "cases": [{
                "case_id": "case-a",
                "fixture_ids": ["fixture"],
                "scorer_id": "scorer",
                "scorer_version": "1.0.0",
                "subject": {"kind": "planr_cli", "argv": ["printf", "ok"]},
                "assertions": [{"kind": "exit_code", "expected": 0}],
                "measures": ["duration_ms"],
                "sampling": {"repetitions": 1, "warmups": 0, "seed": 10, "max_attempts": 4},
                "attempt_transitions": [
                    {"attempt_index": 1, "kind": "retry", "parent_attempt_index": 0},
                    {"attempt_index": 2, "kind": "fallback", "parent_attempt_index": 1},
                    {"attempt_index": 3, "kind": "escalation", "parent_attempt_index": 2}
                ],
                "timeout_ms": 1000,
                "output_limit_bytes": 1024
            }],
            "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
        }))
        .unwrap();
        let output = run_eval_manifest(root.path(), &manifest, EvalRunOptions::default()).unwrap();
        let case = &output.cases[0];

        let samples = runner_sample_inputs("evrun-a", case);
        let first_wall = samples
            .iter()
            .find(|sample| sample.measure == "wall_time_ms" && sample.attempt.attempt_index == 0)
            .unwrap();
        let retry_cost = samples
            .iter()
            .find(|sample| sample.measure == "cost_micros" && sample.attempt.attempt_index == 1)
            .unwrap();
        let fallback_wall = samples
            .iter()
            .find(|sample| sample.measure == "wall_time_ms" && sample.attempt.attempt_index == 2)
            .unwrap();
        let escalated_wall = samples
            .iter()
            .find(|sample| sample.measure == "wall_time_ms" && sample.attempt.attempt_index == 3)
            .unwrap();

        assert_eq!(
            first_wall.attempt.id.as_deref(),
            Some("evattempt-evrun-a-case-a-measured-0-10-0")
        );
        assert_eq!(first_wall.basis_confidence, "verified");
        assert_eq!(
            first_wall.attempt.effective_client.as_deref(),
            Some("planr")
        );
        assert_eq!(
            first_wall.attempt.effective_provider.as_deref(),
            Some("local_process")
        );
        assert!(first_wall.attempt.route_observation.is_some());
        assert_eq!(retry_cost.value, Value::Null);
        assert_eq!(retry_cost.metering_basis, "unavailable");
        assert_eq!(retry_cost.basis_confidence, "unavailable");
        assert_eq!(retry_cost.attempt.attempt_index, 1);
        assert_eq!(
            retry_cost.attempt.retry_of_attempt_id.as_deref(),
            first_wall.attempt.id.as_deref()
        );
        assert_eq!(retry_cost.attempt.fallback_of_attempt_id.as_deref(), None);
        assert_eq!(fallback_wall.attempt.attempt_index, 2);
        assert_eq!(
            fallback_wall.attempt.fallback_of_attempt_id.as_deref(),
            retry_cost.attempt.id.as_deref()
        );
        assert_eq!(
            escalated_wall.attempt.escalation_of_attempt_id.as_deref(),
            fallback_wall.attempt.id.as_deref()
        );
    }

    #[test]
    fn runner_attempt_lineage_persists_under_distinct_run_scoped_ids() {
        let app = test_app();
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("fixture.txt"),
            b"fixture:basic-project:v1\n",
        )
        .unwrap();
        let manifest = eval_runner_manifest_from_value(&json!({
            "schema_version": "eval.suite.v1",
            "suite_id": "suite",
            "fixtures": [{
                "id": "fixture",
                "path": "fixture.txt",
                "digest": "sha256:339966ca2904a4e86569a7f407874b6a421a910edea972a2bee1b949ea3c1112"
            }],
            "cases": [{
                "case_id": "case-a",
                "fixture_ids": ["fixture"],
                "scorer_id": "scorer",
                "scorer_version": "1.0.0",
                "subject": {
                    "kind": "planr_cli",
                    "argv": [
                        "python3",
                        "-c",
                        "import pathlib, sys; p=pathlib.Path('retry-counter.txt'); n=int(p.read_text() or '0') if p.exists() else 0; p.write_text(str(n+1)); print('ok'); sys.exit(1 if n == 0 else 0)"
                    ]
                },
                "assertions": [{"kind": "exit_code", "expected": 0}],
                "measures": ["duration_ms"],
                "sampling": {"repetitions": 1, "warmups": 0, "seed": 10, "max_attempts": 4},
                "attempt_transitions": [
                    {"attempt_index": 1, "kind": "retry", "parent_attempt_index": 0},
                    {"attempt_index": 2, "kind": "fallback", "parent_attempt_index": 1},
                    {"attempt_index": 3, "kind": "escalation", "parent_attempt_index": 2}
                ],
                "timeout_ms": 1000,
                "output_limit_bytes": 1024
            }],
            "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
        }))
        .unwrap();
        app.insert_eval_suite_snapshot(EvalSuiteSnapshotInput {
            digest: "sha256:suite".to_string(),
            suite_id: "suite".to_string(),
            suite_version: "1.0.0".to_string(),
            normalized_manifest: json!({"suite_id": "suite"}),
            fixture_digests: json!({"fixture": "sha256:339966ca2904a4e86569a7f407874b6a421a910edea972a2bee1b949ea3c1112"}),
            scorer_identities: json!(["scorer@1.0.0"]),
            case_order: json!(["case-a"]),
            baseline_selector: json!({}),
            comparison_policy_digest: "sha256:policy".to_string(),
            metadata: json!({}),
            source_path: None,
        })
        .unwrap();

        let retry_counter = root.path().join("retry-counter.txt");
        for run_id in ["evrun-lineage-a", "evrun-lineage-b"] {
            let _ = fs::remove_file(&retry_counter);
            app.start_eval_run(EvalRunInput {
                id: Some(run_id.to_string()),
                suite_digest: "sha256:suite".to_string(),
                subject: EvalSubjectInput {
                    kind: "planr_cli".to_string(),
                    revision: "rev-a".to_string(),
                    path: Some(".".to_string()),
                    argv: manifest.cases[0].subject.argv.clone().into(),
                    label: None,
                },
                runner_version: "eval-runner-v1".to_string(),
                planr_version: "1.5.2".to_string(),
                testbed_fingerprint: json!({"os": "test"}),
                source_state: json!({"commit": "abc"}),
                parent_run_id: None,
                resume_of: None,
                rescore_of: None,
                recompute_of: None,
            })
            .unwrap();
            let output =
                run_eval_manifest(root.path(), &manifest, EvalRunOptions::default()).unwrap();
            let case = &output.cases[0];
            app.record_eval_case_result(
                run_id,
                runner_case_input(&manifest.cases[0], case, &output.fixture_digests),
                runner_sample_inputs(run_id, case),
            )
            .unwrap();
            app.finish_eval_run(run_id, "success").unwrap();
        }

        let attempt_ids = app
            .conn
            .prepare("SELECT id FROM eval_attempts ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(attempt_ids.len(), 8);
        assert!(
            attempt_ids
                .iter()
                .any(|id| id.starts_with("evattempt-evrun-lineage-a-"))
        );
        assert!(
            attempt_ids
                .iter()
                .any(|id| id.starts_with("evattempt-evrun-lineage-b-"))
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT retry_of_attempt_id FROM eval_attempts WHERE run_id = 'evrun-lineage-a' AND attempt_index = 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "evattempt-evrun-lineage-a-case-a-measured-0-10-0"
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT retry_of_attempt_id FROM eval_attempts WHERE run_id = 'evrun-lineage-b' AND attempt_index = 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "evattempt-evrun-lineage-b-case-a-measured-0-10-0"
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT fallback_of_attempt_id FROM eval_attempts WHERE run_id = 'evrun-lineage-b' AND attempt_index = 2",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "evattempt-evrun-lineage-b-case-a-measured-0-10-1"
        );
        assert_eq!(
            app.conn
                .query_row(
                    "SELECT escalation_of_attempt_id FROM eval_attempts WHERE run_id = 'evrun-lineage-a' AND attempt_index = 3",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "evattempt-evrun-lineage-a-case-a-measured-0-10-2"
        );
    }

    #[test]
    fn runner_attempt_ids_include_warmup_phase_for_persistence() {
        let app = test_app();
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("fixture.txt"),
            b"fixture:basic-project:v1\n",
        )
        .unwrap();
        let manifest = eval_runner_manifest_from_value(&json!({
            "schema_version": "eval.suite.v1",
            "suite_id": "suite",
            "fixtures": [{
                "id": "fixture",
                "path": "fixture.txt",
                "digest": "sha256:339966ca2904a4e86569a7f407874b6a421a910edea972a2bee1b949ea3c1112"
            }],
            "cases": [{
                "case_id": "case-a",
                "fixture_ids": ["fixture"],
                "scorer_id": "scorer",
                "scorer_version": "1.0.0",
                "subject": {"kind": "planr_cli", "argv": ["printf", "ok"]},
                "assertions": [{"kind": "exit_code", "expected": 0}],
                "measures": ["duration_ms"],
                "sampling": {"repetitions": 1, "warmups": 1, "seed": 10, "max_attempts": 1},
                "timeout_ms": 1000,
                "output_limit_bytes": 1024
            }],
            "safety": {"allow_shell": false, "max_concurrency": 1, "allow_environment_capture": false}
        }))
        .unwrap();
        app.insert_eval_suite_snapshot(EvalSuiteSnapshotInput {
            digest: "sha256:suite-warmup".to_string(),
            suite_id: "suite".to_string(),
            suite_version: "1.0.0".to_string(),
            normalized_manifest: json!({"suite_id": "suite"}),
            fixture_digests: json!({"fixture": "sha256:339966ca2904a4e86569a7f407874b6a421a910edea972a2bee1b949ea3c1112"}),
            scorer_identities: json!(["scorer@1.0.0"]),
            case_order: json!(["case-a"]),
            baseline_selector: json!({}),
            comparison_policy_digest: "sha256:policy".to_string(),
            metadata: json!({}),
            source_path: None,
        })
        .unwrap();
        app.start_eval_run(EvalRunInput {
            id: Some("evrun-warmup".to_string()),
            suite_digest: "sha256:suite-warmup".to_string(),
            subject: EvalSubjectInput {
                kind: "planr_cli".to_string(),
                revision: "rev-a".to_string(),
                path: Some(".".to_string()),
                argv: manifest.cases[0].subject.argv.clone().into(),
                label: None,
            },
            runner_version: "eval-runner-v1".to_string(),
            planr_version: "1.5.2".to_string(),
            testbed_fingerprint: json!({"os": "test"}),
            source_state: json!({"commit": "abc"}),
            parent_run_id: None,
            resume_of: None,
            rescore_of: None,
            recompute_of: None,
        })
        .unwrap();
        let output = run_eval_manifest(root.path(), &manifest, EvalRunOptions::default()).unwrap();
        let case = &output.cases[0];
        app.record_eval_case_result(
            "evrun-warmup",
            runner_case_input(&manifest.cases[0], case, &output.fixture_digests),
            runner_sample_inputs("evrun-warmup", case),
        )
        .unwrap();
        app.finish_eval_run("evrun-warmup", "success").unwrap();

        let attempts = app
            .conn
            .prepare("SELECT id, warmup, repetition_index, seed, countable FROM eval_attempts ORDER BY warmup DESC")
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            attempts,
            vec![
                (
                    "evattempt-evrun-warmup-case-a-warmup-0-10-0".to_string(),
                    1,
                    0,
                    10,
                    0
                ),
                (
                    "evattempt-evrun-warmup-case-a-measured-0-10-0".to_string(),
                    0,
                    0,
                    10,
                    1
                ),
            ]
        );
    }
}
