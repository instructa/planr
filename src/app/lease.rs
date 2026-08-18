//! Worker lease selection: how the next ready item is leased or adopted.

use super::App;
use crate::execution_run::ORDINARY_IMPLEMENTATION_WORK_TYPE_NAMES;
use crate::util::{short_id, worker_id};
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::json;

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

}
