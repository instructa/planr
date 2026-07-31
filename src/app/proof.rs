use super::App;
use super::proof_coverage::proof_status_from_coverages;
use crate::evidence::coverage::{
    authoritative_obligation_ids_for_scope, canonical_evaluation_error_proof,
};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

impl App {
    pub(crate) fn proof_status_for_item(&self, item_id: &str) -> Result<Value> {
        let item = self.get_item(item_id)?;
        let project = self.default_project()?;
        let binding_ids =
            match authoritative_obligation_ids_for_scope(&self.conn, &project.id, "item", &item.id)
            {
                Ok(ids) => ids,
                Err(err) => {
                    return Ok(canonical_evaluation_error_proof(
                        json!({"kind": "item", "id": item.id}),
                        err,
                    ));
                }
            };
        if binding_ids.is_empty() {
            return self.legacy_proof_status_for_item(&item);
        }
        match self.evidence_item_criterion_coverages_value(&item.id) {
            Ok(coverages) => Ok(proof_status_from_coverages(
                json!({"kind": "item", "id": item.id, "binding_ids": binding_ids}),
                coverages,
            )),
            Err(err) => Ok(canonical_evaluation_error_proof(
                json!({"kind": "item", "id": item.id, "binding_ids": binding_ids}),
                err,
            )),
        }
    }

    pub(crate) fn proof_status_for_plan(&self, plan_id: &str) -> Result<Value> {
        let coverages = self.evidence_plan_criterion_coverages_value(plan_id)?;
        if coverages.is_empty() {
            return self.legacy_proof_status_for_plan(plan_id);
        }
        Ok(proof_status_from_coverages(
            json!({"kind": "plan", "id": plan_id}),
            coverages,
        ))
    }

    pub(crate) fn proof_close_blocker(&self, item_id: &str) -> Result<Option<String>> {
        let proof = self.proof_status_for_item(item_id)?;
        if proof["active_binding"].as_bool() == Some(true) && proof["pass"].as_bool() != Some(true)
        {
            return Ok(Some(
                proof["next_action"]
                    .as_str()
                    .unwrap_or("planr evidence explain --scope item --id <item-id>")
                    .to_string(),
            ));
        }
        Ok(None)
    }
}

impl App {
    fn legacy_proof_status_for_item(&self, item: &crate::model::Item) -> Result<Value> {
        let plan_id = if let Some(plan_path) = &item.plan_path {
            self.conn
                .query_row(
                    "SELECT id FROM plans WHERE project_id = ?1 AND path = ?2 AND archived = 0 ORDER BY created_at DESC LIMIT 1",
                    params![item.project_id, plan_path],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
        } else {
            None
        };
        let logs = self.legacy_verification_logs_for_item(&item.project_id, &item.id)?;
        Ok(legacy_proof_status(
            "item",
            &item.id,
            plan_id.as_deref(),
            logs,
        ))
    }

    fn legacy_proof_status_for_plan(&self, plan_id: &str) -> Result<Value> {
        let logs = self.verification_logs_for_plan(plan_id)?;
        Ok(legacy_proof_status("plan", plan_id, Some(plan_id), logs))
    }

    fn legacy_verification_logs_for_item(
        &self,
        project_id: &str,
        item_id: &str,
    ) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, item_id, summary, created_at
             FROM logs
             WHERE project_id = ?1
               AND item_id = ?2
               AND kind = 'verification'
             ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map(params![project_id, item_id], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "item_id": row.get::<_, String>(1)?,
                "summary": row.get::<_, String>(2)?,
                "created_at": row.get::<_, String>(3)?,
                "authority": "claim_only",
            }))
        })?;
        crate::util::collect_rows(rows)
    }
}

fn legacy_proof_status(
    scope_kind: &str,
    scope_id: &str,
    plan_id: Option<&str>,
    claim_only_logs: Vec<Value>,
) -> Value {
    let suggested_next_action = plan_id
        .map(|id| {
            format!(
                "create planr.evidence.migration.v1 payload with plan_id {id}, then run planr evidence migrate --input <migration-file-for-plan-{id}> --apply"
            )
        })
        .unwrap_or_else(|| {
            format!(
                "create binding Evidence obligations for {scope_kind} {scope_id} to replace claim-only verification logs"
            )
        });
    json!({
        "scope": {"kind": scope_kind, "id": scope_id},
        "active_binding": false,
        "pass": true,
        "status": "legacy_nonbinding",
        "completion_language": "legacy/non-binding scope; verification logs are claim-only and closure is not proven by Evidence coverage",
        "actionable_now": false,
        "actionable_gaps": [],
        "non_actionable_blockers": [],
        "legacy_diagnostics": [{
            "kind": "legacy_verification_claims",
            "authority": "claim_only",
            "message": "legacy verification logs remain visible diagnostics but do not satisfy binding Evidence coverage",
            "logs": claim_only_logs,
        }],
        "receipts": [],
        "attempts": [],
        "waivers": [],
        "criteria": [],
        "legacy_claims": claim_only_logs,
        "suggested_next_action": suggested_next_action,
        "next_action": suggested_next_action,
    })
}

pub(crate) fn append_proof_status_human(human: &mut String, proof: &Value) {
    if proof.is_null() {
        return;
    }
    if let Some(language) = proof["completion_language"].as_str() {
        human.push_str(&format!(
            "\n    proof {}: {language}",
            proof["status"].as_str().unwrap_or("unknown")
        ));
    }
    if proof["legacy_diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|diagnostic| diagnostic["authority"].as_str() == Some("claim_only"))
    {
        human.push_str("\n    legacy verification logs: claim_only");
    }
    if let Some(next_action) = proof["next_action"].as_str() {
        human.push_str(&format!("\n    next proof action: {next_action}"));
    }
}
