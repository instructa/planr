use super::{artifact_row, event_row, App};
use crate::cli::{ItemAmendArgs, ItemInsertArgs, ItemReplanArgs};
use crate::model::{Item, Plan, Project};
use crate::planpack::{extract_work_specs, hash_path, parse_plan_metadata, plan_search_body};
use crate::storage::{row_to_item, row_to_log, row_to_plan, row_to_project};
use crate::util::{collect_rows, item_id, print_json, short_id, worker_id};
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

impl App {
    pub(crate) fn emit(&self, value: Value, human: String) -> Result<()> {
        if self.json {
            print_json(&value)
        } else {
            println!("{human}");
            Ok(())
        }
    }

    pub(crate) fn default_project(&self) -> Result<Project> {
        self.conn
            .query_row(
                "SELECT id, name, root_path, description, status FROM projects WHERE status = 'active' ORDER BY created_at DESC LIMIT 1",
                [],
                row_to_project,
            )
            .optional()?
            .ok_or_else(|| anyhow!("no project found; run planr project init"))
    }

    pub(crate) fn get_project(&self, id: &str) -> Result<Project> {
        self.conn
            .query_row(
                "SELECT id, name, root_path, description, status FROM projects WHERE id = ?1",
                params![id],
                row_to_project,
            )
            .optional()?
            .ok_or_else(|| anyhow!("project not found: {id}"))
    }

    pub(crate) fn list_projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare("SELECT id, name, root_path, description, status FROM projects ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], row_to_project)?;
        collect_rows(rows)
    }

    pub(crate) fn upsert_plan(
        &self,
        project_id: &str,
        stage: &str,
        path: &Path,
        title: &str,
        slug: &str,
        manifest: Value,
    ) -> Result<Plan> {
        let id = short_id("pln");
        let hash = hash_path(path)?;
        let (frontmatter, parse_status) = parse_plan_metadata(path);
        self.conn.execute(
            "INSERT INTO plans(id, project_id, stage, path, title, slug, package_manifest, frontmatter, parse_status, content_hash, archived, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, datetime('now'), datetime('now'))",
            params![id, project_id, stage, path.to_string_lossy(), title, slug, manifest.to_string(), frontmatter.to_string(), parse_status, hash],
        )?;
        self.index_search(
            "plan",
            &id,
            title,
            &plan_search_body(path)?,
            Some(&path.to_string_lossy()),
        )?;
        self.get_plan(&id)
    }

    pub(crate) fn get_plan(&self, id: &str) -> Result<Plan> {
        self.conn
            .query_row(
                "SELECT id, project_id, stage, path, title, slug, parse_status, archived FROM plans WHERE id = ?1",
                params![id],
                row_to_plan,
            )
            .optional()?
            .ok_or_else(|| anyhow!("plan not found: {id}"))
    }

