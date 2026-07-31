use super::super::App;
use crate::model::Project;
use crate::storage::row_to_project;
use crate::util::collect_rows;
use anyhow::{Result, anyhow};
use rusqlite::{OptionalExtension, params};

impl App {
    pub(crate) fn default_project(&self) -> Result<Project> {
        let root = self.root.to_string_lossy().to_string();
        if let Some(project) = self
            .conn
            .query_row(
                "SELECT id, name, root_path, description, status
                 FROM projects
                 WHERE status = 'active' AND root_path = ?1
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![root],
                row_to_project,
            )
            .optional()?
        {
            return Ok(project);
        }
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
}
