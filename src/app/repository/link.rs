use super::super::App;
use crate::model::LinkKind;
use crate::util::collect_rows;
use anyhow::Result;
use rusqlite::params;
use serde_json::{Value, json};

impl App {
    pub(crate) fn add_link(&self, from: &str, to: &str, kind: &str) -> Result<()> {
        let kind = LinkKind::try_from(kind)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, ?3, 'all')",
            params![from, to, kind.as_str()],
        )?;
        self.demote_if_blocked(to)?;
        self.record_event(
            "link_added",
            Some(to),
            json!({"from": from, "to": to, "kind": kind.as_str()}),
        )
    }

    pub(crate) fn all_links(&self) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare("SELECT from_item, to_item, kind FROM links ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            Ok(json!({"from": row.get::<_, String>(0)?, "to": row.get::<_, String>(1)?, "kind": row.get::<_, String>(2)?}))
        })?;
        collect_rows(rows)
    }
}
