use super::lease::PickFilter;
use super::App;
use crate::storage::row_to_item;
use crate::util::collect_rows;
use anyhow::Result;
use rusqlite::params;
use serde_json::{json, Value};

impl App {
    /// Picks the next ready item and returns it as a flat work packet, or a
    /// null pick with `reason` and `remaining` when nothing is pickable.
    /// `work_type` restricts the lease for role-aware picks (checkers lease
    /// only reviews), `plan` (a plan id) restricts it to one plan's items so
    /// goal runs never lease work outside their contract, and `exclude`
    /// keeps a worker from picking an item it must not own.
    pub(crate) fn next_pick_value(
        &self,
        exclude: Option<&str>,
        work_type: Option<&str>,
        plan: Option<&str>,
    ) -> Result<Value> {
        let plan_path = plan.map(|id| self.get_plan(id)).transpose()?;
        let filter = PickFilter {
            exclude,
            work_type,
            plan_path: plan_path.as_ref().map(|plan| plan.path.as_str()),
        };
        if let Some((id, worker)) = self.pick_next_ready_item_filtered(&filter)? {
            self.work_packet(&id, &worker)
        } else {
            let remaining = self.progress_value()?;
            let total = remaining["total"].as_i64().unwrap_or(0);
            let settled = remaining["settled"].as_i64().unwrap_or(0);
            let ready = remaining["counts"]["ready"].as_i64().unwrap_or(0);
            let reason = if total == 0 {
                "empty_map"
            } else if settled == total {
                "all_settled"
            } else if ready == 0 {
                "nothing_ready"
            } else if plan.is_some() {
                "no_ready_item_in_plan"
            } else if work_type.is_some() {
                "no_ready_item_of_work_type"
            } else {
                "ready_items_not_pickable"
            };
            Ok(json!({"item": null, "reason": reason, "remaining": remaining}))
        }
    }

    /// The flat pick work packet: each fact appears exactly once. Base is
    /// the trace packet (item, links, logs, runtime, recovery, conditions,
    /// approval) extended with recall context and board progress. Empty
    /// collections and null sub-objects are omitted — a missing key means
    /// "empty"; `remaining.counts` always carries the full status vocabulary.
    pub(crate) fn work_packet(&self, item_id: &str, worker: &str) -> Result<Value> {
        let mut packet = self.trace_item_value(item_id)?;
        // Worker identity lives in `item.worker_id` and `runtime.worker_id`
        // already; `worker` is the same value, so no third top-level copy.
        debug_assert_eq!(packet["item"]["worker_id"].as_str(), Some(worker));
        let mut stmt = self.conn.prepare("SELECT source_type, source_id, section_id, relationship FROM source_links WHERE item_id = ?1 ORDER BY id")?;
        let source_links = collect_rows(stmt.query_map(params![item_id], |row| {
            Ok(json!({
                "source_type": row.get::<_, String>(0)?,
                "source_id": row.get::<_, String>(1)?,
                "section_id": row.get::<_, Option<String>>(2)?,
                "relationship": row.get::<_, String>(3)?,
            }))
        })?)?;
        packet["source_links"] = json!(source_links);
        packet["contexts"] = json!(self.list_contexts(Some(item_id))?);
        packet["relevant_contexts"] = json!(self.relevant_contexts_for_pick(item_id, 5)?);
        packet["upstream_handoffs"] = json!(self.upstream_handoffs(item_id, 5)?);
        packet["review_history"] = json!(self.review_history(item_id)?);
        let close_effect = self.close_effect(item_id)?;
        packet["close_effect"] = json!({
            "would_unlock": close_effect.would_unlock,
            "would_remain_blocked": close_effect.would_remain_blocked,
        });
        packet["possible_file_conflicts"] = json!(self.possible_file_conflicts(item_id)?);
        packet["privacy"] = json!({
            "source_file_content_included": false,
            "prompt_or_response_content_included": false,
            "large_artifact_content_included": false,
            "secret_like_contexts_omitted": true
        });
        packet["deeper_reads"] = json!([
            format!("planr trace item {item_id} --json"),
            "planr map show --json",
            format!("planr log list --item {item_id} --json"),
            format!("planr context list --item {item_id} --json"),
            "planr search <query> --json"
        ]);
        packet["remaining"] = self.progress_value()?;
        if packet["approval"]["status"].is_null() {
            packet["approval"] = Value::Null;
        }
        if packet["conditions"]["pre"].is_null() && packet["conditions"]["post"].is_null() {
            packet["conditions"] = Value::Null;
        }
        if let Value::Object(map) = &mut packet {
            map.retain(|_, value| {
                !value.is_null() && !value.as_array().is_some_and(|array| array.is_empty())
            });
        }
        Ok(packet)
    }

    pub(crate) fn relevant_contexts_for_pick(
        &self,
        item_id: &str,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let item = self.get_item(item_id)?;
        let tokens = recall_tokens(&format!("{} {}", item.title, item.description));
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let mut ranked = Vec::new();
        for context in self.list_contexts(None)? {
            if context.get("item_id").and_then(Value::as_str) == Some(item_id) {
                continue;
            }
            let content = context.get("content").and_then(Value::as_str).unwrap_or("");
            if looks_secret_like(content) {
                continue;
            }
            let kind = context.get("kind").and_then(Value::as_str).unwrap_or("");
            let score = recall_score(&tokens, &format!("{kind} {content}"));
            if score == 0 {
                continue;
            }
            ranked.push((
                score,
                json!({
                    "id": context["id"],
                    "kind": context["kind"],
                    "item_id": context["item_id"],
                    "score": score,
                    "content": compact_text(content, 320),
                    "created_at": context["created_at"],
                }),
            ));
        }
        ranked.sort_by(|(a_score, a), (b_score, b)| {
            b_score
                .cmp(a_score)
                .then_with(|| b["created_at"].as_str().cmp(&a["created_at"].as_str()))
        });
        Ok(ranked
            .into_iter()
            .take(limit.max(1))
            .map(|(_, value)| value)
            .collect())
    }

