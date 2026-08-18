use super::App;
use super::proof_coverage::proof_status_from_coverages;
use crate::evidence::coverage::{
    authoritative_obligation_ids_for_scope, authoritative_plan_obligation_binding_identities,
    canonical_evaluation_error_proof,
};
use crate::planpack::{BuildPlanCriterion, build_plan_criteria, parse_plan_metadata};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlanEvidenceAuthority {
    NonBinding,
    BindingUnsatisfied,
    BindingActive,
}

#[derive(Debug)]
struct PlanEvidenceAuthorityEvaluation {
    authority: PlanEvidenceAuthority,
    declared_criteria: Vec<BuildPlanCriterion>,
    gaps: Vec<Value>,
}

impl App {
    pub(crate) fn plan_evidence_authority(&self, plan_id: &str) -> Result<PlanEvidenceAuthority> {
        Ok(self.plan_evidence_authority_evaluation(plan_id)?.authority)
    }

    fn plan_evidence_authority_evaluation(
        &self,
        plan_id: &str,
    ) -> Result<PlanEvidenceAuthorityEvaluation> {
        let project = self.default_project()?;
        let active_bindings =
            authoritative_plan_obligation_binding_identities(&self.conn, &project.id, plan_id)
                .map_err(|error| anyhow!(error))?
                .into_iter()
                .map(|row| (row.id, row.criterion_id))
                .collect::<Vec<_>>();
        if active_bindings.is_empty() && !self.evidence_policy_requires_binding()? {
            return Ok(PlanEvidenceAuthorityEvaluation {
                authority: PlanEvidenceAuthority::NonBinding,
                declared_criteria: Vec::new(),
                gaps: Vec::new(),
            });
        }

        self.evaluate_plan_criterion_bindings(plan_id, &active_bindings)
    }

    pub(crate) fn require_complete_plan_criterion_bindings(
        &self,
        plan_id: &str,
        bindings: &[(String, String)],
    ) -> Result<()> {
        let evaluation = self.evaluate_plan_criterion_bindings(plan_id, bindings)?;
        if evaluation.authority == PlanEvidenceAuthority::BindingActive {
            return Ok(());
        }
        let diagnostics = evaluation
            .gaps
            .iter()
            .map(|gap| {
                let code = gap["code"].as_str().unwrap_or("invalid_binding_set");
                let criterion = gap["scope"]["id"].as_str();
                criterion.map_or_else(|| code.to_string(), |id| format!("{code}:{id}"))
            })
            .collect::<Vec<_>>()
            .join(",");
        Err(anyhow!(
            "criterion bindings must exactly match declared build-plan criteria: {diagnostics}"
        ))
    }

