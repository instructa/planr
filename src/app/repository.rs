use super::App;
use crate::cli::{ItemAmendArgs, ItemInsertArgs, ItemReplanArgs};
use crate::model::Item;
use crate::model::LinkKind;
use crate::storage::row_to_item;
use crate::util::{collect_rows, item_id, print_json, short_id, worker_id};
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::env;
use std::io::{IsTerminal, Write, stdout};
use std::thread;
use std::time::Duration;

mod context;
pub(crate) mod eval;
mod evidence;
mod item;
mod link;
mod plan;
mod project;
mod search;

impl App {
    pub(crate) fn emit(&self, value: Value, human: String) -> Result<()> {
        if self.json {
            print_json(&value)
        } else {
            println!("{human}");
            Ok(())
        }
    }

    /// Splits a parent into chained children: each title becomes a child,
    /// consecutive children are linked with `blocks` in the given order, and
    /// the parent parks as a blocked gate until the children settle. Shared
    /// by CLI and MCP so both surfaces produce the same graph shape.
    pub(crate) fn breakdown_item(&self, parent_id: &str, titles: &[String]) -> Result<Vec<Item>> {
        if titles.is_empty() {
            bail!("breakdown requires at least one child title via --into");
        }
        let parent = self.get_item(parent_id)?;
        let mut created = Vec::new();
        let mut previous: Option<String> = None;
        for title in titles {
            let child = self.create_item(
                Some(&parent.id),
                title,
                &format!("Sub-item for {}", parent.title),
                "generic",
                parent.plan_path.as_deref(),
            )?;
            if let Some(prev) = previous {
                self.add_link(&prev, &child.id, "blocks")?;
            }
            previous = Some(child.id.clone());
            created.push(child);
        }
        self.conn.execute(
            "UPDATE items SET status = 'blocked', updated_at = datetime('now') WHERE id = ?1",
            params![parent.id],
        )?;
        self.promote_ready()?;
        // Re-fetch: chaining demotes later children to blocked after creation.
        created.iter().map(|item| self.get_item(&item.id)).collect()
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
            "SELECT COUNT(*) FROM links l JOIN items i ON i.id = l.from_item WHERE l.to_item = ?1 AND l.kind IN ('blocks','hands_to') AND i.status NOT IN ('closed','closed_partial')",
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
               WHERE l.kind IN ('blocks','hands_to') AND upstream.status NOT IN ('closed','closed_partial')
             )",
            [],
        )?;
        self.roll_up_parent_gates()?;
        Ok(())
    }

    /// Parent items are gates over their children: breakdown/replan park them
    /// in `blocked`. Once every child is settled the gate becomes `ready`, and
    /// auto-closes when no review or approval remains open. Loops so closures
    /// roll up through grandparents.
    fn roll_up_parent_gates(&self) -> Result<()> {
        loop {
            let promoted = self.conn.execute(
                "UPDATE items SET status = 'ready', updated_at = datetime('now')
                 WHERE status = 'blocked'
                 AND EXISTS (SELECT 1 FROM items c WHERE c.parent_item_id = items.id)
                 AND NOT EXISTS (
                   SELECT 1 FROM items c WHERE c.parent_item_id = items.id
                   AND c.status NOT IN ('closed','closed_partial','cancelled')
                 )
                 AND id NOT IN (
                   SELECT l.to_item FROM links l JOIN items upstream ON upstream.id = l.from_item
                   WHERE l.kind IN ('blocks','hands_to') AND upstream.status NOT IN ('closed','closed_partial')
                 )",
                [],
            )?;
            let auto_closed: Vec<(String, String)> = {
                let mut stmt = self.conn.prepare(
                    "UPDATE items SET
                       status = CASE WHEN EXISTS (
                         SELECT 1 FROM items c WHERE c.parent_item_id = items.id
                         AND c.status IN ('closed_partial','cancelled')
                       ) THEN 'closed_partial' ELSE 'closed' END,
                       completed_at = datetime('now'),
                       updated_at = datetime('now')
                     WHERE status = 'ready'
                     AND EXISTS (SELECT 1 FROM items c WHERE c.parent_item_id = items.id)
                     AND NOT EXISTS (
                       SELECT 1 FROM items c WHERE c.parent_item_id = items.id
                       AND c.status NOT IN ('closed','closed_partial','cancelled')
                     )
                     AND NOT EXISTS (
                       SELECT 1 FROM links l JOIN items r ON r.id = l.from_item
                       WHERE l.to_item = items.id AND l.kind = 'reviews'
                       AND r.status NOT IN ('closed','closed_partial','cancelled')
                     )
                     AND COALESCE(approval_status, '') NOT IN ('requested','denied')
                     RETURNING id, status",
                )?;
                collect_rows(stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?)?
            };
            for (item_id, status) in &auto_closed {
                let log_id = short_id("log");
                self.conn.execute(
                    "INSERT INTO logs(id, project_id, item_id, kind, summary, created_at)
                     SELECT ?1, project_id, id, 'completion', ?2, datetime('now') FROM items WHERE id = ?3",
                    params![
                        log_id,
                        "parent gate auto-closed: all child items settled",
                        item_id
                    ],
                )?;
                self.record_event(
                    "item_closed",
                    Some(item_id),
                    json!({"auto": true, "status": status, "log_id": log_id}),
                )?;
            }
            if promoted == 0 && auto_closed.is_empty() {
                break;
            }
        }
        Ok(())
    }

    pub(crate) fn ensure_can_close(&self, item_id: &str) -> Result<()> {
        let item = self.get_item(item_id)?;
        match item.status.as_str() {
            "pending" | "blocked" => bail!(
                "invalid_transition: cannot close item {} from status {}; settle its blockers first (`planr trace item {} --json` lists them)",
                item.id,
                item.status,
                item.id
            ),
            "cancelled" | "failed" => bail!(
                "invalid_transition: cannot close item {} from status {}; the item is settled, create a follow-up with `planr item create` instead",
                item.id,
                item.status
            ),
            _ => {}
        }
        let open_children: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM items WHERE parent_item_id = ?1 AND status NOT IN ('closed','closed_partial','cancelled')",
            params![item_id],
            |row| row.get(0),
        )?;
        if open_children > 0 {
            bail!(
                "invalid_transition: cannot close item with open child items; close or cancel them first (`planr trace item {item_id} --json` lists them)"
            );
        }
        let open_review: Option<String> = self.conn.query_row(
            "SELECT r.id FROM links l JOIN items r ON r.id = l.from_item WHERE l.to_item = ?1 AND l.kind = 'reviews' AND r.status NOT IN ('closed','closed_partial','cancelled') LIMIT 1",
            params![item_id],
            |row| row.get(0),
        ).optional()?;
        if let Some(review_id) = open_review {
            bail!(
                "invalid_transition: cannot close item with open reviews; close the review first: `planr review close {review_id} --verdict complete --close-target`"
            );
        }
        let approval_status: Option<String> = self.conn.query_row(
            "SELECT approval_status FROM items WHERE id = ?1",
            params![item_id],
            |row| row.get(0),
        )?;
        match approval_status.as_deref() {
            Some("requested") => {
                bail!(
                    "invalid_transition: cannot close item with pending approval; resolve it first: `planr approval approve {item_id}` or `planr approval deny {item_id}`"
                )
            }
            Some("denied") => bail!(
                "invalid_transition: cannot close item with denied approval; request again after fixes: `planr approval request {item_id}`"
            ),
            _ => {}
        }
        if let Some(next_action) = self.proof_close_blocker(item_id)? {
            bail!(
                "invalid_transition: cannot close item {item_id}; binding Evidence coverage is not proven. next proof action: {next_action}"
            );
        }
        Ok(())
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
            .map(|id| {
                Ok(json!({
                    "item": self.get_item(id)?,
                    "approval": self.item_approval(id)?,
                    "proof": self.proof_status_for_item(id)?,
                }))
            })
            .collect()
    }

    pub(crate) fn map_show(
        &self,
        plan: Option<&str>,
        view: crate::cli::MapViewArg,
        full: bool,
    ) -> Result<()> {
        Self::validate_map_detail(view, full)?;
        let value = self.map_value(plan)?;
        if self.json {
            return self.emit(value, String::new());
        }
        let human = self.render_map_human(&value, view, full)?;
        self.emit(value, human)
    }

    pub(crate) fn map_watch(&self, args: crate::cli::MapWatchArgs) -> Result<()> {
        Self::validate_map_detail(args.view, args.full)?;
        if self.json {
            bail!(
                "refusing JSON map watch: use `planr map show --json` for snapshots or `planr serve` with `/v1/events/stream` for machine event streaming"
            );
        }

        let clear_screen = !args.no_clear
            && stdout().is_terminal()
            && env::var("TERM").ok().as_deref() != Some("dumb");
        let mut previous = None;
        let mut polls = 0u64;
        let mut frames = 0u64;
        loop {
            let value = self.map_value(args.plan.as_deref())?;
            let key = serde_json::to_string(&value)?;
            if previous.as_deref() != Some(key.as_str()) {
                frames += 1;
                let human = self.render_map_human(&value, args.view, args.full)?;
                let view = match args.view {
                    crate::cli::MapViewArg::Tree => "tree",
                    crate::cli::MapViewArg::Diagram => "diagram",
                };
                let scope = args
                    .plan
                    .as_deref()
                    .map(|plan| format!(" · plan {plan}"))
                    .unwrap_or_default();
                let mut output = stdout().lock();
                if clear_screen {
                    write!(output, "\x1b[2J\x1b[H")?;
                }
                writeln!(
                    output,
                    "watching map · {view}{scope} · every {}ms · update {frames} · Ctrl-C to stop",
                    args.interval_ms
                )?;
                writeln!(output)?;
                writeln!(output, "{human}")?;
                output.flush()?;
                previous = Some(key);
            }

            polls += 1;
            let settled = value["settled"].as_u64() == value["total"].as_u64();
            if (args.until_settled && settled)
                || args.iterations.is_some_and(|limit| polls >= limit)
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(args.interval_ms));
        }
    }

    fn validate_map_detail(view: crate::cli::MapViewArg, full: bool) -> Result<()> {
        if full && !matches!(view, crate::cli::MapViewArg::Diagram) {
            bail!("`--full` requires `--view diagram`");
        }
        Ok(())
    }

    fn render_map_human(
        &self,
        value: &Value,
        view: crate::cli::MapViewArg,
        full: bool,
    ) -> Result<String> {
        let project_name = self
            .default_project()
            .map(|project| project.name)
            .unwrap_or_else(|_| "planr".to_string());
        // Render from the (possibly plan-scoped) value, not from a second
        // unscoped fetch, so human and JSON output show the same slice.
        let items: Vec<Item> = serde_json::from_value(value["items"].clone())?;
        let edges = value["links"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|link| {
                let kind = link.get("kind")?.as_str()?;
                let kind = LinkKind::try_from(kind).ok()?;
                if !kind.blocks_readiness() {
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
        let human = match view {
            crate::cli::MapViewArg::Tree => {
                super::render::render_map(&project_name, &items, &edges, &critical, &cycles)
            }
            crate::cli::MapViewArg::Diagram => super::render::render_diagram_map(
                &project_name,
                &items,
                &edges,
                &critical,
                &cycles,
                full,
            ),
        };
        let human = super::render::colorize_map(&human, self.color);
        Ok(human)
    }

    /// `plan` narrows the map to one plan's items and the links among them —
    /// plan-scoped goal runs on shared boards see their contract's slice, not
    /// the whole board. An unknown plan id is an error, never a silent
    /// unscoped view (same rule as `pick --plan`).
    pub(crate) fn map_value(&self, plan: Option<&str>) -> Result<Value> {
        let plan_path = plan
            .map(|plan_id| self.get_plan(plan_id))
            .transpose()?
            .map(|plan| plan.path);
        let mut items = self.all_items()?;
        let mut links = self.all_links()?;
        if let Some(path) = plan_path.as_deref() {
            items.retain(|item| item.plan_path.as_deref() == Some(path));
            let ids: std::collections::HashSet<&str> =
                items.iter().map(|item| item.id.as_str()).collect();
            links.retain(|link| {
                let from = link["from"].as_str().unwrap_or_default();
                let to = link["to"].as_str().unwrap_or_default();
                ids.contains(from) && ids.contains(to)
            });
        }
        // Same explicit-zero status vocabulary as the `remaining` snapshot in
        // pick/done/close responses: one counts shape across all surfaces.
        let progress = self.progress_value_scoped(plan_path.as_deref())?;
        Ok(json!({
            "items": items,
            "links": links,
            "counts": progress["counts"],
            "settled": progress["settled"],
            "total": progress["total"],
        }))
    }

    pub(crate) fn map_status_value(&self) -> Result<Value> {
        let items = self.all_items()?;
        let links = self.all_links()?;
        // Same explicit-zero status vocabulary as the `remaining` snapshot in
        // pick/done/close responses: one counts shape across all surfaces.
        let progress = self.progress_value()?;
        let mut ready = Vec::new();
        let mut picked = Vec::new();
        let mut in_review = Vec::new();
        let mut blocked = Vec::new();
        let mut reviews = Vec::new();
        for item in items {
            match item.status.as_str() {
                "ready" => ready.push(json!({
                    "item": item,
                    "proof": self.proof_status_for_item(&item.id)?,
                })),
                "picked" | "running" => picked.push(json!({
                    "item": item,
                    "runtime": self.item_runtime(&item.id)?,
                    "approval": self.item_approval(&item.id)?,
                    "proof": self.proof_status_for_item(&item.id)?,
                })),
                "in_review" => in_review.push(json!({
                    "item": item,
                    "open_reviews": self.open_review_items(&item.id)?,
                    "proof": self.proof_status_for_item(&item.id)?,
                })),
                "pending" | "blocked" => blocked.push(json!({
                    "item": item,
                    "blockers": self.blocking_items_for(&item.id)?,
                    "proof": self.proof_status_for_item(&item.id)?,
                })),
                _ => {
                    if item.work_type == "review" && item.status != "closed" {
                        reviews.push(item);
                    }
                }
            }
        }
        Ok(json!({
            "counts": progress["counts"],
            "settled": progress["settled"],
            "total": progress["total"],
            "ready": ready,
            "picked": picked,
            "in_review": in_review,
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
        let proof = self.proof_status_for_item(item_id)?;
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
            && !approval_blocks_close
            && !(proof["active_binding"].as_bool() == Some(true)
                && proof["pass"].as_bool() != Some(true));
        Ok(json!({
            "mode": "preview",
            "action": "close",
            "item": item,
            "can_close": can_close,
            "status_blocks_close": invalid_status,
            "approval_blocks_close": approval_blocks_close,
            "approval": approval,
            "proof_blocks_close": proof["active_binding"].as_bool() == Some(true)
                && proof["pass"].as_bool() != Some(true),
            "proof": proof,
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
             WHERE l.to_item = ?1 AND l.kind IN ('blocks','hands_to') AND i.status NOT IN ('closed','closed_partial')
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
}
