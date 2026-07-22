use super::App;
use crate::agents::{REGISTRY_RELATIVE_PATH, registry_path};
use crate::cli::{ExportArgs, ImportArgs};
use anyhow::{Result, anyhow};
use rusqlite::params;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

mod eval;

impl App {
    pub(crate) fn export(&self, args: ExportArgs) -> Result<()> {
        let data = self.export_value(
            args.include_plans,
            args.include_logs,
            args.template_name.as_deref(),
            &args.tag,
        )?;
        fs::write(&args.out, serde_json::to_vec_pretty(&data)?)?;
        self.record_event(
            "export_written",
            None,
            json!({"out": args.out, "include_plans": args.include_plans, "include_logs": args.include_logs}),
        )?;
        self.emit(json!({"out": args.out}), "export written".to_string())
    }

    pub(crate) fn import(&self, args: ImportArgs) -> Result<()> {
        let data: Value = serde_json::from_slice(&fs::read(&args.file)?)?;
        let report = self.import_package_report(&data)?;
        if args.preview || !args.confirm {
            return self.emit(
                json!({"file": args.file, "mode": "preview", "report": report}),
                "import preview".to_string(),
            );
        }
        let imported = self.import_package_apply(&data)?;
        self.record_event(
            "import_completed",
            None,
            json!({"file": args.file, "mode": "package", "imported": imported}),
        )?;
        self.emit(
            json!({"file": args.file, "mode": "apply", "imported": imported}),
            "import applied".to_string(),
        )
    }

    fn import_package_report(&self, data: &Value) -> Result<Value> {
        let template = package_template(data)?;
        let map = required_object(data, "map")?;
        let items = required_array(map, "items", "map.items")?;
        let links = required_array(map, "links", "map.links")?;
        let contexts = required_array(data, "contexts", "contexts")?;
        let logs = nullable_array(data, "logs", "logs")?;
        let eval_suite_snapshots = optional_nullable_array(data, "eval_suite_snapshots")?;
        let eval_runs = optional_nullable_array(data, "eval_runs")?;
        let eval_comparisons = optional_nullable_array(data, "eval_comparisons")?;
        let eval_invalidations = optional_nullable_array(data, "eval_invalidations")?;
        let eval_evidence_refs = optional_nullable_array(data, "eval_evidence_refs")?;
        let artifacts = required_array(data, "review_artifacts", "review_artifacts")?;
        self.validate_packaged_eval_identities(data)?;
        let mut conflicts = Vec::new();
        for item in items {
            let id = required_str(item, "id", "map.items[].id")?;
            if self.get_item(id).is_ok() {
                conflicts.push(json!({"type": "item", "id": id}));
            }
        }
        Ok(json!({
            "template": template.clone(),
            "would_create": {
                "items": items.len().saturating_sub(conflicts.len()),
                "links": links.len(),
                "contexts": contexts.len(),
                "logs": logs.len(),
                "eval_suite_snapshots": eval_suite_snapshots.len(),
                "eval_runs": eval_runs.len(),
                "eval_case_results": eval::packaged_eval_case_count(eval_runs),
                "eval_samples": eval::packaged_eval_sample_count(eval_runs),
                "eval_comparisons": eval_comparisons.len(),
                "eval_invalidations": eval_invalidations.len(),
                "eval_evidence_refs": eval_evidence_refs.len(),
                "review_artifacts": artifacts.len(),
            },
            "would_skip": conflicts,
            "agent_registry": self.registry_import_plan(data),
            "requires_confirm": true,
        }))
    }

    /// What importing the packaged registry would do at this destination:
    /// `create` when no registry exists, `identical` when the local file
    /// already matches, `conflict` when it differs — a conflict is never
    /// resolved silently, the local file must be removed first. Null when
    /// the package carries no registry (older packages included).
    fn registry_import_plan(&self, data: &Value) -> Value {
        let Some(content) = packaged_registry(data) else {
            return Value::Null;
        };
        let action = match fs::read_to_string(registry_path(&self.root)) {
            Err(_) => "create",
            Ok(existing) if existing == content => "identical",
            Ok(_) => "conflict",
        };
        json!({
            "path": REGISTRY_RELATIVE_PATH,
            "action": action,
            "hint": (action == "conflict").then_some(
                "a different .planr/agents.toml already exists; it is never overwritten — remove it and re-import to accept the packaged registry"
            ),
        })
    }

