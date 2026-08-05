use super::super::App;
use crate::model::Project;
use crate::storage::row_to_project;
use crate::util::collect_rows;
use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ProjectPathUpdate {
    pub(crate) id: String,
    pub(crate) from: String,
    pub(crate) to: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ProjectRelocation {
    pub(crate) project: ProjectPathUpdate,
    pub(crate) plans: Vec<ProjectPathUpdate>,
    pub(crate) items: Vec<ProjectPathUpdate>,
}

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

    pub(crate) fn preview_project_relocation(
        &self,
        project_id: &str,
        destination: &Path,
    ) -> Result<ProjectRelocation> {
        project_relocation_projection(&self.conn, project_id, destination)
    }

    pub(crate) fn apply_project_relocation(
        &self,
        project_id: &str,
        destination: &Path,
    ) -> Result<ProjectRelocation> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let relocation = project_relocation_projection(&tx, project_id, destination)?;
        require_single_update(
            tx.execute(
                "UPDATE projects SET root_path = ?1, updated_at = datetime('now') WHERE id = ?2 AND root_path = ?3",
                params![relocation.project.to, relocation.project.id, relocation.project.from],
            )?,
            "project",
            &relocation.project.id,
        )?;
        for update in &relocation.plans {
            require_single_update(
                tx.execute(
                    "UPDATE plans SET path = ?1, updated_at = datetime('now') WHERE id = ?2 AND project_id = ?3 AND path = ?4",
                    params![update.to, update.id, project_id, update.from],
                )?,
                "plan",
                &update.id,
            )?;
        }
        for update in &relocation.items {
            require_single_update(
                tx.execute(
                    "UPDATE items SET plan_path = ?1, updated_at = datetime('now') WHERE id = ?2 AND project_id = ?3 AND plan_path = ?4",
                    params![update.to, update.id, project_id, update.from],
                )?,
                "item",
                &update.id,
            )?;
        }
        validate_relocated_references(&tx, project_id)?;
        tx.commit()?;
        Ok(relocation)
    }
}

fn project_relocation_projection(
    conn: &Connection,
    project_id: &str,
    destination: &Path,
) -> Result<ProjectRelocation> {
    let destination = destination.canonicalize().with_context(|| {
        format!(
            "project_relocation_destination_invalid:{}",
            destination.display()
        )
    })?;
    if !destination.is_dir() {
        bail!(
            "project_relocation_destination_not_directory:{}",
            destination.display()
        );
    }
    let project = conn
        .query_row(
            "SELECT id, name, root_path, description, status FROM projects WHERE id = ?1",
            params![project_id],
            row_to_project,
        )
        .optional()?
        .ok_or_else(|| anyhow!("project not found: {project_id}"))?;
    let source = PathBuf::from(&project.root_path);
    if !source.is_absolute() {
        bail!(
            "project_relocation_source_not_absolute:{}",
            source.display()
        );
    }
    if source == destination {
        bail!(
            "project_relocation_destination_unchanged:{}",
            destination.display()
        );
    }
    let destination_string = destination.to_string_lossy().to_string();
    let collision: Option<String> = conn
        .query_row(
            "SELECT id FROM projects WHERE id <> ?1 AND status = 'active' AND root_path = ?2 LIMIT 1",
            params![project_id, destination_string],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = collision {
        bail!("project_relocation_destination_owned:{id}");
    }

    let mut stmt = conn.prepare("SELECT id, path FROM plans WHERE project_id = ?1 ORDER BY id")?;
    let plan_rows = stmt
        .query_map(params![project_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut path_map = BTreeMap::new();
    let mut plans = Vec::with_capacity(plan_rows.len());
    for (id, from) in plan_rows {
        let relative = Path::new(&from)
            .strip_prefix(&source)
            .map_err(|_| anyhow!("project_relocation_plan_outside_source:{id}:{from}"))?;
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            bail!("project_relocation_relative_path_component_rejected:{id}:{from}");
        }
        let to = destination.join(relative);
        let to = to.canonicalize().with_context(|| {
            format!(
                "project_relocation_plan_missing_at_destination:{id}:{}",
                to.display()
            )
        })?;
        if !to.starts_with(&destination) {
            bail!(
                "project_relocation_destination_escape:{id}:{}",
                to.display()
            );
        }
        let to = to.to_string_lossy().to_string();
        path_map.insert(from.clone(), to.clone());
        plans.push(ProjectPathUpdate { id, from, to });
    }

    let mut stmt = conn.prepare(
        "SELECT id, plan_path FROM items WHERE project_id = ?1 AND plan_path IS NOT NULL ORDER BY id",
    )?;
    let item_rows = stmt
        .query_map(params![project_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut items = Vec::with_capacity(item_rows.len());
    for (id, from) in item_rows {
        let to = path_map
            .get(&from)
            .cloned()
            .ok_or_else(|| anyhow!("project_relocation_dangling_item_plan_path:{id}:{from}"))?;
        items.push(ProjectPathUpdate { id, from, to });
    }

    Ok(ProjectRelocation {
        project: ProjectPathUpdate {
            id: project.id,
            from: project.root_path,
            to: destination_string,
        },
        plans,
        items,
    })
}

fn require_single_update(changed: usize, entity: &str, id: &str) -> Result<()> {
    if changed != 1 {
        bail!("project_relocation_concurrent_{entity}_change:{id}");
    }
    Ok(())
}

fn validate_relocated_references(conn: &Connection, project_id: &str) -> Result<()> {
    let dangling: Option<(String, String)> = conn
        .query_row(
            "SELECT items.id, items.plan_path FROM items
             LEFT JOIN plans ON plans.project_id = items.project_id AND plans.path = items.plan_path
             WHERE items.project_id = ?1 AND items.plan_path IS NOT NULL AND plans.id IS NULL
             LIMIT 1",
            params![project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((id, path)) = dangling {
        bail!("project_relocation_reference_integrity_failed:{id}:{path}");
    }
    Ok(())
}
