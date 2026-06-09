use super::App;
use crate::storage::row_to_context;
use crate::util::{
    collect_rows, detect_client, now_string, query_json, quote_fts, short_id, worker_id,
};
use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use slug::slugify;
use std::fs;
use std::path::Path;

impl App {
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

    pub(crate) fn get_context(&self, id: &str) -> Result<Value> {
        self.conn.query_row("SELECT id, item_id, kind, content, worker_id, created_at FROM contexts WHERE id = ?1", params![id], row_to_context).optional()?.ok_or_else(|| anyhow!("context not found: {id}"))
    }

    pub(crate) fn list_contexts(&self, item: Option<&str>) -> Result<Vec<Value>> {
        let sql = if item.is_some() {
            "SELECT id, item_id, kind, content, worker_id, created_at FROM contexts WHERE item_id = ?1 ORDER BY created_at DESC"
        } else {
            "SELECT id, item_id, kind, content, worker_id, created_at FROM contexts ORDER BY created_at DESC"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = if let Some(item) = item {
            stmt.query_map(params![item], row_to_context)?
        } else {
            stmt.query_map([], row_to_context)?
        };
        collect_rows(rows)
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
        let patterns = ["sk-", "ghp_", "BEGIN PRIVATE KEY", "AKIA"];
        for log in self.list_logs(None)? {
            let summary = log.get("summary").and_then(Value::as_str).unwrap_or("");
            if patterns.iter().any(|p| summary.contains(p)) {
                findings.push(json!({"type": "log", "id": log.get("id"), "field": "summary"}));
            }
        }
        for ctx in self.list_contexts(None)? {
            let content = ctx.get("content").and_then(Value::as_str).unwrap_or("");
            if patterns.iter().any(|p| content.contains(p)) {
                findings.push(json!({"type": "context", "id": ctx.get("id"), "field": "content"}));
            }
        }
        Ok(findings)
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
                "compatibility": {
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
            "map": self.map_value()?,
            "logs": if include_logs { json!(self.list_logs(None)?) } else { Value::Null },
            "contexts": self.list_contexts(None)?,
            "artifacts": self.list_artifacts(None)?,
            "review_artifacts": json!(self.export_review_artifacts()?),
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
    ) -> Result<String> {
        let id = short_id("run");
        self.conn.execute(
            "INSERT INTO runs(id, project_id, item_id, worker_id, client, command, cwd, status, started_at, ended_at, metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'), datetime('now'), ?9)",
            params![
                id,
                self.default_project()?.id,
                item_id,
                worker_id(),
                detect_client(),
                commands.join(" && "),
                self.root.to_string_lossy(),
                status,
                json!({"recorded_from": "log"}).to_string(),
            ],
        )?;
        Ok(id)
    }

    pub(crate) fn index_search(
        &self,
        source_type: &str,
        source_id: &str,
        title: &str,
        body: &str,
        path: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO search_index(source_type, source_id, title, body, path) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![source_type, source_id, title, body, path],
        )?;
        Ok(())
    }

    pub(crate) fn search_results(&self, query: &str) -> Result<Vec<Value>> {
        let mut results = Vec::new();
        let fts = quote_fts(query);
        let mut stmt = self.conn.prepare(
            "SELECT source_type, source_id, title, body, path FROM search_index WHERE search_index MATCH ?1 ORDER BY rank LIMIT 30",
        )?;
        let rows = stmt.query_map(params![fts], |row| {
            Ok(json!({
                "type": row.get::<_, String>(0)?,
                "id": row.get::<_, String>(1)?,
                "title": row.get::<_, String>(2)?,
                "text": row.get::<_, String>(3)?,
                "path": row.get::<_, Option<String>>(4)?,
            }))
        })?;
        for row in rows {
            results.push(row?);
        }
        if results.is_empty() {
            let like = format!("%{}%", query);
            query_json(&self.conn, "SELECT 'item', id, title, description FROM items WHERE title LIKE ?1 OR description LIKE ?1 LIMIT 20", params![like.clone()], &mut results)?;
            query_json(&self.conn, "SELECT 'plan', id, title, path FROM plans WHERE title LIKE ?1 OR path LIKE ?1 LIMIT 20", params![like.clone()], &mut results)?;
            query_json(
                &self.conn,
                "SELECT 'log', id, summary, item_id FROM logs WHERE summary LIKE ?1 LIMIT 20",
                params![like.clone()],
                &mut results,
            )?;
            query_json(
                &self.conn,
                "SELECT 'context', id, kind, content FROM contexts WHERE content LIKE ?1 LIMIT 20",
                params![like],
                &mut results,
            )?;
        }
        Ok(results)
    }

    pub(crate) fn import_planr_dir(&self, dir: &Path) -> Result<Value> {
        let project = self.default_project()?;
        let base = if dir.join(".planr").exists() {
            dir.join(".planr")
        } else {
            dir.to_path_buf()
        };
        let mut imported_plans = 0usize;
        let mut imported_items = 0usize;
        let mut imported_reviews = 0usize;
        let product_root = base.join("plans/product");
        if product_root.exists() {
            for entry in fs::read_dir(product_root)? {
                let path = entry?.path();
                if path.is_dir() {
                    let title = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("product-plan")
                        .replace('-', " ");
                    let slug = slugify(&title);
                    self.upsert_plan(
                        &project.id,
                        "product",
                        &path,
                        &title,
                        &slug,
                        json!({"imported": true}),
                    )?;
                    imported_plans += 1;
                }
            }
        }
        let build_root = base.join("plans/build");
        if build_root.exists() {
            for entry in fs::read_dir(build_root)? {
                let path = entry?.path();
                if path.extension().and_then(|s| s.to_str()) == Some("md") {
                    let title = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("build-plan")
                        .replace('-', " ");
                    let slug = slugify(&title);
                    self.upsert_plan(
                        &project.id,
                        "build",
                        &path,
                        &title,
                        &slug,
                        json!({"imported": true}),
                    )?;
                    imported_plans += 1;
                }
            }
        }
        let status = base.join("status/current.json");
        if status.exists() {
            let data: Value = serde_json::from_slice(&fs::read(status)?)?;
            if let Some(scopes) = data.get("scopes").and_then(Value::as_array) {
                for scope in scopes {
                    let title = scope
                        .get("title")
                        .or_else(|| scope.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or("Imported scope");
                    let description = serde_json::to_string_pretty(scope)?;
                    self.create_item(None, title, &description, "generic", None)?;
                    imported_items += 1;
                }
            }
        }
        let review_root = base.join("reviews");
        if review_root.exists() {
            for entry in fs::read_dir(review_root)? {
                let path = entry?.path();
                if path.extension().and_then(|s| s.to_str()) == Some("md") {
                    self.conn.execute(
                        "INSERT INTO artifacts(id, project_id, name, kind, path, mime_type, size_bytes, created_at) VALUES (?1, ?2, ?3, 'review', ?4, 'text/markdown', ?5, datetime('now'))",
                        params![
                            short_id("art"),
                            project.id,
                            path.file_name().and_then(|s| s.to_str()).unwrap_or("review.md"),
                            path.to_string_lossy(),
                            fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0),
                        ],
                    )?;
                    imported_reviews += 1;
                }
            }
        }
        self.promote_ready()?;
        Ok(json!({"plans": imported_plans, "items": imported_items, "reviews": imported_reviews}))
    }
}