    fn import_package_apply(&self, data: &Value) -> Result<Value> {
        let project = self.default_project()?;
        package_template(data)?;
        let map = required_object(data, "map")?;
        let items = required_array(map, "items", "map.items")?;
        let links = required_array(map, "links", "map.links")?;
        let contexts = required_array(data, "contexts", "contexts")?;
        let logs = nullable_array(data, "logs", "logs")?;
        let eval_suite_snapshots = optional_nullable_array(data, "eval_suite_snapshots")?;
        let eval_runs = optional_nullable_array(data, "eval_runs")?;
        let eval_comparisons = optional_nullable_array(data, "eval_comparisons")?;
        let eval_invalidations = optional_nullable_array(data, "eval_invalidations")?;
        let eval_evidence_refs = optional_nullable_array(data, "eval_evidence_refs")?;
        let artifacts = required_array(data, "review_artifacts", "review_artifacts")?;
        self.validate_packaged_eval_identities(data)?;

        self.conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;
        let result = (|| -> Result<Value> {
            let mut imported_items = 0usize;
            let mut imported_links = 0usize;
            let mut imported_contexts = 0usize;
            let mut imported_logs = 0usize;
            let mut imported_eval_suite_snapshots = 0usize;
            let mut imported_eval_runs = 0usize;
            let mut imported_eval_case_results = 0usize;
            let mut imported_eval_samples = 0usize;
            let mut imported_eval_comparisons = 0usize;
            let mut imported_eval_invalidations = 0usize;
            let mut imported_eval_evidence_refs = 0usize;
            let mut imported_review_artifacts = 0usize;
            for item in items {
                let changed = self.conn.execute(
                    "INSERT OR IGNORE INTO items(id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, datetime('now'), datetime('now'))",
                    params![
                        required_str(item, "id", "map.items[].id")?,
                        &project.id,
                        nullable_str(item, "parent_item_id", "map.items[].parent_item_id")?,
                        required_str(item, "title", "map.items[].title")?,
                        required_str(item, "description", "map.items[].description")?,
                        required_str(item, "status", "map.items[].status")?,
                        required_str(item, "work_type", "map.items[].work_type")?,
                        required_i64(item, "priority", "map.items[].priority")?,
                        nullable_str(item, "plan_path", "map.items[].plan_path")?,
                    ],
                )?;
                imported_items += changed;
            }
            for link in links {
                imported_links += self.conn.execute(
                    "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, ?3, 'all')",
                    params![
                        required_str(link, "from", "map.links[].from")?,
                        required_str(link, "to", "map.links[].to")?,
                        required_str(link, "kind", "map.links[].kind")?,
                    ],
                )?;
            }
            for context in contexts {
                imported_contexts += self.conn.execute(
                    "INSERT OR IGNORE INTO contexts(id, project_id, item_id, worker_id, kind, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', datetime('now'))",
                    params![
                        required_str(context, "id", "contexts[].id")?,
                        &project.id,
                        nullable_str(context, "item_id", "contexts[].item_id")?,
                        nullable_str(context, "worker_id", "contexts[].worker_id")?,
                        required_str(context, "kind", "contexts[].kind")?,
                        required_str(context, "content", "contexts[].content")?,
                    ],
                )?;
            }
            for log in logs {
                imported_logs += self.conn.execute(
                    "INSERT OR IGNORE INTO logs(id, project_id, item_id, kind, summary, files, commands, tests, review_findings, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
                    params![
                        required_str(log, "id", "logs[].id")?,
                        &project.id,
                        required_str(log, "item_id", "logs[].item_id")?,
                        required_str(log, "kind", "logs[].kind")?,
                        required_str(log, "summary", "logs[].summary")?,
                        serde_json::to_string(required_value(log, "files", "logs[].files")?)?,
                        serde_json::to_string(required_value(log, "commands", "logs[].commands")?)?,
                        serde_json::to_string(required_value(log, "tests", "logs[].tests")?)?,
                        serde_json::to_string(required_value(log, "review_findings", "logs[].review_findings")?)?,
                    ],
                )?;
            }
            imported_eval_suite_snapshots +=
                self.import_packaged_eval_suite_snapshots(eval_suite_snapshots, &project.id)?;
            for run in eval_runs {
                imported_eval_runs += self.conn.execute(
                    "INSERT OR IGNORE INTO eval_runs(id, project_id, suite_digest, subject_kind, subject_revision, subject_path, subject_argv, subject_label, runner_version, planr_version, status, created_at, started_at, completed_at, testbed_fingerprint, source_state, case_counts, parent_run_id, resume_of, rescore_of, recompute_of, invalidated_by, aggregate_summary) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
                    params![
                        required_str(run, "id", "eval_runs[].id")?,
                        &project.id,
                        required_str(run, "suite_digest", "eval_runs[].suite_digest")?,
                        required_str(run, "subject_kind", "eval_runs[].subject_kind")?,
                        required_str(run, "subject_revision", "eval_runs[].subject_revision")?,
                        nullable_str(run, "subject_path", "eval_runs[].subject_path")?,
                        serde_json::to_string(required_value(run, "subject_argv", "eval_runs[].subject_argv")?)?,
                        nullable_str(run, "subject_label", "eval_runs[].subject_label")?,
                        required_str(run, "runner_version", "eval_runs[].runner_version")?,
                        required_str(run, "planr_version", "eval_runs[].planr_version")?,
                        required_str(run, "status", "eval_runs[].status")?,
                        required_str(run, "created_at", "eval_runs[].created_at")?,
                        nullable_str(run, "started_at", "eval_runs[].started_at")?,
                        nullable_str(run, "completed_at", "eval_runs[].completed_at")?,
                        serde_json::to_string(required_value(run, "testbed_fingerprint", "eval_runs[].testbed_fingerprint")?)?,
                        serde_json::to_string(required_value(run, "source_state", "eval_runs[].source_state")?)?,
                        serde_json::to_string(required_value(run, "case_counts", "eval_runs[].case_counts")?)?,
                        nullable_str(run, "parent_run_id", "eval_runs[].parent_run_id")?,
                        nullable_str(run, "resume_of", "eval_runs[].resume_of")?,
                        nullable_str(run, "rescore_of", "eval_runs[].rescore_of")?,
                        nullable_str(run, "recompute_of", "eval_runs[].recompute_of")?,
                        nullable_str(run, "invalidated_by", "eval_runs[].invalidated_by")?,
                        serde_json::to_string(required_value(run, "aggregate_summary", "eval_runs[].aggregate_summary")?)?,
                    ],
                )?;
                let (case_results, samples) = self.import_packaged_eval_run_cases(run)?;
                imported_eval_case_results += case_results;
                imported_eval_samples += samples;
            }
            for comparison in eval_comparisons {
                imported_eval_comparisons += self.conn.execute(
                    "INSERT OR IGNORE INTO eval_comparisons(id, project_id, baseline_run_id, candidate_run_id, policy_digest, runner_version, verdict, reasons, gates, effect_estimates, uncertainty, protected_dimensions, recompute_of, rescore_of, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    params![
                        required_str(comparison, "id", "eval_comparisons[].id")?,
                        &project.id,
                        required_str(comparison, "baseline_run_id", "eval_comparisons[].baseline_run_id")?,
                        required_str(comparison, "candidate_run_id", "eval_comparisons[].candidate_run_id")?,
                        required_str(comparison, "policy_digest", "eval_comparisons[].policy_digest")?,
                        required_str(comparison, "runner_version", "eval_comparisons[].runner_version")?,
                        required_str(comparison, "verdict", "eval_comparisons[].verdict")?,
                        serde_json::to_string(required_value(comparison, "reasons", "eval_comparisons[].reasons")?)?,
                        serde_json::to_string(required_value(comparison, "gates", "eval_comparisons[].gates")?)?,
                        serde_json::to_string(required_value(comparison, "effect_estimates", "eval_comparisons[].effect_estimates")?)?,
                        serde_json::to_string(required_value(comparison, "uncertainty", "eval_comparisons[].uncertainty")?)?,
                        serde_json::to_string(required_value(comparison, "protected_dimensions", "eval_comparisons[].protected_dimensions")?)?,
                        nullable_str(comparison, "recompute_of", "eval_comparisons[].recompute_of")?,
                        nullable_str(comparison, "rescore_of", "eval_comparisons[].rescore_of")?,
                        required_str(comparison, "created_at", "eval_comparisons[].created_at")?,
                    ],
                )?;
            }
            for invalidation in eval_invalidations {
                imported_eval_invalidations += self.conn.execute(
                    "INSERT OR IGNORE INTO eval_invalidations(id, project_id, target_kind, target_id, reason, reason_codes, created_at, created_by, replacement_hint) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        required_str(invalidation, "id", "eval_invalidations[].id")?,
                        &project.id,
                        required_str(invalidation, "target_kind", "eval_invalidations[].target_kind")?,
                        required_str(invalidation, "target_id", "eval_invalidations[].target_id")?,
                        required_str(invalidation, "reason", "eval_invalidations[].reason")?,
                        serde_json::to_string(required_value(invalidation, "reason_codes", "eval_invalidations[].reason_codes")?)?,
                        required_str(invalidation, "created_at", "eval_invalidations[].created_at")?,
                        required_str(invalidation, "created_by", "eval_invalidations[].created_by")?,
                        nullable_str(invalidation, "replacement_hint", "eval_invalidations[].replacement_hint")?,
                    ],
                )?;
            }
            for package in artifacts {
                imported_review_artifacts +=
                    self.import_review_artifact_package(&project.id, package)?;
            }
            for evidence_ref in eval_evidence_refs {
                imported_eval_evidence_refs += self.conn.execute(
                    "INSERT OR IGNORE INTO eval_evidence_refs(id, project_id, target_kind, target_id, planr_attachment_kind, planr_attachment_id, item_id, closure_authority, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        required_str(evidence_ref, "id", "eval_evidence_refs[].id")?,
                        &project.id,
                        required_str(evidence_ref, "target_kind", "eval_evidence_refs[].target_kind")?,
                        required_str(evidence_ref, "target_id", "eval_evidence_refs[].target_id")?,
                        required_str(evidence_ref, "planr_attachment_kind", "eval_evidence_refs[].planr_attachment_kind")?,
                        required_str(evidence_ref, "planr_attachment_id", "eval_evidence_refs[].planr_attachment_id")?,
                        required_str(evidence_ref, "item_id", "eval_evidence_refs[].item_id")?,
                        if required_value(evidence_ref, "closure_authority", "eval_evidence_refs[].closure_authority")?.as_bool().unwrap_or(false) { 1 } else { 0 },
                        required_str(evidence_ref, "created_at", "eval_evidence_refs[].created_at")?,
                    ],
                )?;
            }
            let agent_registry = self.registry_import_plan(data);
            if agent_registry["action"] == "create" {
                let path = registry_path(&self.root);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(
                    &path,
                    packaged_registry(data).expect("action=create implies packaged content"),
                )?;
            }
            self.promote_ready()?;
            Ok(json!({
                "items": imported_items,
                "links": imported_links,
                "contexts": imported_contexts,
                "logs": imported_logs,
                "eval_suite_snapshots": imported_eval_suite_snapshots,
                "eval_runs": imported_eval_runs,
                "eval_case_results": imported_eval_case_results,
                "eval_samples": imported_eval_samples,
                "eval_comparisons": imported_eval_comparisons,
                "eval_invalidations": imported_eval_invalidations,
                "eval_evidence_refs": imported_eval_evidence_refs,
                "review_artifacts": imported_review_artifacts,
                "agent_registry": agent_registry,
            }))
        })();
        match result {
            Ok(imported) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(imported)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    fn import_review_artifact_package(&self, project_id: &str, package: &Value) -> Result<usize> {
        let artifact = required_object(package, "artifact")?;
        let name = required_str(artifact, "name", "review_artifacts[].artifact.name")?;
        let safe_name = Path::new(name)
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow!("invalid Planr package: review artifact name has no file name")
            })?;
        let path = self.root.join(".planr/reviews").join(safe_name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = required_str(package, "content", "review_artifacts[].content")?;
        fs::write(&path, content)?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO artifacts(id, project_id, item_id, name, kind, path, content, mime_type, size_bytes, metadata, created_at) VALUES (?1, ?2, ?3, ?4, 'review', ?5, NULL, 'text/markdown', ?6, ?7, datetime('now'))",
                params![
                    required_str(artifact, "id", "review_artifacts[].artifact.id")?,
                    project_id,
                    nullable_str(artifact, "item_id", "review_artifacts[].artifact.item_id")?,
                    safe_name,
                    path.to_string_lossy(),
                    content.len() as i64,
                    json!({"imported": true}).to_string(),
                ],
            )
            .map_err(Into::into)
    }
}

