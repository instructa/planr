use super::{App, ReviewAnnotationInput};
use crate::model::Item;
use crate::storage::row_to_item;
use crate::util::{now_string, short_id, worker_id};
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

pub(crate) struct ReviewArtifactInput<'a> {
    pub(crate) review_id: &'a str,
    pub(crate) verdict: Option<&'a str>,
    pub(crate) findings: &'a [String],
    pub(crate) created: &'a [Item],
    pub(crate) out: Option<PathBuf>,
    pub(crate) reviewer: Option<&'a str>,
    pub(crate) review_mode: Option<&'a str>,
}

impl<'a> ReviewArtifactInput<'a> {
    /// Bare artifact render (no close in flight): only the review id is known.
    pub(crate) fn bare(review_id: &'a str) -> Self {
        Self {
            review_id,
            verdict: None,
            findings: &[],
            created: &[],
            out: None,
            reviewer: None,
            review_mode: None,
        }
    }
}

impl App {
    pub(crate) fn add_review_annotation(&self, input: ReviewAnnotationInput<'_>) -> Result<Value> {
        self.get_item(input.item_id)?;
        let severity = match input.severity {
            "info" | "warning" | "blocking" => input.severity,
            other => bail!("unsupported review annotation severity: {other}"),
        };
        let id = short_id("ctx");
        let tags = json!(["review", "annotation", severity, input.source]).to_string();
        let mut content = format!("[{severity}] {}", input.message);
        if let Some(file) = input.file {
            content.push_str(&format!(" ({file}"));
            if let Some(line) = input.line {
                content.push_str(&format!(":{line}"));
            }
            content.push(')');
        }
        self.conn.execute(
            "INSERT INTO contexts(id, project_id, item_id, worker_id, kind, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, 'review_annotation', ?5, ?6, datetime('now'))",
            params![id, self.default_project()?.id, input.item_id, worker_id(), content, tags],
        )?;
        self.index_search("context", &id, "review_annotation", &content, None)?;
        self.record_event(
            "review_annotation_added",
            Some(input.item_id),
            json!({
                "context_id": id.clone(),
                "severity": severity,
                "author": input.author,
                "file": input.file,
                "line": input.line,
                "source": input.source
            }),
        )?;
        Ok(json!({
            "id": id,
            "item_id": input.item_id,
            "kind": "review_annotation",
            "message": input.message,
            "severity": severity,
            "author": input.author,
            "file": input.file,
            "line": input.line,
            "content": content
        }))
    }

