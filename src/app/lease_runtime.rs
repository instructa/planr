//! Runtime state maintenance for leased work.

use super::App;
use crate::util::{collect_rows, worker_id};
use anyhow::{Result, bail};
use rusqlite::params;
use serde_json::{Value, json};

impl App {
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
            let repair = match item.status.as_str() {
                "ready" => "lease it first: `planr pick`",
                "pending" | "blocked" => {
                    "settle its blockers first (`planr trace item <id> --json` lists them)"
                }
                "closed" | "closed_partial" | "cancelled" | "failed" => {
                    "the item is settled; create a follow-up with `planr item create` instead"
                }
                _ => "lease it first: `planr pick`",
            };
            bail!(
                "invalid_transition: item {} is {}; {}",
                item.id,
                item.status,
                repair
            );
        }
        Ok(())
    }

    pub(crate) fn heartbeat_item(&self, item_id: &str) -> Result<()> {
        self.ensure_worker_owns_or_unowned(item_id)?;
        let changed = self.conn.execute(
            "UPDATE items SET status = CASE WHEN status = 'picked' THEN 'running' ELSE status END, last_heartbeat_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1 AND status IN ('picked','running','in_review')",
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
}