/// The registry snapshot carried by a package, if any. Optional so
/// pre-registry packages import unchanged.
fn packaged_registry(data: &Value) -> Option<&str> {
    data.get("agent_registry")?.get("content")?.as_str()
}

fn package_template(data: &Value) -> Result<&Value> {
    let template = required_value(data, "planr_template", "planr_template")?;
    let schema_version = required_i64(template, "schema_version", "planr_template.schema_version")?;
    if schema_version != 1 {
        return Err(anyhow!(
            "invalid Planr package: planr_template.schema_version must be 1"
        ));
    }
    required_object(template, "requirements")?;
    Ok(template)
}

fn required_object<'a>(value: &'a Value, field: &str) -> Result<&'a Value> {
    let object = required_value(value, field, field)?;
    object
        .as_object()
        .ok_or_else(|| anyhow!("invalid Planr package: {field} must be an object"))?;
    Ok(object)
}

fn required_array<'a>(value: &'a Value, field: &str, label: &str) -> Result<&'a Vec<Value>> {
    required_value(value, field, label)?
        .as_array()
        .ok_or_else(|| anyhow!("invalid Planr package: {label} must be an array"))
}

fn nullable_array<'a>(value: &'a Value, field: &str, label: &str) -> Result<&'a [Value]> {
    match required_value(value, field, label)? {
        Value::Array(values) => Ok(values.as_slice()),
        Value::Null => Ok(&[]),
        _ => Err(anyhow!(
            "invalid Planr package: {label} must be an array or null"
        )),
    }
}

