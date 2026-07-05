use super::super::App;
use crate::model::Item;
use crate::storage::row_to_item;
use crate::util::{collect_rows, item_id};
use anyhow::{Result, anyhow};
use rusqlite::{OptionalExtension, params};
use serde_json::json;

impl App {
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

    /// The item's pinned routing profile id, if any. Lives outside the
    /// `Item` model because only routing reads it; every other item
    /// consumer stays untouched by the column.
    pub(crate) fn item_route_override(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT route_override FROM items WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Runs recorded for an item, oldest first. Read by the trace routing
    /// section; runs were write-only before declared-vs-actual auditing.
    pub(crate) fn item_runs(&self, item_id: &str) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, client, profile, status, started_at, observed_client FROM runs WHERE item_id = ?1 ORDER BY started_at, rowid",
        )?;
        let rows = stmt.query_map(params![item_id], |row| {
            let mut run = json!({
                "id": row.get::<_, String>(0)?,
                "client": row.get::<_, String>(1)?,
                "profile": row.get::<_, Option<String>>(2)?,
                "status": row.get::<_, String>(3)?,
                "started_at": row.get::<_, Option<String>>(4)?,
            });
            // Key omitted when unknown so pre-feature traces stay
            // byte-identical.
            if let Some(observed) = row.get::<_, Option<String>>(5)? {
                run["observed_client"] = json!(observed);
            }
            Ok(run)
        })?;
        collect_rows(rows)
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
}
