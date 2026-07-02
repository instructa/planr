use super::super::{App, artifact_row, event_row};
use crate::storage::row_to_log;
use crate::util::{collect_rows, worker_id};
use anyhow::{Result, anyhow};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

impl App {
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
