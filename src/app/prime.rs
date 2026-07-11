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

/// Char-boundary-safe truncation: prime must never panic on non-ASCII
/// content (it runs inside session hooks).
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

impl App {
    pub(crate) fn prime(&self, envelope: crate::cli::PrimeEnvelope) -> Result<()> {
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
        // Bounded output: prime is injected context, so a worker with
        // many stale leases must not blow the token budget.
        for (id, title, status, completion_logs) in held.iter().take(5) {
            out.push_str(&format!(
                "\nyou hold: {id} [{status}] {}{}",
                truncate_chars(title, 80),
                if *completion_logs == 0 {
                    " (no completion log yet)"
                } else {
                    ""
                }
            ));
        }
        if held.len() > 5 {
            out.push_str(&format!("\n(+{} more held items)", held.len() - 5));
        }

        let contract: Option<String> = self
            .conn
            .query_row(
                "SELECT content FROM contexts WHERE kind = 'goal-contract' ORDER BY created_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let mut contract_plan: Option<String> = None;
        if let Some(contract) = &contract {
            let compact = contract.split_whitespace().collect::<Vec<_>>().join(" ");
            contract_plan = compact
                .split_whitespace()
                .find(|word| word.starts_with("pln-"))
                .map(|word| {
                    word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                        .to_string()
                });
            out.push_str(&format!(
                "\ngoal contract: {}",
                truncate_chars(&compact, 300)
            ));
        }

        if let RegistryLoad::Loaded(registry) = load_registry(&self.root) {
            out.push_str(&format!(
                "\nrouting: registry active ({} profile(s); pick packets carry model routing)",
                registry.profiles.len()
            ));
        }

        let ready = counts["ready"].as_i64().unwrap_or(0);
        let settled = progress["settled"].as_i64().unwrap_or(0);
        let total = progress["total"].as_i64().unwrap_or(0);
        let next = if !held.is_empty() {
            format!(
                "continue {} (log evidence with planr done, or release)",
                held[0].0
            )
        } else if ready > 0 {
            "planr pick --json".to_string()
        } else if total > 0 && settled == total {
            // Everything settled: the useful next step is the contract
            // verdict, not another status read.
            match &contract_plan {
                Some(plan) => format!("planr plan audit {plan} --json (all settled)"),
                None => {
                    "all settled — start new work with planr plan new or item create".to_string()
                }
            }
        } else {
            "planr map status".to_string()
        };
        out.push_str(&format!("\nnext: {next}"));

        match envelope {
            // Claude Code SessionStart envelope: additionalContext is
            // injected into the session. Only wired for SessionStart
            // (matcher includes `compact`, which covers post-compaction).
            crate::cli::PrimeEnvelope::HookJson => {
                let value = json!({
                    "hookSpecificOutput": {
                        "hookEventName": "SessionStart",
                        "additionalContext": out,
                    }
                });
                let human = serde_json::to_string(&value)?;
                self.emit(value, human)
            }
            // Cursor command-hook envelope for context injection.
            crate::cli::PrimeEnvelope::CursorJson => {
                let value = json!({"additional_context": out});
                let human = serde_json::to_string(&value)?;
                self.emit(value, human)
            }
            crate::cli::PrimeEnvelope::Plain => self.emit(json!({"prime": out}), out),
        }
    }
}
