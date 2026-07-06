use super::super::App;
use crate::model::{Item, Plan};
use crate::planpack::{extract_work_specs, hash_path, parse_plan_metadata, plan_search_body};
use crate::storage::row_to_plan;
use crate::util::{collect_rows, short_id};
use anyhow::{Result, anyhow};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use std::path::Path;

impl App {
    pub(crate) fn upsert_plan(
        &self,
        project_id: &str,
        stage: &str,
        path: &Path,
        title: &str,
        slug: &str,
        manifest: Value,
    ) -> Result<Plan> {
        let id = short_id("pln");
        let hash = hash_path(path)?;
        let (frontmatter, parse_status) = parse_plan_metadata(path);
        self.conn.execute(
            "INSERT INTO plans(id, project_id, stage, path, title, slug, package_manifest, frontmatter, parse_status, content_hash, archived, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, datetime('now'), datetime('now'))",
            params![id, project_id, stage, path.to_string_lossy(), title, slug, manifest.to_string(), frontmatter.to_string(), parse_status, hash],
        )?;
        self.index_search(
            "plan",
            &id,
            title,
            &plan_search_body(path)?,
            Some(&path.to_string_lossy()),
        )?;
        self.get_plan(&id)
    }

    pub(crate) fn get_plan(&self, id: &str) -> Result<Plan> {
        self.conn
            .query_row(
                "SELECT id, project_id, stage, path, title, slug, parse_status, archived FROM plans WHERE id = ?1",
                params![id],
                row_to_plan,
            )
            .optional()?
            .ok_or_else(|| anyhow!("plan not found: {id}"))
    }

    pub(crate) fn plan_id_for_path(&self, path: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT id FROM plans WHERE path = ?1 AND archived = 0 ORDER BY created_at DESC LIMIT 1",
                params![path],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub(crate) fn list_plans(&self, stage: Option<&str>) -> Result<Vec<Plan>> {
        let sql = if stage.is_some() {
            "SELECT id, project_id, stage, path, title, slug, parse_status, archived FROM plans WHERE archived = 0 AND stage = ?1 ORDER BY created_at DESC"
        } else {
            "SELECT id, project_id, stage, path, title, slug, parse_status, archived FROM plans WHERE archived = 0 ORDER BY created_at DESC"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = if let Some(stage) = stage {
            stmt.query_map(params![stage], row_to_plan)?
        } else {
            stmt.query_map([], row_to_plan)?
        };
        collect_rows(rows)
    }

    pub(crate) fn rehash_plan(&self, id: &str) -> Result<()> {
        let plan = self.get_plan(id)?;
        let path = Path::new(&plan.path);
        let hash = hash_path(path)?;
        let (frontmatter, parse_status) = parse_plan_metadata(path);
        self.conn.execute(
            "UPDATE plans SET content_hash = ?1, frontmatter = ?2, parse_status = ?3, updated_at = datetime('now') WHERE id = ?4",
            params![hash, frontmatter.to_string(), parse_status, id],
        )?;
        Ok(())
    }

    pub(crate) fn seed_items_from_plan(&self, plan: &Plan) -> Result<Vec<Item>> {
        let mut specs = extract_work_specs(Path::new(&plan.path))?;
        if specs.is_empty() {
            specs.push(crate::planpack::WorkSpec {
                title: format!("Implement {}", plan.title),
                description: format!("Execute build plan {}", plan.id),
                work_type: None,
            });
        }
        let mut created = Vec::new();
        for spec in specs {
            let already_seeded: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM source_links sl JOIN items i ON i.id = sl.item_id
                 WHERE sl.source_type = 'plan' AND sl.source_id = ?1
                 AND sl.relationship = 'implements' AND i.title = ?2
                 AND i.status != 'cancelled'",
                params![plan.id, spec.title],
                |row| row.get(0),
            )?;
            if already_seeded > 0 {
                continue;
            }
            // Annotated tasks seed their declared use case so routing
            // binds at map build; unannotated tasks keep `code`.
            let item = self.create_item(
                None,
                &spec.title,
                &spec.description,
                spec.work_type.as_deref().unwrap_or("code"),
                Some(&plan.path),
            )?;
            self.conn.execute(
                "INSERT INTO source_links(source_type, source_id, item_id, section_id, relationship) VALUES ('plan', ?1, ?2, NULL, 'implements')",
                params![plan.id, item.id],
            )?;
            created.push(item);
        }
        for pair in created.windows(2) {
            self.add_link(&pair[0].id, &pair[1].id, "blocks")?;
        }
        Ok(created)
    }
}
