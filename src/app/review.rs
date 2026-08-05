use super::{App, ReviewAnnotationInput};
use crate::util::{short_id, worker_id};
use anyhow::{Result, anyhow, bail};
use rusqlite::params;
use serde_json::{Value, json};

impl App {
    pub(crate) fn ensure_plan_final_product_review_value(&self, plan_id: &str) -> Result<Value> {
        self.ensure_final_product_review_gate_value(plan_id)
    }

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
                "settle the outcome with structured escalation when a durable gate is required",
                "planr review close <review-gate-id> --verdict complete|not-complete|unclear"
            ]
        }))
    }

    pub(crate) fn complete_review_gate_surface_value(
        &self,
        review_id: &str,
        verdict: &str,
        findings: Vec<String>,
        _source: &str,
        reviewer: Option<&str>,
    ) -> Result<Value> {
        let verdict = match verdict {
            "complete" => super::repository::execution_run::ReviewVerdict::Accepted,
            "not-complete" => super::repository::execution_run::ReviewVerdict::ChangesRequested,
            "unclear" => super::repository::execution_run::ReviewVerdict::Blocked,
            other => bail!("unsupported review verdict: {other}"),
        };
        self.complete_review_gate_value(review_id, verdict, &findings, reviewer)
    }
}