    pub(crate) fn list_plans(&self, stage: Option<&str>) -> Result<Vec<Plan>> {
        let sql = if stage.is_some() {
            "SELECT id, project_id, stage, path, title, slug, parse_status, archived FROM plans WHERE archived = 0 AND stage = ?1 ORDER BY created_at DESC"
        } else {
            "SELECT id, project_id, stage, path, title, slug, parse_status, archived FROM plans WHERE archived = 0 ORDER BY created_at DESC"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = if let Some(stage) = stage {
            stmt.query_map(params![stage], row_to_plan)?
        } else {
            stmt.query_map([], row_to_plan)?
        };
        collect_rows(rows)
    }

    pub(crate) fn rehash_plan(&self, id: &str) -> Result<()> {
        let plan = self.get_plan(id)?;
        let hash = hash_path(Path::new(&plan.path))?;
        self.conn.execute(
            "UPDATE plans SET content_hash = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![hash, id],
        )?;
        Ok(())
    }

    pub(crate) fn seed_items_from_plan(&self, plan: &Plan) -> Result<Vec<Item>> {
        let mut specs = extract_work_specs(Path::new(&plan.path))?;
        if specs.is_empty() {
            specs.push((
                format!("Implement {}", plan.title),
                format!("Execute build plan {}", plan.id),
            ));
        }
        let mut created = Vec::new();
        for (title, description) in specs {
            let item = self.create_item(None, &title, &description, "code", Some(&plan.path))?;
            self.conn.execute(
                "INSERT INTO source_links(source_type, source_id, item_id, section_id, relationship) VALUES ('plan', ?1, ?2, NULL, 'implements')",
                params![plan.id, item.id],
            )?;
            created.push(item);
        }
        Ok(created)
    }

    pub(crate) fn create_item(
        &self,
        parent: Option<&str>,
        title: &str,
        description: &str,
        work_type: &str,
        plan_path: Option<&str>,
    ) -> Result<Item> {
        let project = self.default_project()?;
        let id = item_id(title);
        self.conn.execute(
            "INSERT INTO items(id, project_id, parent_item_id, title, description, status, work_type, priority, plan_path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, 0, ?7, datetime('now'), datetime('now'))",
            params![id, project.id, parent, title, description, work_type, plan_path],
        )?;
        self.index_search("item", &id, title, description, plan_path)?;
        self.promote_ready()?;
        let item = self.get_item(&id)?;
        self.record_event(
            "item_created",
            Some(&id),
            json!({"title": title, "work_type": work_type, "status": item.status}),
        )?;
        Ok(item)
    }

    pub(crate) fn get_item(&self, id: &str) -> Result<Item> {
        self.conn
            .query_row(
                "SELECT id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path FROM items WHERE id = ?1",
                params![id],
                row_to_item,
            )
            .optional()?
            .ok_or_else(|| anyhow!("item not found: {id}"))
    }

    pub(crate) fn list_items_by_type(
        &self,
        work_type: &str,
        not_status: Option<&str>,
    ) -> Result<Vec<Item>> {
        let sql = if not_status.is_some() {
            "SELECT id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path FROM items WHERE work_type = ?1 AND status != ?2 ORDER BY created_at"
        } else {
            "SELECT id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path FROM items WHERE work_type = ?1 ORDER BY created_at"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = if let Some(status) = not_status {
            stmt.query_map(params![work_type, status], row_to_item)?
        } else {
            stmt.query_map(params![work_type], row_to_item)?
        };
        collect_rows(rows)
    }

    pub(crate) fn add_link(&self, from: &str, to: &str, kind: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, ?3, 'all')",
            params![from, to, kind],
        )?;
        self.record_event(
            "link_added",
            Some(to),
            json!({"from": from, "to": to, "kind": kind}),
        )?;
        Ok(())
    }

    pub(crate) fn item_insert(&self, args: ItemInsertArgs) -> Result<()> {
        let after = self.get_item(&args.after)?;
        let before = args
            .before
            .as_deref()
            .map(|id| self.get_item(id))
            .transpose()?;
        let would_remove = if let Some(before) = &before {
            let exists: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM links WHERE from_item = ?1 AND to_item = ?2 AND kind = 'blocks'",
                params![after.id, before.id],
                |row| row.get(0),
            )?;
            if exists > 0 {
                json!([{"from": after.id, "to": before.id, "kind": "blocks"}])
            } else {
                json!([])
            }
        } else {
            json!([])
        };
        let would_add = if let Some(before) = &before {
            json!([
                {"from": after.id, "to": "<new-item>", "kind": "blocks"},
                {"from": "<new-item>", "to": before.id, "kind": "blocks"}
            ])
        } else {
            json!([{"from": after.id, "to": "<new-item>", "kind": "blocks"}])
        };
        if args.preview || !args.confirm {
            return self.emit(
                json!({
                    "mode": "preview",
                    "action": "insert",
                    "would_create": {"title": args.title, "description": args.description, "after": after.id, "before": before.as_ref().map(|item| item.id.clone())},
                    "would_remove_links": would_remove,
                    "would_add_links": would_add
                }),
                "preview only".to_string(),
            );
        }

