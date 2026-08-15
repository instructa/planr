use super::App;
use super::proof::PlanEvidenceAuthority;
use anyhow::Result;
use serde_json::{Value, json};

pub(crate) fn plan_evidence_coverage_clause(
    app: &App,
    plan_id: &str,
    verification_logs: Vec<Value>,
) -> Result<Value> {
    match app.plan_evidence_authority(plan_id)? {
        PlanEvidenceAuthority::NonBinding => {
            return Ok(json!({
                "clause": "verification_logged",
                "pass": true,
                "required": false,
                "authority": "nonbinding",
                "logs": verification_logs,
                "log_authority": "claim_only",
                "criteria": [],
                "detail": "repository policy does not require binding Evidence; verification claims are optional diagnostics"
            }));
        }
        PlanEvidenceAuthority::BindingUnsatisfied => {
            let proof = app.proof_status_for_plan(plan_id)?;
            return Ok(json!({
                "clause": "verification_logged",
                "pass": false,
                "required": true,
                "authority": "evidence_policy",
                "logs": verification_logs,
                "log_authority": "claim_only",
                "criteria": [],
                "binding_gaps": proof["actionable_gaps"],
                "actionable_now": proof["actionable_now"],
                "next_action": proof["next_action"],
                "detail": proof["completion_language"],
            }));
        }
        PlanEvidenceAuthority::BindingActive => {}
    }

    let coverages = app.evidence_plan_criterion_coverages_value(plan_id)?;
    if coverages.is_empty() {
        let proof = app.proof_status_for_plan(plan_id)?;
        return Ok(json!({
            "clause": "verification_logged",
            "pass": false,
            "required": true,
            "authority": "evidence_coverage",
            "logs": verification_logs,
            "log_authority": "claim_only",
            "criteria": [],
            "actionable_now": proof["actionable_now"],
            "next_action": proof["next_action"],
            "detail": proof["completion_language"],
        }));
    }

    let coverage = coverages
        .into_iter()
        .map(evidence_criterion_audit_value)
        .collect::<Vec<_>>();
    let pass = coverage
        .iter()
        .all(|criterion| criterion["pass"].as_bool() == Some(true));
    let actionable_now = coverage.iter().any(|criterion| {
        criterion["pass"].as_bool() != Some(true)
            && criterion["actionable_now"].as_bool().unwrap_or(true)
    });
    Ok(json!({
        "clause": "verification_logged",
        "pass": pass,
        "required": true,
        "authority": "evidence_coverage",
        "logs": verification_logs,
        "log_authority": "claim_only",
        "criteria": coverage,
        "actionable_now": actionable_now,
        "detail": "active binding Evidence criteria require satisfied or waived canonical coverage; verification logs are diagnostics only"
    }))
}

pub(crate) fn append_evidence_clause_human(human: &mut String, clause: &Value) {
    for claim in clause["logs"].as_array().into_iter().flatten() {
        human.push_str(&format!(
            "\n  claim-only untrusted verification log: {} item={} summary={}",
            claim["id"].as_str().unwrap_or_default(),
            claim["item_id"].as_str().unwrap_or_default(),
            claim["summary"].as_str().unwrap_or_default()
        ));
    }
    for criterion in clause["criteria"].as_array().into_iter().flatten() {
        human.push_str(&format!(
            "\n  criterion: {} [{}]",
            criterion["criterion_id"].as_str().unwrap_or_default(),
            criterion["status"].as_str().unwrap_or_default()
        ));
        if let Some(scope) = criterion["scope"].as_object() {
            human.push_str(&format!(
                " scope={}:{}",
                scope
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                scope.get("id").and_then(Value::as_str).unwrap_or_default()
            ));
        }
        if criterion["actionable_now"].as_bool().unwrap_or(false) {
            human.push_str(" actionable");
        } else {
            human.push_str(" non-actionable");
        }
        for observation in criterion["observations"].as_array().into_iter().flatten() {
            if let Some(gap) = observation["gap_reason"].as_str() {
                human.push_str(&format!(
                    "\n    gap: {} {}",
                    observation["requirement_id"].as_str().unwrap_or_default(),
                    gap
                ));
            }
            let receipts = observation["covering_receipt_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if !receipts.is_empty() {
                human.push_str(&format!("\n    receipts: {}", receipts.join(",")));
            }
            if let Some(waiver_id) = observation["waiver_id"].as_str() {
                human.push_str(&format!("\n    waiver: {waiver_id}"));
            }
        }
        let receipt_digests = criterion["receipt_digests"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        if !receipt_digests.is_empty() {
            human.push_str(&format!(
                "\n    receipt_digests: {}",
                receipt_digests.join(",")
            ));
        }
        let waiver_digests = criterion["waiver_digests"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        if !waiver_digests.is_empty() {
            human.push_str(&format!(
                "\n    waiver_digests: {}",
                waiver_digests.join(",")
            ));
        }
    }
}

pub(crate) fn append_audit_proof_human(human: &mut String, proof: &Value) {
    if let Some(language) = proof["completion_language"].as_str() {
        human.push_str(&format!("\nproof: {language}"));
    }
    if let Some(next_action) = proof["next_action"].as_str() {
        human.push_str(&format!("\nnext proof action: {next_action}"));
    }
}

fn evidence_criterion_audit_value(coverage: Value) -> Value {
    let status = coverage["status"].as_str().unwrap_or("error");
    let pass = matches!(status, "satisfied" | "waived");
    let verdict = &coverage["coverage"];
    let observations = verdict["observation_coverage"]
        .as_array()
        .map(|observations| {
            observations
                .iter()
                .map(|observation| {
                    json!({
                        "requirement_id": observation["requirement_id"],
                        "status": observation["status"],
                        "gap_reason": observation.get("gap_reason").cloned().unwrap_or(Value::Null),
                        "covering_receipt_ids": observation.get("covering_receipt_ids").cloned().unwrap_or_else(|| json!([])),
                        "attempted_receipt_ids": observation.get("attempted_receipt_ids").cloned().unwrap_or_else(|| json!([])),
                        "waiver_id": observation.get("waiver_id").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "criterion_id": verdict["scope"]["id"].as_str().unwrap_or_default(),
        "pass": pass,
        "status": status,
        "coverage_id": coverage["coverage_id"],
        "scope": verdict["scope"],
        "actionable_now": verdict["actionable_now"].as_bool().unwrap_or(!pass),
        "receipt_digests": coverage["receipt_digests"],
        "waiver_digests": coverage["waiver_digests"],
        "observations": observations,
    })
}
