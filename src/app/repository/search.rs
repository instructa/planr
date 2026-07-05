use super::super::App;
use crate::util::{query_json, quote_fts};
use anyhow::Result;
use rusqlite::params;
use serde_json::{Value, json};

impl App {
    pub(crate) fn index_search(
        &self,
        source_type: &str,
        source_id: &str,
        title: &str,
        body: &str,
        path: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO search_index(source_type, source_id, title, body, path) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![source_type, source_id, title, body, path],
        )?;
        Ok(())
    }

    pub(crate) fn search_results(&self, query: &str) -> Result<Vec<Value>> {
        let mut results = Vec::new();
        let fts = quote_fts(query);
        let mut stmt = self.conn.prepare(
            "SELECT source_type, source_id, title, body, path FROM search_index WHERE search_index MATCH ?1 ORDER BY rank LIMIT 30",
        )?;
        let rows = stmt.query_map(params![fts], |row| {
            Ok(json!({
                "type": row.get::<_, String>(0)?,
                "id": row.get::<_, String>(1)?,
                "title": row.get::<_, String>(2)?,
                "text": row.get::<_, String>(3)?,
                "path": row.get::<_, Option<String>>(4)?,
            }))
        })?;
        for row in rows {
            results.push(row?);
        }
        if results.is_empty() {
            let like = format!("%{}%", query);
            query_json(
                &self.conn,
                "SELECT 'item', id, title, description FROM items WHERE title LIKE ?1 OR description LIKE ?1 LIMIT 20",
                params![like.clone()],
                &mut results,
            )?;
            query_json(
                &self.conn,
                "SELECT 'plan', id, title, path FROM plans WHERE title LIKE ?1 OR path LIKE ?1 LIMIT 20",
                params![like.clone()],
                &mut results,
            )?;
            query_json(
                &self.conn,
                "SELECT 'log', id, summary, item_id FROM logs WHERE summary LIKE ?1 LIMIT 20",
                params![like.clone()],
                &mut results,
            )?;
            query_json(
                &self.conn,
                "SELECT 'context', id, kind, content FROM contexts WHERE content LIKE ?1 LIMIT 20",
                params![like],
                &mut results,
            )?;
        }
        Ok(results)
    }
}