    fn evaluate_plan_criterion_bindings(
        &self,
        plan_id: &str,
        bindings: &[(String, String)],
    ) -> Result<PlanEvidenceAuthorityEvaluation> {
        let plan = self.get_plan(plan_id)?;
        let (frontmatter, parse_status) = parse_plan_metadata(std::path::Path::new(&plan.path));
        let declared_criteria = if parse_status == "ok" {
            match build_plan_criteria(&frontmatter) {
                Ok(criteria) => criteria,
                Err(problems) => {
                    return Ok(PlanEvidenceAuthorityEvaluation {
                        authority: PlanEvidenceAuthority::BindingUnsatisfied,
                        declared_criteria: Vec::new(),
                        gaps: problems
                            .into_iter()
                            .map(|message| {
                                json!({
                                    "code": "invalid_plan_criteria",
                                    "scope": {"kind": "plan", "id": plan_id},
                                    "message": message,
                                })
                            })
                            .collect(),
                    });
                }
            }
        } else {
            return Ok(PlanEvidenceAuthorityEvaluation {
                authority: PlanEvidenceAuthority::BindingUnsatisfied,
                declared_criteria: Vec::new(),
                gaps: vec![json!({
                    "code": "invalid_plan_criteria",
                    "scope": {"kind": "plan", "id": plan_id},
                    "message": frontmatter["error"].as_str().unwrap_or("invalid build-plan frontmatter"),
                })],
            });
        };

        let declared_ids = declared_criteria
            .iter()
            .map(|criterion| criterion.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut bindings_by_criterion = BTreeMap::<&str, Vec<&str>>::new();
        for (obligation_id, criterion_id) in bindings {
            bindings_by_criterion
                .entry(criterion_id)
                .or_default()
                .push(obligation_id);
        }

        let mut gaps = Vec::new();
        for criterion in &declared_criteria {
            match bindings_by_criterion.get(criterion.id.as_str()) {
                None => gaps.push(json!({
                    "code": "missing_obligation",
                    "scope": {"kind": "criterion", "id": criterion.id, "plan_id": plan_id},
                })),
                Some(obligation_ids) if obligation_ids.len() > 1 => gaps.push(json!({
                    "code": "duplicate_criterion_binding",
                    "scope": {"kind": "criterion", "id": criterion.id, "plan_id": plan_id},
                    "obligation_ids": obligation_ids,
                })),
                Some(_) => {}
            }
        }
        for (criterion_id, obligation_ids) in &bindings_by_criterion {
            if !declared_ids.contains(criterion_id) {
                gaps.push(json!({
                    "code": "undeclared_criterion_binding",
                    "scope": {"kind": "criterion", "id": criterion_id, "plan_id": plan_id},
                    "obligation_ids": obligation_ids,
                }));
            }
        }

        Ok(PlanEvidenceAuthorityEvaluation {
            authority: if gaps.is_empty() {
                PlanEvidenceAuthority::BindingActive
            } else {
                PlanEvidenceAuthority::BindingUnsatisfied
            },
            declared_criteria,
            gaps,
        })
    }

    pub(crate) fn proof_status_for_item(&self, item_id: &str) -> Result<Value> {
        let item = self.get_item(item_id)?;
        let plan_id = item
            .plan_path
            .as_deref()
            .map(|path| self.plan_id_for_path(path))
            .transpose()?
            .flatten();
        if let Some(plan_id) = &plan_id {
            let authority = self.plan_evidence_authority_evaluation(plan_id)?;
            if authority.authority == PlanEvidenceAuthority::BindingUnsatisfied {
                return Ok(binding_unsatisfied_item_proof_status(
                    &item.id, plan_id, &authority,
                ));
            }
        }
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
        let authority = self.plan_evidence_authority_evaluation(plan_id)?;
        match authority.authority {
            PlanEvidenceAuthority::NonBinding => {
                return Ok(nonbinding_proof_status("plan", plan_id));
            }
            PlanEvidenceAuthority::BindingUnsatisfied => {
                return Ok(binding_unsatisfied_proof_status(plan_id, &authority));
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

fn binding_unsatisfied_proof_status(
    plan_id: &str,
    authority: &PlanEvidenceAuthorityEvaluation,
) -> Value {
    let next_action = format!(
        "create planr.evidence.migration.v1 payload with plan_id {plan_id}, then run planr evidence migrate --input <migration-file-for-plan-{plan_id}> --apply"
    );
    json!({
        "scope": {"kind": "plan", "id": plan_id},
        "active_binding": true,
        "pass": false,
        "status": "binding_unsatisfied",
        "completion_language": "binding Evidence requires exactly one authoritative obligation for every declared build-plan criterion",
        "actionable_now": true,
        "actionable_gaps": authority.gaps,
        "non_actionable_blockers": [],
        "receipts": [],
        "attempts": [],
        "waivers": [],
        "criteria": authority.declared_criteria,
        "suggested_next_action": next_action,
        "next_action": next_action,
    })
}

fn binding_unsatisfied_item_proof_status(
    item_id: &str,
    plan_id: &str,
    authority: &PlanEvidenceAuthorityEvaluation,
) -> Value {
    let mut proof = binding_unsatisfied_proof_status(plan_id, authority);
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