        let project = self.default_project()?;
        let id = item_id(&args.title);
        let plan_path = before
            .as_ref()
            .and_then(|item| item.plan_path.as_deref())
            .or(after.plan_path.as_deref());
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO items(id, project_id, parent_item_id, title, description, status, work_type, priority, plan_path, created_at, updated_at) VALUES (?1, ?2, NULL, ?3, ?4, 'pending', 'generic', 0, ?5, datetime('now'), datetime('now'))",
            params![id, project.id, args.title, args.description, plan_path],
        )?;
        if let Some(before) = &before {
            tx.execute(
                "DELETE FROM links WHERE from_item = ?1 AND to_item = ?2 AND kind = 'blocks'",
                params![after.id, before.id],
            )?;
        }
        tx.execute(
            "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, 'blocks', 'all')",
            params![after.id, id],
        )?;
        if let Some(before) = &before {
            tx.execute(
                "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, 'blocks', 'all')",
                params![id, before.id],
            )?;
        }
        tx.execute(
            "INSERT INTO search_index(source_type, source_id, title, body, path) VALUES ('item', ?1, ?2, ?3, ?4)",
            params![id, args.title, args.description, plan_path],
        )?;
        tx.commit()?;
        if let Some(before) = &before {
            self.demote_if_blocked(&before.id)?;
        }
        self.demote_if_blocked(&id)?;
        self.promote_ready()?;
        self.emit(
            json!({"item": self.get_item(&id)?, "map": self.map_status_value()?}),
            "item inserted".to_string(),
        )
    }

    pub(crate) fn item_amend(&self, args: ItemAmendArgs) -> Result<()> {
        let item = self.get_item(&args.id)?;
        if matches!(
            item.status.as_str(),
            "closed" | "closed_partial" | "cancelled"
        ) {
            bail!("cannot amend item {} from status {}", item.id, item.status);
        }
        let id = short_id("ctx");
        self.conn.execute(
            "INSERT INTO contexts(id, project_id, item_id, worker_id, kind, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
            params![id, self.default_project()?.id, item.id, worker_id(), args.tag, args.note, json!(["amend"]).to_string()],
        )?;
        self.index_search("context", &id, &args.tag, &args.note, None)?;
        self.emit(
            json!({"item": item, "context": self.get_context(&id)?}),
            "item amended".to_string(),
        )
    }

    pub(crate) fn item_replan(&self, args: ItemReplanArgs) -> Result<()> {
        let parent = self.get_item(&args.parent_id)?;
        let titles: Vec<_> = args
            .into
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if titles.is_empty() {
            bail!("replan requires at least one target item title");
        }
        let active_children =
            self.child_items_by_statuses(&parent.id, &["picked", "running", "in_review"])?;
        if !active_children.is_empty() {
            bail!("cannot replan while child items are picked, running, or in review");
        }
        let cancellable =
            self.child_items_by_statuses(&parent.id, &["pending", "ready", "blocked"])?;
        if args.preview || !args.confirm {
            return self.emit(
                json!({
                    "mode": "preview",
                    "action": "replan",
                    "parent": parent,
                    "would_cancel": cancellable,
                    "would_create": titles,
                }),
                "preview only".to_string(),
            );
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
            params![parent.id],
        )?;
        let mut previous: Option<String> = None;
        let mut created_ids = Vec::new();
        for title in titles {
            let id = item_id(title);
            tx.execute(
                "INSERT INTO items(id, project_id, parent_item_id, title, description, status, work_type, priority, plan_path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 'generic', 0, ?6, datetime('now'), datetime('now'))",
                params![id, project.id, parent.id, title, format!("Replanned child for {}", parent.title), parent.plan_path.as_deref()],
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
        tx.execute(
            "UPDATE items SET status = 'blocked', updated_at = datetime('now') WHERE id = ?1",
            params![parent.id],
        )?;
        tx.commit()?;
        self.promote_ready()?;
        let created = created_ids
            .iter()
            .map(|id| self.get_item(id))
            .collect::<Result<Vec<_>>>()?;
        self.emit(
            json!({"cancelled": cancellable, "created": created}),
            "item replanned".to_string(),
        )
    }

    pub(crate) fn demote_if_blocked(&self, item_id: &str) -> Result<()> {
        let blocked: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM links l JOIN items i ON i.id = l.from_item WHERE l.to_item = ?1 AND l.kind IN ('blocks','feeds_into') AND i.status NOT IN ('closed','closed_partial')",
            params![item_id],
            |row| row.get(0),
        )?;
        if blocked > 0 {
            self.conn.execute("UPDATE items SET status = 'pending', updated_at = datetime('now') WHERE id = ?1 AND status = 'ready'", params![item_id])?;
        }
        Ok(())
    }

    pub(crate) fn promote_ready(&self) -> Result<()> {
        self.conn.execute(
            "UPDATE items SET status = 'ready', updated_at = datetime('now')
             WHERE status = 'pending'
             AND id NOT IN (
               SELECT l.to_item FROM links l JOIN items upstream ON upstream.id = l.from_item
               WHERE l.kind IN ('blocks','feeds_into') AND upstream.status NOT IN ('closed','closed_partial')
             )",
            [],
        )?;
        Ok(())
    }

    pub(crate) fn ensure_can_close(&self, item_id: &str) -> Result<()> {
        let item = self.get_item(item_id)?;
        if matches!(
            item.status.as_str(),
            "pending" | "blocked" | "cancelled" | "failed"
        ) {
            bail!(
                "invalid_transition: cannot close item {} from status {}",
                item.id,
                item.status
            );
        }
        let open_children: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM items WHERE parent_item_id = ?1 AND status NOT IN ('closed','closed_partial','cancelled')",
            params![item_id],
            |row| row.get(0),
        )?;
        if open_children > 0 {
            bail!("invalid_transition: cannot close item with open child items");
        }
        let open_reviews: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM links l JOIN items r ON r.id = l.from_item WHERE l.to_item = ?1 AND l.kind = 'reviews' AND r.status NOT IN ('closed','closed_partial','cancelled')",
            params![item_id],
            |row| row.get(0),
        )?;
        if open_reviews > 0 {
            bail!("invalid_transition: cannot close item with open reviews");
        }
        let approval_status: Option<String> = self.conn.query_row(
            "SELECT approval_status FROM items WHERE id = ?1",
            params![item_id],
            |row| row.get(0),
        )?;
        match approval_status.as_deref() {
            Some("requested") => {
                bail!("invalid_transition: cannot close item with pending approval")
            }
            Some("denied") => bail!("invalid_transition: cannot close item with denied approval"),
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn current_item_for_worker(&self) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT id FROM items WHERE worker_id = ?1 AND status IN ('picked','running') ORDER BY picked_at DESC LIMIT 1",
                params![worker_id()],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub(crate) fn pick_next_ready_item(&self) -> Result<Option<(String, String)>> {
        let project = self.default_project()?;
        self.promote_ready()?;
        let worker = worker_id();
        let token = short_id("pick");
        let picked: Option<String> = self
            .conn
            .query_row(
                "UPDATE items
                 SET status = 'picked',
                     worker_id = ?1,
                     pick_token = ?2,
                     picked_at = datetime('now'),
                     last_heartbeat_at = datetime('now'),
                     updated_at = datetime('now')
                 WHERE id = (
                     SELECT id FROM items
                     WHERE project_id = ?3 AND status = 'ready'
                     ORDER BY priority DESC, created_at ASC
                     LIMIT 1
                 )
                 AND status = 'ready'
                 RETURNING id",
                params![worker, token, project.id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = picked {
            self.record_event(
                "item_picked",
                Some(&id),
                json!({"worker_id": worker.clone(), "pick_token": token}),
            )?;
            Ok(Some((id, worker)))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn ensure_worker_owns_or_unowned(&self, item_id: &str) -> Result<()> {
        let item = self.get_item(item_id)?;
        let worker = worker_id();
        if let Some(owner) = item.worker_id.as_deref() {
            if owner != worker {
                bail!("item is owned by {owner}; current worker is {worker}");
            }
        }
        Ok(())
    }

    pub(crate) fn ensure_runtime_update_changed(
        &self,
        item_id: &str,
        changed: usize,
    ) -> Result<()> {
        if changed == 0 {
            let item = self.get_item(item_id)?;
            bail!(
                "invalid_transition: item {} is {}; pick it before updating runtime",
                item.id,
                item.status
            );
        }
        Ok(())
    }

    pub(crate) fn heartbeat_item(&self, item_id: &str) -> Result<()> {
        self.ensure_worker_owns_or_unowned(item_id)?;
        let changed = self.conn.execute(
            "UPDATE items SET status = CASE WHEN status = 'picked' THEN 'running' ELSE status END, last_heartbeat_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1 AND status IN ('picked','running')",
            params![item_id],
        )?;
        self.ensure_runtime_update_changed(item_id, changed)?;
        self.record_event(
            "item_heartbeat",
            Some(item_id),
            json!({"worker_id": worker_id()}),
        )
    }

    pub(crate) fn progress_item(
        &self,
        item_id: &str,
        percent: i64,
        note: Option<&str>,
    ) -> Result<()> {
        self.ensure_worker_owns_or_unowned(item_id)?;
        let changed = self.conn.execute(
            "UPDATE items SET progress_percent = ?1, progress_note = ?2, last_heartbeat_at = datetime('now'), updated_at = datetime('now') WHERE id = ?3 AND status IN ('picked','running')",
            params![percent, note, item_id],
        )?;
        self.ensure_runtime_update_changed(item_id, changed)?;
        self.record_event(
            "item_progress",
            Some(item_id),
            json!({"percent": percent, "has_note": note.is_some()}),
        )
    }

    pub(crate) fn pause_item(&self, item_id: &str, note: Option<&str>) -> Result<()> {
        self.ensure_worker_owns_or_unowned(item_id)?;
        let changed = self.conn.execute(
            "UPDATE items SET status = 'picked', paused_at = datetime('now'), progress_note = COALESCE(?1, progress_note), updated_at = datetime('now') WHERE id = ?2 AND status IN ('picked','running')",
            params![note, item_id],
        )?;
        self.ensure_runtime_update_changed(item_id, changed)?;
        self.record_event(
            "item_paused",
            Some(item_id),
            json!({"has_note": note.is_some()}),
        )
    }

    pub(crate) fn resume_item(&self, item_id: &str) -> Result<()> {
        self.ensure_worker_owns_or_unowned(item_id)?;
        let changed = self.conn.execute(
            "UPDATE items SET status = 'running', paused_at = NULL, last_heartbeat_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1 AND status IN ('picked','running')",
            params![item_id],
        )?;
        self.ensure_runtime_update_changed(item_id, changed)?;
        self.record_event(
            "item_resumed",
            Some(item_id),
            json!({"worker_id": worker_id()}),
        )
    }

    pub(crate) fn item_runtime(&self, item_id: &str) -> Result<Value> {
        self.conn.query_row(
            "SELECT worker_id, pick_token, picked_at, last_heartbeat_at, progress_percent, progress_note, paused_at, timeout_seconds FROM items WHERE id = ?1",
            params![item_id],
            |row| {
                Ok(json!({
                    "worker_id": row.get::<_, Option<String>>(0)?,
                    "pick_token": row.get::<_, Option<String>>(1)?,
                    "picked_at": row.get::<_, Option<String>>(2)?,
                    "last_heartbeat_at": row.get::<_, Option<String>>(3)?,
                    "progress_percent": row.get::<_, Option<i64>>(4)?,
                    "progress_note": row.get::<_, Option<String>>(5)?,
                    "paused_at": row.get::<_, Option<String>>(6)?,
                    "timeout_seconds": row.get::<_, Option<i64>>(7)?,
                }))
            },
        ).map_err(Into::into)
    }

    pub(crate) fn item_approval(&self, item_id: &str) -> Result<Value> {
        self.conn.query_row(
            "SELECT approval_status, approval_requested_at, approved_by, approval_comment FROM items WHERE id = ?1",
            params![item_id],
            |row| {
                Ok(json!({
                    "status": row.get::<_, Option<String>>(0)?,
                    "requested_at": row.get::<_, Option<String>>(1)?,
                    "by": row.get::<_, Option<String>>(2)?,
                    "comment": row.get::<_, Option<String>>(3)?,
                }))
            },
        ).map_err(Into::into)
    }

    pub(crate) fn list_approvals(&self, open: bool) -> Result<Vec<Value>> {
        let sql = if open {
            "SELECT id FROM items WHERE approval_status IN ('requested','denied') ORDER BY updated_at DESC"
        } else {
            "SELECT id FROM items WHERE approval_status IS NOT NULL ORDER BY updated_at DESC"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let ids = collect_rows(stmt.query_map([], |row| row.get::<_, String>(0))?)?;
        ids.iter()
            .map(|id| Ok(json!({"item": self.get_item(id)?, "approval": self.item_approval(id)?})))
            .collect()
    }

    pub(crate) fn stale_picks(&self, older_than_seconds: i64) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM items
             WHERE status IN ('picked','running')
             AND datetime(COALESCE(last_heartbeat_at, picked_at, updated_at), '+' || ?1 || ' seconds') < datetime('now')
             ORDER BY COALESCE(last_heartbeat_at, picked_at, updated_at)",
        )?;
        let ids = collect_rows(
            stmt.query_map(params![older_than_seconds], |row| row.get::<_, String>(0))?,
        )?;
        ids.iter()
            .map(|id| Ok(json!({"item": self.get_item(id)?, "runtime": self.item_runtime(id)?})))
            .collect()
    }

    pub(crate) fn map_show(&self) -> Result<()> {
        let value = self.map_value()?;
        if self.json {
            return self.emit(value, String::new());
        }
        let project_name = self
            .default_project()
            .map(|project| project.name)
            .unwrap_or_else(|_| "planr".to_string());
        let items = self.all_items()?;
        let edges = self
            .all_links()?
            .into_iter()
            .filter_map(|link| {
                let kind = link.get("kind")?.as_str()?;
                if kind != "blocks" && kind != "feeds_into" {
                    return None;
                }
                Some(super::render::RenderEdge {
                    from: link.get("from")?.as_str()?.to_string(),
                    to: link.get("to")?.as_str()?.to_string(),
                    kind: kind.to_string(),
                })
            })
            .collect::<Vec<_>>();
        let critical = self
            .critical_lane()
            .map(|lane| lane.into_iter().map(|item| item.id).collect())
            .unwrap_or_default();
        let cycles = self.graph_cycles().unwrap_or_default();
        let human = super::render::render_map(&project_name, &items, &edges, &critical, &cycles);
        self.emit(value, human)
    }

    pub(crate) fn map_value(&self) -> Result<Value> {
        let items = self.all_items()?;
        let links = self.all_links()?;
        let mut counts = BTreeMap::new();
        for item in &items {
            *counts.entry(item.status.clone()).or_insert(0usize) += 1;
        }
        Ok(json!({"items": items, "links": links, "counts": counts}))
    }

    pub(crate) fn map_status_value(&self) -> Result<Value> {
        let items = self.all_items()?;
        let links = self.all_links()?;
        let mut counts = BTreeMap::new();
        let mut ready = Vec::new();
        let mut picked = Vec::new();
        let mut blocked = Vec::new();
        let mut reviews = Vec::new();
        for item in items {
            *counts.entry(item.status.clone()).or_insert(0usize) += 1;
            match item.status.as_str() {
                "ready" => ready.push(item),
                "picked" | "running" => picked.push(json!({
                    "item": item,
                    "runtime": self.item_runtime(&item.id)?,
                    "approval": self.item_approval(&item.id)?,
                })),
                "pending" | "blocked" => blocked.push(json!({
                    "item": item,
                    "blockers": self.blocking_items_for(&item.id)?,
                })),
                _ => {
                    if item.work_type == "review" && item.status != "closed" {
                        reviews.push(item);
                    }
                }
            }
        }
        Ok(json!({
            "counts": counts,
            "ready": ready,
            "picked": picked,
            "blocked": blocked,
            "reviews": reviews,
            "links": links,
            "analysis": self.graph_status_value()?,
        }))
    }

    pub(crate) fn preview_close_value(&self, item_id: &str) -> Result<Value> {
        let item = self.get_item(item_id)?;
        let blockers = self.blocking_items_for(item_id)?;
        let child_blockers = self.open_child_items(item_id)?;
        let review_blockers = self.open_review_items(item_id)?;
        let approval = self.item_approval(item_id)?;
        let recovery = self.item_recovery(item_id)?;
        let conditions = self.item_conditions(item_id)?;
        let approval_blocks_close = matches!(
            approval.get("status").and_then(Value::as_str),
            Some("requested") | Some("denied")
        );
        let invalid_status = matches!(
            item.status.as_str(),
            "pending" | "blocked" | "cancelled" | "failed"
        );
        let close_effect = self.close_effect(item_id)?;
        let can_close = !invalid_status
            && blockers.is_empty()
            && child_blockers.is_empty()
            && review_blockers.is_empty()
            && !approval_blocks_close;
        Ok(json!({
            "mode": "preview",
            "action": "close",
            "item": item,
            "can_close": can_close,
            "status_blocks_close": invalid_status,
            "approval_blocks_close": approval_blocks_close,
            "approval": approval,
            "recovery": recovery,
            "conditions": conditions,
            "post_condition_unverified": conditions
                .get("post")
                .and_then(Value::as_str)
                .is_some(),
            "blockers": blockers,
            "open_children": child_blockers,
            "open_reviews": review_blockers,
            "would_unlock": close_effect.would_unlock,
            "would_remain_blocked": close_effect.would_remain_blocked,
        }))
    }

    pub(crate) fn lookahead_value(&self, from: Option<&str>, limit: usize) -> Result<Value> {
        let limit = limit.max(1) as i64;
        if let Some(item_id) = from {
            let effect = self.close_effect(item_id)?;
            return Ok(json!({
                "from": self.get_item(item_id)?,
                "would_unlock": effect.would_unlock,
                "would_remain_blocked": effect.would_remain_blocked,
                "close_preview": self.preview_close_value(item_id)?,
            }));
        }
        let ready = self.items_with_status("ready", limit)?;
        let pending = self.items_with_status("pending", limit)?;
        Ok(json!({
            "ready_next": ready,
            "pending_next": pending,
            "analysis": self.graph_status_value()?,
        }))
    }

    pub(crate) fn all_items(&self) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare("SELECT id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path FROM items ORDER BY created_at")?;
        let rows = stmt.query_map([], row_to_item)?;
        collect_rows(rows)
    }

    pub(crate) fn items_with_status(&self, status: &str, limit: i64) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare("SELECT id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path FROM items WHERE status = ?1 ORDER BY priority DESC, created_at LIMIT ?2")?;
        let rows = stmt.query_map(params![status, limit], row_to_item)?;
        collect_rows(rows)
    }

    pub(crate) fn child_items_by_statuses(
        &self,
        parent_id: &str,
        statuses: &[&str],
    ) -> Result<Vec<Item>> {
        if statuses.is_empty() {
            return Ok(Vec::new());
        }
        let status_list = statuses
            .iter()
            .map(|status| format!("'{}'", status.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path FROM items WHERE parent_item_id = ?1 AND status IN ({status_list}) ORDER BY created_at"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![parent_id], row_to_item)?;
        collect_rows(rows)
    }

    pub(crate) fn blocking_items_for(&self, item_id: &str) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.project_id, i.parent_item_id, i.title, i.description, i.status, i.work_type, i.priority, i.worker_id, i.plan_path
             FROM links l JOIN items i ON i.id = l.from_item
             WHERE l.to_item = ?1 AND l.kind IN ('blocks','feeds_into') AND i.status NOT IN ('closed','closed_partial')
             ORDER BY i.created_at",
        )?;
        let rows = stmt.query_map(params![item_id], row_to_item)?;
        collect_rows(rows)
    }

    pub(crate) fn open_child_items(&self, item_id: &str) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path
             FROM items WHERE parent_item_id = ?1 AND status NOT IN ('closed','closed_partial','cancelled') ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![item_id], row_to_item)?;
        collect_rows(rows)
    }

    pub(crate) fn open_review_items(&self, item_id: &str) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.project_id, r.parent_item_id, r.title, r.description, r.status, r.work_type, r.priority, r.worker_id, r.plan_path
             FROM links l JOIN items r ON r.id = l.from_item
             WHERE l.to_item = ?1 AND l.kind = 'reviews' AND r.status NOT IN ('closed','closed_partial','cancelled')
             ORDER BY r.created_at",
        )?;
        let rows = stmt.query_map(params![item_id], row_to_item)?;
        collect_rows(rows)
    }

    pub(crate) fn would_unlock_items(&self, item_id: &str) -> Result<Vec<Item>> {
        Ok(self.close_effect(item_id)?.would_unlock)
    }

    pub(crate) fn all_links(&self) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare("SELECT from_item, to_item, kind FROM links ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            Ok(json!({"from": row.get::<_, String>(0)?, "to": row.get::<_, String>(1)?, "kind": row.get::<_, String>(2)?}))
        })?;
        collect_rows(rows)
    }

    pub(crate) fn get_log(&self, id: &str) -> Result<Value> {
        self.conn.query_row("SELECT id, item_id, kind, summary, files, commands, tests, review_findings, created_at FROM logs WHERE id = ?1", params![id], row_to_log).optional()?.ok_or_else(|| anyhow!("log not found: {id}"))
    }

    pub(crate) fn list_logs(&self, item: Option<&str>) -> Result<Vec<Value>> {
        let sql = if item.is_some() {
            "SELECT id, item_id, kind, summary, files, commands, tests, review_findings, created_at FROM logs WHERE item_id = ?1 ORDER BY created_at DESC"
        } else {
            "SELECT id, item_id, kind, summary, files, commands, tests, review_findings, created_at FROM logs ORDER BY created_at DESC"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = if let Some(item) = item {
            stmt.query_map(params![item], row_to_log)?
        } else {
            stmt.query_map([], row_to_log)?
        };
        collect_rows(rows)
    }

    pub(crate) fn get_artifact(&self, id: &str) -> Result<Value> {
        self.conn
            .query_row(
                "SELECT id, project_id, item_id, name, kind, path, content, mime_type, size_bytes, metadata, created_at FROM artifacts WHERE id = ?1",
                params![id],
                artifact_row,
            )
            .optional()?
            .ok_or_else(|| anyhow!("artifact not found: {id}"))
    }

    pub(crate) fn latest_review_artifact(&self, review_id: &str) -> Result<Value> {
        self.get_item(review_id)?;
        self.conn
            .query_row(
                "SELECT id, project_id, item_id, name, kind, path, content, mime_type, size_bytes, metadata, created_at FROM artifacts WHERE item_id = ?1 AND kind = 'review' ORDER BY created_at DESC, id DESC LIMIT 1",
                params![review_id],
                artifact_row,
            )
            .optional()?
            .ok_or_else(|| anyhow!("review artifact not found: {review_id}"))
    }

    pub(crate) fn list_artifacts(&self, item: Option<&str>) -> Result<Vec<Value>> {
        let sql = if item.is_some() {
            "SELECT id, project_id, item_id, name, kind, path, content, mime_type, size_bytes, metadata, created_at FROM artifacts WHERE item_id = ?1 ORDER BY created_at DESC LIMIT 100"
        } else {
            "SELECT id, project_id, item_id, name, kind, path, content, mime_type, size_bytes, metadata, created_at FROM artifacts ORDER BY created_at DESC LIMIT 100"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = if let Some(item) = item {
            stmt.query_map(params![item], artifact_row)?
        } else {
            stmt.query_map([], artifact_row)?
        };
        collect_rows(rows)
    }

    pub(crate) fn record_event(
        &self,
        event_type: &str,
        item_id: Option<&str>,
        payload: Value,
    ) -> Result<()> {
        let project_id = self.default_project().ok().map(|project| project.id);
        self.conn.execute(
            "INSERT INTO events(project_id, item_id, worker_id, event_type, payload, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
            params![
                project_id.as_deref(),
                item_id,
                worker_id(),
                event_type,
                payload.to_string(),
            ],
        )?;
        Ok(())
    }

    pub(crate) fn list_events(&self, item: Option<&str>, limit: usize) -> Result<Vec<Value>> {
        let limit = limit.clamp(1, 500) as i64;
        let sql = if item.is_some() {
            "SELECT id, project_id, item_id, worker_id, event_type, payload, timestamp FROM events WHERE item_id = ?1 ORDER BY id DESC LIMIT ?2"
        } else {
            "SELECT id, project_id, item_id, worker_id, event_type, payload, timestamp FROM events ORDER BY id DESC LIMIT ?1"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = if let Some(item) = item {
            stmt.query_map(params![item, limit], event_row)?
        } else {
            stmt.query_map(params![limit], event_row)?
        };
        collect_rows(rows)
    }

    pub(crate) fn events_after(&self, after_id: i64) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, item_id, worker_id, event_type, payload, timestamp FROM events WHERE id > ?1 ORDER BY id LIMIT 500",
        )?;
        let rows = stmt.query_map(params![after_id], event_row)?;
        collect_rows(rows)
    }
}
