use super::super::App;
use crate::storage::row_to_context;
use crate::util::collect_rows;
use anyhow::{Result, anyhow};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

impl App {
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
}
