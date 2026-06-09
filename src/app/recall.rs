use super::App;
use crate::storage::row_to_item;
use crate::util::collect_rows;
use anyhow::Result;
use rusqlite::params;
use serde_json::{json, Value};

impl App {
    pub(crate) fn pick_context(&self, item_id: &str) -> Result<Value> {
        let item = self.get_item(item_id)?;
        let links = self.links_for(item_id)?;
        let contexts = self.list_contexts(Some(item_id))?;
        let mut stmt = self.conn.prepare("SELECT source_type, source_id, section_id, relationship FROM source_links WHERE item_id = ?1 ORDER BY id")?;
        let source_links = collect_rows(stmt.query_map(params![item_id], |row| {
            Ok(json!({
                "source_type": row.get::<_, String>(0)?,
                "source_id": row.get::<_, String>(1)?,
                "section_id": row.get::<_, Option<String>>(2)?,
                "relationship": row.get::<_, String>(3)?,
            }))
        })?)?;
        let close_effect = self.close_effect(item_id)?;
        Ok(json!({
            "summary": {
                "item_id": item.id,
                "title": item.title,
                "work_type": item.work_type,
                "status": item.status,
            },
            "links": links,
            "source_links": source_links,
            "contexts": contexts,
            "relevant_contexts": self.relevant_contexts_for_pick(item_id, 5)?,
            "upstream_handoffs": self.upstream_handoffs(item_id, 5)?,
            "review_history": self.review_history(item_id)?,
            "recovery": self.item_recovery(item_id)?,
            "conditions": self.item_conditions(item_id)?,
            "close_effect": {
                "would_unlock": close_effect.would_unlock,
                "would_remain_blocked": close_effect.would_remain_blocked,
            },
            "possible_file_conflicts": self.possible_file_conflicts(item_id)?,
            "privacy": {
                "source_file_content_included": false,
                "prompt_or_response_content_included": false,
                "large_artifact_content_included": false,
                "secret_like_contexts_omitted": true
            },
            "deeper_reads": [
                format!("planr trace item {item_id}"),
                "planr map show --json",
                format!("planr log list --item {item_id}"),
                format!("planr context list --item {item_id}"),
                "planr search <query>"
            ]
        }))
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
             WHERE l.to_item = ?1 AND l.kind IN ('blocks','feeds_into')
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

fn compact_text(text: &str, max_chars: usize) -> String {
    let mut compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() > max_chars {
        compact.truncate(max_chars.saturating_sub(3));
        compact.push_str("...");
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
