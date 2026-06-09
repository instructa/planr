use super::App;
use crate::cli::{ExportArgs, ImportArgs};
use anyhow::{anyhow, Result};
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
        let artifacts = required_array(data, "review_artifacts", "review_artifacts")?;
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
                "review_artifacts": artifacts.len(),
            },
            "would_skip": conflicts,
            "requires_confirm": true,
        }))
    }

    fn import_package_apply(&self, data: &Value) -> Result<Value> {
        let project = self.default_project()?;
        package_template(data)?;
        let map = required_object(data, "map")?;
        let items = required_array(map, "items", "map.items")?;
        let links = required_array(map, "links", "map.links")?;
        let contexts = required_array(data, "contexts", "contexts")?;
        let logs = nullable_array(data, "logs", "logs")?;
        let artifacts = required_array(data, "review_artifacts", "review_artifacts")?;
        let mut imported_items = 0usize;
        let mut imported_links = 0usize;
        let mut imported_contexts = 0usize;
        let mut imported_logs = 0usize;
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
        for package in artifacts {
            imported_review_artifacts +=
                self.import_review_artifact_package(&project.id, package)?;
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
