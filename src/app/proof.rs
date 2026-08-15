use super::App;
use super::proof_coverage::proof_status_from_coverages;
use crate::evidence::coverage::{
    authoritative_obligation_ids_for_scope, canonical_evaluation_error_proof,
};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlanEvidenceAuthority {
    NonBinding,
    BindingUnsatisfied,
    BindingActive,
}

impl App {
    pub(crate) fn plan_evidence_authority(&self, plan_id: &str) -> Result<PlanEvidenceAuthority> {
        let project = self.default_project()?;
        let obligation_ids =
            authoritative_obligation_ids_for_scope(&self.conn, &project.id, "plan", plan_id)
                .map_err(|error| anyhow!(error))?;
        if !obligation_ids.is_empty() {
            return Ok(PlanEvidenceAuthority::BindingActive);
        }
        if self.evidence_policy_requires_binding()? {
            return Ok(PlanEvidenceAuthority::BindingUnsatisfied);
        }
        Ok(PlanEvidenceAuthority::NonBinding)
    }

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
            if let Some(plan_id) = item
                .plan_path
                .as_deref()
                .map(|path| self.plan_id_for_path(path))
                .transpose()?
                .flatten()
                && self.plan_evidence_authority(&plan_id)?
                    == PlanEvidenceAuthority::BindingUnsatisfied
            {
                return Ok(binding_unsatisfied_item_proof_status(&item.id, &plan_id));
            }
            return Ok(nonbinding_proof_status("item", &item.id));
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
        match self.plan_evidence_authority(plan_id)? {
            PlanEvidenceAuthority::NonBinding => {
                return Ok(nonbinding_proof_status("plan", plan_id));
            }
            PlanEvidenceAuthority::BindingUnsatisfied => {
                return Ok(binding_unsatisfied_proof_status(plan_id));
            }
            PlanEvidenceAuthority::BindingActive => {}
        }
        match self.evidence_plan_criterion_coverages_value(plan_id) {
            Ok(coverages) if !coverages.is_empty() => Ok(proof_status_from_coverages(
                json!({"kind": "plan", "id": plan_id}),
                coverages,
            )),
            Ok(_) => Ok(canonical_evaluation_error_proof(
                json!({"kind": "plan", "id": plan_id}),
                anyhow!("binding Evidence obligations exist but produced no coverage"),
            )),
            Err(error) => Ok(canonical_evaluation_error_proof(
                json!({"kind": "plan", "id": plan_id}),
                error,
            )),
        }
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

    pub(crate) fn binding_evidence_hold_for_item(&self, item_id: &str) -> Result<Option<Value>> {
        let proof = self.proof_status_for_item(item_id)?;
        if proof["status"] != "binding_unsatisfied" {
            return Ok(None);
        }
        Ok(Some(json!({
            "kind": "hold",
            "item_id": item_id,
            "classification": "binding_evidence_obligations_missing",
            "reason_code": "missing_obligation",
            "proof": proof,
            "next_action": proof["next_action"],
        })))
    }
}

fn nonbinding_proof_status(scope_kind: &str, scope_id: &str) -> Value {
    json!({
        "scope": {"kind": scope_kind, "id": scope_id},
        "active_binding": false,
        "pass": true,
        "status": "nonbinding",
        "completion_language": "repository policy does not require binding Evidence for this scope",
        "actionable_now": false,
        "actionable_gaps": [],
        "non_actionable_blockers": [],
        "receipts": [],
        "attempts": [],
        "waivers": [],
        "criteria": [],
        "suggested_next_action": null,
        "next_action": null,
    })
}

fn binding_unsatisfied_proof_status(plan_id: &str) -> Value {
    let next_action = format!(
        "create planr.evidence.migration.v1 payload with plan_id {plan_id}, then run planr evidence migrate --input <migration-file-for-plan-{plan_id}> --apply"
    );
    json!({
        "scope": {"kind": "plan", "id": plan_id},
        "active_binding": true,
        "pass": false,
        "status": "binding_unsatisfied",
        "completion_language": "repository policy requires binding Evidence, but the plan has no materialized ProofObligation",
        "actionable_now": true,
        "actionable_gaps": [{
            "code": "missing_obligation",
            "scope": {"kind": "plan", "id": plan_id},
        }],
        "non_actionable_blockers": [],
        "receipts": [],
        "attempts": [],
        "waivers": [],
        "criteria": [],
        "suggested_next_action": next_action,
        "next_action": next_action,
    })
}

fn binding_unsatisfied_item_proof_status(item_id: &str, plan_id: &str) -> Value {
    let mut proof = binding_unsatisfied_proof_status(plan_id);
    proof["scope"] = json!({"kind": "item", "id": item_id, "plan_id": plan_id});
    proof
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
    if let Some(next_action) = proof["next_action"].as_str() {
        human.push_str(&format!("\n    next proof action: {next_action}"));
    }
}