fn optional_nullable_array<'a>(value: &'a Value, field: &str) -> Result<&'a [Value]> {
    match value.get(field) {
        Some(Value::Array(values)) => Ok(values.as_slice()),
        Some(Value::Null) | None => Ok(&[]),
        Some(_) => Err(anyhow!(
            "invalid Planr package: {field} must be an array or null"
        )),
    }
}

fn required_value<'a>(value: &'a Value, field: &str, label: &str) -> Result<&'a Value> {
    value
        .get(field)
        .ok_or_else(|| anyhow!("invalid Planr package: missing {label}"))
}

fn required_str<'a>(value: &'a Value, field: &str, label: &str) -> Result<&'a str> {
    required_value(value, field, label)?
        .as_str()
        .ok_or_else(|| anyhow!("invalid Planr package: {label} must be a string"))
}

fn nullable_str<'a>(value: &'a Value, field: &str, label: &str) -> Result<Option<&'a str>> {
    match required_value(value, field, label)? {
        Value::String(text) => Ok(Some(text)),
        Value::Null => Ok(None),
        _ => Err(anyhow!(
            "invalid Planr package: {label} must be a string or null"
        )),
    }
}

fn required_i64(value: &Value, field: &str, label: &str) -> Result<i64> {
    required_value(value, field, label)?
        .as_i64()
        .ok_or_else(|| anyhow!("invalid Planr package: {label} must be an integer"))
}
