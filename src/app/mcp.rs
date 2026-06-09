use super::{recovery::ItemRecoveryInput, App, ReviewAnnotationInput};
use crate::integrations::{mcp_json, mcp_resources, mcp_tools};
use crate::planpack::{build_plan_body, product_plan_files};
use crate::util::{
    append_line, item_id, now_string, required_arg, short_id, worker_id, write_if_missing,
};
use anyhow::{anyhow, bail, Result};
use rusqlite::params;
use serde_json::{json, Value};
use slug::slugify;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

impl App {
    pub(crate) fn mcp(&self) -> Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        for line in stdin.lock().lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let request: Value = serde_json::from_str(&line).unwrap_or_else(|_| json!({}));
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let method = request.get("method").and_then(Value::as_str).unwrap_or("");
            let result = match method {
                "initialize" => {
                    json!({"protocolVersion": "2025-03-26", "serverInfo": {"name": "planr", "version": env!("CARGO_PKG_VERSION")}, "capabilities": {"tools": {}, "resources": {}, "prompts": {}}})
                }
                "tools/list" => json!({"tools": mcp_tools()}),
                "resources/list" => json!({"resources": mcp_resources()}),
                "resources/read" => {
                    self.mcp_resource_read(request.get("params").cloned().unwrap_or(Value::Null))?
                }
                "prompts/list" => {
                    json!({"prompts": [{"name": "planr-plan"}, {"name": "planr-work"}, {"name": "planr-review"}, {"name": "planr-map"}, {"name": "planr-summary"}]})
                }
                "prompts/get" => {
                    self.mcp_prompt_get(request.get("params").cloned().unwrap_or(Value::Null))
                }
                "tools/call" => {
                    self.mcp_tool_call(request.get("params").cloned().unwrap_or(Value::Null))?
                }
                _ => json!({"ok": true}),
            };
            writeln!(
                stdout,
                "{}",
                json!({"jsonrpc": "2.0", "id": id, "result": result})
            )?;
            stdout.flush()?;
        }
        Ok(())
    }

    pub(crate) fn mcp_tool_call(&self, params: Value) -> Result<Value> {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        match name {
            "planr_project_show" => Ok(mcp_json(self.default_project()?)),
            "planr_map_show" => Ok(mcp_json(self.map_value()?)),
            "planr_map_status" => Ok(mcp_json(self.map_status_value()?)),
            "planr_map_preview" => {
                let item_id = required_arg(&args, "close")?;
                Ok(mcp_json(self.preview_close_value(item_id)?))
            }
            "planr_map_unlocks" => {
                let item_id = required_arg(&args, "item_id")?;
                Ok(mcp_json(
                    json!({"item_id": item_id, "would_unlock": self.would_unlock_items(item_id)?}),
                ))
            }
            "planr_map_lookahead" => {
                let from = args.get("from").and_then(Value::as_str);
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
                Ok(mcp_json(self.lookahead_value(from, limit)?))
            }
            "planr_plan_create" => {
                let title = required_arg(&args, "title")?;
                let platform = args
                    .get("platform")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let project = self.default_project()?;
                let slug = slugify(title);
                let dir = self.root.join(".planr/plans/product").join(&slug);
                fs::create_dir_all(&dir)?;
                for (name, body) in product_plan_files(
                    title,
                    platform.as_deref(),
                    args.get("ai").and_then(Value::as_bool).unwrap_or(false),
                    args.get("backend")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ) {
                    write_if_missing(&dir.join(name), &body, false)?;
                }
                let plan = self.upsert_plan(
                    &project.id,
                    "product",
                    &dir,
                    title,
                    &slug,
                    json!({"created_from": "mcp", "platform": platform}),
                )?;
                Ok(mcp_json(
                    json!({"plan": plan, "next": "planr plan split <plan-id> --slice \"first build slice\""}),
                ))
            }
            "planr_plan_refine" => {
                let id = required_arg(&args, "id")?;
                let note = args
                    .get("note")
                    .and_then(Value::as_str)
                    .unwrap_or("Refined through MCP.");
                let plan = self.get_plan(id)?;
                let path = PathBuf::from(&plan.path);
                let target = if path.is_dir() {
                    path.join("README.md")
                } else {
                    path
                };
                append_line(
                    &target,
                    &format!("\n## Refinement {}\n\n{}\n", now_string(), note),
                )?;
                self.rehash_plan(id)?;
                Ok(mcp_json(
                    json!({"plan": self.get_plan(id)?, "next": "planr plan check <plan-id>"}),
                ))
            }
            "planr_plan_split" => {
                let id = required_arg(&args, "id")?;
                let slice = required_arg(&args, "slice")?;
                let source = self.get_plan(id)?;
                let project = self.default_project()?;
                let title = format!("{} - {}", source.title, slice);
                let slug = slugify(&title);
                let path = self
                    .root
                    .join(".planr/plans/build")
                    .join(format!("{slug}.plan.md"));
                write_if_missing(&path, &build_plan_body(&title, &source.id, slice), false)?;
                let plan = self.upsert_plan(
                    &project.id,
                    "build",
                    &path,
                    &title,
                    &slug,
                    json!({"source_plan": source.id, "slice": slice}),
                )?;
                Ok(mcp_json(
                    json!({"plan": plan, "next": "planr map build --from <plan-id>"}),
                ))
            }
            "planr_plan_check" => {
                let plan = self.get_plan(required_arg(&args, "id")?)?;
                Ok(mcp_json(
                    json!({"plan": plan, "ok": Path::new(&plan.path).exists()}),
                ))
            }
            "planr_plan_link" => {
                let source_id = required_arg(&args, "source_id")?;
                let item_id = required_arg(&args, "item_id")?;
                let relationship = args
                    .get("relationship")
                    .and_then(Value::as_str)
                    .unwrap_or("references");
                self.conn.execute(
                    "INSERT INTO source_links(source_type, source_id, item_id, section_id, relationship) VALUES ('plan', ?1, ?2, ?3, ?4)",
                    params![source_id, item_id, args.get("section_id").and_then(Value::as_str), relationship],
                )?;
                Ok(mcp_json(
                    json!({"linked": true, "source_id": source_id, "item_id": item_id, "relationship": relationship}),
                ))
            }
            "planr_map_build" => {
                let plan = self.get_plan(required_arg(&args, "from")?)?;
                let created = self.seed_items_from_plan(&plan)?;
                self.promote_ready()?;
                Ok(mcp_json(json!({"created": created, "next": "planr pick"})))
            }
            "planr_item_create" => {
                let item = self.create_item(
                    None,
                    required_arg(&args, "title")?,
                    required_arg(&args, "description")?,
                    args.get("work_type")
                        .and_then(Value::as_str)
                        .unwrap_or("generic"),
                    None,
                )?;
                if let Some(after) = args.get("after").and_then(Value::as_str) {
                    self.add_link(after, &item.id, "blocks")?;
                }
                if args.get("timeout_seconds").is_some()
                    || args.get("max_retries").is_some()
                    || args.get("retry_delay_ms").is_some()
                    || args.get("retry_backoff").is_some()
                    || args.get("pre").is_some()
                    || args.get("post").is_some()
                {
                    self.configure_item_recovery(
                        &item.id,
                        ItemRecoveryInput {
                            timeout_seconds: args.get("timeout_seconds").and_then(Value::as_i64),
                            max_retries: args.get("max_retries").and_then(Value::as_i64),
                            retry_backoff: args.get("retry_backoff").and_then(Value::as_str),
                            retry_delay_ms: args.get("retry_delay_ms").and_then(Value::as_i64),
                            pre_condition: args.get("pre").and_then(Value::as_str),
                            post_condition: args.get("post").and_then(Value::as_str),
                        },
                    )?;
                }
                self.promote_ready()?;
                Ok(mcp_json(json!({"item": self.get_item(&item.id)?})))
            }
            "planr_item_breakdown" => {
                let id = required_arg(&args, "id")?;
                let into = required_arg(&args, "into")?;
                let parent = self.get_item(id)?;
                let mut created = Vec::new();
                for title in into.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    created.push(self.create_item(
                        Some(id),
                        title,
                        &format!("Sub-item for {}", parent.title),
                        "generic",
                        parent.plan_path.as_deref(),
                    )?);
                }
                self.promote_ready()?;
                Ok(mcp_json(json!({"items": created})))
            }
            "planr_item_insert" => {
                let title = required_arg(&args, "title")?.to_string();
                let description = required_arg(&args, "description")?.to_string();
                let after = required_arg(&args, "after")?.to_string();
                let before = args
                    .get("before")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let confirm = args
                    .get("confirm")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if !confirm {
                    let after_item = self.get_item(&after)?;
                    return Ok(mcp_json(json!({
                        "mode": "preview",
                        "action": "insert",
                        "would_create": {"title": title, "description": description, "after": after_item.id, "before": before}
                    })));
                }
                let project = self.default_project()?;
                let id = item_id(&title);
                let tx = self.conn.unchecked_transaction()?;
                tx.execute(
                    "INSERT INTO items(id, project_id, parent_item_id, title, description, status, work_type, priority, created_at, updated_at) VALUES (?1, ?2, NULL, ?3, ?4, 'pending', 'generic', 0, datetime('now'), datetime('now'))",
                    params![id, project.id, title, description],
                )?;
                if let Some(before_id) = before.as_deref() {
                    tx.execute(
                        "DELETE FROM links WHERE from_item = ?1 AND to_item = ?2 AND kind = 'blocks'",
                        params![after, before_id],
                    )?;
                }
                tx.execute(
                    "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, 'blocks', 'all')",
                    params![after, id],
                )?;
                if let Some(before_id) = before.as_deref() {
                    tx.execute(
                        "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, 'blocks', 'all')",
                        params![id, before_id],
                    )?;
                }
                tx.execute(
                    "INSERT INTO search_index(source_type, source_id, title, body, path) VALUES ('item', ?1, ?2, ?3, NULL)",
                    params![id, title, description],
                )?;
                tx.commit()?;
                if let Some(before_id) = before.as_deref() {
                    self.demote_if_blocked(before_id)?;
                }
                self.demote_if_blocked(&id)?;
                self.promote_ready()?;
                Ok(mcp_json(json!({"item": self.get_item(&id)?})))
            }
            "planr_item_amend" => {
                let item_id = required_arg(&args, "id")?;
                let content = required_arg(&args, "note")?;
                let kind = args
                    .get("tag")
                    .and_then(Value::as_str)
                    .unwrap_or("amendment");
                let item = self.get_item(item_id)?;
                if matches!(
                    item.status.as_str(),
                    "closed" | "closed_partial" | "cancelled"
                ) {
                    bail!("cannot amend item {} from status {}", item.id, item.status);
                }
                let id = short_id("ctx");
                self.conn.execute(
                    "INSERT INTO contexts(id, project_id, item_id, worker_id, kind, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
                    params![id, self.default_project()?.id, item.id, worker_id(), kind, content, json!(["amend"]).to_string()],
                )?;
                self.index_search("context", &id, kind, content, None)?;
                Ok(mcp_json(
                    json!({"item": item, "context": self.get_context(&id)?}),
                ))
            }
            "planr_item_replan" => {
                let parent_id = required_arg(&args, "parent_id")?;
                let into = required_arg(&args, "into")?;
                let titles = into
                    .split(',')
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .collect::<Vec<_>>();
                let parent = self.get_item(parent_id)?;
                let cancellable =
                    self.child_items_by_statuses(parent_id, &["pending", "ready", "blocked"])?;
                let confirm = args
                    .get("confirm")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if !confirm {
                    return Ok(mcp_json(
                        json!({"mode": "preview", "action": "replan", "parent": parent, "would_cancel": cancellable, "would_create": titles}),
                    ));
                }
                if !self
                    .child_items_by_statuses(parent_id, &["picked", "running", "in_review"])?
                    .is_empty()
                {
                    bail!("cannot replan while child items are picked, running, or in review");
                }
                let project = self.default_project()?;
                let tx = self.conn.unchecked_transaction()?;
                for child in &cancellable {
                    tx.execute(
                        "DELETE FROM links WHERE from_item = ?1 OR to_item = ?1",
                        params![child.id],
                    )?;
                }
                tx.execute(
                    "UPDATE items SET status = 'cancelled', updated_at = datetime('now') WHERE parent_item_id = ?1 AND status IN ('pending','ready','blocked')",
                    params![parent_id],
                )?;
                let mut created_ids = Vec::new();
                let mut previous: Option<String> = None;
                for title in titles {
                    let id = item_id(title);
                    tx.execute(
                        "INSERT INTO items(id, project_id, parent_item_id, title, description, status, work_type, priority, plan_path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 'generic', 0, ?6, datetime('now'), datetime('now'))",
                        params![id, project.id, parent_id, title, format!("Replanned child for {}", parent.title), parent.plan_path.as_deref()],
                    )?;
                    tx.execute(
                        "INSERT INTO search_index(source_type, source_id, title, body, path) VALUES ('item', ?1, ?2, ?3, ?4)",
                        params![id, title, format!("Replanned child for {}", parent.title), parent.plan_path.as_deref()],
                    )?;
                    if let Some(prev) = previous {
                        tx.execute(
                            "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, 'blocks', 'all')",
                            params![prev, id],
                        )?;
                    }
                    previous = Some(id.clone());
                    created_ids.push(id);
                }
                tx.commit()?;
                self.promote_ready()?;
                let created = created_ids
                    .iter()
                    .map(|id| self.get_item(id))
                    .collect::<Result<Vec<_>>>()?;
                Ok(mcp_json(
                    json!({"cancelled": cancellable, "created": created}),
                ))
            }
            "planr_pick_item" => {
                if let Some((id, worker)) = self.pick_next_ready_item()? {
                    return Ok(mcp_json(
                        json!({"item": self.get_item(&id)?, "worker_id": worker, "runtime": self.item_runtime(&id)?, "context": self.pick_context(&id)?}),
                    ));
                }
                Ok(mcp_json(json!({"item": null})))
            }
            "planr_pick_heartbeat" => {
                let item_id = args
                    .get("item_id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or(self.current_item_for_worker()?)
                    .ok_or_else(|| anyhow!("no picked item for this worker"))?;
                self.heartbeat_item(&item_id)?;
                Ok(mcp_json(
                    json!({"item": self.get_item(&item_id)?, "runtime": self.item_runtime(&item_id)?}),
                ))
            }
            "planr_pick_progress" => {
                let item_id = required_arg(&args, "item_id")?;
                let percent = args.get("percent").and_then(Value::as_i64).unwrap_or(0);
                if !(0..=100).contains(&percent) {
                    bail!("progress percent must be between 0 and 100");
                }
                self.progress_item(item_id, percent, args.get("note").and_then(Value::as_str))?;
                Ok(mcp_json(
                    json!({"item": self.get_item(item_id)?, "runtime": self.item_runtime(item_id)?}),
                ))
            }
            "planr_pick_pause" => {
                let item_id = required_arg(&args, "item_id")?;
                self.pause_item(item_id, args.get("note").and_then(Value::as_str))?;
                Ok(mcp_json(
                    json!({"item": self.get_item(item_id)?, "runtime": self.item_runtime(item_id)?}),
                ))
            }
            "planr_pick_resume" => {
                let item_id = required_arg(&args, "item_id")?;
                self.resume_item(item_id)?;
                Ok(mcp_json(
                    json!({"item": self.get_item(item_id)?, "runtime": self.item_runtime(item_id)?}),
                ))
            }
            "planr_pick_stale" => {
                let older_than_seconds = args
                    .get("older_than_seconds")
                    .and_then(Value::as_i64)
                    .unwrap_or(900);
                Ok(mcp_json(
                    json!({"stale": self.stale_picks(older_than_seconds)?}),
                ))
            }
            "planr_recover_sweep" => {
                let older_than_seconds = args
                    .get("older_than_seconds")
                    .and_then(Value::as_i64)
                    .unwrap_or(900);
                let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(false);
                Ok(mcp_json(
                    self.recovery_sweep_value(older_than_seconds, apply)?,
                ))
            }
            "planr_approval_request" => {
                let item_id = required_arg(&args, "item_id")?;
                self.conn.execute(
                    "UPDATE items SET approval_status = 'requested', approval_requested_at = datetime('now'), approval_comment = ?1, approved_by = NULL, updated_at = datetime('now') WHERE id = ?2",
                    params![args.get("reason").and_then(Value::as_str), item_id],
                )?;
                Ok(mcp_json(
                    json!({"item": self.get_item(item_id)?, "approval": self.item_approval(item_id)?}),
                ))
            }
            "planr_approval_approve" => {
                let item_id = required_arg(&args, "item_id")?;
                let by = required_arg(&args, "by")?;
                self.conn.execute(
                    "UPDATE items SET approval_status = 'approved', approved_by = ?1, approval_comment = ?2, updated_at = datetime('now') WHERE id = ?3",
                    params![by, args.get("comment").and_then(Value::as_str), item_id],
                )?;
                Ok(mcp_json(
                    json!({"item": self.get_item(item_id)?, "approval": self.item_approval(item_id)?}),
                ))
            }
            "planr_approval_deny" => {
                let item_id = required_arg(&args, "item_id")?;
                let by = required_arg(&args, "by")?;
                self.conn.execute(
                    "UPDATE items SET approval_status = 'denied', approved_by = ?1, approval_comment = ?2, updated_at = datetime('now') WHERE id = ?3",
                    params![by, args.get("comment").and_then(Value::as_str), item_id],
                )?;
                Ok(mcp_json(
                    json!({"item": self.get_item(item_id)?, "approval": self.item_approval(item_id)?}),
                ))
            }
            "planr_approval_list" => {
                let open = args.get("open").and_then(Value::as_bool).unwrap_or(false);
                Ok(mcp_json(json!({"approvals": self.list_approvals(open)?})))
            }
            "planr_artifact_add" => {
                let name = required_arg(&args, "name")?;
                let item = args.get("item").and_then(Value::as_str);
                if let Some(item_id) = item {
                    self.get_item(item_id)?;
                }
                let id = short_id("art");
                self.conn.execute(
                    "INSERT INTO artifacts(id, project_id, item_id, name, kind, path, content, mime_type, size_bytes, metadata, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))",
                    params![
                        id,
                        self.default_project()?.id,
                        item,
                        name,
                        args.get("kind").and_then(Value::as_str).unwrap_or("evidence"),
                        args.get("path").and_then(Value::as_str),
                        args.get("content").and_then(Value::as_str),
                        args.get("mime").and_then(Value::as_str).unwrap_or("text/plain"),
                        args.get("content").and_then(Value::as_str).map(|content| content.len() as i64),
                        json!({"source": "mcp"}).to_string(),
                    ],
                )?;
                self.record_event(
                    "artifact_created",
                    item,
                    json!({"artifact_id": id.clone(), "name": name}),
                )?;
                Ok(mcp_json(json!({"artifact": self.get_artifact(&id)?})))
            }
            "planr_artifact_list" => Ok(mcp_json(json!({
                "artifacts": self.list_artifacts(args.get("item").and_then(Value::as_str))?
            }))),
            "planr_artifact_show" => {
                let id = required_arg(&args, "id")?;
                Ok(mcp_json(json!({"artifact": self.get_artifact(id)?})))
            }
            "planr_event_list" => Ok(mcp_json(json!({
                "events": self.list_events(
                    args.get("item").and_then(Value::as_str),
                    args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize
                )?
            }))),
            "planr_debug_bundle" => Ok(mcp_json(
                self.debug_bundle(args.get("item").and_then(Value::as_str))?,
            )),
            "planr_review_annotate" => {
                let item_id = required_arg(&args, "item_id")?;
                let message = required_arg(&args, "message")?;
                Ok(mcp_json(json!({"annotation": self.add_review_annotation(
                    ReviewAnnotationInput {
                        item_id,
                        message,
                        severity: args.get("severity").and_then(Value::as_str).unwrap_or("info"),
                        author: args.get("author").and_then(Value::as_str),
                        file: args.get("file").and_then(Value::as_str),
                        line: args.get("line").and_then(Value::as_u64),
                        source: "mcp",
                    }
                )?})))
            }
            "planr_review_ingest" => {
                let item_id = required_arg(&args, "item_id")?;
                let feedback = args
                    .get("feedback")
                    .cloned()
                    .or_else(|| args.get("payload").cloned())
                    .unwrap_or_else(|| args.clone());
                Ok(mcp_json(
                    self.ingest_review_feedback(item_id, feedback, "mcp")?,
                ))
            }
            "planr_review_artifact" => {
                let review_id = required_arg(&args, "review_item_id")?;
                Ok(mcp_json(json!({"artifact": self.write_review_artifact(
                    review_id,
                    None,
                    &[],
                    &[],
                    None,
                )?})))
            }
            "planr_review_evidence" => {
                let item_id = required_arg(&args, "item_id")?;
                let pr_context = args
                    .get("pr_url")
                    .and_then(Value::as_str)
                    .map(|url| self.record_pr_url(item_id, url))
                    .transpose()?;
                Ok(mcp_json(json!({
                    "evidence": self.review_evidence_value(item_id)?,
                    "pr_context": pr_context
                })))
            }
            "planr_log_add" => {
                let item = required_arg(&args, "item")?;
                let summary = required_arg(&args, "summary")?;
                let id = short_id("log");
                self.conn.execute(
                        "INSERT INTO logs(id, project_id, item_id, kind, summary, files, commands, tests, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))",
                    params![
                        id,
                        self.default_project()?.id,
                        item,
                        args.get("kind").and_then(Value::as_str).unwrap_or("completion"),
                        summary,
                        serde_json::to_string(args.get("files").unwrap_or(&json!([])))?,
                        serde_json::to_string(args.get("commands").unwrap_or(&json!([])))?,
                        serde_json::to_string(args.get("tests").unwrap_or(&json!([])))?,
                    ],
                )?;
                self.index_search("log", &id, summary, summary, None)?;
                self.record_event(
                    "log_created",
                    Some(item),
                    json!({"log_id": id.clone(), "kind": args.get("kind").and_then(Value::as_str).unwrap_or("completion")}),
                )?;
                Ok(mcp_json(json!({"log": self.get_log(&id)?})))
            }
            "planr_review_close" => {
                let review_id = required_arg(&args, "review_item_id")?;
                let verdict = args
                    .get("verdict")
                    .and_then(Value::as_str)
                    .unwrap_or("unclear");
                let findings = args
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
                Ok(mcp_json(
                    self.close_review_item(review_id, verdict, findings, "mcp")?,
                ))
            }
            "planr_close_item" => {
                let item_id = required_arg(&args, "item_id")?;
                self.ensure_can_close(item_id)?;
                self.conn.execute("UPDATE items SET status = 'closed', completed_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1", params![item_id])?;
                self.promote_ready()?;
                self.record_event("item_closed", Some(item_id), json!({"source": "mcp"}))?;
                Ok(mcp_json(json!({"closed": item_id, "next": "planr pick"})))
            }
            "planr_context_create" => {
                let id = short_id("ctx");
                self.conn.execute(
                    "INSERT INTO contexts(id, project_id, item_id, worker_id, kind, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', datetime('now'))",
                    params![id, self.default_project()?.id, args.get("item").and_then(Value::as_str), worker_id(), args.get("kind").and_then(Value::as_str).unwrap_or("discovery"), required_arg(&args, "content")?],
                )?;
                self.index_search(
                    "context",
                    &id,
                    args.get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("discovery"),
                    required_arg(&args, "content")?,
                    None,
                )?;
                self.record_event(
                    "context_created",
                    args.get("item").and_then(Value::as_str),
                    json!({"context_id": id.clone(), "source": "mcp"}),
                )?;
                Ok(mcp_json(json!({"context": self.get_context(&id)?})))
            }
            "planr_search" => {
                let query = required_arg(&args, "query")?;
                let results = self.search_results(query)?;
                Ok(mcp_json(json!({"results": results})))
            }
            "planr_log_read" => Ok(mcp_json(self.get_log(required_arg(&args, "id")?)?)),
            _ => Ok(mcp_json(
                json!({"error": {"code": "not_found", "message": format!("unknown Planr MCP tool: {name}")}}),
            )),
        }
    }

    pub(crate) fn mcp_resource_read(&self, params: Value) -> Result<Value> {
        let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
        let text = match uri {
            "planr://project/map" => serde_json::to_string(&self.map_value()?)?,
            "planr://project/context" => serde_json::to_string(&self.list_contexts(None)?)?,
            u if u.starts_with("planr://item/") => {
                serde_json::to_string(&self.get_item(u.trim_start_matches("planr://item/"))?)?
            }
            u if u.starts_with("planr://plan/") => {
                serde_json::to_string(&self.get_plan(u.trim_start_matches("planr://plan/"))?)?
            }
            u if u.starts_with("planr://log/") => {
                serde_json::to_string(&self.get_log(u.trim_start_matches("planr://log/"))?)?
            }
            _ => serde_json::to_string(
                &json!({"error": {"code": "not_found", "message": "resource not found"}}),
            )?,
        };
        Ok(json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": text}]}))
    }

    pub(crate) fn mcp_prompt_get(&self, params: Value) -> Value {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("planr-work");
        let text = match name {
            "planr-plan" => "Create or refine a Planr product or build plan. Keep scope, ownership, verification, and acceptance criteria explicit.",
            "planr-work" => "Use Planr as: inspect map, pick one ready item, implement, log evidence, request or close review when appropriate.",
            "planr-review" => "Review item evidence against plan, changed files, commands, and acceptance criteria before closure.",
            "planr-map" => "Summarize ready, blocked, picked, review, and closed items. Identify critical path and pressure points.",
            "planr-summary" => "Produce a concise status summary grounded in Planr map, logs, contexts, and reviews.",
            _ => "Use Planr map, plans, logs, and reviews as the source of truth.",
        };
        json!({"description": name, "messages": [{"role": "user", "content": {"type": "text", "text": text}}]})
    }
}
