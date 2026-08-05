use super::App;
use super::audit_evidence::{
    append_audit_proof_human, append_evidence_clause_human, plan_evidence_coverage_clause,
};
use super::repository::execution_run::{ExecutionRunRepository, ReviewGateKind};
use crate::storage::row_to_item;
use crate::util::collect_rows;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

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
        let is_open = |status: &crate::model::ItemStatus| !status.is_settled();

        let canonical_scope: Vec<_> = scope
            .iter()
            .filter(|item| {
                !matches!(
                    item.work_type.as_str(),
                    "review" | "fix" | "follow-up-review"
                )
            })
            .cloned()
            .collect();
        let open_items: Vec<_> = canonical_scope
            .iter()
            .filter(|item| is_open(&item.status))
            .cloned()
            .collect();
        let repository = ExecutionRunRepository::new(&self.conn);
        let open_non_final_reviews = match self.canonical_execution_run_id_for_plan(plan_id)? {
            Some(run_id) => repository.review_gates_for_run(&run_id, true)?,
            None => Vec::new(),
        }
        .into_iter()
        .filter(|gate| gate.kind != ReviewGateKind::FinalProduct)
        .map(|gate| self.review_gate_projection_value(&gate))
        .collect::<Result<Vec<_>>>()?;
        let items_clause = if canonical_scope.is_empty() {
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

        let contract: Option<Value> = self.conn
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

        let evidence_clause =
            plan_evidence_coverage_clause(self, plan_id, contract.is_some(), verification_logs)?;
        let proof = self.proof_status_for_plan(plan_id)?;
        let material_review_clause = self.required_material_reviews_clause(plan_id)?;
        let final_review_clause = self.final_product_review_clause_value(plan_id)?;
        let final_review_for_next = final_review_clause.clone();
        let clauses = vec![
            items_clause,
            json!({"clause": "reviews_complete", "pass": open_non_final_reviews.is_empty(), "open": open_non_final_reviews}),
            material_review_clause,
            final_review_clause,
            json!({"clause": "approvals_clear", "pass": approval_blocked.is_empty(), "open": approval_blocked}),
            evidence_clause,
        ];
        let holds = clauses.iter().all(|clause| {
            clause["pass"].as_bool().unwrap_or(false)
                || !clause["required"].as_bool().unwrap_or(true)
        });
        let next = if holds {
            None
        } else if canonical_scope.is_empty() {
            Some(format!("planr map build --from {plan_id}"))
        } else if open_non_final_reviews.iter().any(|entry| {
            entry["review_gate"]["status"] == "pending"
                || entry["review_gate"]["status"] == "leased"
        }) {
            Some(format!(
                "planr pick --plan {plan_id} --work-type review --json"
            ))
        } else if open_items.iter().any(|item| item.status == "ready") {
            Some(format!("planr pick --plan {plan_id} --json"))
        } else if let Some(blocked) = approval_blocked.first() {
            Some(format!(
                "planr approval approve {} --by \"<approver>\" (or `deny`)",
                blocked["id"].as_str().unwrap_or_default()
            ))
        } else if !open_items.is_empty() {
            Some(
                "planr map status (then `planr recover sweep --apply` if leases are stale)"
                    .to_string(),
            )
        } else if final_review_for_next["pass"].as_bool() != Some(true) {
            final_review_for_next["next"]
                .as_str()
                .map(ToOwned::to_owned)
        } else if let Some(evidence) = clauses
            .iter()
            .find(|clause| {
                clause["clause"] == "verification_logged"
                    && clause["authority"] == "evidence_coverage"
                    && !clause["pass"].as_bool().unwrap_or(false)
                    && clause["required"].as_bool().unwrap_or(true)
            })
            .and_then(|clause| clause["criteria"].as_array())
            .and_then(|criteria| {
                criteria
                    .iter()
                    .find(|criterion| criterion["pass"].as_bool() != Some(true))
            })
        {
            Some(format!(
                "planr evidence explain --scope criterion --id {}",
                evidence["criterion_id"].as_str().unwrap_or_default()
            ))
        } else {
            // Frozen pre-Evidence compatibility is the only remaining owner
            // that may direct an operator to a verification log.
            Some(format!(
                "planr log add --item <verifier-item-id> --kind verification --summary \"verified <flow>: <observed outcome>\" --cmd \"<exact command>\" (scope: plan {plan_id})"
            ))
        };
        Ok(json!({
            "plan": plan,
            "execution_state": self.canonical_execution_state_for_plan_value(plan_id)?,
            "contract": contract,
            "clauses": clauses,
            "proof": proof,
            "holds": holds,
            "next": next,
            "remaining": self.progress_value()?,
        }))
    }

    fn required_material_reviews_clause(&self, plan_id: &str) -> Result<Value> {
        let plan = self.get_plan(plan_id)?;
        let repository = ExecutionRunRepository::new(&self.conn);
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path
             FROM items
             WHERE plan_path = ?1
             ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map(params![plan.path], row_to_item)?;
        let mut reviewed = Vec::new();
        let mut open = Vec::new();
        for row in rows {
            let item = row?;
            let Some(materiality) = self.item_metadata_field(&item.id, "materiality")? else {
                continue;
            };
            if materiality["effective_review"]["required"].as_bool() != Some(true) {
                continue;
            }
            let projection = repository
                .review_gate_for_outcome_item(&item.id)?
                .map(|gate| self.review_gate_projection_value(&gate))
                .transpose()?;
            let pass = projection
                .as_ref()
                .is_some_and(|entry| entry["accepted"] == true);
            let entry = json!({
                "id": item.id,
                "status": item.status,
                "title": item.title,
                "review_gate": projection,
                "pass": pass,
            });
            if !pass {
                open.push(entry.clone());
            }
            reviewed.push(entry);
        }
        Ok(json!({
            "clause": "required_material_reviews_complete",
            "pass": open.is_empty(),
            "required": true,
            "open": open,
            "reviewed": reviewed,
        }))
    }

    pub(crate) fn plan_audit_human(value: &Value) -> String {
        let mut human = value["execution_state"]
            .as_object()
            .map(|_| {
                format!(
                    "{}\n",
                    Self::canonical_execution_state_human(&value["execution_state"])
                )
            })
            .unwrap_or_default();
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
                let status = open["status"]
                    .as_str()
                    .or(open["approval_status"].as_str())
                    .unwrap_or_default();
                human.push_str(&format!(
                    "\n  open: {} [{status}]",
                    open["id"].as_str().unwrap_or_default()
                ));
                if let Some(reason) = open["reason"].as_str() {
                    human.push_str(&format!(" — {reason}"));
                }
            }
            append_evidence_clause_human(&mut human, clause);
            human.push('\n');
        }
        if value["holds"].as_bool().unwrap_or(false) {
            human.push_str("contract holds");
        } else {
            human.push_str("contract open");
            if let Some(next) = value["next"].as_str() {
                human.push_str(&format!("\nnext: {next}"));
            }
        }
        append_audit_proof_human(&mut human, &value["proof"]);
        human
    }
}
