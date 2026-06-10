//! Goal contract audit: clause-by-clause verdict over a plan's map scope.
//! One command answers "does the contract hold?" with evidence instead of
//! forcing agents to stitch the verdict together from map/log/approval calls.

use super::App;
use crate::storage::row_to_item;
use crate::util::collect_rows;
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

impl App {
    pub(crate) fn plan_audit_value(&self, plan_id: &str) -> Result<Value> {
        let plan = self.get_plan(plan_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path FROM items WHERE plan_path = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![plan.path], row_to_item)?;
        let scope = collect_rows(rows)?;

        let open_evidence = |items: &[crate::model::Item]| -> Vec<Value> {
            items
                .iter()
                .map(|item| json!({"id": item.id, "status": item.status, "title": item.title}))
                .collect()
        };
        let is_open = |status: &str| !matches!(status, "closed" | "closed_partial" | "cancelled");

        let open_items: Vec<_> = scope
            .iter()
            .filter(|item| is_open(&item.status))
            .cloned()
            .collect();
        let open_reviews: Vec<_> = open_items
            .iter()
            .filter(|item| item.work_type == "review")
            .cloned()
            .collect();
        let items_clause = if scope.is_empty() {
            json!({"clause": "items_settled", "pass": false, "open": [], "detail": format!("no map items exist for this plan; run `planr map build --from {plan_id}` first")})
        } else {
            json!({"clause": "items_settled", "pass": open_items.is_empty(), "open": open_evidence(&open_items)})
        };

        let approval_blocked: Vec<Value> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, status, approval_status FROM items WHERE plan_path = ?1 AND approval_status IN ('requested','denied') ORDER BY created_at",
            )?;
            let rows = stmt.query_map(params![plan.path], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "status": row.get::<_, String>(1)?,
                    "approval_status": row.get::<_, String>(2)?,
                }))
            })?;
            collect_rows(rows)?
        };

        let verification_logs: Vec<Value> = {
            let mut stmt = self.conn.prepare(
                "SELECT l.id, l.item_id, l.summary FROM logs l JOIN items i ON i.id = l.item_id WHERE i.plan_path = ?1 AND l.kind = 'verification' ORDER BY l.created_at",
            )?;
            let rows = stmt.query_map(params![plan.path], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "item_id": row.get::<_, String>(1)?,
                    "summary": row.get::<_, String>(2)?,
                }))
            })?;
            collect_rows(rows)?
        };

        let contract: Option<Value> = self
            .conn
            .query_row(
                "SELECT id, content FROM contexts WHERE kind = 'goal-contract' AND content LIKE ?1 ORDER BY created_at DESC LIMIT 1",
                params![format!("%{plan_id}%")],
                |row| {
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "content": row.get::<_, String>(1)?,
                    }))
                },
            )
            .optional()?;

        // Live verification is contract-scoped: only goal runs promise an
        // oracle, so the clause is binding only when a contract is stored.
        let verification_required = contract.is_some();
        let verification_pass = !verification_logs.is_empty();
        let clauses = vec![
            items_clause,
            json!({"clause": "reviews_complete", "pass": open_reviews.is_empty(), "open": open_evidence(&open_reviews)}),
            json!({"clause": "approvals_clear", "pass": approval_blocked.is_empty(), "open": approval_blocked}),
            json!({"clause": "verification_logged", "pass": verification_pass, "required": verification_required, "logs": verification_logs}),
        ];
        let holds = clauses.iter().all(|clause| {
            clause["pass"].as_bool().unwrap_or(false)
                || !clause["required"].as_bool().unwrap_or(true)
        });
        Ok(json!({
            "plan": plan,
            "contract": contract,
            "clauses": clauses,
            "holds": holds,
            "remaining": self.progress_value()?,
        }))
    }

    pub(crate) fn plan_audit_human(value: &Value) -> String {
        let mut human = String::new();
        for clause in value["clauses"].as_array().into_iter().flatten() {
            let name = clause["clause"].as_str().unwrap_or_default();
            let pass = clause["pass"].as_bool().unwrap_or(false);
            let required = clause["required"].as_bool().unwrap_or(true);
            let verdict = if pass {
                "PASS"
            } else if !required {
                "SKIP"
            } else {
                "FAIL"
            };
            human.push_str(&format!("{verdict} {name}"));
            if let Some(detail) = clause["detail"].as_str() {
                human.push_str(&format!(" — {detail}"));
            }
            for open in clause["open"].as_array().into_iter().flatten() {
                human.push_str(&format!(
                    "\n  open: {} [{}]",
                    open["id"].as_str().unwrap_or_default(),
                    open["status"]
                        .as_str()
                        .or(open["approval_status"].as_str())
                        .unwrap_or_default()
                ));
            }
            human.push('\n');
        }
        if value["holds"].as_bool().unwrap_or(false) {
            human.push_str("contract holds");
        } else {
            human.push_str("contract open");
        }
        human
    }
}
