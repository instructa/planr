use super::App;
use crate::cli::DoneArgs;
use crate::model::Item;
use crate::util::{short_id, worker_id};
use anyhow::{anyhow, Result};
use rusqlite::params;
use serde_json::{json, Value};

/// Owner of the compound work flow: evidence logging, the close transition,
/// review requests, and the `done` command that chains them. CLI, HTTP, and
/// MCP surfaces call these instead of duplicating the steps.
impl App {
    /// Single owner for writing evidence logs. Logging from the pick owner
    /// also refreshes the runtime heartbeat: evidence is a liveness signal,
    /// so agents do not need a separate `pick heartbeat` call.
    pub(crate) fn add_log_entry(
        &self,
        item_id: &str,
        kind: &str,
        summary: &str,
        files: &[String],
        cmd: &[String],
        tests: &[String],
    ) -> Result<String> {
        let id = short_id("log");
        let run_id = if cmd.is_empty() && tests.is_empty() {
            None
        } else {
            Some(self.record_run(item_id, cmd, "closed")?)
        };
        self.conn.execute(
            "INSERT INTO logs(id, project_id, item_id, run_id, kind, summary, files, commands, tests, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
            params![
                id,
                self.default_project()?.id,
                item_id,
                run_id,
                kind,
                summary,
                serde_json::to_string(
                    &files
                        .iter()
                        .map(|file| file.trim())
                        .filter(|file| !file.is_empty())
                        .collect::<Vec<_>>(),
                )?,
                serde_json::to_string(cmd)?,
                serde_json::to_string(tests)?,
            ],
        )?;
        self.index_search("log", &id, summary, summary, None)?;
        self.record_event(
            "log_created",
            Some(item_id),
            json!({"log_id": id, "kind": kind}),
        )?;
        self.conn.execute(
            "UPDATE items SET status = CASE WHEN status = 'picked' THEN 'running' ELSE status END, last_heartbeat_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1 AND worker_id = ?2 AND status IN ('picked','running','in_review')",
            params![item_id, worker_id()],
        )?;
        Ok(id)
    }

    /// Single owner for the close transition. Reconciles gate state first so
    /// a parent whose children are already settled is closable instead of
    /// stuck in `blocked`, then validates, closes, and promotes unlocked work.
    pub(crate) fn close_item_core(
        &self,
        item_id: &str,
        summary: &str,
        write_log: bool,
    ) -> Result<Option<String>> {
        self.promote_ready()?;
        self.ensure_can_close(item_id)?;
        self.conn.execute("UPDATE items SET status = 'closed', completed_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1", params![item_id])?;
        let log_id = if write_log {
            Some(self.add_log_entry(item_id, "completion", summary, &[], &[], &[])?)
        } else {
            None
        };
        self.promote_ready()?;
        self.record_event(
            "item_closed",
            Some(item_id),
            json!({"log_id": log_id, "summary": summary}),
        )?;
        Ok(log_id)
    }

    /// Single owner for creating a review gate on an item. A picked or
    /// running target moves to `in_review` (ownership kept) so the wait
    /// state is visible instead of masquerading as active work.
    pub(crate) fn request_review_for(&self, item_id: &str) -> Result<Item> {
        let target = self.get_item(item_id)?;
        let review = self.create_item(
            None,
            &format!("Review {}", target.title),
            "Review item against plan, logs, diff, and verification.",
            "review",
            target.plan_path.as_deref(),
        )?;
        self.add_link(&review.id, &target.id, "reviews")?;
        self.conn.execute(
            "UPDATE items SET status = 'in_review', updated_at = datetime('now') WHERE id = ?1 AND status IN ('picked','running')",
            params![target.id],
        )?;
        self.promote_ready()?;
        let review = self.get_item(&review.id)?;
        self.record_event(
            "review_requested",
            Some(&target.id),
            json!({"review_id": review.id.clone()}),
        )?;
        Ok(review)
    }