    pub(crate) fn upstream_handoffs(&self, item_id: &str, limit: usize) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT upstream.id, upstream.project_id, upstream.parent_item_id, upstream.title, upstream.description, upstream.status, upstream.work_type, upstream.priority, upstream.worker_id, upstream.plan_path
             FROM links l JOIN items upstream ON upstream.id = l.from_item
             WHERE l.to_item = ?1 AND l.kind IN ('blocks','hands_to')
             ORDER BY upstream.updated_at DESC, upstream.created_at DESC LIMIT ?2",
        )?;
        let upstream = collect_rows(stmt.query_map(params![item_id, limit as i64], row_to_item)?)?;
        upstream
            .iter()
            .map(|item| {
                Ok(json!({
                    "item": item,
                    "logs": self.list_logs(Some(&item.id))?.into_iter().take(3).map(compact_log).collect::<Vec<_>>(),
                }))
            })
            .collect()
    }

    pub(crate) fn review_history(&self, item_id: &str) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.project_id, r.parent_item_id, r.title, r.description, r.status, r.work_type, r.priority, r.worker_id, r.plan_path
             FROM links l JOIN items r ON r.id = l.from_item
             WHERE l.to_item = ?1 AND l.kind = 'reviews'
             ORDER BY r.updated_at DESC, r.created_at DESC LIMIT 10",
        )?;
        let reviews = collect_rows(stmt.query_map(params![item_id], row_to_item)?)?;
        reviews
            .iter()
            .map(|review| {
                Ok(json!({
                    "review": review,
                    "logs": self.list_logs(Some(&review.id))?.into_iter().take(3).map(compact_log).collect::<Vec<_>>(),
                }))
            })
            .collect()
    }

    pub(crate) fn possible_file_conflicts(&self, item_id: &str) -> Result<Vec<Value>> {
        let current_files = files_for_item_logs(&self.list_logs(Some(item_id))?);
        if current_files.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT id FROM items WHERE id != ?1 AND status IN ('picked','running') ORDER BY updated_at DESC LIMIT 25",
        )?;
        let active_ids =
            collect_rows(stmt.query_map(params![item_id], |row| row.get::<_, String>(0))?)?;
        let mut conflicts = Vec::new();
        for active_id in active_ids {
            let active_files = files_for_item_logs(&self.list_logs(Some(&active_id))?);
            let overlap = current_files
                .iter()
                .filter(|file| active_files.contains(*file))
                .cloned()
                .collect::<Vec<_>>();
            if !overlap.is_empty() {
                conflicts.push(json!({
                    "item": self.get_item(&active_id)?,
                    "files": overlap,
                }));
            }
        }
        Ok(conflicts)
    }
}

fn recall_tokens(input: &str) -> Vec<String> {
    let mut tokens = input
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 3)
        .map(str::to_ascii_lowercase)
        .filter(|token| {
            !matches!(
                token.as_str(),
                "the"
                    | "and"
                    | "for"
                    | "with"
                    | "this"
                    | "that"
                    | "from"
                    | "into"
                    | "item"
                    | "task"
                    | "plan"
                    | "build"
                    | "implement"
                    | "create"
            )
        })
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens.truncate(12);
    tokens
}

fn recall_score(tokens: &[String], text: &str) -> usize {
    let haystack = text.to_ascii_lowercase();
    tokens
        .iter()
        .map(|token| haystack.matches(token).count())
        .sum()
}

fn looks_secret_like(text: &str) -> bool {
    ["sk-", "ghp_", "BEGIN PRIVATE KEY", "AKIA"]
        .iter()
        .any(|pattern| text.contains(pattern))
}

/// Truncates at a word boundary so handoff summaries never cut a token in
/// half; the marker tells agents the full text is one `log list` away.
fn compact_text(text: &str, max_chars: usize) -> String {
    let mut compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() > max_chars {
        let mut budget = max_chars.saturating_sub(12).max(1);
        while !compact.is_char_boundary(budget) {
            budget -= 1;
        }
        let cut = compact[..budget]
            .rfind(' ')
            .filter(|index| *index > budget / 2)
            .unwrap_or(budget);
        compact.truncate(cut);
        compact.push_str(" [truncated]");
    }
    compact
}

fn compact_log(log: Value) -> Value {
    json!({
        "id": log["id"],
        "item_id": log["item_id"],
        "kind": log["kind"],
        "summary": log["summary"].as_str().map(|text| compact_text(text, 320)),
        "files": log["files"],
        "commands": log["commands"],
        "tests": log["tests"],
        "review_findings": log["review_findings"],
        "created_at": log["created_at"],
    })
}

fn files_for_item_logs(logs: &[Value]) -> Vec<String> {
    let mut files = logs
        .iter()
        .flat_map(|log| {
            log.get("files")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|file| file.as_str().map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    files
}
