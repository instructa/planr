//! Worker lease ownership: who holds an item, how the next ready item is
//! leased, and the runtime state (heartbeat, progress, pause) of held work.

use super::App;
use crate::execution_run::ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES;
use crate::util::{collect_rows, short_id, worker_id};
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

/// Lease filter for the single pick query. All pick surfaces (CLI, MCP,
/// HTTP, `done --next`) narrow the same query through this one contract.
#[derive(Default)]
pub(crate) struct PickFilter<'a> {
    pub(crate) exclude: Option<&'a str>,
    pub(crate) work_type: Option<&'a str>,
    pub(crate) plan_path: Option<&'a str>,
    pub(crate) ordinary_implementation: bool,
}

impl App {
    pub(crate) fn current_item_for_worker(&self) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT id FROM items WHERE worker_id = ?1 AND status IN ('picked','running','in_review') ORDER BY picked_at DESC LIMIT 1",
                params![worker_id()],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// `exclude` keeps a worker from picking an item it must not own.
    /// ReviewGate work is leased outside the map-item query. `work_type`
    /// and `plan_path` narrow the lease for role-aware and plan-scoped picks.
    /// Read-only counterpart of `pick_next_ready_item_filtered`: the same
    /// candidate selection, no lease, no heartbeat, no event.
    pub(crate) fn peek_next_ready_item_filtered(
        &self,
        filter: &PickFilter<'_>,
    ) -> Result<Option<String>> {
        let project = self.default_project()?;
        self.promote_ready()?;
        let worker = worker_id();
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM items
                 WHERE project_id = ?1 AND status = 'ready'
                 AND id IS NOT ?2
                 AND (?3 = 0 OR work_type IN (?4, ?5, ?6, ?7))
                 AND (?8 IS NULL OR work_type = ?8)
                 AND (?9 IS NULL OR plan_path = ?9)
                 AND (worker_id IS NULL OR worker_id = ?10)
                 AND NOT EXISTS (
                   SELECT 1 FROM plans p
                   JOIN feature_runs run ON run.plan_id = p.id
                   JOIN review_gates gate ON gate.run_id = run.id
                   WHERE p.path = items.plan_path
                   AND run.status IN ('active','held')
                   AND gate.status IN ('pending','leased','changes_requested')
                 )
                 AND NOT EXISTS (
                   SELECT 1 FROM items c WHERE c.parent_item_id = items.id
                   AND c.status NOT IN ('cancelled')
                 )
                 ORDER BY CASE WHEN worker_id = ?10 THEN 0 ELSE 1 END, priority DESC, created_at ASC
                 LIMIT 1",
                params![
                    project.id,
                    filter.exclude,
                    filter.ordinary_implementation,
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[0],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[1],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[2],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[3],
                    filter.work_type,
                    filter.plan_path,
                    worker
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }

    pub(crate) fn pick_next_ready_item_filtered(
        &self,
        filter: &PickFilter<'_>,
    ) -> Result<Option<(String, String)>> {
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
                     AND id IS NOT ?4
                     AND (?5 = 0 OR work_type IN (?6, ?7, ?8, ?9))
                     AND (?10 IS NULL OR work_type = ?10)
                     AND (?11 IS NULL OR plan_path = ?11)
                     AND (worker_id IS NULL OR worker_id = ?1)
                     AND NOT EXISTS (
                       SELECT 1 FROM plans p
                       JOIN feature_runs run ON run.plan_id = p.id
                       JOIN review_gates gate ON gate.run_id = run.id
                       WHERE p.path = items.plan_path
                       AND run.status IN ('active','held')
                       AND gate.status IN ('pending','leased','changes_requested')
                     )
                     AND NOT EXISTS (
                       SELECT 1 FROM items c WHERE c.parent_item_id = items.id
                       AND c.status NOT IN ('cancelled')
                     )
                     ORDER BY CASE WHEN worker_id = ?1 THEN 0 ELSE 1 END, priority DESC, created_at ASC
                     LIMIT 1
                 )
                 AND status = 'ready'
                 RETURNING id",
                params![
                    worker,
                    token,
                    project.id,
                    filter.exclude,
                    filter.ordinary_implementation,
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[0],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[1],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[2],
                    ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES[3],
                    filter.work_type,
                    filter.plan_path
                ],
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

    /// `done` on a never-picked ready item adopts it first: the lease is
    /// written retroactively so completion always carries worker attribution
    /// (review_mode stays derivable) and the in_review transition can never
    /// be skipped silently. Same bookkeeping as a pick, scoped to one id.
    pub(crate) fn adopt_ready_item(&self, item_id: &str) -> Result<bool> {
        if let Some(hold) = self.binding_evidence_hold_for_item(item_id)? {
            bail!(
                "binding_evidence_obligations_missing:{item_id}; next action: {}",
                hold["next_action"]
                    .as_str()
                    .unwrap_or("planr evidence migrate --input <migration-file> --apply")
            );
        }
        let worker = worker_id();
        let token = short_id("pick");
        let adopted = self.conn.execute(
            "UPDATE items
             SET status = 'picked',
                 worker_id = ?1,
                 pick_token = ?2,
                 picked_at = datetime('now'),
                 last_heartbeat_at = datetime('now'),
                 updated_at = datetime('now')
             WHERE id = ?3
             AND status = 'ready'
             AND (worker_id IS NULL OR worker_id = ?1)",
            params![worker, token, item_id],
        )?;
        if adopted == 0 {
            self.ensure_worker_owns_or_unowned(item_id)?;
        }
        if adopted > 0 {
            self.record_event(
                "item_adopted",
                Some(item_id),
                json!({"worker_id": worker, "pick_token": token}),
            )?;
        }
        Ok(adopted > 0)
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
