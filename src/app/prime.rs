//! `planr prime`: the compact state block host hooks inject at session
//! start and after context compaction. One deterministic read over
//! existing data — map counts, the caller's own leases, the goal
//! contract, registry presence, and the next command — kept well under
//! ~1k tokens so injection is cheap. Prime must never break a session:
//! callers wire it fail-open, and a repo without a database exits
//! silently before this code runs (see `main.rs`).

use super::App;
use crate::agents::{RegistryLoad, load_registry};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::json;

impl App {
    pub(crate) fn prime(&self, hook_json: bool) -> Result<()> {
        let Ok(project) = self.default_project() else {
            // Database exists but was never initialized: stay silent,
            // a hook output of noise would be worse than nothing.
            return Ok(());
        };
        let progress = self.progress_value()?;
        let counts = &progress["counts"];
        let mut out = format!(
            "## planr state\nproject: {} | map: {}/{} settled | {} ready, {} picked, {} in_review",
            project.name,
            progress["settled"],
            progress["total"],
            counts["ready"],
            counts["picked"],
            counts["in_review"],
        );

        let worker = crate::util::worker_id();
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.title, i.status,
                    (SELECT COUNT(*) FROM logs l WHERE l.item_id = i.id AND l.kind = 'completion')
             FROM items i
             WHERE i.worker_id = ?1 AND i.status IN ('picked', 'running', 'in_review')
             ORDER BY i.picked_at",
        )?;
        let held: Vec<(String, String, String, i64)> =
            crate::util::collect_rows(stmt.query_map(params![worker], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?)?;
        for (id, title, status, completion_logs) in &held {
            out.push_str(&format!(
                "\nyou hold: {id} [{status}] {title}{}",
                if *completion_logs == 0 {
                    " (no completion log yet)"
                } else {
                    ""
                }
            ));
        }

        let contract: Option<String> = self
            .conn
            .query_row(
                "SELECT content FROM contexts WHERE kind = 'goal-contract' ORDER BY created_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(contract) = contract {
            let mut compact = contract.split_whitespace().collect::<Vec<_>>().join(" ");
            if compact.len() > 300 {
                compact.truncate(297);
                compact.push_str("...");
            }
            out.push_str(&format!("\ngoal contract: {compact}"));
        }

        if let RegistryLoad::Loaded(registry) = load_registry(&self.root) {
            out.push_str(&format!(
                "\nrouting: registry active ({} profile(s); pick packets carry model routing)",
                registry.profiles.len()
            ));
        }

        let ready = counts["ready"].as_i64().unwrap_or(0);
        let next = if !held.is_empty() {
            format!(
                "continue {} (log evidence with planr done, or release)",
                held[0].0
            )
        } else if ready > 0 {
            "planr pick --json".to_string()
        } else {
            "planr map status".to_string()
        };
        out.push_str(&format!("\nnext: {next}"));

        if hook_json {
            // Claude Code SessionStart envelope: additionalContext is
            // injected into the session.
            self.emit(
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "SessionStart",
                        "additionalContext": out,
                    }
                }),
                serde_json::to_string(&json!({
                    "hookSpecificOutput": {
                        "hookEventName": "SessionStart",
                        "additionalContext": out,
                    }
                }))?,
            )
        } else {
            self.emit(json!({"prime": out}), out)
        }
    }
}