    pub(crate) fn ingest_review_feedback(
        &self,
        item_id: &str,
        feedback: Value,
        source: &str,
    ) -> Result<Value> {
        self.get_item(item_id)?;
        let reviewer = feedback
            .get("reviewer")
            .or_else(|| feedback.get("author"))
            .and_then(Value::as_str);
        let verdict = feedback.get("verdict").and_then(Value::as_str);
        let findings = feedback
            .get("findings")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut annotations = Vec::new();
        if let Some(values) = feedback.get("annotations").and_then(Value::as_array) {
            for value in values {
                let message = value
                    .get("message")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("review annotation is missing message"))?;
                let severity = value
                    .get("severity")
                    .and_then(Value::as_str)
                    .unwrap_or("info");
                let annotation = self.add_review_annotation(ReviewAnnotationInput {
                    item_id,
                    message,
                    severity,
                    author: value.get("author").and_then(Value::as_str).or(reviewer),
                    file: value.get("file").and_then(Value::as_str),
                    line: value.get("line").and_then(Value::as_u64),
                    source,
                })?;
                annotations.push(annotation);
            }
        }
        let mut log = Value::Null;
        if verdict.is_some() || !findings.is_empty() {
            let id = short_id("log");
            let summary = format!(
                "review feedback{}",
                verdict
                    .map(|value| format!(": {value}"))
                    .unwrap_or_default()
            );
            self.conn.execute(
                "INSERT INTO logs(id, project_id, item_id, kind, summary, review_findings, created_at) VALUES (?1, ?2, ?3, 'review_feedback', ?4, ?5, datetime('now'))",
                params![id, self.default_project()?.id, item_id, summary, serde_json::to_string(&findings)?],
            )?;
            self.index_search("log", &id, &summary, &findings.join("\n"), None)?;
            log = self.get_log(&id)?;
        }
        self.record_event(
            "review_feedback_ingested",
            Some(item_id),
            json!({
                "source": source,
                "reviewer": reviewer,
                "verdict": verdict,
                "findings": findings.len(),
                "annotations": annotations.len(),
                "auto_closed": false,
                "auto_approved": false
            }),
        )?;
        Ok(json!({
            "item_id": item_id,
            "reviewer": reviewer,
            "verdict": verdict,
            "findings": findings,
            "annotations": annotations,
            "log": log,
            "auto_closed": false,
            "auto_approved": false,
            "next": [
                "planr review request <item-id>",
                "planr review close <review-item-id> --verdict complete|not-complete|unclear"
            ]
        }))
    }

    pub(crate) fn close_review_item(
        &self,
        review_id: &str,
        verdict: &str,
        findings: Vec<String>,
        source: &str,
        reviewer: Option<&str>,
        close_target: bool,
    ) -> Result<Value> {
        let review = self.get_item(review_id)?;
        if review.work_type != "review" {
            bail!("invalid_transition: item is not a review: {review_id}");
        }
        // Closing twice would duplicate review logs and the auto-completion
        // log on the target, polluting handoff evidence for downstream work.
        if matches!(
            review.status.as_str(),
            "closed" | "closed_partial" | "cancelled"
        ) {
            bail!("already_closed: review {review_id} is already settled; a second close would duplicate evidence logs");
        }
        let verdict = match verdict {
            "complete" | "not-complete" | "unclear" => verdict,
            other => bail!("unsupported review verdict: {other}"),
        };
        // Validate the --close-target preconditions before any mutation so a
        // rejected target close never leaves the review half-settled.
        let target_to_close = if close_target {
            if verdict != "complete" {
                bail!(
                    "--close-target requires --verdict complete; findings create fix work instead"
                );
            }
            let target = self.review_target(review_id)?.ok_or_else(|| {
                anyhow!("review {review_id} has no `reviews` link to a target item")
            })?;
            let completion_logs: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM logs WHERE item_id = ?1 AND kind = 'completion'",
                params![target.id],
                |row| row.get(0),
            )?;
            if completion_logs == 0 {
                bail!(
                    "cannot close target {}: no completion log; the worker must log evidence first (planr done / planr log add)",
                    target.id
                );
            }
            Some(target)
        } else {
            None
        };
        let reviewer = reviewer
            .map(ToOwned::to_owned)
            .unwrap_or_else(crate::util::worker_id);
        // Maker/checker split is derived from recorded identity, not from a
        // ceremony note: the target's lease holder is the maker.
        let maker = self
            .review_target(review_id)?
            .and_then(|target| target.worker_id);
        let review_mode = match maker.as_deref() {
            Some(maker) if maker == reviewer => "single_agent",
            Some(_) => "independent",
            None => "unattributed",
        };
        let summary =
            format!("review verdict: {verdict} (reviewer: {reviewer}, mode: {review_mode})");
        let log_id = short_id("log");
        self.conn.execute(
            "INSERT INTO logs(id, project_id, item_id, kind, summary, review_findings, created_at) VALUES (?1, ?2, ?3, 'review', ?4, ?5, datetime('now'))",
            params![
                log_id,
                self.default_project()?.id,
                review.id,
                summary,
                serde_json::to_string(&findings)?
            ],
        )?;
        self.conn.execute("UPDATE items SET status = 'closed', completed_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1", params![review.id])?;
        let mut created = Vec::new();
        if verdict != "complete" {
            let fix = self.create_item(
                None,
                &format!("Fix findings for {}", review.title),
                &findings.join("\n"),
                "fix",
                review.plan_path.as_deref(),
            )?;
            self.add_link(&review.id, &fix.id, "blocks")?;
            let next_review = self.create_item(
                None,
                &format!("Follow-up review for {}", review.title),
                "Review after fixes.",
                "review",
                review.plan_path.as_deref(),
            )?;
            self.add_link(&fix.id, &next_review.id, "blocks")?;
            // The follow-up review gates the same target, so the chain keeps
            // working with `review close --close-target` and the target stays
            // visibly `in_review` until the chain settles.
            if let Some(target) = self.review_target(&review.id)? {
                self.add_link(&next_review.id, &target.id, "reviews")?;
            }
            created.push(fix);
            created.push(next_review);
        }
        // Close the target before rendering the artifact so the artifact
        // snapshot shows the final target status instead of `in_review`.
        let closed_target = if let Some(target) = &target_to_close {
            self.close_item_core(
                &target.id,
                &format!("closed by review {} (verdict complete)", review.id),
                true,
            )?;
            Some(self.get_item(&target.id)?)
        } else {
            None
        };
        let artifact = self.write_review_artifact(ReviewArtifactInput {
            review_id,
            verdict: Some(verdict),
            findings: &findings,
            created: &created,
            out: None,
            reviewer: Some(&reviewer),
            review_mode: Some(review_mode),
        })?;
        self.promote_ready()?;
        self.record_event(
            "review_closed",
            Some(&review.id),
            json!({
                "verdict": verdict,
                "reviewer": reviewer,
                "review_mode": review_mode,
                "created": created.len(),
                "source": source,
                "artifact_id": artifact["id"]
            }),
        )?;
        let mut result = json!({
            "closed": review.id,
            "verdict": verdict,
            "reviewer": reviewer,
            "review_mode": review_mode,
            "log_id": log_id,
            "created": created,
            "artifact": artifact
        });
        if let Some(target) = closed_target {
            result["closed_target"] = json!(target);
        }
        Ok(result)
    }

    pub(crate) fn write_review_artifact(&self, input: ReviewArtifactInput<'_>) -> Result<Value> {
        let ReviewArtifactInput {
            review_id,
            verdict,
            findings,
            created,
            out,
            reviewer,
            review_mode,
        } = input;
        let review = self.get_item(review_id)?;
        let target = self.review_target(review_id)?;
        let evidence = target
            .as_ref()
            .map(|item| self.review_evidence_value(&item.id))
            .transpose()?;
        let review_logs = self.list_logs(Some(review_id))?;
        let mut annotations = self.list_contexts(Some(review_id))?;
        if let Some(target) = &target {
            annotations.extend(self.list_contexts(Some(&target.id))?);
        }
        annotations
            .retain(|entry| entry.get("kind").and_then(Value::as_str) == Some("review_annotation"));
        let path = out.unwrap_or_else(|| {
            self.root
                .join(".planr")
                .join("reviews")
                .join(format!("{review_id}.review.md"))
        });
        let path = if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut body = String::new();
        body.push_str("# Review Artifact\n\n");
        body.push_str(&format!("- Generated: {}\n", now_string()));
        body.push_str(&format!(
            "- Review item: {} ({})\n",
            review.id, review.status
        ));
        body.push_str(&format!("- Review title: {}\n", review.title));
        if let Some(target) = &target {
            body.push_str(&format!(
                "- Target item: {} ({})\n",
                target.id, target.status
            ));
            body.push_str(&format!("- Target title: {}\n", target.title));
        }
        if let Some(verdict) = verdict {
            body.push_str(&format!("- Verdict: {verdict}\n"));
        }
        if let Some(reviewer) = reviewer {
            body.push_str(&format!("- Reviewer: {reviewer}\n"));
        }
        if let Some(review_mode) = review_mode {
            body.push_str(&format!("- Review mode: {review_mode}\n"));
        }
        body.push_str("\n## Findings\n\n");
        if findings.is_empty() {
            body.push_str("- None recorded\n");
        } else {
            for finding in findings {
                body.push_str(&format!("- {finding}\n"));
            }
        }
        body.push_str("\n## Annotations\n\n");
        if annotations.is_empty() {
            body.push_str("- None recorded\n");
        } else {
            for annotation in &annotations {
                body.push_str(&format!(
                    "- {}: {}\n",
                    annotation["id"].as_str().unwrap_or("context"),
                    annotation["content"].as_str().unwrap_or("")
                ));
            }
        }
        body.push_str("\n## Review Logs\n\n");
        if review_logs.is_empty() {
            body.push_str("- None recorded\n");
        } else {
            for log in &review_logs {
                body.push_str(&format!(
                    "- {}: {}\n",
                    log["id"].as_str().unwrap_or("log"),
                    log["summary"].as_str().unwrap_or("")
                ));
            }
        }
        body.push_str("\n## Git And PR Evidence\n\n");
        if let Some(evidence) = &evidence {
            body.push_str(&format!(
                "- Source content included: {}\n",
                evidence["dirty_worktree_safety"]["source_content_included"]
                    .as_bool()
                    .unwrap_or(false)
            ));
            body.push_str(&format!(
                "- Agent-owned files: {}\n",
                compact_json(&evidence["provenance"]["agent_owned_files"])
            ));
            body.push_str(&format!(
                "- Scoped changed files: {}\n",
                compact_json(&evidence["git"]["scoped_files"])
            ));
            body.push_str(&format!(
                "- Unrelated dirty files: {}\n",
                compact_json(&evidence["git"]["unrelated_dirty_files"])
            ));
            body.push_str(&format!(
                "- PR URLs: {}\n",
                compact_json(&evidence["provenance"]["pr_urls"])
            ));
        } else {
            body.push_str("- No review target linked\n");
        }
        body.push_str("\n## Follow-up Work\n\n");
        if created.is_empty() {
            body.push_str("- None created\n");
        } else {
            for item in created {
                body.push_str(&format!(
                    "- {} [{}] {}\n",
                    item.id, item.work_type, item.title
                ));
            }
        }
        body.push_str("\n## Privacy\n\n");
        body.push_str("- Source file content included: false\n");
        body.push_str("- Prompt or response content included: false\n");
        fs::write(&path, body.as_bytes())?;
        let size = fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
        let artifact_id = short_id("art");
        self.conn.execute(
            "INSERT INTO artifacts(id, project_id, item_id, name, kind, path, mime_type, size_bytes, metadata, created_at) VALUES (?1, ?2, ?3, ?4, 'review', ?5, 'text/markdown', ?6, ?7, datetime('now'))",
            params![
                artifact_id,
                self.default_project()?.id,
                review_id,
                path.file_name().and_then(|s| s.to_str()).unwrap_or("review.md"),
                path.to_string_lossy(),
                size,
                json!({"review_item_id": review_id, "target_item_id": target.as_ref().map(|item| item.id.clone()), "verdict": verdict, "reviewer": reviewer}).to_string()
            ],
        )?;
        self.record_event(
            "review_artifact_written",
            Some(review_id),
            json!({"artifact_id": artifact_id.clone(), "path": path}),
        )?;
        self.get_artifact(&artifact_id)
    }

    pub(crate) fn review_target(&self, review_id: &str) -> Result<Option<Item>> {
        self.conn
            .query_row(
                "SELECT target.id, target.project_id, target.parent_item_id, target.title, target.description, target.status, target.work_type, target.priority, target.worker_id, target.plan_path
                 FROM links link JOIN items target ON target.id = link.to_item
                 WHERE link.from_item = ?1 AND link.kind = 'reviews'
                 ORDER BY link.id LIMIT 1",
                params![review_id],
                row_to_item,
            )
            .optional()
            .map_err(Into::into)
    }
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}
