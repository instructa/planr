use super::App;
use crate::cli::RecoverCommand;
use anyhow::{bail, Result};
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(crate) struct ItemRecoveryInput<'a> {
    pub(crate) timeout_seconds: Option<i64>,
    pub(crate) max_retries: Option<i64>,
    pub(crate) retry_backoff: Option<&'a str>,
    pub(crate) retry_delay_ms: Option<i64>,
    pub(crate) pre_condition: Option<&'a str>,
    pub(crate) post_condition: Option<&'a str>,
}

impl App {
    pub(crate) fn recover(&self, command: RecoverCommand) -> Result<()> {
        match command {
            RecoverCommand::Sweep(args) => {
                let value = self.recovery_sweep_value(args.older_than_seconds, args.apply)?;
                let released = value["released"].as_u64().unwrap_or(0);
                let failed = value["failed"].as_u64().unwrap_or(0);
                let retried = value["retried"].as_u64().unwrap_or(0);
                self.emit(
                    value,
                    format!(
                        "recovery sweep: {released} released, {failed} failed, {retried} retried"
                    ),
                )
            }
        }
    }

    pub(crate) fn configure_item_recovery(
        &self,
        item_id: &str,
        input: ItemRecoveryInput<'_>,
    ) -> Result<()> {
        if let Some(value) = input.timeout_seconds {
            if value < 0 {
                bail!("timeout_seconds must be >= 0");
            }
        }
        if let Some(value) = input.max_retries {
            if value < 0 {
                bail!("max_retries must be >= 0");
            }
        }
        if let Some(value) = input.retry_delay_ms {
            if value < 0 {
                bail!("retry_delay_ms must be >= 0");
            }
        }
        if let Some(backoff) = input.retry_backoff {
            if !matches!(backoff, "fixed" | "exponential") {
                bail!("retry_backoff must be fixed or exponential");
            }
        }
        self.conn.execute(
            "UPDATE items
             SET timeout_seconds = COALESCE(?1, timeout_seconds),
                 max_retries = COALESCE(?2, max_retries),
                 retry_backoff = COALESCE(?3, retry_backoff),
                 retry_delay_ms = COALESCE(?4, retry_delay_ms),
                 pre_condition = COALESCE(?5, pre_condition),
                 post_condition = COALESCE(?6, post_condition),
                 updated_at = datetime('now')
             WHERE id = ?7",
            params![
                input.timeout_seconds,
                input.max_retries,
                input.retry_backoff,
                input.retry_delay_ms,
                input.pre_condition,
                input.post_condition,
                item_id
            ],
        )?;
        self.record_event(
            "item_recovery_configured",
            Some(item_id),
            json!({
                "timeout_seconds": input.timeout_seconds,
                "max_retries": input.max_retries,
                "retry_backoff": input.retry_backoff,
                "retry_delay_ms": input.retry_delay_ms,
                "has_pre_condition": input.pre_condition.is_some(),
                "has_post_condition": input.post_condition.is_some()
            }),
        )
    }

    pub(crate) fn item_recovery(&self, item_id: &str) -> Result<Value> {
        self.conn
            .query_row(
                "SELECT timeout_seconds, max_retries, retry_count, retry_backoff, retry_delay_ms
                 FROM items WHERE id = ?1",
                params![item_id],
                |row| {
                    Ok(json!({
                        "timeout_seconds": row.get::<_, Option<i64>>(0)?,
                        "max_retries": row.get::<_, i64>(1)?,
                        "retry_count": row.get::<_, i64>(2)?,
                        "retry_backoff": row.get::<_, String>(3)?,
                        "retry_delay_ms": row.get::<_, i64>(4)?,
                    }))
                },
            )
            .map_err(Into::into)
    }

    pub(crate) fn item_conditions(&self, item_id: &str) -> Result<Value> {
        self.conn
            .query_row(
                "SELECT pre_condition, post_condition FROM items WHERE id = ?1",
                params![item_id],
                |row| {
                    let pre = row.get::<_, Option<String>>(0)?;
                    let post = row.get::<_, Option<String>>(1)?;
                    Ok(json!({
                        "pre": pre,
                        "post": post,
                        "pre_condition_requires_attention": pre.is_some(),
                        "post_condition_requires_manual_verification": post.is_some(),
                    }))
                },
            )
            .map_err(Into::into)
    }