    /// Board progress snapshot included in `done` and `close` responses so a
    /// loop agent can evaluate its stop condition without an extra
    /// `map status` call.
    pub(crate) fn progress_value(&self) -> Result<Value> {
        let mut stmt = self
            .conn
            .prepare("SELECT status, COUNT(*) FROM items GROUP BY status ORDER BY status")?;
        let rows: Vec<(String, i64)> =
            crate::util::collect_rows(stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?)?;
        let total: i64 = rows.iter().map(|(_, n)| n).sum();
        let settled: i64 = rows
            .iter()
            .filter(|(status, _)| {
                matches!(status.as_str(), "closed" | "closed_partial" | "cancelled")
            })
            .map(|(_, n)| n)
            .sum();
        // Counts always carry the full status vocabulary, so consumers never
        // have to treat a missing key as zero.
        const STATUS_VOCABULARY: [&str; 10] = [
            "pending",
            "ready",
            "picked",
            "running",
            "in_review",
            "blocked",
            "failed",
            "cancelled",
            "closed",
            "closed_partial",
        ];
        let mut counts = serde_json::Map::new();
        for status in STATUS_VOCABULARY {
            counts.insert(status.to_string(), json!(0));
        }
        for (status, n) in rows {
            counts.insert(status, json!(n));
        }
        Ok(json!({"counts": counts, "settled": settled, "total": total}))
    }

    pub(crate) fn progress_human(progress: &Value) -> String {
        let ready = progress["counts"]["ready"].as_i64().unwrap_or(0);
        format!(
            " [{}/{} settled · {} ready]",
            progress["settled"], progress["total"], ready
        )
    }

    /// Picks the next ready item and returns it as a JSON work packet, or
    /// `{"item": null}` when nothing is ready.
    pub(crate) fn next_pick_value(&self) -> Result<Value> {
        self.next_pick_value_excluding(None)
    }

    pub(crate) fn next_pick_value_excluding(&self, exclude: Option<&str>) -> Result<Value> {
        if let Some((id, worker)) = self.pick_next_ready_item_excluding(exclude)? {
            self.work_packet(&id, &worker)
        } else {
            Ok(json!({"item": null}))
        }
    }

    /// `planr done`: completion log, then review request or close, then an
    /// optional next pick — one command instead of three, same evidence.
    pub(crate) fn done(&self, args: DoneArgs) -> Result<()> {
        let item_id = if let Some(id) = args.item_id {
            id
        } else {
            self.current_item_for_worker()?
                .ok_or_else(|| anyhow!("no picked item for this worker"))?
        };
        let log_id = self.add_log_entry(
            &item_id,
            "completion",
            &args.summary,
            &args.files,
            &args.cmd,
            &args.tests,
        )?;
        let review = if args.review {
            Some(self.request_review_for(&item_id)?)
        } else {
            self.close_item_core(&item_id, &args.summary, false)?;
            None
        };
        let next = if args.next {
            // A worker must not pick the review it just requested:
            // maker and checker stay separate.
            Some(self.next_pick_value_excluding(review.as_ref().map(|r| r.id.as_str()))?)
        } else {
            None
        };
        let mut human = if let Some(review) = &review {
            format!("logged {item_id} and requested review {}", review.id)
        } else {
            format!("logged and closed {item_id}")
        };
        if let Some(next) = &next {
            match next["item"]["id"].as_str() {
                Some(next_id) => human.push_str(&format!("; picked {next_id}")),
                None => human.push_str("; no ready item"),
            }
        }
        let progress = self.progress_value()?;
        human.push_str(&Self::progress_human(&progress));
        self.emit(
            json!({
                "item": self.get_item(&item_id)?,
                "log_id": log_id,
                "review": review,
                "closed": if args.review { Value::Null } else { json!(item_id) },
                "next": next,
                "remaining": progress,
            }),
            human,
        )
    }
}
