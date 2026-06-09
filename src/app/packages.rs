use super::App;
use crate::cli::{ExportArgs, ImportArgs};
use anyhow::Result;
use rusqlite::params;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

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
        if args.file.is_dir() {
            let imported = self.import_planr_dir(&args.file)?;
            self.record_event(
                "import_completed",
                None,
                json!({"path": args.file, "mode": "directory", "imported": imported}),
            )?;
            return self.emit(
                json!({"path": args.file, "imported": imported}),
                "directory imported".to_string(),
            );
        }
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
            json!({"file": args.file, "mode": "json", "imported": imported}),
        )?;
        self.emit(
            json!({"file": args.file, "mode": "apply", "imported": imported}),
            "import applied".to_string(),
        )
    }

    fn import_package_report(&self, data: &Value) -> Result<Value> {
        let items = data["map"]["items"].as_array().map(Vec::len).unwrap_or(0);
        let links = data["map"]["links"].as_array().map(Vec::len).unwrap_or(0);
        let contexts = data["contexts"].as_array().map(Vec::len).unwrap_or(0);
        let logs = data["logs"].as_array().map(Vec::len).unwrap_or(0);
        let artifacts = data["review_artifacts"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0);
        let mut conflicts = Vec::new();
        if let Some(values) = data["map"]["items"].as_array() {
            for item in values {
                if let Some(id) = item.get("id").and_then(Value::as_str) {
                    if self.get_item(id).is_ok() {
                        conflicts.push(json!({"type": "item", "id": id}));
                    }
                }
            }
        }
        Ok(json!({
            "template": data.get("planr_template").cloned().unwrap_or(Value::Null),
            "would_create": {
                "items": items.saturating_sub(conflicts.len()),
                "links": links,
                "contexts": contexts,
                "logs": logs,
                "review_artifacts": artifacts,
            },
            "would_skip": conflicts,
            "requires_confirm": true,
        }))
    }

    fn import_package_apply(&self, data: &Value) -> Result<Value> {
        let project = self.default_project()?;
        let mut imported_items = 0usize;
        let mut imported_links = 0usize;
        let mut imported_contexts = 0usize;
        let mut imported_logs = 0usize;
        let mut imported_review_artifacts = 0usize;
        if let Some(items) = data["map"]["items"].as_array() {
            for item in items {
                let id = item["id"].as_str().unwrap_or("item");
                let changed = self.conn.execute(
                    "INSERT OR IGNORE INTO items(id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, datetime('now'), datetime('now'))",
                    params![
                        id,
                        &project.id,
                        item.get("parent_item_id").and_then(Value::as_str),
                        item.get("title").and_then(Value::as_str).unwrap_or("Imported item"),
                        item.get("description").and_then(Value::as_str).unwrap_or("Imported item"),
                        item.get("status").and_then(Value::as_str).unwrap_or("pending"),
                        item.get("work_type").and_then(Value::as_str).unwrap_or("generic"),
                        item.get("priority").and_then(Value::as_i64).unwrap_or(0),
                        item.get("plan_path").and_then(Value::as_str),
                    ],
                )?;
                imported_items += changed;
            }
        }
        if let Some(links) = data["map"]["links"].as_array() {
            for link in links {
                imported_links += self.conn.execute(
                    "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, ?3, 'all')",
                    params![
                        link.get("from").and_then(Value::as_str).unwrap_or(""),
                        link.get("to").and_then(Value::as_str).unwrap_or(""),
                        link.get("kind").and_then(Value::as_str).unwrap_or("blocks"),
                    ],
                )?;
            }
        }
        if let Some(contexts) = data["contexts"].as_array() {
            for context in contexts {
                imported_contexts += self.conn.execute(
                    "INSERT OR IGNORE INTO contexts(id, project_id, item_id, worker_id, kind, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', datetime('now'))",
                    params![
                        context.get("id").and_then(Value::as_str).unwrap_or("ctx"),
                        &project.id,
                        context.get("item_id").and_then(Value::as_str),
                        context.get("worker_id").and_then(Value::as_str),
                        context.get("kind").and_then(Value::as_str).unwrap_or("imported"),
                        context.get("content").and_then(Value::as_str).unwrap_or(""),
                    ],
                )?;
            }
        }
        if let Some(logs) = data["logs"].as_array() {
            for log in logs {
                imported_logs += self.conn.execute(
                    "INSERT OR IGNORE INTO logs(id, project_id, item_id, kind, summary, files, commands, tests, review_findings, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
                    params![
                        log.get("id").and_then(Value::as_str).unwrap_or("log"),
                        &project.id,
                        log.get("item_id").and_then(Value::as_str).unwrap_or(""),
                        log.get("kind").and_then(Value::as_str).unwrap_or("imported"),
                        log.get("summary").and_then(Value::as_str).unwrap_or("Imported log"),
                        serde_json::to_string(log.get("files").unwrap_or(&json!([])))?,
                        serde_json::to_string(log.get("commands").unwrap_or(&json!([])))?,
                        serde_json::to_string(log.get("tests").unwrap_or(&json!([])))?,
                        serde_json::to_string(log.get("review_findings").unwrap_or(&json!([])))?,
                    ],
                )?;
            }
        }
        if let Some(artifacts) = data["review_artifacts"].as_array() {
            for package in artifacts {
                imported_review_artifacts +=
                    self.import_review_artifact_package(&project.id, package)?;
            }
        }
        self.promote_ready()?;
        Ok(json!({
            "items": imported_items,
            "links": imported_links,
            "contexts": imported_contexts,
            "logs": imported_logs,
            "review_artifacts": imported_review_artifacts,
        }))
    }

    fn import_review_artifact_package(&self, project_id: &str, package: &Value) -> Result<usize> {
        let artifact = &package["artifact"];
        let name = artifact
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("imported.review.md");
        let safe_name = Path::new(name)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("imported.review.md");
        let path = self.root.join(".planr/reviews").join(safe_name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(content) = package.get("content").and_then(Value::as_str) {
            fs::write(&path, content)?;
        }
        self.conn
            .execute(
                "INSERT OR IGNORE INTO artifacts(id, project_id, item_id, name, kind, path, content, mime_type, size_bytes, metadata, created_at) VALUES (?1, ?2, ?3, ?4, 'review', ?5, NULL, 'text/markdown', ?6, ?7, datetime('now'))",
                params![
                    artifact.get("id").and_then(Value::as_str).unwrap_or("art"),
                    project_id,
                    artifact.get("item_id").and_then(Value::as_str),
                    safe_name,
                    path.to_string_lossy(),
                    fs::metadata(&path).map(|meta| meta.len() as i64).unwrap_or(0),
                    json!({"imported": true}).to_string(),
                ],
            )
            .map_err(Into::into)
    }
}