    pub(crate) fn recovery_sweep_value(
        &self,
        older_than_seconds: i64,
        apply: bool,
    ) -> Result<Value> {
        if older_than_seconds < 0 {
            bail!("older_than_seconds must be >= 0");
        }
        let stale = self.recovery_ids(
            "SELECT id FROM items
             WHERE status IN ('picked','running')
             AND datetime(COALESCE(last_heartbeat_at, picked_at, updated_at), '+' || ?1 || ' seconds') < datetime('now')
             ORDER BY COALESCE(last_heartbeat_at, picked_at, updated_at)",
            older_than_seconds,
        )?;
        let timed_out = self.simple_recovery_ids(
            "SELECT id FROM items
             WHERE status IN ('picked','running')
             AND timeout_seconds IS NOT NULL
             AND datetime(COALESCE(last_heartbeat_at, picked_at, updated_at), '+' || timeout_seconds || ' seconds') < datetime('now')
             ORDER BY COALESCE(last_heartbeat_at, picked_at, updated_at)",
        )?;
        let retryable = self.retryable_failed_ids()?;
        let exhausted = self.simple_recovery_ids(
            "SELECT id FROM items
             WHERE status = 'failed'
             AND retry_count >= max_retries
             ORDER BY updated_at",
        )?;

        let mut released = 0usize;
        let mut failed = 0usize;
        let mut retried = 0usize;
        if apply {
            // Timed-out items with a retry budget enter the failed/retry
            // lifecycle; everything else is released straight back to ready.
            let mut release_ids: BTreeSet<String> = stale.iter().cloned().collect();
            for item_id in &timed_out {
                let (max_retries, timeout_seconds): (i64, Option<i64>) = self.conn.query_row(
                    "SELECT max_retries, timeout_seconds FROM items WHERE id = ?1",
                    params![item_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                if max_retries > 0 {
                    release_ids.remove(item_id);
                    self.conn.execute(
                        "UPDATE items
                         SET status = 'failed',
                             error = ?2,
                             worker_id = NULL,
                             pick_token = NULL,
                             last_heartbeat_at = NULL,
                             paused_at = NULL,
                             updated_at = datetime('now')
                         WHERE id = ?1 AND status IN ('picked','running')",
                        params![
                            item_id,
                            format!(
                                "timed out after {} seconds without heartbeat",
                                timeout_seconds.unwrap_or(older_than_seconds)
                            )
                        ],
                    )?;
                    self.record_event(
                        "item_timed_out",
                        Some(item_id),
                        json!({"timeout_seconds": timeout_seconds, "marked": "failed"}),
                    )?;
                    failed += 1;
                } else {
                    release_ids.insert(item_id.clone());
                }
            }
            for item_id in &release_ids {
                self.conn.execute(
                    "UPDATE items
                     SET status = 'ready',
                         worker_id = NULL,
                         pick_token = NULL,
                         last_heartbeat_at = NULL,
                         progress_note = COALESCE(progress_note, 'released by recovery sweep'),
                         paused_at = NULL,
                         updated_at = datetime('now')
                     WHERE id = ?1 AND status IN ('picked','running')",
                    params![item_id],
                )?;
                self.record_event(
                    "recovery_released",
                    Some(item_id),
                    json!({"older_than_seconds": older_than_seconds}),
                )?;
                released += 1;
            }
            for item_id in &retryable {
                self.conn.execute(
                    "UPDATE items
                     SET status = 'ready',
                         retry_count = retry_count + 1,
                         error = NULL,
                         worker_id = NULL,
                         pick_token = NULL,
                         picked_at = NULL,
                         last_heartbeat_at = NULL,
                         paused_at = NULL,
                         updated_at = datetime('now')
                     WHERE id = ?1 AND status = 'failed' AND retry_count < max_retries",
                    params![item_id],
                )?;
                self.record_event("recovery_retry_scheduled", Some(item_id), json!({}))?;
                retried += 1;
            }
            self.promote_ready()?;
        }

        Ok(json!({
            "mode": if apply { "apply" } else { "preview" },
            "older_than_seconds": older_than_seconds,
            "stale": self.recovery_items(&stale)?,
            "timed_out": self.recovery_items(&timed_out)?,
            "retryable": self.recovery_items(&retryable)?,
            "exhausted": self.recovery_items(&exhausted)?,
            "released": released,
            "failed": failed,
            "retried": retried,
        }))
    }

    /// Failed items whose retry budget is open and whose backoff delay has
    /// elapsed. Exponential backoff doubles the base delay per retry.
    fn retryable_failed_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, retry_count, retry_delay_ms, retry_backoff,
                    (julianday('now') - julianday(updated_at)) * 86400000.0
             FROM items
             WHERE status = 'failed' AND retry_count < max_retries
             ORDER BY updated_at",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
            ))
        })?;
        let mut ids = Vec::new();
        for row in rows {
            let (id, retry_count, delay_ms, backoff, elapsed_ms) = row?;
            let factor = if backoff == "exponential" {
                1i64 << retry_count.clamp(0, 20)
            } else {
                1
            };
            let effective_delay_ms = delay_ms.max(0).saturating_mul(factor);
            if elapsed_ms >= effective_delay_ms as f64 {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    fn recovery_ids(&self, sql: &str, value: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![value], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn simple_recovery_ids(&self, sql: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn recovery_items(&self, ids: &[String]) -> Result<Vec<Value>> {
        ids.iter()
            .map(|id| {
                Ok(json!({
                    "item": self.get_item(id)?,
                    "runtime": self.item_runtime(id)?,
                    "recovery": self.item_recovery(id)?,
                    "conditions": self.item_conditions(id)?,
                }))
            })
            .collect()
    }
}
