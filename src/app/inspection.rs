use super::App;
use crate::agents::{REGISTRY_RELATIVE_PATH, registry_path};
use crate::planpack::{
    BUILD_PLAN_REQUIRED_SECTIONS, PRODUCT_PLAN_REQUIRED_SECTIONS, parse_plan_metadata,
    unfilled_required_sections,
};
use crate::route_audit::RouteObservation;
use crate::secrets::redact_secrets;
use crate::util::{collect_rows, detect_client, now_string, short_id, worker_id};
use anyhow::Result;
use rusqlite::params;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

impl App {
    /// Shared plan-check logic for CLI and MCP: path, frontmatter, and
    /// required-section content. Structure alone is not enough; the
    /// load-bearing sections must have content before a plan checks out.
    pub(crate) fn plan_check_value(&self, plan_id: &str) -> Result<Value> {
        let plan = self.get_plan(plan_id)?;
        let path = std::path::PathBuf::from(&plan.path);
        let mut warnings = Vec::new();
        let warning = |file: &str, section: Option<&str>, message: String, fix: String| json!({"file": file, "section": section, "message": message, "fix": fix});
        if !path.exists() {
            warnings.push(warning(
                &plan.path,
                None,
                "plan path missing".to_string(),
                format!(
                    "restore the plan file at {} or recreate it with `planr plan create`",
                    plan.path
                ),
            ));
        } else {
            self.rehash_plan(&plan.id)?;
            let (frontmatter, parse_status) = parse_plan_metadata(&path);
            if parse_status != "ok" {
                let detail = frontmatter["error"]
                    .as_str()
                    .unwrap_or("invalid frontmatter");
                let frontmatter_file = if path.is_dir() {
                    path.join("README.md").display().to_string()
                } else {
                    plan.path.clone()
                };
                warnings.push(warning(
                    &frontmatter_file,
                    None,
                    format!("frontmatter parse error: {detail}"),
                    format!("fix the YAML frontmatter in {frontmatter_file}, then re-run `planr plan check {plan_id}`"),
                ));
            }
            let (section_file, required) = if path.is_dir() {
                (path.join("PRODUCT_SPEC.md"), PRODUCT_PLAN_REQUIRED_SECTIONS)
            } else {
                (path.clone(), BUILD_PLAN_REQUIRED_SECTIONS)
            };
            let section_path = section_file.display().to_string();
            match fs::read_to_string(&section_file) {
                Ok(text) => {
                    for (section, state) in unfilled_required_sections(&text, required) {
                        warnings.push(warning(
                            &section_path,
                            Some(&section),
                            format!("required section `## {section}` is {state}"),
                            format!("edit {section_path} and fill `## {section}` with content, then re-run `planr plan check {plan_id}`"),
                        ));
                    }
                }
                Err(_) => warnings.push(warning(
                    &section_path,
                    None,
                    format!(
                        "missing plan file: {}",
                        section_file
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| section_path.clone())
                    ),
                    format!("create {section_path} with the required sections, then re-run `planr plan check {plan_id}`"),
                )),
            }
        }
        // An unexpanded scaffold task list maps to a single coarse item and
        // forces a breakdown guess later; catch it before `map build` does.
        if let Some(message) =
            crate::planpack::scaffold_placeholder_state(&path)?.filter(|_| path.exists())
        {
            let task_file = if path.is_dir() {
                path.join("TASKS.md").display().to_string()
            } else {
                plan.path.clone()
            };
            warnings.push(warning(
                &task_file,
                Some("task list"),
                format!("{message}; `map build` would create a single coarse item"),
                format!("edit {task_file} and replace the placeholder with one `### TASK-00n: <slice>` heading (or `- [ ]` line) per verifiable slice — typically 4-8, in execution order — then re-run `planr plan check {plan_id}`"),
            ));
        }
        let plan = self.get_plan(plan_id)?;
        let ok = warnings.is_empty();
        Ok(json!({"plan": plan, "ok": ok, "warnings": warnings}))
    }

    pub(crate) fn debug_bundle(&self, item: Option<&str>) -> Result<Value> {
        let events = self.list_events(item, 50)?;
        let artifacts = self.list_artifacts(item)?;
        let logs = self.list_logs(item)?;
        let artifact_index = artifacts
            .iter()
            .map(|artifact| {
                json!({
                    "id": artifact["id"],
                    "item_id": artifact["item_id"],
                    "name": artifact["name"],
                    "kind": artifact["kind"],
                    "path": artifact["path"],
                    "mime_type": artifact["mime_type"],
                    "size_bytes": artifact["size_bytes"],
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "mode": "preview",
            "project": self.default_project()?,
            "item": item.map(|id| self.get_item(id)).transpose()?,
            "counts": {"events": events.len(), "artifacts": artifacts.len(), "logs": logs.len()},
            "events": events,
            "artifacts": artifact_index,
            "logs": logs,
            "privacy": {
                "inline_artifact_content_included": false,
                "prompt_or_response_content_included": false,
                "source_file_content_included": false
            }
        }))
    }

    pub(crate) fn links_for(&self, item_id: &str) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare("SELECT from_item, to_item, kind FROM links WHERE from_item = ?1 OR to_item = ?1 ORDER BY id")?;
        let rows = stmt.query_map(params![item_id], |row| {
            Ok(json!({"from": row.get::<_, String>(0)?, "to": row.get::<_, String>(1)?, "kind": row.get::<_, String>(2)?}))
        })?;
        collect_rows(rows)
    }

    pub(crate) fn secret_findings(&self) -> Result<Vec<Value>> {
        let mut findings = Vec::new();
        let mut scan = |kind: &str, id: &Value, field: &str, text: &str| {
            if redact_secrets(text).is_some() {
                findings.push(json!({"type": kind, "id": id, "field": field}));
            }
        };
        for log in self.list_logs(None)? {
            let id = log.get("id").cloned().unwrap_or(Value::Null);
            for field in ["summary", "files", "commands", "tests"] {
                let text = match log.get(field) {
                    Some(Value::String(text)) => text.clone(),
                    Some(value) if !value.is_null() => value.to_string(),
                    _ => continue,
                };
                scan("log", &id, field, &text);
            }
        }
        for ctx in self.list_contexts(None)? {
            let id = ctx.get("id").cloned().unwrap_or(Value::Null);
            let content = ctx.get("content").and_then(Value::as_str).unwrap_or("");
            scan("context", &id, "content", content);
        }
        for artifact in self.list_artifacts(None)? {
            let id = artifact.get("id").cloned().unwrap_or(Value::Null);
            if let Some(content) = artifact.get("content").and_then(Value::as_str) {
                scan("artifact", &id, "content", content);
            }
        }
        Ok(findings)
    }

    /// Rewrite flagged secret-like values in place. Returns the number of
    /// rows whose stored values were redacted.
    pub(crate) fn apply_scrub(&self) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut scrubbed = 0usize;
        let mut scrubbed_rows: Vec<(String, String)> = Vec::new();

        {
            let mut stmt =
                tx.prepare("SELECT id, summary, files, commands, tests FROM logs ORDER BY id")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, summary, files, commands, tests) in rows {
                let new_summary = redact_secrets(&summary);
                let new_files = files.as_deref().and_then(redact_secrets);
                let new_commands = commands.as_deref().and_then(redact_secrets);
                let new_tests = tests.as_deref().and_then(redact_secrets);
                if new_summary.is_none()
                    && new_files.is_none()
                    && new_commands.is_none()
                    && new_tests.is_none()
                {
                    continue;
                }
                let summary = new_summary.unwrap_or(summary);
                tx.execute(
                    "UPDATE logs SET summary = ?2, files = COALESCE(?3, files), commands = COALESCE(?4, commands), tests = COALESCE(?5, tests) WHERE id = ?1",
                    params![id, summary, new_files, new_commands, new_tests],
                )?;
                tx.execute(
                    "UPDATE search_index SET title = ?2, body = ?2 WHERE source_type = 'log' AND source_id = ?1",
                    params![id, summary],
                )?;
                scrubbed_rows.push(("log".to_string(), id));
                scrubbed += 1;
            }
        }

        {
            let mut stmt = tx.prepare("SELECT id, content FROM contexts ORDER BY created_at")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, content) in rows {
                let Some(redacted) = redact_secrets(&content) else {
                    continue;
                };
                tx.execute(
                    "UPDATE contexts SET content = ?2 WHERE id = ?1",
                    params![id, redacted],
                )?;
                tx.execute(
                    "UPDATE search_index SET body = ?2 WHERE source_type = 'context' AND source_id = ?1",
                    params![id, redacted],
                )?;
                scrubbed_rows.push(("context".to_string(), id));
                scrubbed += 1;
            }
        }

        {
            let mut stmt = tx.prepare(
                "SELECT id, content FROM artifacts WHERE content IS NOT NULL ORDER BY created_at",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, content) in rows {
                let Some(redacted) = redact_secrets(&content) else {
                    continue;
                };
                tx.execute(
                    "UPDATE artifacts SET content = ?2, size_bytes = ?3 WHERE id = ?1",
                    params![id, redacted, redacted.len() as i64],
                )?;
                scrubbed_rows.push(("artifact".to_string(), id));
                scrubbed += 1;
            }
        }

        for (kind, id) in &scrubbed_rows {
            self.record_event(
                "secret_scrubbed",
                None,
                json!({"source_type": kind, "source_id": id}),
            )?;
        }
        tx.commit()?;
        Ok(scrubbed)
    }

    pub(crate) fn export_value(
        &self,
        include_plans: bool,
        include_logs: bool,
        template_name: Option<&str>,
        tags: &[String],
    ) -> Result<Value> {
        let project = self.default_project()?;
        Ok(json!({
            "planr_template": {
                "schema_version": 1,
                "planr_version": env!("CARGO_PKG_VERSION"),
                "created_at": now_string(),
                "name": template_name.unwrap_or("Planr export"),
                "source_project": project.name,
                "tags": tags,
                "requirements": {
                    "min_planr_version": "1.0.0",
                    "requires_confirmed_import": true,
                    "source_content_included": false
                },
                "encrypted_bundle_strategy": {
                    "implemented": false,
                    "local_first_command": "age or gpg encrypt the exported JSON after review",
                    "hosted_share_required": false
                }
            },
            "projects": self.list_projects()?,
            "plans": if include_plans { json!(self.list_plans(None)?) } else { Value::Null },
            "plan_files": if include_plans { json!(self.export_plan_files()?) } else { Value::Null },
            "map": self.map_value(None)?,
            "logs": if include_logs { json!(self.list_logs(None)?) } else { Value::Null },
            "eval_suite_snapshots": if include_logs { json!(self.eval_suite_snapshot_values()?) } else { Value::Null },
            "eval_runs": if include_logs { json!(self.eval_run_record_values()?) } else { Value::Null },
            "eval_comparisons": if include_logs { json!(self.eval_comparison_values()?) } else { Value::Null },
            "eval_invalidations": if include_logs { json!(self.eval_invalidation_values()?) } else { Value::Null },
            "eval_evidence_refs": if include_logs { json!(self.eval_evidence_ref_values()?) } else { Value::Null },
            "contexts": self.list_contexts(None)?,
            "artifacts": self.list_artifacts(None)?,
            "review_artifacts": json!(self.export_review_artifacts()?),
            // Raw file snapshot: routing declarations travel with the
            // package; a malformed registry exports as-is and `agents
            // check` diagnoses it on the other side.
            "agent_registry": fs::read_to_string(registry_path(&self.root)).ok().map_or(
                Value::Null,
                |content| json!({"path": REGISTRY_RELATIVE_PATH, "content": content}),
            ),
            "events": self.list_events(None, 500)?,
        }))
    }

    fn export_plan_files(&self) -> Result<Vec<Value>> {
        let mut files = Vec::new();
        for plan in self.list_plans(None)? {
            let path = Path::new(&plan.path);
            if path.is_file() {
                files.push(json!({
                    "plan_id": plan.id,
                    "stage": plan.stage,
                    "title": plan.title,
                    "path": plan.path,
                    "content": fs::read_to_string(path).unwrap_or_default(),
                }));
            } else if path.is_dir() {
                for entry in fs::read_dir(path)? {
                    let entry = entry?;
                    if entry.path().is_file() {
                        files.push(json!({
                            "plan_id": plan.id,
                            "stage": plan.stage,
                            "title": plan.title,
                            "path": entry.path().to_string_lossy(),
                            "content": fs::read_to_string(entry.path()).unwrap_or_default(),
                        }));
                    }
                }
            }
        }
        Ok(files)
    }

    fn export_review_artifacts(&self) -> Result<Vec<Value>> {
        Ok(self
            .list_artifacts(None)?
            .into_iter()
            .filter(|artifact| artifact.get("kind").and_then(Value::as_str) == Some("review"))
            .map(|artifact| {
                let path = artifact.get("path").and_then(Value::as_str);
                json!({
                    "artifact": artifact,
                    "content": path.and_then(|path| fs::read_to_string(path).ok()),
                })
            })
            .collect())
    }

    pub(crate) fn record_run(
        &self,
        item_id: &str,
        commands: &[String],
        status: &str,
        profile: Option<&str>,
        route_observation: Option<&RouteObservation>,
    ) -> Result<String> {
        let id = short_id("run");
        let mut metadata = json!({"recorded_from": "log"});
        if let Some(observation) = route_observation {
            metadata["route_observation"] = json!(observation);
        }
        self.conn.execute(
            "INSERT INTO runs(id, project_id, item_id, worker_id, client, profile, observed_client, command, cwd, status, started_at, ended_at, metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'), datetime('now'), ?11)",
            params![
                id,
                self.default_project()?.id,
                item_id,
                worker_id(),
                detect_client(),
                profile,
                crate::util::observed_client(),
                commands.join(" && "),
                self.root.to_string_lossy(),
                status,
                metadata.to_string(),
            ],
        )?;
        Ok(id)
    }
}
