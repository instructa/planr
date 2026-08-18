use super::model::{
    CoverageStatus, EvidenceReceipt, EvidenceWaiver, GapReason, ObservationRequirement,
    ProvenanceSourceKind, TrustedReceiptBinding, VerificationCapabilityInstance,
};
use super::policy::{
    EvidenceRepositorySnapshot, capture_repository_snapshot, parse_trusted_receipt_binding,
};
use super::{AttemptStatus, CoverageObservationStatus, EvidenceDomainError};
use crate::canonical_json::sha256_json_digest;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Debug, Clone)]
pub struct CoverageEvaluation {
    pub id: String,
    pub status: CoverageStatus,
    pub verdict: Value,
    pub receipt_digests: Vec<String>,
    pub waiver_digests: Vec<String>,
    pub receipt_lineage: Value,
}

#[derive(Debug, Clone, Copy)]
enum ScopeKind {
    Criterion,
    Item,
    Plan,
}

impl ScopeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Criterion => "criterion",
            Self::Item => "item",
            Self::Plan => "plan",
        }
    }
}

#[derive(Debug, Clone)]
struct EvaluationScope {
    kind: ScopeKind,
    id: String,
    plan_id: String,
    item_id: Option<String>,
    criterion_id: Option<String>,
}

/// One authoritative active obligation row selected by the Evidence coverage
/// domain. Consumers may decide completeness from these typed facts, but must
/// not duplicate the binding/supersession query that selects them.
#[derive(Debug, Clone)]
pub struct AuthoritativeObligationBindingRow {
    pub id: String,
    pub observations: Vec<ObservationRequirement>,
}

/// The canonical active binding identity selected by the Evidence coverage
/// domain. Completeness consumers need this identity but never observations.
#[derive(Debug, Clone)]
pub struct AuthoritativeObligationBindingIdentity {
    pub id: String,
    pub criterion_id: String,
}

#[derive(Debug, Clone)]
struct ObligationRow {
    id: String,
    project_id: String,
    plan_id: String,
    item_id: Option<String>,
    criterion_id: String,
    obligation_version: i64,
    supersedes_obligation_id: Option<String>,
    observations: Vec<ObservationRequirement>,
    fixture_policy: Value,
    freshness_policy: Value,
    assurance_policy: Value,
    retry_aggregation: RetryAggregationPolicy,
}

#[derive(Debug, Clone)]
struct ReceiptCandidate {
    id: String,
    digest: String,
    project_id: String,
    obligation_id: String,
    obligation_plan_id: String,
    obligation_item_id: Option<String>,
    obligation_criterion_id: String,
    obligation_version: i64,
    obligation_supersedes_obligation_id: Option<String>,
    attempt_artifacts: Vec<ArtifactBinding>,
    supersedes_receipt_id: Option<String>,
    value: Value,
    trusted_binding: TrustedReceiptBinding,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct ArtifactBinding {
    id: String,
    kind: String,
    digest: String,
    uri: Option<String>,
    root_kind: Option<String>,
}

#[derive(Debug, Clone)]
struct WaiverCandidate {
    id: String,
    observation_id: String,
    digest: String,
    waiver: EvidenceWaiver,
}

#[derive(Debug, Clone)]
struct ObservationCoverage {
    requirement_id: String,
    status: CoverageObservationStatus,
    covering_receipt_ids: BTreeSet<String>,
    covered_observation_ids: BTreeSet<String>,
    attempted_receipt_ids: BTreeSet<String>,
    waiver_id: Option<String>,
    gap_reasons: BTreeSet<&'static str>,
    aggregation_policy: RetryAggregationPolicy,
    superseded_receipt_ids: BTreeSet<String>,
    diagnostic_receipt_ids: BTreeSet<String>,
    rejected_receipt_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryAggregationPolicy {
    LatestApplicablePass,
    AllApplicablePass,
}

impl RetryAggregationPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::LatestApplicablePass => "latest_applicable_pass",
            Self::AllApplicablePass => "all_applicable_pass",
        }
    }
}

struct PersistCoverage<'a> {
    id: &'a str,
    status: &'a CoverageStatus,
    verdict: &'a Value,
    receipt_digests: &'a [String],
    waiver_digests: &'a [String],
    evaluated_at: &'a str,
}

struct ObservationEvaluationContext<'a> {
    conn: &'a Connection,
    scope: &'a EvaluationScope,
    obligation: &'a ObligationRow,
    receipts: &'a [ReceiptCandidate],
    waivers: &'a [WaiverCandidate],
    repository_snapshot: Option<&'a EvidenceRepositorySnapshot>,
    validation: &'a mut Map<String, Value>,
}

pub fn evaluate_obligation_coverage(
    conn: &Connection,
    project_id: &str,
    obligation_id: &str,
    evaluated_at: &str,
) -> Result<CoverageEvaluation, EvidenceDomainError> {
    let obligation = load_authoritative_obligation(conn, project_id, obligation_id)?;
    let scope = EvaluationScope {
        kind: ScopeKind::Criterion,
        id: obligation.criterion_id.clone(),
        plan_id: obligation.plan_id.clone(),
        item_id: obligation.item_id.clone(),
        criterion_id: Some(obligation.criterion_id.clone()),
    };
    evaluate_scope_coverage(conn, project_id, scope, vec![obligation], evaluated_at)
}

pub fn authoritative_obligation_ids_for_scope(
    conn: &Connection,
    project_id: &str,
    scope: &str,
    scope_id: &str,
) -> Result<Vec<String>, EvidenceDomainError> {
    authoritative_obligation_bindings_for_scope(conn, project_id, scope, scope_id)
        .map(|rows| rows.into_iter().map(|row| row.id).collect())
}

pub fn authoritative_obligation_bindings_for_scope(
    conn: &Connection,
    project_id: &str,
    scope: &str,
    scope_id: &str,
) -> Result<Vec<AuthoritativeObligationBindingRow>, EvidenceDomainError> {
    if scope == "criterion" {
        let canonical_ids =
            load_authoritative_criterion_obligation_ids(conn, project_id, scope_id)?
                .into_iter()
                .collect::<BTreeSet<_>>();
        return authoritative_obligation_bindings_by_clause(
            conn,
            project_id,
            "WHERE project_id = ?1 AND criterion_id = ?2",
            scope_id,
        )
        .map(|rows| {
            rows.into_iter()
                .filter(|row| canonical_ids.contains(&row.id))
                .collect()
        });
    }
    let where_clause = match scope {
        "obligation" => "WHERE project_id = ?1 AND id = ?2",
        "item" => "WHERE project_id = ?1 AND item_id = ?2",
        "plan" => "WHERE project_id = ?1 AND plan_id = ?2",
        _ => return Err(EvidenceDomainError::InvalidTrustedBinding("coverage.scope")),
    };
    authoritative_obligation_bindings_by_clause(conn, project_id, where_clause, scope_id)
}

pub fn authoritative_plan_obligation_binding_identities(
    conn: &Connection,
    project_id: &str,
    plan_id: &str,
) -> Result<Vec<AuthoritativeObligationBindingIdentity>, EvidenceDomainError> {
    let sql = authoritative_binding_selection_sql("id, criterion_id", "WHERE project_id = ?1 AND plan_id = ?2");
    let mut statement = conn
        .prepare(&sql)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    statement
        .query_map(params![project_id, plan_id], |row| {
            Ok(AuthoritativeObligationBindingIdentity {
                id: row.get(0)?,
                criterion_id: row.get(1)?,
            })
        })
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .map(|row| row.map_err(|err| EvidenceDomainError::Digest(err.to_string())))
        .collect()
}

pub fn evaluate_criterion_coverage(
    conn: &Connection,
    project_id: &str,
    criterion_id: &str,
    evaluated_at: &str,
) -> Result<CoverageEvaluation, EvidenceDomainError> {
    let obligations = load_authoritative_criterion_obligation_ids(conn, project_id, criterion_id)?
        .iter()
        .map(|id| load_obligation(conn, project_id, id))
        .collect::<Result<Vec<_>, _>>()?;
    let first = obligations
        .first()
        .ok_or(EvidenceDomainError::MissingTrustedBinding(
            "proof_obligation",
        ))?;
    let scope = EvaluationScope {
        kind: ScopeKind::Criterion,
        id: criterion_id.to_string(),
        plan_id: first.plan_id.clone(),
        item_id: first.item_id.clone(),
        criterion_id: Some(criterion_id.to_string()),
    };
    evaluate_scope_coverage(conn, project_id, scope, obligations, evaluated_at)
}

pub fn evaluate_plan_criterion_coverages(
    conn: &Connection,
    project_id: &str,
    plan_id: &str,
    evaluated_at: &str,
) -> Result<Vec<CoverageEvaluation>, EvidenceDomainError> {
    let obligations = load_obligations_for_scope(
        conn,
        project_id,
        "WHERE project_id = ?1 AND plan_id = ?2",
        plan_id,
    )?;
    let mut by_criterion = BTreeMap::<String, Vec<ObligationRow>>::new();
    for obligation in obligations {
        by_criterion
            .entry(obligation.criterion_id.clone())
            .or_default()
            .push(obligation);
    }
    by_criterion
        .into_iter()
        .map(|(criterion_id, obligations)| {
            let first = obligations
                .first()
                .ok_or(EvidenceDomainError::MissingTrustedBinding(
                    "proof_obligation",
                ))?;
            let scope = EvaluationScope {
                kind: ScopeKind::Criterion,
                id: criterion_id.clone(),
                plan_id: plan_id.to_string(),
                item_id: first.item_id.clone(),
                criterion_id: Some(criterion_id),
            };
            evaluate_scope_coverage(conn, project_id, scope, obligations, evaluated_at)
        })
        .collect()
}

pub fn evaluate_item_criterion_coverages(
    conn: &Connection,
    project_id: &str,
    item_id: &str,
    evaluated_at: &str,
) -> Result<Vec<CoverageEvaluation>, EvidenceDomainError> {
    let obligations = load_obligations_for_scope(
        conn,
        project_id,
        "WHERE project_id = ?1 AND item_id = ?2",
        item_id,
    )?;
    let mut by_criterion = BTreeMap::<String, Vec<ObligationRow>>::new();
    for obligation in obligations {
        by_criterion
            .entry(obligation.criterion_id.clone())
            .or_default()
            .push(obligation);
    }
    by_criterion
        .into_iter()
        .map(|(criterion_id, obligations)| {
            let first = obligations
                .first()
                .ok_or(EvidenceDomainError::MissingTrustedBinding(
                    "proof_obligation",
                ))?;
            let scope = EvaluationScope {
                kind: ScopeKind::Criterion,
                id: criterion_id.clone(),
                plan_id: first.plan_id.clone(),
                item_id: Some(item_id.to_string()),
                criterion_id: Some(criterion_id),
            };
            evaluate_scope_coverage(conn, project_id, scope, obligations, evaluated_at)
        })
        .collect()
}

pub fn evaluate_item_coverage(
    conn: &Connection,
    project_id: &str,
    item_id: &str,
    evaluated_at: &str,
) -> Result<CoverageEvaluation, EvidenceDomainError> {
    let obligations = load_obligations_for_scope(
        conn,
        project_id,
        "WHERE project_id = ?1 AND item_id = ?2",
        item_id,
    )?;
    let first = obligations
        .first()
        .ok_or(EvidenceDomainError::MissingTrustedBinding(
            "proof_obligation",
        ))?;
    let scope = EvaluationScope {
        kind: ScopeKind::Item,
        id: item_id.to_string(),
        plan_id: first.plan_id.clone(),
        item_id: Some(item_id.to_string()),
        criterion_id: None,
    };
    evaluate_scope_coverage(conn, project_id, scope, obligations, evaluated_at)
}

pub fn canonical_coverage_projection(coverage: &Value) -> Value {
    let verdict = &coverage["coverage"];
    let status = coverage["status"].as_str().unwrap_or("error");
    let pass = matches!(status, "satisfied" | "waived");
    let criterion_id = verdict["scope"]["criterion_id"]
        .as_str()
        .or_else(|| verdict["scope"]["id"].as_str())
        .unwrap_or_default();
    let observations = verdict["observation_coverage"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut actionable_gaps = Vec::new();
    let mut non_actionable_blockers = Vec::new();
    let mut receipt_refs = Vec::new();
    let mut attempt_refs = Vec::new();
    let mut waiver_refs = Vec::new();
    for observation in &observations {
        push_string_refs(&mut receipt_refs, observation, "covering_receipt_ids");
        push_string_refs(&mut attempt_refs, observation, "attempted_receipt_ids");
        if let Some(waiver_id) = observation["waiver_id"].as_str() {
            waiver_refs.push(json!(waiver_id));
        }
        let Some(observation_status) = observation["status"].as_str() else {
            continue;
        };
        if matches!(observation_status, "covered" | "waived") {
            continue;
        }
        let gap = json!({
            "criterion_id": criterion_id,
            "requirement_id": observation["requirement_id"],
            "status": observation["status"],
            "reason": observation.get("gap_reason").cloned().unwrap_or(Value::Null),
            "receipt_refs": observation.get("covering_receipt_ids").cloned().unwrap_or_else(|| json!([])),
            "attempt_refs": observation.get("attempted_receipt_ids").cloned().unwrap_or_else(|| json!([])),
            "waiver_ref": observation.get("waiver_id").cloned().unwrap_or(Value::Null),
        });
        if canonical_gap_is_actionable(observation_status) {
            actionable_gaps.push(gap);
        } else {
            non_actionable_blockers.push(gap);
        }
    }
    let suggested_next_action = verdict["suggested_next_action"]
        .as_str()
        .unwrap_or_else(|| suggested_next_action_for_status(status));
    json!({
        "criterion_id": criterion_id,
        "coverage_id": coverage["coverage_id"],
        "scope": verdict["scope"],
        "status": status,
        "pass": pass,
        "actionable_now": verdict["actionable_now"].as_bool().unwrap_or(!pass),
        "suggested_next_action": suggested_next_action,
        "observations": observations,
        "actionable_gaps": actionable_gaps,
        "non_actionable_blockers": non_actionable_blockers,
        "receipt_refs": receipt_refs,
        "attempt_refs": attempt_refs,
        "waiver_refs": waiver_refs,
        "receipt_digests": coverage["receipt_digests"].as_array().cloned().unwrap_or_default(),
        "waiver_digests": coverage["waiver_digests"].as_array().cloned().unwrap_or_default(),
    })
}

pub fn canonical_evaluation_error_proof(scope: Value, error: impl std::fmt::Display) -> Value {
    let gap = json!({
        "criterion_id": Value::Null,
        "requirement_id": Value::Null,
        "status": "blocked",
        "reason": "coverage_evaluation_failed",
        "error": error.to_string(),
        "receipt_refs": [],
        "attempt_refs": [],
        "waiver_ref": Value::Null,
    });
    json!({
        "scope": scope,
        "active_binding": true,
        "pass": false,
        "status": "error",
        "completion_language": "not proven by canonical Evidence coverage; evaluation failed closed",
        "actionable_now": true,
        "suggested_next_action": "repair Evidence coverage evaluation failure",
        "next_action": "repair Evidence coverage evaluation failure",
        "actionable_gaps": [gap],
        "non_actionable_blockers": [],
        "receipts": [],
        "attempts": [],
        "waivers": [],
        "criteria": [],
    })
}

pub fn evaluate_plan_coverage(
    conn: &Connection,
    project_id: &str,
    plan_id: &str,
    evaluated_at: &str,
) -> Result<CoverageEvaluation, EvidenceDomainError> {
    let obligations = load_obligations_for_scope(
        conn,
        project_id,
        "WHERE project_id = ?1 AND plan_id = ?2",
        plan_id,
    )?;
    let scope = EvaluationScope {
        kind: ScopeKind::Plan,
        id: plan_id.to_string(),
        plan_id: plan_id.to_string(),
        item_id: None,
        criterion_id: None,
    };
    evaluate_scope_coverage(conn, project_id, scope, obligations, evaluated_at)
}

fn evaluate_scope_coverage(
    conn: &Connection,
    project_id: &str,
    scope: EvaluationScope,
    obligations: Vec<ObligationRow>,
    evaluated_at: &str,
) -> Result<CoverageEvaluation, EvidenceDomainError> {
    if obligations.is_empty() {
        return Err(EvidenceDomainError::MissingTrustedBinding(
            "proof_obligation",
        ));
    }
    let mut observations = Vec::new();
    let mut receipt_lineage = Vec::new();
    let mut validation = validation_scaffold(evaluated_at);
    let mut accepted_receipt_ids = BTreeSet::new();
    let mut accepted_receipt_digests = BTreeSet::new();
    let mut accepted_waiver_digests = BTreeSet::new();
    let repository_snapshot =
        project_root(conn, project_id)?.and_then(|root| capture_repository_snapshot(&root).ok());

    for obligation in &obligations {
        let receipts = load_receipts(conn, project_id, &obligation.id)?;
        let waivers = load_waivers(conn, project_id, &obligation.id, evaluated_at)?;
        for requirement in &obligation.observations {
            let mut context = ObservationEvaluationContext {
                conn,
                scope: &scope,
                obligation,
                receipts: &receipts,
                waivers: &waivers,
                repository_snapshot: repository_snapshot.as_ref(),
                validation: &mut validation,
            };
            let coverage = evaluate_observation(&mut context, requirement)?;
            receipt_lineage.push(observation_lineage_value(&coverage));
            accepted_receipt_ids.extend(coverage.covering_receipt_ids.iter().cloned());
            for receipt in &receipts {
                if coverage.covering_receipt_ids.contains(&receipt.id) {
                    accepted_receipt_digests.insert(receipt.digest.clone());
                }
            }
            for waiver in &waivers {
                if coverage.waiver_id.as_deref() == Some(waiver.id.as_str()) {
                    accepted_waiver_digests.insert(waiver.digest.clone());
                }
            }
            observations.push(coverage_to_value(coverage));
        }
    }

    let status = aggregate_observation_status(&observations);
    finalize_validation(&mut validation, &accepted_receipt_ids);
    let scope_json = scope_to_value(&scope);
    let verdict = json!({
        "id": coverage_id(project_id, &scope, &obligations)?,
        "schema_version": super::model::EVIDENCE_CONTRACT_V1,
        "scope": scope_json,
        "evaluated_at": evaluated_at,
        "status": status.as_str(),
        "observation_coverage": observations,
        "validation_details": validation,
        "suggested_next_action": suggested_next_action(&status),
        "actionable_now": matches!(
            status,
            CoverageStatus::Unsatisfied | CoverageStatus::Stale | CoverageStatus::Inconclusive
        ),
    });
    let id = verdict["id"].as_str().unwrap().to_string();
    let receipt_digests = accepted_receipt_digests.into_iter().collect::<Vec<_>>();
    let waiver_digests = accepted_waiver_digests.into_iter().collect::<Vec<_>>();
    persist_coverage_verdict(
        conn,
        project_id,
        &scope,
        &obligations,
        PersistCoverage {
            id: &id,
            status: &status,
            verdict: &verdict,
            receipt_digests: &receipt_digests,
            waiver_digests: &waiver_digests,
            evaluated_at,
        },
    )?;
    Ok(CoverageEvaluation {
        id,
        status,
        verdict,
        receipt_digests,
        waiver_digests,
        receipt_lineage: Value::Array(receipt_lineage),
    })
}

fn evaluate_observation(
    context: &mut ObservationEvaluationContext<'_>,
    requirement: &ObservationRequirement,
) -> Result<ObservationCoverage, EvidenceDomainError> {
    let mut coverage = ObservationCoverage {
        requirement_id: requirement.id.as_str().to_string(),
        status: CoverageObservationStatus::Missing,
        covering_receipt_ids: BTreeSet::new(),
        covered_observation_ids: BTreeSet::new(),
        attempted_receipt_ids: BTreeSet::new(),
        waiver_id: None,
        gap_reasons: BTreeSet::new(),
        aggregation_policy: context.obligation.retry_aggregation,
        superseded_receipt_ids: BTreeSet::new(),
        diagnostic_receipt_ids: BTreeSet::new(),
        rejected_receipt_ids: BTreeSet::new(),
    };
    let mut matching_waivers = Vec::new();
    for waiver in context.waivers {
        if waiver_covers(
            context.scope,
            requirement,
            waiver,
            context.repository_snapshot,
        ) {
            matching_waivers.push(waiver);
        }
    }
    let mut passed = Vec::new();
    let mut failed = Vec::new();
    let mut rejected_gaps: Vec<(String, BTreeSet<&'static str>)> = Vec::new();
    let mut historical_rejected_gaps: Vec<(String, BTreeSet<&'static str>)> = Vec::new();

    let retry_superseded_ids = context
        .receipts
        .iter()
        .filter(|receipt| receipt.obligation_id == context.obligation.id)
        .filter_map(|receipt| receipt.supersedes_receipt_id.clone())
        .collect::<BTreeSet<_>>();

    for receipt in context.receipts {
        let Some(observation) = receipt
            .value
            .get("observations")
            .and_then(Value::as_array)
            .and_then(|observations| {
                observations.iter().find(|observation| {
                    observation.get("requirement_id").and_then(Value::as_str)
                        == Some(requirement.id.as_str())
                        && observation.get("type").and_then(Value::as_str)
                            == Some(requirement.observation_type.as_str())
                })
            })
        else {
            continue;
        };
        coverage.attempted_receipt_ids.insert(receipt.id.clone());
        let historical_obligation = receipt.obligation_id != context.obligation.id;
        let superseded_retry = retry_superseded_ids.contains(&receipt.id);
        let completion_relevant = !historical_obligation
            && (coverage.aggregation_policy == RetryAggregationPolicy::AllApplicablePass
                || !superseded_retry);
        if historical_obligation || superseded_retry {
            coverage.superseded_receipt_ids.insert(receipt.id.clone());
            coverage.diagnostic_receipt_ids.insert(receipt.id.clone());
        }
        let gaps = candidate_gaps(
            context.conn,
            context.obligation,
            requirement,
            receipt,
            observation,
            context.repository_snapshot,
        )?;
        if !completion_relevant {
            historical_rejected_gaps.push((receipt.id.clone(), gaps));
            continue;
        }
        if gaps.is_empty()
            && observation.get("outcome").and_then(Value::as_str)
                == Some(AttemptStatus::Passed.as_str())
        {
            coverage.covering_receipt_ids.insert(receipt.id.clone());
            coverage
                .covered_observation_ids
                .insert(requirement.id.as_str().to_string());
            passed.push(receipt.id.clone());
        } else {
            coverage.rejected_receipt_ids.insert(receipt.id.clone());
            if observation.get("outcome").and_then(Value::as_str)
                != Some(AttemptStatus::Passed.as_str())
            {
                failed.push(receipt.id.clone());
            }
            rejected_gaps.push((receipt.id.clone(), gaps));
        }
    }

    if !passed.is_empty() && !failed.is_empty() {
        coverage.status = CoverageObservationStatus::Inconclusive;
        coverage
            .gap_reasons
            .insert(GapReason::InconclusiveResult.as_str());
        push_validation_gap(
            context.validation,
            "trust",
            GapReason::InconclusiveResult.as_str(),
            &passed,
        );
    } else if !passed.is_empty()
        && !has_required_independence(
            context.conn,
            context.obligation,
            requirement,
            context.receipts,
            &passed,
        )?
    {
        coverage.status = CoverageObservationStatus::Unsatisfied;
        coverage
            .gap_reasons
            .insert(GapReason::InsufficientAssurance.as_str());
        push_validation_gap(
            context.validation,
            "provenance",
            GapReason::InsufficientAssurance.as_str(),
            &passed,
        );
    } else if !passed.is_empty() {
        coverage.status = CoverageObservationStatus::Covered;
    } else if !matching_waivers.is_empty() {
        coverage.status = CoverageObservationStatus::Waived;
        coverage.waiver_id = Some(matching_waivers[0].id.clone());
        if coverage.gap_reasons.is_empty() {
            coverage
                .gap_reasons
                .insert(GapReason::MissingObservation.as_str());
        }
    } else {
        if rejected_gaps.is_empty() {
            rejected_gaps = historical_rejected_gaps;
        }
        for (receipt_id, gaps) in rejected_gaps {
            for gap in gaps {
                coverage.gap_reasons.insert(gap);
                push_validation_gap(
                    context.validation,
                    validation_section_for_gap(gap),
                    gap,
                    std::slice::from_ref(&receipt_id),
                );
            }
        }
        if coverage.gap_reasons.is_empty() {
            coverage
                .gap_reasons
                .insert(GapReason::MissingObservation.as_str());
        }
        coverage.status = status_for_gap(primary_gap(&coverage));
    }
    Ok(coverage)
}

fn retry_aggregation_policy(value: &str) -> Result<RetryAggregationPolicy, EvidenceDomainError> {
    match value {
        "latest_applicable_pass" => Ok(RetryAggregationPolicy::LatestApplicablePass),
        "all_applicable_pass" => Ok(RetryAggregationPolicy::AllApplicablePass),
        other => Err(EvidenceDomainError::Digest(format!(
            "unsupported proof obligation retry aggregation: {other}"
        ))),
    }
}

fn observation_lineage_value(coverage: &ObservationCoverage) -> Value {
    json!({
        "requirement_id": coverage.requirement_id,
        "aggregation_policy": coverage.aggregation_policy.as_str(),
        "covering_receipt_ids": coverage.covering_receipt_ids.iter().cloned().collect::<Vec<_>>(),
        "superseded_receipt_ids": coverage.superseded_receipt_ids.iter().cloned().collect::<Vec<_>>(),
        "diagnostic_receipt_ids": coverage.diagnostic_receipt_ids.iter().cloned().collect::<Vec<_>>(),
        "rejected_receipt_ids": coverage.rejected_receipt_ids.iter().cloned().collect::<Vec<_>>(),
    })
}

fn candidate_gaps(
    conn: &Connection,
    obligation: &ObligationRow,
    requirement: &ObservationRequirement,
    receipt: &ReceiptCandidate,
    observation: &Value,
    repository_snapshot: Option<&EvidenceRepositorySnapshot>,
) -> Result<BTreeSet<&'static str>, EvidenceDomainError> {
    let mut gaps = BTreeSet::new();
    let receipt_typed = EvidenceReceipt::from_trusted_value(receipt.value.clone())?;
    if receipt_typed.receipt_digest().as_str() != receipt.digest {
        gaps.insert(GapReason::UntrustedProvenance.as_str());
    }
    let target_binding_mismatch = receipt.project_id != obligation.project_id
        || receipt.obligation_plan_id != obligation.plan_id
        || receipt.obligation_item_id != obligation.item_id
        || receipt.obligation_criterion_id != obligation.criterion_id;
    if target_binding_mismatch || receipt_typed.obligation_id().as_str() != receipt.obligation_id {
        gaps.insert(GapReason::TargetMismatch.as_str());
    }
    if !target_binding_mismatch
        && (receipt.obligation_id != obligation.id
            || receipt.obligation_version != obligation.obligation_version
            || receipt.obligation_supersedes_obligation_id != obligation.supersedes_obligation_id)
    {
        gaps.insert(GapReason::StaleConfiguration.as_str());
    }
    if !receipt.attempt_artifacts.is_empty()
        && receipt_artifact_bindings(&receipt.value) != receipt.attempt_artifacts
    {
        gaps.insert(GapReason::StaleSource.as_str());
    }
    let mut has_explicit_proof_gap = false;
    for proof_gap in receipt
        .value
        .get("proof_gaps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        has_explicit_proof_gap = true;
        gaps.insert(canonical_gap(proof_gap));
    }
    if !has_explicit_proof_gap
        && let Some(outcome_gap) = outcome_gap(observation.get("outcome").and_then(Value::as_str))
    {
        gaps.insert(outcome_gap);
    }
    if observation.get("predicate") != Some(&requirement.expected) {
        gaps.insert(GapReason::SchemaMismatch.as_str());
    }
    if !target_matches(&receipt.value["target"], &requirement.target) {
        gaps.insert(GapReason::TargetMismatch.as_str());
    }
    if let Some(schema) = &requirement.payload_schema {
        let actual_schema = observation
            .get("actual")
            .and_then(Value::as_object)
            .and_then(|actual| actual.get("schema_ref"))
            .and_then(Value::as_str);
        if actual_schema != Some(schema.schema_ref.as_str()) {
            gaps.insert(GapReason::SchemaMismatch.as_str());
        }
    }
    if receipt.value["provenance"]["assigned_by"].as_str() != Some("planr") {
        gaps.insert(GapReason::UntrustedProvenance.as_str());
    }
    let accepted_provenance = obligation
        .assurance_policy
        .get("accepted_provenance")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_else(|| {
            [ProvenanceSourceKind::PlanrObservedExecution.as_str()]
                .into_iter()
                .collect()
        });
    if !accepted_provenance.contains(
        receipt.value["provenance"]["source"]
            .as_str()
            .unwrap_or_default(),
    ) {
        gaps.insert(GapReason::UntrustedProvenance.as_str());
    }
    if obligation
        .fixture_policy
        .get("fixtures_allowed")
        .and_then(Value::as_bool)
        == Some(false)
        && receipt.value["fixture_disclosure"]["fixtures_used"].as_bool() == Some(true)
    {
        gaps.insert(GapReason::FixtureDisallowed.as_str());
    }
    if obligation
        .fixture_policy
        .get("mocks_allowed")
        .and_then(Value::as_bool)
        == Some(false)
        && receipt.value["fixture_disclosure"]["mocks_used"].as_bool() == Some(true)
    {
        gaps.insert(GapReason::MockDisallowed.as_str());
    }
    if invalidate_on(&obligation.freshness_policy, "source_change")
        && repository_snapshot.is_some_and(|snapshot| receipt_typed.source() != &snapshot.source)
    {
        gaps.insert(GapReason::StaleSource.as_str());
    }
    if invalidate_on(&obligation.freshness_policy, "target_change")
        && stale_target(&receipt.value["target"], &requirement.target)
    {
        gaps.insert(GapReason::StaleTarget.as_str());
    }
    if invalidate_on(&obligation.freshness_policy, "policy_change")
        && policy_is_stale(
            conn,
            obligation,
            &receipt_typed,
            &receipt.trusted_binding,
            repository_snapshot,
        )?
    {
        gaps.insert(GapReason::StalePolicy.as_str());
    }
    if invalidate_on(&obligation.freshness_policy, "configuration_change")
        && capability_instance_is_stale(conn, &receipt.value["capability"])?
    {
        gaps.insert(GapReason::StaleConfiguration.as_str());
    }
    if invalidate_on(&obligation.freshness_policy, "environment_change")
        && capability_environment_is_stale(
            conn,
            &receipt.value["capability"],
            receipt_typed.environment(),
        )?
    {
        gaps.insert(GapReason::StaleEnvironment.as_str());
    }
    if invalidate_on(&obligation.freshness_policy, "adapter_schema_change")
        && manifest_is_stale(conn, &receipt.value["capability"])?
    {
        gaps.insert(GapReason::StaleAdapterSchema.as_str());
    }
    Ok(gaps)
}

fn outcome_gap(outcome: Option<&str>) -> Option<&'static str> {
    match outcome {
        Some("passed") => None,
        Some("failed") => Some(GapReason::ProductFailed.as_str()),
        Some("timed_out") => Some(GapReason::TimedOut.as_str()),
        Some("aborted") => Some(GapReason::Aborted.as_str()),
        Some("inconclusive") => Some(GapReason::InconclusiveResult.as_str()),
        Some("unavailable") => Some(GapReason::MissingCapability.as_str()),
        Some("skipped") => Some(GapReason::VerifierFailed.as_str()),
        _ => Some(GapReason::VerifierFailed.as_str()),
    }
}

fn canonical_gap(gap: &str) -> &'static str {
    GapReason::canonicalize(gap).as_str()
}

fn load_obligation(
    conn: &Connection,
    project_id: &str,
    obligation_id: &str,
) -> Result<ObligationRow, EvidenceDomainError> {
    let row = conn
        .query_row(
            "SELECT id, project_id, plan_id, item_id, criterion_id, obligation_version,
                    observation_requirements_json, fixture_policy_json,
                    freshness_policy_json, assurance_policy_json, retry_aggregation,
                    supersedes_obligation_id
             FROM proof_obligations
             WHERE project_id = ?1 AND id = ?2",
            params![project_id, obligation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            },
        )
        .optional()
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .ok_or(EvidenceDomainError::MissingTrustedBinding(
            "proof_obligation",
        ))?;
    let observations = serde_json::from_str::<Vec<ObservationRequirement>>(&row.6)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    Ok(ObligationRow {
        id: row.0,
        project_id: row.1,
        plan_id: row.2,
        item_id: row.3,
        criterion_id: row.4,
        obligation_version: row.5,
        supersedes_obligation_id: row.11,
        observations,
        fixture_policy: serde_json::from_str(&row.7)
            .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?,
        freshness_policy: serde_json::from_str(&row.8)
            .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?,
        assurance_policy: serde_json::from_str(&row.9)
            .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?,
        retry_aggregation: retry_aggregation_policy(&row.10)?,
    })
}

fn load_authoritative_obligation(
    conn: &Connection,
    project_id: &str,
    obligation_id: &str,
) -> Result<ObligationRow, EvidenceDomainError> {
    let id = conn
        .query_row(
            "SELECT id
             FROM proof_obligations
             WHERE project_id = ?1 AND id = ?2
               AND binding = 1
               AND NOT EXISTS (
                 SELECT 1
                 FROM proof_obligations AS superseding
                 WHERE superseding.supersedes_obligation_id = proof_obligations.id
                   AND superseding.project_id = proof_obligations.project_id
               )
             LIMIT 1",
            params![project_id, obligation_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .ok_or(EvidenceDomainError::MissingTrustedBinding(
            "proof_obligation",
        ))?;
    load_obligation(conn, project_id, &id)
}

fn load_obligations_for_scope(
    conn: &Connection,
    project_id: &str,
    where_clause: &str,
    scope_id: &str,
) -> Result<Vec<ObligationRow>, EvidenceDomainError> {
    let ids =
        authoritative_obligation_bindings_by_clause(conn, project_id, where_clause, scope_id)?
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
    ids.iter()
        .map(|id| load_obligation(conn, project_id, id))
        .collect()
}

fn authoritative_obligation_bindings_by_clause(
    conn: &Connection,
    project_id: &str,
    where_clause: &str,
    scope_id: &str,
) -> Result<Vec<AuthoritativeObligationBindingRow>, EvidenceDomainError> {
    let sql = authoritative_binding_selection_sql("id, observation_requirements_json", where_clause);
    let mut statement = conn
        .prepare(&sql)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    statement
        .query_map(params![project_id, scope_id], |row| {
            Ok(AuthoritativeObligationBindingRow {
                id: row.get(0)?,
                observations: serde_json::from_str(&row.get::<_, String>(1)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
            })
        })
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .map(|row| row.map_err(|err| EvidenceDomainError::Digest(err.to_string())))
        .collect::<Result<Vec<_>, _>>()
}

fn authoritative_binding_selection_sql(columns: &str, where_clause: &str) -> String {
    format!(
        "SELECT {columns}
         FROM proof_obligations
         {where_clause}
           AND binding = 1
           AND NOT EXISTS (
             SELECT 1
             FROM proof_obligations AS superseding
             WHERE superseding.supersedes_obligation_id = proof_obligations.id
               AND superseding.project_id = proof_obligations.project_id
           )
         ORDER BY plan_id, item_id, criterion_id, obligation_version, id"
    )
}

fn load_authoritative_criterion_obligation_ids(
    conn: &Connection,
    project_id: &str,
    criterion_id: &str,
) -> Result<Vec<String>, EvidenceDomainError> {
    let active_clause = "obligations.project_id = ?1
        AND obligations.criterion_id = ?2
        AND obligations.binding = 1
        AND NOT EXISTS (
          SELECT 1
          FROM proof_obligations AS superseding
          WHERE superseding.supersedes_obligation_id = obligations.id
            AND superseding.project_id = obligations.project_id
        )";
    let load_plan_ids = |sql: &str| -> Result<Vec<String>, EvidenceDomainError> {
        let mut statement = conn
            .prepare(sql)
            .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
        statement
            .query_map(params![project_id, criterion_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
            .map(|row| row.map_err(|err| EvidenceDomainError::Digest(err.to_string())))
            .collect::<Result<Vec<_>, _>>()
    };
    let run_owned = load_plan_ids(&format!(
        "SELECT DISTINCT obligations.plan_id
         FROM proof_obligations AS obligations
         JOIN feature_runs AS runs
           ON runs.project_id = obligations.project_id
          AND runs.plan_id = obligations.plan_id
          AND runs.status IN ('active', 'held')
         WHERE {active_clause}
         ORDER BY obligations.plan_id"
    ))?;
    let candidates = if run_owned.is_empty() {
        load_plan_ids(&format!(
            "SELECT DISTINCT obligations.plan_id
             FROM proof_obligations AS obligations
             WHERE {active_clause}
             ORDER BY obligations.plan_id"
        ))?
    } else {
        run_owned
    };
    let [plan_id] = candidates.as_slice() else {
        return Err(EvidenceDomainError::MissingTrustedBinding(
            "proof_obligation",
        ));
    };
    let mut statement = conn
        .prepare(
            "SELECT id
             FROM proof_obligations
             WHERE project_id = ?1
               AND plan_id = ?2
               AND criterion_id = ?3
               AND binding = 1
               AND NOT EXISTS (
                 SELECT 1
                 FROM proof_obligations AS superseding
                 WHERE superseding.supersedes_obligation_id = proof_obligations.id
                   AND superseding.project_id = proof_obligations.project_id
               )
             ORDER BY item_id, obligation_version, id",
        )
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    statement
        .query_map(params![project_id, plan_id, criterion_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .map(|row| row.map_err(|err| EvidenceDomainError::Digest(err.to_string())))
        .collect::<Result<Vec<_>, _>>()
}

fn load_receipts(
    conn: &Connection,
    project_id: &str,
    obligation_id: &str,
) -> Result<Vec<ReceiptCandidate>, EvidenceDomainError> {
    let obligation_ids = receipt_obligation_ids(conn, project_id, obligation_id)?;
    let placeholders = std::iter::repeat_n("?", obligation_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT receipts.id, receipts.receipt_digest, receipts.project_id, receipts.obligation_id,
                receipts.attempt_id,
                obligations.plan_id, obligations.item_id, obligations.criterion_id,
                obligations.obligation_version, obligations.supersedes_obligation_id,
                receipts.trusted_binding_json, receipts.receipt_json,
                receipts.supersedes_receipt_id
         FROM evidence_receipts AS receipts
         JOIN proof_obligations AS obligations
           ON obligations.project_id = receipts.project_id
          AND obligations.id = receipts.obligation_id
         WHERE receipts.project_id = ?
           AND receipts.obligation_id IN ({placeholders})
           AND receipts.receipt_status = 'trusted'
           AND NOT EXISTS (
             SELECT 1
             FROM feature_run_evidence_invalidations AS invalidations
             JOIN feature_runs AS runs ON runs.id = invalidations.run_id
             JOIN json_each(invalidations.affected_evidence_ids_json) AS affected
             WHERE runs.project_id = receipts.project_id
               AND affected.value = receipts.id
           )
         ORDER BY receipts.created_at, receipts.id"
    );
    let mut args = Vec::with_capacity(obligation_ids.len() + 1);
    args.push(project_id.to_string());
    args.extend(obligation_ids);
    let mut statement = conn
        .prepare(&sql)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    statement
        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, Option<String>>(12)?,
            ))
        })
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .map(|row| {
            let (
                id,
                digest,
                candidate_project_id,
                obligation_id,
                attempt_id,
                obligation_plan_id,
                obligation_item_id,
                obligation_criterion_id,
                obligation_version,
                obligation_supersedes_obligation_id,
                trusted_binding_json,
                receipt_json,
                supersedes_receipt_id,
            ) = row.map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
            let value: Value = serde_json::from_str(&receipt_json)
                .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
            let receipt = EvidenceReceipt::from_trusted_value(value.clone())?;
            let trusted_binding = parse_trusted_receipt_binding(&trusted_binding_json, &receipt)?;
            let attempt_artifacts = load_attempt_artifacts(conn, &attempt_id)?;
            Ok(ReceiptCandidate {
                id,
                digest,
                project_id: candidate_project_id,
                obligation_id,
                obligation_plan_id,
                obligation_item_id,
                obligation_criterion_id,
                obligation_version,
                obligation_supersedes_obligation_id,
                attempt_artifacts,
                supersedes_receipt_id,
                value,
                trusted_binding,
            })
        })
        .collect()
}

fn load_attempt_artifacts(
    conn: &Connection,
    attempt_id: &str,
) -> Result<Vec<ArtifactBinding>, EvidenceDomainError> {
    let mut artifacts = conn
        .prepare(
            "SELECT attempt_artifacts.logical_name, attempt_artifacts.path,
                    attempt_artifacts.digest, attempt_artifacts.root_kind,
                    attempt_artifacts.artifact_id, artifacts.kind, artifacts.path
             FROM evidence_attempt_artifacts AS attempt_artifacts
             LEFT JOIN artifacts ON artifacts.id = attempt_artifacts.artifact_id
             WHERE attempt_artifacts.attempt_id = ?1
             ORDER BY attempt_artifacts.logical_name, attempt_artifacts.id",
        )
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .query_map(params![attempt_id], |row| {
            let logical_name: String = row.get(0)?;
            let path: Option<String> = row.get(1)?;
            let digest: String = row.get(2)?;
            let root_kind: String = row.get(3)?;
            let artifact_id: Option<String> = row.get(4)?;
            let artifact_kind: Option<String> = row.get(5)?;
            let artifact_path: Option<String> = row.get(6)?;
            Ok(ArtifactBinding {
                id: artifact_id.unwrap_or_else(|| logical_name.clone()),
                kind: artifact_kind.unwrap_or(logical_name),
                digest,
                uri: path.or(artifact_path).map(|path| format!("file://{path}")),
                root_kind: Some(root_kind),
            })
        })
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    artifacts.sort();
    Ok(artifacts)
}

fn receipt_artifact_bindings(receipt: &Value) -> Vec<ArtifactBinding> {
    let mut artifacts = receipt
        .get("artifacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|artifact| {
            let id = artifact.get("id")?.as_str()?;
            let kind = artifact.get("kind")?.as_str()?;
            let digest = artifact.get("digest")?.as_str()?;
            Some(ArtifactBinding {
                id: id.to_string(),
                kind: kind.to_string(),
                digest: digest.to_string(),
                uri: artifact
                    .get("uri")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                root_kind: artifact
                    .get("root_kind")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect::<Vec<_>>();
    artifacts.sort();
    artifacts
}

fn receipt_obligation_ids(
    conn: &Connection,
    project_id: &str,
    obligation_id: &str,
) -> Result<Vec<String>, EvidenceDomainError> {
    let mut ids = vec![obligation_id.to_string()];
    let mut current = obligation_id.to_string();
    loop {
        let supersedes = conn
            .query_row(
                "SELECT supersedes_obligation_id
                 FROM proof_obligations
                 WHERE project_id = ?1 AND id = ?2",
                params![project_id, current],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
            .flatten();
        let Some(previous) = supersedes else {
            break;
        };
        if ids.iter().any(|id| id == &previous) {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "proof_obligation.supersedes",
            ));
        }
        ids.push(previous.clone());
        current = previous;
    }
    Ok(ids)
}

fn load_waivers(
    conn: &Connection,
    project_id: &str,
    obligation_id: &str,
    evaluated_at: &str,
) -> Result<Vec<WaiverCandidate>, EvidenceDomainError> {
    let mut statement = conn
        .prepare(
            "SELECT id, observation_id, waiver_digest, waiver_json
             FROM evidence_waivers
             WHERE project_id = ?1 AND obligation_id = ?2 AND expires_at > ?3
             ORDER BY created_at, id",
        )
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    statement
        .query_map(params![project_id, obligation_id, evaluated_at], |row| {
            let id = row.get::<_, String>(0)?;
            let observation_id = row.get::<_, String>(1)?;
            let waiver_json = row.get::<_, String>(3)?;
            let waiver = serde_json::from_str::<EvidenceWaiver>(&waiver_json)
                .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
            Ok(WaiverCandidate {
                id,
                observation_id,
                digest: row.get(2)?,
                waiver,
            })
        })
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .map(|row| row.map_err(|err| EvidenceDomainError::Digest(err.to_string())))
        .collect()
}

fn persist_coverage_verdict(
    conn: &Connection,
    project_id: &str,
    scope: &EvaluationScope,
    obligations: &[ObligationRow],
    coverage: PersistCoverage<'_>,
) -> Result<(), EvidenceDomainError> {
    let binding_digest = sha256_json_digest(&json!({
        "scope_kind": scope.kind.as_str(),
        "scope_id": scope.id,
        "obligations": obligations.iter().map(|obligation| json!({
            "id": obligation.id,
            "observations": obligation.observations,
        })).collect::<Vec<_>>(),
    }))
    .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    let policy_digest = combined_obligation_digest("semantic-policy", obligations, |obligation| {
        json!({
            "fixture_policy": obligation.fixture_policy,
            "freshness_policy": obligation.freshness_policy,
            "assurance_policy": obligation.assurance_policy,
        })
    })?;
    let config_digest =
        combined_obligation_digest("semantic-requirements", obligations, |obligation| {
            json!({
                "observations": obligation.observations,
                "retry_aggregation": obligation.retry_aggregation.as_str(),
            })
        })?;
    let verdict_json = serde_json::to_string(coverage.verdict)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    let receipts_json = serde_json::to_string(coverage.receipt_digests)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    let waivers_json = serde_json::to_string(coverage.waiver_digests)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    conn.execute(
        "INSERT INTO coverage_verdicts(
          id, project_id, scope_kind, scope_id, binding_digest, policy_digest,
          config_digest, coverage_status, source_receipt_digest_set,
          waiver_digest_set, verdict_json, computed_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        ON CONFLICT(project_id, scope_kind, scope_id, binding_digest, policy_digest, config_digest)
        DO UPDATE SET
          coverage_status = excluded.coverage_status,
          source_receipt_digest_set = excluded.source_receipt_digest_set,
          waiver_digest_set = excluded.waiver_digest_set,
          verdict_json = excluded.verdict_json,
          computed_at = excluded.computed_at",
        params![
            coverage.id,
            project_id,
            scope.kind.as_str(),
            scope.id,
            binding_digest,
            policy_digest,
            config_digest,
            coverage.status.as_str(),
            receipts_json,
            waivers_json,
            verdict_json,
            coverage.evaluated_at,
        ],
    )
    .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    let history_id = sha256_json_digest(&json!({
        "verdict_id": coverage.id,
        "computed_at": coverage.evaluated_at,
        "status": coverage.status.as_str(),
        "receipts": coverage.receipt_digests,
        "waivers": coverage.waiver_digests,
    }))
    .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    conn.execute(
        "INSERT OR IGNORE INTO coverage_verdict_history(
          id, verdict_id, project_id, scope_kind, scope_id, binding_digest,
          policy_digest, config_digest, coverage_status, verdict_json, computed_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            format!("cvhist-{}", &history_id["sha256:".len()..24]),
            coverage.id,
            project_id,
            scope.kind.as_str(),
            scope.id,
            binding_digest,
            policy_digest,
            config_digest,
            coverage.status.as_str(),
            verdict_json,
            coverage.evaluated_at,
        ],
    )
    .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    Ok(())
}

fn target_matches(receipt_target: &Value, required_target: &Value) -> bool {
    let Some(required) = required_target.as_object() else {
        return receipt_target == required_target;
    };
    required
        .iter()
        .all(|(key, value)| receipt_target.get(key) == Some(value))
}

fn stale_target(receipt_target: &Value, required_target: &Value) -> bool {
    required_target
        .get("digest")
        .and_then(Value::as_str)
        .is_some_and(|digest| receipt_target.get("digest").and_then(Value::as_str) != Some(digest))
        || required_target
            .get("deployment_id")
            .and_then(Value::as_str)
            .is_some_and(|deployment_id| {
                receipt_target.get("deployment_id").and_then(Value::as_str) != Some(deployment_id)
            })
}

fn scope_to_value(scope: &EvaluationScope) -> Value {
    let mut value = json!({
        "kind": scope.kind.as_str(),
        "id": scope.id,
    });
    if matches!(
        scope.kind,
        ScopeKind::Criterion | ScopeKind::Item | ScopeKind::Plan
    ) {
        value["plan_id"] = json!(scope.plan_id);
    }
    if let Some(item_id) = &scope.item_id {
        value["item_id"] = json!(item_id);
    }
    if let Some(criterion_id) = &scope.criterion_id {
        value["criterion_id"] = json!(criterion_id);
    }
    value
}

fn combined_obligation_digest(
    kind: &str,
    obligations: &[ObligationRow],
    value: impl Fn(&ObligationRow) -> Value,
) -> Result<String, EvidenceDomainError> {
    sha256_json_digest(&json!({
        "kind": kind,
        "values": obligations.iter().map(value).collect::<Vec<_>>(),
    }))
    .map_err(|err| EvidenceDomainError::Digest(err.to_string()))
}

fn coverage_id(
    project_id: &str,
    scope: &EvaluationScope,
    obligations: &[ObligationRow],
) -> Result<String, EvidenceDomainError> {
    let digest = sha256_json_digest(&json!({
        "project_id": project_id,
        "scope_kind": scope.kind.as_str(),
        "scope_id": scope.id,
        "binding_digest": sha256_json_digest(&json!({
            "scope_kind": scope.kind.as_str(),
            "scope_id": scope.id,
            "obligations": obligations.iter().map(|obligation| json!({
                "id": obligation.id,
                "observations": obligation.observations,
            })).collect::<Vec<_>>(),
        })).map_err(|err| EvidenceDomainError::Digest(err.to_string()))?,
        "policy_digest": combined_obligation_digest("semantic-policy", obligations, |obligation| json!({
            "fixture_policy": obligation.fixture_policy,
            "freshness_policy": obligation.freshness_policy,
            "assurance_policy": obligation.assurance_policy,
        }))?,
        "config_digest": combined_obligation_digest("semantic-requirements", obligations, |obligation| json!({
            "observations": obligation.observations,
            "retry_aggregation": obligation.retry_aggregation.as_str(),
        }))?,
    }))
    .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    Ok(format!("cverdict-{}", &digest["sha256:".len()..24]))
}

fn primary_gap(coverage: &ObservationCoverage) -> &'static str {
    let priority = [
        GapReason::StaleSource.as_str(),
        GapReason::StaleTarget.as_str(),
        GapReason::StaleEnvironment.as_str(),
        GapReason::StalePolicy.as_str(),
        GapReason::StaleAdapterSchema.as_str(),
        GapReason::StaleConfiguration.as_str(),
        GapReason::MissingCapability.as_str(),
        GapReason::PermissionDenied.as_str(),
        GapReason::SandboxBlocked.as_str(),
        GapReason::EnvironmentUnavailable.as_str(),
        GapReason::ExternalDependencyUnavailable.as_str(),
        GapReason::TimedOut.as_str(),
        GapReason::Aborted.as_str(),
        GapReason::InconclusiveResult.as_str(),
        GapReason::ProductFailed.as_str(),
        GapReason::VerifierFailed.as_str(),
        GapReason::TargetMismatch.as_str(),
        GapReason::SchemaMismatch.as_str(),
        GapReason::ManifestMismatch.as_str(),
        GapReason::UntrustedProvenance.as_str(),
        GapReason::FixtureDisallowed.as_str(),
        GapReason::MockDisallowed.as_str(),
        GapReason::InsufficientAssurance.as_str(),
        GapReason::WaiverExpired.as_str(),
        GapReason::WaiverMissing.as_str(),
        GapReason::UnknownObservationType.as_str(),
        GapReason::UnsupportedRuntimeTarget.as_str(),
        GapReason::MissingObservation.as_str(),
    ];
    priority
        .into_iter()
        .find(|gap| coverage.gap_reasons.contains(gap))
        .unwrap_or(GapReason::MissingObservation.as_str())
}

fn status_for_gap(gap: &str) -> CoverageObservationStatus {
    match gap {
        "stale_source"
        | "stale_target"
        | "stale_environment"
        | "stale_policy"
        | "stale_adapter_schema"
        | "stale_configuration" => CoverageObservationStatus::Stale,
        "missing_capability"
        | "permission_denied"
        | "sandbox_blocked"
        | "environment_unavailable"
        | "external_dependency_unavailable" => CoverageObservationStatus::Blocked,
        "timed_out" | "aborted" | "inconclusive_result" => CoverageObservationStatus::Inconclusive,
        "missing_observation" => CoverageObservationStatus::Missing,
        _ => CoverageObservationStatus::Unsatisfied,
    }
}

fn waiver_covers(
    scope: &EvaluationScope,
    requirement: &ObservationRequirement,
    candidate: &WaiverCandidate,
    repository_snapshot: Option<&EvidenceRepositorySnapshot>,
) -> bool {
    let Some(repository_snapshot) = repository_snapshot else {
        return false;
    };
    let current_source = &repository_snapshot.source;
    candidate.observation_id == requirement.id.as_str()
        && candidate
            .waiver
            .observation_ids
            .iter()
            .any(|id| id.as_str() == requirement.id.as_str())
        && candidate.waiver.scope.kind == scope.kind.as_str()
        && candidate.waiver.scope.id.as_str() == scope.id
        && candidate.waiver.source.revision == current_source.revision
        && candidate.waiver.source.tree_digest.as_str() == current_source.tree_digest.as_str()
        && candidate.waiver.source.dirty == current_source.dirty
        && target_matches(
            &serde_json::to_value(&candidate.waiver.target).unwrap_or(Value::Null),
            &requirement.target,
        )
        && target_matches(
            &requirement.target,
            &serde_json::to_value(&candidate.waiver.target).unwrap_or(Value::Null),
        )
}

fn has_required_independence(
    conn: &Connection,
    obligation: &ObligationRow,
    requirement: &ObservationRequirement,
    receipts: &[ReceiptCandidate],
    passed_receipt_ids: &[String],
) -> Result<bool, EvidenceDomainError> {
    let required = requirement
        .assurance_policy
        .as_ref()
        .and_then(|policy| policy.get("independent_methods"))
        .or_else(|| obligation.assurance_policy.get("independent_methods"))
        .and_then(Value::as_u64)
        .unwrap_or(1);
    if required <= 1 {
        return Ok(true);
    }
    let mut methods = BTreeSet::new();
    for receipt in receipts {
        if !passed_receipt_ids.iter().any(|id| id == &receipt.id) {
            continue;
        }
        let method = capability_method_identity(conn, &receipt.value["capability"])?
            .unwrap_or_else(|| "unknown".to_string());
        methods.insert(method);
    }
    Ok(methods.len() as u64 >= required)
}

fn capability_method_identity(
    conn: &Connection,
    capability: &Value,
) -> Result<Option<String>, EvidenceDomainError> {
    let Some(manifest_id) = capability.get("manifest_id").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(manifest_digest) = capability.get("manifest_digest").and_then(Value::as_str) else {
        return Ok(None);
    };
    let manifest_exists = conn
        .query_row(
            "SELECT 1
             FROM verification_capability_manifests
             WHERE id = ?1 AND manifest_digest = ?2
             LIMIT 1",
            params![manifest_id, manifest_digest],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    Ok(manifest_exists.map(|_| manifest_id.to_string()))
}

fn policy_is_stale(
    _conn: &Connection,
    obligation: &ObligationRow,
    receipt: &EvidenceReceipt,
    trusted_binding: &TrustedReceiptBinding,
    repository_snapshot: Option<&EvidenceRepositorySnapshot>,
) -> Result<bool, EvidenceDomainError> {
    if receipt.obligation_id().as_str() != obligation.id {
        return Ok(true);
    }
    trusted_binding.validate_receipt_exact(receipt)?;
    Ok(repository_snapshot
        .and_then(|snapshot| snapshot.policy.as_ref())
        .is_none_or(|current| current.digest.as_str() != trusted_binding.policy_digest.as_str()))
}

fn project_root(
    conn: &Connection,
    project_id: &str,
) -> Result<Option<PathBuf>, EvidenceDomainError> {
    conn.query_row(
        "SELECT root_path FROM projects WHERE id = ?1 LIMIT 1",
        params![project_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map(|path| path.map(PathBuf::from))
    .map_err(|err| EvidenceDomainError::Digest(err.to_string()))
}

fn manifest_is_stale(conn: &Connection, capability: &Value) -> Result<bool, EvidenceDomainError> {
    let Some(manifest_id) = capability.get("manifest_id").and_then(Value::as_str) else {
        return Ok(true);
    };
    let Some(manifest_digest) = capability.get("manifest_digest").and_then(Value::as_str) else {
        return Ok(true);
    };
    let current = conn
        .query_row(
            "SELECT manifest_digest
             FROM verification_capability_manifests
             WHERE id = ?1
             ORDER BY created_at DESC, version DESC, manifest_digest DESC
             LIMIT 1",
            params![manifest_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    Ok(current.is_some_and(|current| current != manifest_digest))
}

struct CurrentCapabilityState {
    instance_id: String,
    instance: Option<VerificationCapabilityInstance>,
}

fn current_capability_instance(
    conn: &Connection,
    capability: &Value,
) -> Result<Option<CurrentCapabilityState>, EvidenceDomainError> {
    let Some(manifest_id) = capability.get("manifest_id").and_then(Value::as_str) else {
        return Ok(None);
    };
    let current = conn
        .query_row(
            "SELECT instances.id, instances.capability_snapshot_json
             FROM verification_capability_instances AS instances
             JOIN verification_capability_manifests AS manifests
               ON manifests.id = instances.manifest_id
              AND manifests.version = instances.manifest_version
              AND manifests.manifest_digest = instances.manifest_digest
             WHERE instances.manifest_id = ?1
               AND instances.availability_status = 'available'
             ORDER BY manifests.created_at DESC, manifests.version DESC,
                      instances.created_at DESC, instances.id DESC
             LIMIT 1",
            params![manifest_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    Ok(
        current.map(|(instance_id, snapshot)| CurrentCapabilityState {
            instance_id,
            instance: serde_json::from_str(&snapshot).ok(),
        }),
    )
}

fn capability_instance_is_stale(
    conn: &Connection,
    capability: &Value,
) -> Result<bool, EvidenceDomainError> {
    let Some(current) = current_capability_instance(conn, capability)? else {
        return Ok(true);
    };
    Ok(capability.get("instance_id").and_then(Value::as_str) != Some(current.instance_id.as_str()))
}

fn capability_environment_is_stale(
    conn: &Connection,
    capability: &Value,
    receipt_environment: &super::model::EnvironmentBinding,
) -> Result<bool, EvidenceDomainError> {
    Ok(
        current_capability_instance(conn, capability)?.is_none_or(|current| {
            current
                .instance
                .is_some_and(|instance| instance.environment != *receipt_environment)
        }),
    )
}

fn invalidate_on(policy: &Value, event: &str) -> bool {
    policy
        .get("invalidate_on")
        .and_then(Value::as_array)
        .is_none_or(|events| events.iter().any(|value| value.as_str() == Some(event)))
}

fn aggregate_observation_status(observations: &[Value]) -> CoverageStatus {
    let statuses = observations
        .iter()
        .filter_map(|value| value.get("status").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    if statuses.iter().all(|status| *status == "covered") {
        CoverageStatus::Satisfied
    } else if statuses.contains("stale") {
        CoverageStatus::Stale
    } else if statuses.contains("blocked") {
        CoverageStatus::Blocked
    } else if statuses.contains("inconclusive") {
        CoverageStatus::Inconclusive
    } else if statuses
        .iter()
        .all(|status| *status == "covered" || *status == "waived")
    {
        CoverageStatus::Waived
    } else {
        CoverageStatus::Unsatisfied
    }
}

fn coverage_to_value(coverage: ObservationCoverage) -> Value {
    if coverage.status == CoverageObservationStatus::Covered {
        return json!({
            "requirement_id": coverage.requirement_id,
            "status": coverage.status.as_str(),
            "covering_receipt_ids": coverage.covering_receipt_ids.into_iter().collect::<Vec<_>>(),
            "covered_observation_ids": coverage.covered_observation_ids.into_iter().collect::<Vec<_>>(),
        });
    }
    let mut value = json!({
        "requirement_id": coverage.requirement_id,
        "status": coverage.status.as_str(),
        "gap_reason": primary_gap(&coverage),
        "gap_reasons": coverage.gap_reasons.iter().copied().collect::<Vec<_>>(),
    });
    if !coverage.attempted_receipt_ids.is_empty() {
        value["attempted_receipt_ids"] = json!(
            coverage
                .attempted_receipt_ids
                .into_iter()
                .collect::<Vec<_>>()
        );
    }
    if let Some(waiver_id) = coverage.waiver_id {
        value["waiver_id"] = json!(waiver_id);
    }
    value
}

fn validation_scaffold(evaluated_at: &str) -> Map<String, Value> {
    [
        "trust",
        "freshness",
        "target",
        "schema",
        "provenance",
        "fixture",
        "completion",
    ]
    .into_iter()
    .map(|name| {
        (
            name.to_string(),
            json!({
                "status": "passed",
                "checked_at": evaluated_at,
                "receipt_ids": [],
                "gap_reasons": [],
            }),
        )
    })
    .collect()
}

fn finalize_validation(validation: &mut Map<String, Value>, receipt_ids: &BTreeSet<String>) {
    for detail in validation.values_mut() {
        if !detail.is_object() {
            continue;
        }
        detail["receipt_ids"] = json!(receipt_ids.iter().cloned().collect::<Vec<_>>());
        if detail["gap_reasons"]
            .as_array()
            .is_some_and(|gaps| !gaps.is_empty())
        {
            detail["status"] = json!("failed");
        }
    }
}

fn push_validation_gap(
    validation: &mut Map<String, Value>,
    section: &str,
    gap: &str,
    receipt_ids: &[String],
) {
    if let Some(detail) = validation.get_mut(section) {
        detail["status"] = json!("failed");
        detail["receipt_ids"] = json!(receipt_ids);
        let mut gaps = detail["gap_reasons"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        gaps.push(json!(gap));
        gaps.sort_by_key(|value| value.as_str().unwrap_or_default().to_string());
        gaps.dedup();
        detail["gap_reasons"] = Value::Array(gaps);
    }
}

fn push_string_refs(out: &mut Vec<Value>, value: &Value, key: &str) {
    for entry in value[key].as_array().into_iter().flatten() {
        if let Some(text) = entry.as_str() {
            out.push(json!(text));
        }
    }
}

fn canonical_gap_is_actionable(status: &str) -> bool {
    matches!(status, "missing" | "unsatisfied" | "inconclusive" | "stale")
}

fn suggested_next_action_for_status(status: &str) -> &'static str {
    match status {
        "satisfied" | "waived" => "none",
        "stale" => "refresh stale evidence",
        "blocked" => "restore evidence capability",
        "inconclusive" => "inspect exhausted or inconclusive verifier evidence",
        _ => "collect missing trusted evidence",
    }
}

fn validation_section_for_gap(gap: &str) -> &'static str {
    match gap {
        "untrusted_provenance" | "insufficient_assurance" => "provenance",
        "stale_source"
        | "stale_policy"
        | "stale_adapter_schema"
        | "stale_configuration"
        | "stale_environment"
        | "stale_target" => "freshness",
        "target_mismatch" => "target",
        "schema_mismatch" | "manifest_mismatch" => "schema",
        "fixture_disallowed" | "mock_disallowed" => "fixture",
        "missing_observation" | "unknown_observation_type" | "unsupported_runtime_target" => {
            "completion"
        }
        "inconclusive_result" => "trust",
        _ => "trust",
    }
}

fn suggested_next_action(status: &CoverageStatus) -> &'static str {
    match status {
        CoverageStatus::Satisfied | CoverageStatus::Waived => "none",
        CoverageStatus::Stale => "refresh stale evidence",
        CoverageStatus::Blocked => "restore evidence capability",
        CoverageStatus::Inconclusive => "inspect exhausted or inconclusive verifier evidence",
        CoverageStatus::Unsatisfied => "collect missing trusted evidence",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_json::sha256_json_digest_without_top_level_field;
    use crate::evidence::model::{
        ArtifactRef, CapabilityBinding, CoverageVerdict, EnvironmentBinding, EvidenceId,
        FixtureDisclosure, PermissionState, RawResultRef, SandboxLimits, SandboxState,
        Sha256Digest, SourceBinding, TargetBinding, TrustedProvenance, TrustedReceiptInput,
        VantagePoint, build_trusted_receipt,
    };
    use rusqlite::Connection;
    use std::{fs, process::Command};
    use tempfile::tempdir;

    const DIGEST_A: &str =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DIGEST_B: &str =
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const DIGEST_C: &str =
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const DIGEST_D: &str =
        "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

    struct ReceiptSeed<'a> {
        receipt_id: &'a str,
        attempt_id: &'a str,
        obligation_id: &'a str,
        manifest_id: &'a str,
        instance_id: &'a str,
        observation_id: &'a str,
        status_code: u64,
        manifest_digest: &'a str,
        fixtures_used: bool,
        outcome: AttemptStatus,
    }

    #[test]
    fn coverage_evaluation_matches_union_persists_and_reports_gaps() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_obligation(&conn, &["obs-one", "obs-two"]);
        seed_receipt(
            &conn,
            "receipt-one",
            "attempt-one",
            "obs-one",
            200,
            DIGEST_C,
            false,
        );
        seed_receipt(
            &conn,
            "receipt-two",
            "attempt-two",
            "obs-two",
            201,
            DIGEST_C,
            false,
        );

        let evaluation = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();
        assert_coverage_schema_valid(&evaluation.verdict);

        assert_eq!(evaluation.status, CoverageStatus::Satisfied);
        assert_eq!(evaluation.verdict["scope"]["kind"], "criterion");
        assert_eq!(evaluation.receipt_digests.len(), 2);
        assert_eq!(
            evaluation.verdict["observation_coverage"][0]["status"],
            "covered"
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM coverage_verdicts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );

        seed_obligation_with_id(&conn, "obl-missing", &["obs-one", "obs-missing"]);
        let missing = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-missing",
            "2026-07-29T00:01:00Z",
        )
        .unwrap();
        assert_coverage_schema_valid(&missing.verdict);
        assert_eq!(missing.status, CoverageStatus::Unsatisfied);
        assert_eq!(
            missing.verdict["observation_coverage"][1]["gap_reason"],
            "missing_observation"
        );
    }

    #[test]
    fn coverage_evaluation_distinguishes_policy_schema_stale_and_flaky_candidates() {
        let fixture_conn = conn();
        seed_project(&fixture_conn);
        seed_manifest(&fixture_conn, DIGEST_C);
        seed_obligation(&fixture_conn, &["obs-one"]);
        seed_receipt(
            &fixture_conn,
            "receipt-fixture",
            "attempt-fixture",
            "obs-one",
            200,
            DIGEST_C,
            true,
        );
        let fixture = evaluate_obligation_coverage(
            &fixture_conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();
        assert_coverage_schema_valid(&fixture.verdict);
        assert_eq!(fixture.status, CoverageStatus::Unsatisfied);
        assert_eq!(
            fixture.verdict["observation_coverage"][0]["gap_reason"],
            "fixture_disallowed"
        );

        let stale_conn = conn();
        seed_project(&stale_conn);
        seed_manifest(&stale_conn, DIGEST_C);
        seed_obligation(&stale_conn, &["obs-one"]);
        seed_manifest(&stale_conn, DIGEST_D);
        seed_receipt(
            &stale_conn,
            "receipt-stale",
            "attempt-stale",
            "obs-one",
            200,
            DIGEST_C,
            false,
        );
        let stale = evaluate_obligation_coverage(
            &stale_conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();
        assert_coverage_schema_valid(&stale.verdict);
        assert_eq!(stale.status, CoverageStatus::Stale);

        let flaky_conn = conn();
        seed_project(&flaky_conn);
        seed_manifest(&flaky_conn, DIGEST_C);
        seed_obligation(&flaky_conn, &["obs-one"]);
        seed_receipt(
            &flaky_conn,
            "receipt-pass",
            "attempt-pass",
            "obs-one",
            200,
            DIGEST_C,
            false,
        );
        seed_failed_receipt(
            &flaky_conn,
            "receipt-fail",
            "attempt-fail",
            "obs-one",
            DIGEST_C,
        );
        let flaky = evaluate_obligation_coverage(
            &flaky_conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();
        assert_coverage_schema_valid(&flaky.verdict);
        assert_eq!(flaky.status, CoverageStatus::Inconclusive);
        assert_eq!(
            flaky.verdict["validation_details"]["trust"]["gap_reasons"][0],
            "inconclusive_result"
        );
    }

    #[test]
    fn coverage_evaluation_validates_waived_blocked_and_scoped_aggregates() {
        let waived_conn = conn();
        seed_project(&waived_conn);
        seed_manifest(&waived_conn, DIGEST_C);
        seed_obligation(&waived_conn, &["obs-one"]);
        seed_waiver(&waived_conn, "waiver-one", "obs-one");
        let waived = evaluate_obligation_coverage(
            &waived_conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();
        assert_eq!(waived.status, CoverageStatus::Waived);
        assert_eq!(
            waived.verdict["observation_coverage"][0]["waiver_id"],
            "waiver-one"
        );
        assert_coverage_schema_valid(&waived.verdict);

        let blocked_conn = conn();
        seed_project(&blocked_conn);
        seed_manifest(&blocked_conn, DIGEST_C);
        seed_obligation(&blocked_conn, &["obs-one"]);
        seed_unavailable_receipt(
            &blocked_conn,
            "receipt-unavailable",
            "attempt-unavailable",
            "obs-one",
            DIGEST_C,
        );
        let blocked = evaluate_obligation_coverage(
            &blocked_conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();
        assert_eq!(blocked.status, CoverageStatus::Blocked);
        assert_eq!(
            blocked.verdict["observation_coverage"][0]["gap_reason"],
            "missing_capability"
        );
        assert_coverage_schema_valid(&blocked.verdict);

        let aggregate_conn = conn();
        seed_project(&aggregate_conn);
        seed_manifest(&aggregate_conn, DIGEST_C);
        seed_obligation(&aggregate_conn, &["obs-one"]);
        seed_receipt(
            &aggregate_conn,
            "receipt-one",
            "attempt-one",
            "obs-one",
            200,
            DIGEST_C,
            false,
        );
        for (kind, evaluation) in [
            (
                "criterion",
                evaluate_criterion_coverage(
                    &aggregate_conn,
                    "p-evidence",
                    "crit-coverage",
                    "2026-07-29T00:00:00Z",
                )
                .unwrap(),
            ),
            (
                "item",
                evaluate_item_coverage(
                    &aggregate_conn,
                    "p-evidence",
                    "i-evidence",
                    "2026-07-29T00:00:01Z",
                )
                .unwrap(),
            ),
            (
                "plan",
                evaluate_plan_coverage(
                    &aggregate_conn,
                    "p-evidence",
                    "pln-evidence",
                    "2026-07-29T00:00:02Z",
                )
                .unwrap(),
            ),
        ] {
            assert_eq!(evaluation.status, CoverageStatus::Satisfied);
            assert_eq!(evaluation.verdict["scope"]["kind"], kind);
            assert_coverage_schema_valid(&evaluation.verdict);
        }
        assert_eq!(
            aggregate_conn
                .query_row("SELECT COUNT(*) FROM coverage_verdicts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            3
        );
    }

    #[test]
    fn criterion_coverage_uses_only_the_active_supersession_lineage() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_superseded_obligation_pair(&conn);
        seed_receipt_with_outcome(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-current",
                attempt_id: "attempt-current",
                obligation_id: "obl-current",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 200,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Passed,
            },
        );

        let evaluation = evaluate_criterion_coverage(
            &conn,
            "p-evidence",
            "crit-coverage",
            "2026-07-29T00:00:02Z",
        )
        .unwrap();
        assert_eq!(evaluation.status, CoverageStatus::Satisfied);
        assert_eq!(
            evaluation.verdict["observation_coverage"],
            json!([{
                "requirement_id": "obs-one",
                "status": "covered",
                "covered_observation_ids": ["obs-one"],
                "covering_receipt_ids": ["receipt-current"]
            }])
        );
    }

    #[test]
    fn criterion_coverage_anchors_reused_ids_to_the_current_plan() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_obligation(&conn, &["obs-one"]);
        seed_receipt(
            &conn,
            "receipt-current-plan",
            "attempt-current-plan",
            "obs-one",
            200,
            DIGEST_C,
            false,
        );
        conn.execute_batch(
            "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
             VALUES ('pln-older', 'p-evidence', 'build', '/tmp/planr/older.plan.md', 'Older', 'older', 'ok', 'hash-older', datetime('now'), datetime('now'));
             INSERT INTO items(id, project_id, title, description, status, work_type, priority, plan_path, created_at, updated_at)
             VALUES ('i-older', 'p-evidence', 'Older item', 'Older item', 'ready', 'code', 0, '/tmp/planr/older.plan.md', datetime('now'), datetime('now'));
             INSERT INTO feature_runs(
               id, project_id, plan_id, status, phase, policy_digest, source_revision,
               active_batch_id, outcomes_settled, batch_outcome_count, revision
             ) VALUES (
               'frun-current-plan', 'p-evidence', 'pln-evidence', 'active', 'source_frozen',
               'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
               '0123456789abcdef', NULL, 0, 0, 0
             );
             INSERT INTO proof_obligations(
               id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
               binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
               assurance_policy_json, retry_aggregation, policy_digest, config_digest, source_digest, created_at
             )
             SELECT 'obl-older-plan', project_id, 'pln-older', 'i-older', criterion_id, 1, title,
                    binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                    assurance_policy_json, retry_aggregation, policy_digest, config_digest, source_digest,
                    '2026-07-28T00:00:00Z'
             FROM proof_obligations WHERE id = 'obl-coverage';",
        )
        .unwrap();

        let evaluation = evaluate_criterion_coverage(
            &conn,
            "p-evidence",
            "crit-coverage",
            "2026-07-29T00:00:02Z",
        )
        .unwrap();
        assert_eq!(evaluation.status, CoverageStatus::Satisfied);
        assert_eq!(evaluation.verdict["scope"]["plan_id"], "pln-evidence");
        assert_eq!(
            evaluation.verdict["observation_coverage"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn criterion_coverage_fails_closed_without_an_active_binding() {
        let conn = conn();
        seed_project(&conn);
        seed_obligation(&conn, &["obs-one"]);
        conn.execute_batch(
            "INSERT INTO proof_obligations(
               id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
               binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
               assurance_policy_json, retry_aggregation, policy_digest, config_digest, source_digest,
               supersedes_obligation_id, created_at
             )
             SELECT 'obl-advisory-terminal', project_id, plan_id, item_id, criterion_id, 2, title,
                    0, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                    assurance_policy_json, retry_aggregation, policy_digest, config_digest, source_digest,
                    id, '2026-07-29T00:00:01Z'
             FROM proof_obligations WHERE id = 'obl-coverage';",
        )
        .unwrap();

        assert!(matches!(
            evaluate_criterion_coverage(
                &conn,
                "p-evidence",
                "crit-coverage",
                "2026-07-29T00:00:02Z"
            ),
            Err(EvidenceDomainError::MissingTrustedBinding(
                "proof_obligation"
            ))
        ));
    }

    #[test]
    fn plan_coverage_excludes_superseded_history_but_keeps_active_gaps() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_superseded_obligation_pair(&conn);
        seed_receipt_with_outcome(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-current",
                attempt_id: "attempt-current",
                obligation_id: "obl-current",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 200,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Passed,
            },
        );
        assert_eq!(
            evaluate_plan_coverage(&conn, "p-evidence", "pln-evidence", "2026-07-29T00:00:02Z")
                .unwrap()
                .status,
            CoverageStatus::Satisfied
        );

        seed_obligation_with_id(&conn, "obl-active-gap", &["obs-active-gap"]);
        let active_gap =
            evaluate_plan_coverage(&conn, "p-evidence", "pln-evidence", "2026-07-29T00:00:03Z")
                .unwrap();
        assert_eq!(active_gap.status, CoverageStatus::Unsatisfied);
        assert!(
            active_gap.verdict["observation_coverage"]
                .as_array()
                .unwrap()
                .iter()
                .any(|coverage| coverage["requirement_id"] == "obs-active-gap")
        );
    }

    #[test]
    fn coverage_evaluation_enforces_independent_method_counts() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_obligation_with_id(&conn, "obl-independence", &["obs-one"]);
        seed_receipt_with_outcome(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-one",
                attempt_id: "attempt-one",
                obligation_id: "obl-independence",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 200,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Passed,
            },
        );

        let evaluation = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-independence",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();

        assert_eq!(evaluation.status, CoverageStatus::Unsatisfied);
        assert_eq!(
            evaluation.verdict["observation_coverage"][0]["gap_reason"],
            "insufficient_assurance"
        );
        assert_coverage_schema_valid(&evaluation.verdict);
    }

    #[test]
    fn coverage_evaluation_counts_stable_independent_manifest_identities() {
        let duplicate_conn = conn();
        seed_project(&duplicate_conn);
        seed_manifest(&duplicate_conn, DIGEST_C);
        seed_obligation_with_id(&duplicate_conn, "obl-independence", &["obs-one"]);
        for (receipt_id, attempt_id) in [
            ("receipt-one", "attempt-one"),
            ("receipt-two", "attempt-two"),
        ] {
            seed_receipt_with_outcome(
                &duplicate_conn,
                ReceiptSeed {
                    receipt_id,
                    attempt_id,
                    obligation_id: "obl-independence",
                    manifest_id: "manifest-coverage",
                    instance_id: "instance-coverage",
                    observation_id: "obs-one",
                    status_code: 200,
                    manifest_digest: DIGEST_C,
                    fixtures_used: false,
                    outcome: AttemptStatus::Passed,
                },
            );
        }
        let duplicate = evaluate_obligation_coverage(
            &duplicate_conn,
            "p-evidence",
            "obl-independence",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();
        assert_eq!(duplicate.status, CoverageStatus::Unsatisfied);
        assert_eq!(
            duplicate.verdict["observation_coverage"][0]["gap_reason"],
            "insufficient_assurance"
        );

        let positive_conn = conn();
        seed_project(&positive_conn);
        seed_manifest_with_id(&positive_conn, "manifest-one", "instance-one", DIGEST_C);
        seed_manifest_with_id(&positive_conn, "manifest-two", "instance-two", DIGEST_D);
        seed_obligation_with_id(&positive_conn, "obl-independence", &["obs-one"]);
        for (receipt_id, attempt_id, manifest_id, instance_id, digest) in [
            (
                "receipt-one",
                "attempt-one",
                "manifest-one",
                "instance-one",
                DIGEST_C,
            ),
            (
                "receipt-two",
                "attempt-two",
                "manifest-two",
                "instance-two",
                DIGEST_D,
            ),
        ] {
            seed_receipt_with_outcome(
                &positive_conn,
                ReceiptSeed {
                    receipt_id,
                    attempt_id,
                    obligation_id: "obl-independence",
                    manifest_id,
                    instance_id,
                    observation_id: "obs-one",
                    status_code: 200,
                    manifest_digest: digest,
                    fixtures_used: false,
                    outcome: AttemptStatus::Passed,
                },
            );
        }
        let positive = evaluate_obligation_coverage(
            &positive_conn,
            "p-evidence",
            "obl-independence",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();
        assert_eq!(positive.status, CoverageStatus::Satisfied);
        assert_coverage_schema_valid(&positive.verdict);
    }

    #[test]
    fn coverage_evaluation_ignores_rejected_candidates_when_clean_cover_exists() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_obligation(&conn, &["obs-one"]);
        seed_receipt(
            &conn,
            "receipt-pass",
            "attempt-pass",
            "obs-one",
            200,
            DIGEST_C,
            false,
        );
        seed_receipt(
            &conn,
            "receipt-rejected",
            "attempt-rejected",
            "obs-one",
            200,
            DIGEST_C,
            true,
        );

        let evaluation = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();

        assert_eq!(evaluation.status, CoverageStatus::Satisfied);
        assert_eq!(
            evaluation.verdict["validation_details"]["fixture"]["status"],
            "passed"
        );
        assert_coverage_schema_valid(&evaluation.verdict);
    }

    #[test]
    fn scoped_coverage_rejects_empty_and_filters_advisory_or_superseded_obligations() {
        let empty_conn = conn();
        seed_project(&empty_conn);
        assert!(
            evaluate_plan_coverage(
                &empty_conn,
                "p-evidence",
                "pln-evidence",
                "2026-07-29T00:00:00Z"
            )
            .is_err()
        );

        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_obligation(&conn, &["obs-one"]);
        seed_advisory_obligation(&conn, "obl-advisory");
        seed_superseded_obligation_pair(&conn);
        seed_receipt(
            &conn,
            "receipt-one",
            "attempt-one",
            "obs-one",
            200,
            DIGEST_C,
            false,
        );
        seed_receipt_with_outcome(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-current",
                attempt_id: "attempt-current",
                obligation_id: "obl-current",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 200,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Passed,
            },
        );

        let evaluation =
            evaluate_plan_coverage(&conn, "p-evidence", "pln-evidence", "2026-07-29T00:00:00Z")
                .unwrap();
        assert_eq!(evaluation.status, CoverageStatus::Satisfied);
        assert_eq!(
            evaluation.verdict["observation_coverage"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_coverage_schema_valid(&evaluation.verdict);

        assert!(
            evaluate_obligation_coverage(
                &conn,
                "p-evidence",
                "obl-advisory",
                "2026-07-29T00:00:01Z"
            )
            .is_err()
        );
        assert!(
            evaluate_obligation_coverage(&conn, "p-evidence", "obl-old", "2026-07-29T00:00:02Z")
                .is_err()
        );
    }

    #[test]
    fn superseded_obligation_failure_is_diagnostic_not_completion_relevant() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_superseded_obligation_pair(&conn);
        seed_receipt_with_outcome(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-old-failed",
                attempt_id: "attempt-old-failed",
                obligation_id: "obl-old",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 500,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Failed,
            },
        );
        seed_receipt_with_outcome(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-current-passed",
                attempt_id: "attempt-current-passed",
                obligation_id: "obl-current",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 200,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Passed,
            },
        );

        let evaluation = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-current",
            "2026-07-29T00:00:02Z",
        )
        .unwrap();

        assert_eq!(evaluation.status, CoverageStatus::Satisfied);
        assert_eq!(
            evaluation.receipt_lineage[0]["aggregation_policy"],
            "latest_applicable_pass"
        );
        assert_eq!(
            evaluation.receipt_lineage[0]["covering_receipt_ids"],
            json!(["receipt-current-passed"])
        );
        assert_eq!(
            evaluation.receipt_lineage[0]["superseded_receipt_ids"],
            json!(["receipt-old-failed"])
        );
        assert_eq!(
            evaluation.receipt_lineage[0]["diagnostic_receipt_ids"],
            json!(["receipt-old-failed"])
        );
        assert_coverage_schema_valid(&evaluation.verdict);
    }

    #[test]
    fn current_failed_receipt_gaps_override_superseded_stale_diagnostics() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_superseded_obligation_pair(&conn);
        seed_receipt_with_outcome(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-old-failed",
                attempt_id: "attempt-old-failed",
                obligation_id: "obl-old",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 500,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Failed,
            },
        );
        seed_receipt_with_outcome_with_gaps(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-current-product-failed",
                attempt_id: "attempt-current-product-failed",
                obligation_id: "obl-current",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 200,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Failed,
            },
            &["product_failed"],
        );

        let evaluation = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-current",
            "2026-07-29T00:00:02Z",
        )
        .unwrap();

        assert_eq!(evaluation.status, CoverageStatus::Unsatisfied);
        assert_eq!(
            evaluation.verdict["observation_coverage"][0]["gap_reason"],
            "product_failed"
        );
        assert_eq!(
            evaluation.verdict["observation_coverage"][0]["gap_reasons"],
            json!(["product_failed"])
        );
        assert_eq!(
            evaluation.receipt_lineage[0]["rejected_receipt_ids"],
            json!(["receipt-current-product-failed"])
        );
        assert_eq!(
            evaluation.receipt_lineage[0]["superseded_receipt_ids"],
            json!(["receipt-old-failed"])
        );
        assert_eq!(
            evaluation.receipt_lineage[0]["diagnostic_receipt_ids"],
            json!(["receipt-old-failed"])
        );
        assert_coverage_schema_valid(&evaluation.verdict);
    }

    #[test]
    fn explicit_all_applicable_pass_policy_keeps_mixed_active_results_inconclusive() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_obligation_with_id(&conn, "obl-all-attempts", &["obs-one"]);
        seed_receipt_with_outcome(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-all-passed",
                attempt_id: "attempt-all-passed",
                obligation_id: "obl-all-attempts",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 200,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Passed,
            },
        );
        seed_receipt_with_outcome(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-all-failed",
                attempt_id: "attempt-all-failed",
                obligation_id: "obl-all-attempts",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 500,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Failed,
            },
        );

        let evaluation = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-all-attempts",
            "2026-07-29T00:00:02Z",
        )
        .unwrap();

        assert_eq!(evaluation.status, CoverageStatus::Inconclusive);
        assert_eq!(
            evaluation.receipt_lineage[0]["aggregation_policy"],
            "all_applicable_pass"
        );
        assert_eq!(
            evaluation.receipt_lineage[0]["rejected_receipt_ids"],
            json!(["receipt-all-failed"])
        );
        assert_coverage_schema_valid(&evaluation.verdict);
    }

    #[test]
    fn waiver_requires_exact_source_binding_and_transition_history_keeps_stable_id() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_obligation(&conn, &["obs-one"]);
        seed_waiver_with_source(&conn, "waiver-bad-source", "obs-one", DIGEST_B, false);
        let missing = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();
        assert_eq!(missing.status, CoverageStatus::Unsatisfied);
        assert_eq!(
            missing.verdict["observation_coverage"][0]["gap_reason"],
            "missing_observation"
        );

        let dirty_conn = self::conn();
        seed_project(&dirty_conn);
        seed_manifest(&dirty_conn, DIGEST_C);
        seed_obligation_with_id(&dirty_conn, "obl-dirty-current", &["obs-one"]);
        seed_waiver_for_obligation_with_source(
            &dirty_conn,
            "obl-dirty-current",
            "waiver-dirty-source",
            "obs-one",
            DIGEST_A,
            false,
        );
        let dirty_mismatch = evaluate_obligation_coverage(
            &dirty_conn,
            "p-evidence",
            "obl-dirty-current",
            "2026-07-29T00:00:30Z",
        )
        .unwrap();
        assert_eq!(dirty_mismatch.status, CoverageStatus::Unsatisfied);
        assert_eq!(
            dirty_mismatch.verdict["observation_coverage"][0]["gap_reason"],
            "missing_observation"
        );

        seed_receipt(
            &conn,
            "receipt-one",
            "attempt-one",
            "obs-one",
            200,
            DIGEST_C,
            false,
        );
        let satisfied = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:01:00Z",
        )
        .unwrap();
        seed_manifest(&conn, DIGEST_D);
        let stale = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:02:00Z",
        )
        .unwrap();
        assert_eq!(satisfied.id, stale.id);
        assert_eq!(stale.status, CoverageStatus::Stale);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM coverage_verdict_history", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            3
        );
    }

    #[test]
    fn capability_reprobe_stales_receipt_without_changing_semantic_obligation() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_obligation(&conn, &["obs-one"]);
        seed_receipt(
            &conn,
            "receipt-before-reprobe",
            "attempt-before-reprobe",
            "obs-one",
            200,
            DIGEST_C,
            false,
        );
        let before = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:01:00Z",
        )
        .unwrap();
        assert_eq!(before.status, CoverageStatus::Satisfied);

        seed_manifest_with_id(&conn, "manifest-coverage", "instance-reprobe", DIGEST_C);
        let after = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:02:00Z",
        )
        .unwrap();

        assert_eq!(before.id, after.id);
        assert_eq!(after.status, CoverageStatus::Stale);
        assert_eq!(
            after.verdict["observation_coverage"][0]["gap_reason"],
            "stale_configuration"
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM proof_obligations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT manifests.manifest_digest
                 FROM verification_capability_instances AS instances
                 JOIN verification_capability_manifests AS manifests
                   ON manifests.id = instances.manifest_id
                  AND manifests.version = instances.manifest_version
                 WHERE instances.id = 'instance-coverage'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            DIGEST_C
        );
    }

    #[test]
    fn canonical_gap_priority_covers_all_frozen_branch_families() {
        for gap in GapReason::ALL.iter().map(|reason| reason.as_str()) {
            let coverage = ObservationCoverage {
                requirement_id: "obs-one".to_string(),
                status: status_for_gap(gap),
                covering_receipt_ids: BTreeSet::new(),
                covered_observation_ids: BTreeSet::new(),
                attempted_receipt_ids: BTreeSet::new(),
                waiver_id: None,
                gap_reasons: BTreeSet::from([gap]),
                aggregation_policy: RetryAggregationPolicy::LatestApplicablePass,
                superseded_receipt_ids: BTreeSet::new(),
                diagnostic_receipt_ids: BTreeSet::new(),
                rejected_receipt_ids: BTreeSet::new(),
            };
            assert_eq!(primary_gap(&coverage), gap);
            assert_coverage_schema_valid(&generated_verdict_for_coverage(coverage));
        }

        assert_coverage_schema_valid(&generated_verdict_for_coverage(ObservationCoverage {
            requirement_id: "obs-covered".to_string(),
            status: CoverageObservationStatus::Covered,
            covering_receipt_ids: BTreeSet::from(["receipt-one".to_string()]),
            covered_observation_ids: BTreeSet::from(["obs-covered".to_string()]),
            attempted_receipt_ids: BTreeSet::new(),
            waiver_id: None,
            gap_reasons: BTreeSet::new(),
            aggregation_policy: RetryAggregationPolicy::LatestApplicablePass,
            superseded_receipt_ids: BTreeSet::new(),
            diagnostic_receipt_ids: BTreeSet::new(),
            rejected_receipt_ids: BTreeSet::new(),
        }));
        assert_coverage_schema_valid(&generated_verdict_for_coverage(ObservationCoverage {
            requirement_id: "obs-waived".to_string(),
            status: CoverageObservationStatus::Waived,
            covering_receipt_ids: BTreeSet::new(),
            covered_observation_ids: BTreeSet::new(),
            attempted_receipt_ids: BTreeSet::new(),
            waiver_id: Some("waiver-one".to_string()),
            gap_reasons: BTreeSet::from([GapReason::MissingObservation.as_str()]),
            aggregation_policy: RetryAggregationPolicy::LatestApplicablePass,
            superseded_receipt_ids: BTreeSet::new(),
            diagnostic_receipt_ids: BTreeSet::new(),
            rejected_receipt_ids: BTreeSet::new(),
        }));
    }

    #[test]
    fn canonical_gap_aliases_are_behavioral_and_unknowns_fail_as_verifier() {
        for reason in GapReason::ALL {
            assert_eq!(canonical_gap(reason.as_str()), reason.as_str());
        }
        for (alias, canonical) in GapReason::LEGACY_ALIASES {
            assert_eq!(canonical_gap(alias), canonical.as_str());
        }
        assert_eq!(
            canonical_gap("unclassified_legacy_harness_reason"),
            GapReason::VerifierFailed.as_str()
        );
        assert_ne!(
            canonical_gap("unclassified_legacy_harness_reason"),
            GapReason::ProductFailed.as_str()
        );
    }

    #[test]
    fn candidate_gaps_fail_closed_for_missing_schema_and_stale_target() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_obligation(&conn, &["obs-one"]);
        seed_receipt(
            &conn,
            "receipt-one",
            "attempt-one",
            "obs-one",
            200,
            DIGEST_C,
            false,
        );

        let obligation = load_obligation(&conn, "p-evidence", "obl-coverage").unwrap();
        let requirement = &obligation.observations[0];
        let mut receipt = load_receipts(&conn, "p-evidence", "obl-coverage")
            .unwrap()
            .remove(0);
        receipt.value["observations"][0]["actual"]
            .as_object_mut()
            .unwrap()
            .remove("schema_ref");
        receipt.value["target"]["digest"] = json!(DIGEST_C);
        receipt.trusted_binding.target =
            serde_json::from_value(receipt.value["target"].clone()).unwrap();
        let digest =
            sha256_json_digest_without_top_level_field(&receipt.value, "receipt_digest").unwrap();
        receipt.value["receipt_digest"] = json!(digest);
        receipt.digest = digest;
        let mut stale_target_requirement = requirement.clone();
        stale_target_requirement.target["digest"] = json!(DIGEST_D);

        let gaps = candidate_gaps(
            &conn,
            &obligation,
            &stale_target_requirement,
            &receipt,
            &receipt.value["observations"][0],
            None,
        )
        .unwrap();

        assert!(gaps.contains(GapReason::SchemaMismatch.as_str()));
        assert!(gaps.contains(GapReason::StaleTarget.as_str()));
    }

    #[test]
    fn candidate_gaps_isolate_canonical_binding_dimensions() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_obligation(&conn, &["obs-one"]);
        seed_receipt(
            &conn,
            "receipt-binding-isolation",
            "attempt-binding-isolation",
            "obs-one",
            200,
            DIGEST_C,
            false,
        );

        let obligation = load_obligation(&conn, "p-evidence", "obl-coverage").unwrap();
        let requirement = &obligation.observations[0];
        let receipt = load_receipts(&conn, "p-evidence", "obl-coverage")
            .unwrap()
            .remove(0);
        let observation = receipt.value["observations"][0].clone();
        let snapshot = capture_repository_snapshot(
            project_root(&conn, "p-evidence")
                .unwrap()
                .unwrap()
                .as_path(),
        )
        .unwrap();
        assert!(
            candidate_gaps(
                &conn,
                &obligation,
                requirement,
                &receipt,
                &observation,
                Some(&snapshot)
            )
            .unwrap()
            .is_empty()
        );

        let exact_gaps = |receipt: &ReceiptCandidate| {
            candidate_gaps(
                &conn,
                &obligation,
                requirement,
                receipt,
                &observation,
                Some(&snapshot),
            )
            .unwrap()
        };

        let mut version = receipt.clone();
        version.obligation_version += 1;
        assert_eq!(
            exact_gaps(&version),
            BTreeSet::from([GapReason::StaleConfiguration.as_str()])
        );

        let mut identity = receipt.clone();
        identity.obligation_id = "obl-other".to_string();
        identity.value["obligation_id"] = json!("obl-other");
        let identity_digest =
            sha256_json_digest_without_top_level_field(&identity.value, "receipt_digest").unwrap();
        identity.value["receipt_digest"] = json!(identity_digest);
        identity.digest = identity_digest;
        assert_eq!(
            exact_gaps(&identity),
            BTreeSet::from([
                GapReason::StaleConfiguration.as_str(),
                GapReason::StalePolicy.as_str()
            ])
        );

        let mut supersession = receipt.clone();
        supersession.obligation_supersedes_obligation_id = Some("obl-old".to_string());
        assert_eq!(
            exact_gaps(&supersession),
            BTreeSet::from([GapReason::StaleConfiguration.as_str()])
        );
    }

    #[test]
    fn coverage_evaluation_marks_receipt_policy_binding_stale() {
        let conn = conn();
        seed_project(&conn);
        seed_manifest(&conn, DIGEST_C);
        seed_obligation_with_id(&conn, "obl-policy-transition", &["obs-one"]);
        seed_receipt_with_outcome(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-stale-policy",
                attempt_id: "attempt-stale-policy",
                obligation_id: "obl-policy-transition",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 200,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Passed,
            },
        );
        let root = project_root(&conn, "p-evidence").unwrap().unwrap();
        write_repository_policy(&root, 7200);

        let evaluation = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-policy-transition",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();

        assert_eq!(evaluation.status, CoverageStatus::Stale);
        assert_eq!(
            evaluation.verdict["observation_coverage"][0]["gap_reason"],
            "stale_source"
        );
        assert!(
            evaluation.verdict["validation_details"]["freshness"]["gap_reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap == "stale_policy")
        );
        assert_coverage_schema_valid(&evaluation.verdict);
    }

    #[test]
    fn coverage_evaluation_marks_real_repository_policy_mutation_and_removal_stale() {
        let root = tempdir().unwrap();
        write_repository_policy(root.path(), 3600);
        init_git_repo(root.path());
        let initial_policy = super::super::policy::load_repository_policy_binding(root.path())
            .unwrap()
            .unwrap()
            .digest
            .as_str()
            .to_string();
        let conn = conn();
        seed_project_at(&conn, root.path());
        seed_manifest(&conn, DIGEST_C);
        seed_obligation_with_id(&conn, "obl-repository-policy", &["obs-one"]);
        seed_receipt_with_outcome(
            &conn,
            ReceiptSeed {
                receipt_id: "receipt-repository-policy",
                attempt_id: "attempt-repository-policy",
                obligation_id: "obl-repository-policy",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id: "obs-one",
                status_code: 200,
                manifest_digest: DIGEST_C,
                fixtures_used: false,
                outcome: AttemptStatus::Passed,
            },
        );
        assert_eq!(
            initial_policy,
            load_receipts(&conn, "p-evidence", "obl-repository-policy").unwrap()[0]
                .trusted_binding
                .policy_digest
                .as_str()
        );

        write_repository_policy(root.path(), 7200);
        let mutated = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-repository-policy",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();
        assert_eq!(mutated.status, CoverageStatus::Stale);
        assert_eq!(
            mutated.verdict["observation_coverage"][0]["gap_reason"],
            "stale_source"
        );
        assert!(
            mutated.verdict["validation_details"]["freshness"]["gap_reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap == "stale_policy")
        );

        fs::remove_file(root.path().join(".planr/evidence.yaml")).unwrap();
        let removed = evaluate_obligation_coverage(
            &conn,
            "p-evidence",
            "obl-repository-policy",
            "2026-07-29T00:01:00Z",
        )
        .unwrap();
        assert_eq!(removed.status, CoverageStatus::Stale);
        assert_eq!(
            removed.verdict["observation_coverage"][0]["gap_reason"],
            "stale_source"
        );
        assert!(
            removed.verdict["validation_details"]["freshness"]["gap_reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap == "stale_policy")
        );
    }

    #[test]
    fn waiver_matching_uses_real_repository_source_and_fails_closed() {
        let root = tempdir().unwrap();
        init_git_repo(root.path());
        let clean_conn = conn();
        seed_project_at(&clean_conn, root.path());
        seed_manifest(&clean_conn, DIGEST_C);
        seed_obligation(&clean_conn, &["obs-one"]);
        let clean_source = capture_repository_snapshot(root.path()).unwrap().source;
        seed_waiver_for_obligation_with_exact_source(
            &clean_conn,
            "obl-coverage",
            "waiver-real-source",
            "obs-one",
            &clean_source,
        );
        let clean = evaluate_obligation_coverage(
            &clean_conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:00:00Z",
        )
        .unwrap();
        assert_eq!(clean.status, CoverageStatus::Waived);

        fs::write(root.path().join("dirty-source.txt"), "dirty\n").unwrap();
        let dirty = evaluate_obligation_coverage(
            &clean_conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:01:00Z",
        )
        .unwrap();
        assert_eq!(dirty.status, CoverageStatus::Unsatisfied);
        assert_eq!(
            dirty.verdict["observation_coverage"][0]["gap_reason"],
            "missing_observation"
        );

        let unavailable_root = tempdir().unwrap();
        let unavailable_conn = conn();
        seed_project_at(&unavailable_conn, unavailable_root.path());
        seed_manifest(&unavailable_conn, DIGEST_C);
        seed_obligation(&unavailable_conn, &["obs-one"]);
        seed_waiver_for_obligation_with_exact_source(
            &unavailable_conn,
            "obl-coverage",
            "waiver-unavailable-source",
            "obs-one",
            &clean_source,
        );
        let unavailable = evaluate_obligation_coverage(
            &unavailable_conn,
            "p-evidence",
            "obl-coverage",
            "2026-07-29T00:02:00Z",
        )
        .unwrap();
        assert_eq!(unavailable.status, CoverageStatus::Unsatisfied);
    }

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        crate::storage::ensure_schema(&conn).unwrap();
        conn
    }

    fn seed_project(conn: &Connection) {
        let root = Box::leak(Box::new(tempdir().unwrap()));
        write_repository_policy(root.path(), 3600);
        init_git_repo(root.path());
        seed_project_at(conn, root.path());
    }

    fn seed_project_at(conn: &Connection, root_path: &std::path::Path) {
        conn.execute_batch(
            &format!(
                "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
                 VALUES ('p-evidence', 'Evidence', '{}', 'active', datetime('now'), datetime('now'));
                 INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
                 VALUES ('pln-evidence', 'p-evidence', 'build', '/tmp/planr/coverage.plan.md', 'Evidence Plan', 'evidence-plan', 'ok', 'hash-evidence', datetime('now'), datetime('now'));
                 INSERT INTO items(id, project_id, title, description, status, work_type, priority, plan_path, created_at, updated_at)
                 VALUES ('i-evidence', 'p-evidence', 'Evidence item', 'Evidence item', 'ready', 'code', 0, '/tmp/planr/coverage.plan.md', datetime('now'), datetime('now'));",
                root_path.display().to_string().replace('\'', "''")
            ),
        )
        .unwrap();
        conn.execute(
            "UPDATE items SET approval_status = 'approved' WHERE id = 'i-evidence'",
            [],
        )
        .unwrap();
    }

    fn seed_manifest(conn: &Connection, manifest_digest: &str) {
        seed_manifest_with_id(
            conn,
            "manifest-coverage",
            "instance-coverage",
            manifest_digest,
        );
    }

    fn seed_manifest_with_id(
        conn: &Connection,
        manifest_id: &str,
        instance_id: &str,
        manifest_digest: &str,
    ) {
        let version = manifest_version(manifest_digest);
        let manifest =
            json!({"id": manifest_id, "digest": manifest_digest, "independence": "same wording"});
        conn.execute(
            "INSERT OR IGNORE INTO verification_capability_manifests(
              id, version, adapter_kind, adapter_digest, manifest_digest, manifest_json, created_at
            ) VALUES (?1, ?2, 'process', ?3, ?4, ?5, datetime('now'))",
            params![
                manifest_id,
                version,
                DIGEST_A,
                manifest_digest,
                manifest.to_string()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO verification_capability_instances(
              id, manifest_id, manifest_version, manifest_digest, probe_execution_id,
              availability_status, runtime_target_json, host_fingerprint_json,
              capability_snapshot_json, probe_result_json, created_at
            ) VALUES (
              ?1, ?2, ?3, ?4, ?5,
              'available', '{}', '{}', '{}', '{}', datetime('now')
            )",
            params![
                instance_id,
                manifest_id,
                version,
                manifest_digest,
                format!("probe-{instance_id}")
            ],
        )
        .unwrap();
    }

    fn manifest_version(manifest_digest: &str) -> String {
        format!(
            "1.0.{}",
            &manifest_digest["sha256:".len().."sha256:".len() + 8]
        )
    }

    fn seed_obligation(conn: &Connection, observation_ids: &[&str]) {
        seed_obligation_with_id(conn, "obl-coverage", observation_ids);
    }

    fn seed_obligation_with_id(conn: &Connection, obligation_id: &str, observation_ids: &[&str]) {
        let obligation_version = if obligation_id == "obl-coverage" {
            1
        } else {
            2
        };
        let source_digest = canonical_source_digest(conn);
        let observations = observation_ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let mut observation = json!({
                    "id": id,
                    "type": "planr.test.coverage",
                    "subject": id,
                    "expected": {"status": if index == 0 { 200 } else { 201 }},
                    "target": {"kind": "local_service", "uri": "http://127.0.0.1/health"},
                    "payload_schema": {"schema_ref": "planr.test.coverage@v1"}
                });
                if obligation_id == "obl-independence" {
                    observation["assurance_policy"] = json!({"independent_methods": 2});
                }
                observation
            })
            .collect::<Vec<_>>();
        let assurance_policy = if obligation_id == "obl-all-attempts" {
            json!({
                "accepted_provenance": ["planr_observed_execution"],
                "retry_aggregation": "all_applicable_pass"
            })
        } else {
            json!({"accepted_provenance": ["planr_observed_execution"]})
        };
        conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, retry_aggregation, policy_digest, config_digest, source_digest, created_at
            ) VALUES (
              ?1, 'p-evidence', 'pln-evidence', 'i-evidence', 'crit-coverage',
              ?2, 'Coverage', 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, '2026-07-29T00:00:00Z'
            )",
            params![
                obligation_id,
                obligation_version,
                Value::Array(observations).to_string(),
                json!({"fixtures_allowed": false, "mocks_allowed": false}).to_string(),
                json!({"source_revision": "0123456789abcdef", "invalidate_on": ["source_change", "target_change", "policy_change", "adapter_schema_change", "configuration_change"]}).to_string(),
                assurance_policy.to_string(),
                if obligation_id == "obl-all-attempts" {
                    "all_applicable_pass"
                } else {
                    "latest_applicable_pass"
                },
                if obligation_id == "obl-policy-transition" {
                    DIGEST_D
                } else {
                    DIGEST_A
                },
                DIGEST_B,
                source_digest.as_deref(),
            ],
        )
        .unwrap();
    }

    fn seed_advisory_obligation(conn: &Connection, obligation_id: &str) {
        let source_digest = canonical_source_digest(conn);
        let observations = json!([{
            "id": "obs-advisory",
            "type": "planr.test.coverage",
            "subject": "obs-advisory",
            "expected": {"status": 200},
            "target": {"kind": "local_service", "uri": "http://127.0.0.1/health"},
            "payload_schema": {"schema_ref": "planr.test.coverage@v1"}
        }]);
        conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, policy_digest, config_digest, source_digest, created_at
            ) VALUES (
              ?1, 'p-evidence', 'pln-evidence', 'i-evidence', 'crit-advisory',
              1, 'Advisory', 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '2026-07-29T00:00:00Z'
            )",
            params![
                obligation_id,
                observations.to_string(),
                json!({"fixtures_allowed": false, "mocks_allowed": false}).to_string(),
                json!({"source_revision": "0123456789abcdef", "invalidate_on": ["source_change"]})
                    .to_string(),
                json!({"accepted_provenance": ["planr_observed_execution"]}).to_string(),
                DIGEST_A,
                DIGEST_B,
                source_digest.as_deref(),
            ],
        )
        .unwrap();
    }

    fn seed_superseded_obligation_pair(conn: &Connection) {
        seed_obligation_with_id(conn, "obl-old", &["obs-old"]);
        let source_digest = canonical_source_digest(conn);
        let observations = json!([{
            "id": "obs-one",
            "type": "planr.test.coverage",
            "subject": "obs-one",
            "expected": {"status": 200},
            "target": {"kind": "local_service", "uri": "http://127.0.0.1/health"},
            "payload_schema": {"schema_ref": "planr.test.coverage@v1"}
        }]);
        conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, policy_digest, config_digest, source_digest,
              supersedes_obligation_id, created_at
            ) VALUES (
              'obl-current', 'p-evidence', 'pln-evidence', 'i-evidence', 'crit-coverage',
              3, 'Current', 1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'obl-old', '2026-07-29T00:00:01Z'
            )",
            params![
                observations.to_string(),
                json!({"fixtures_allowed": false, "mocks_allowed": false}).to_string(),
                json!({"source_revision": "0123456789abcdef", "invalidate_on": ["source_change"]})
                    .to_string(),
                json!({"accepted_provenance": ["planr_observed_execution"]}).to_string(),
                DIGEST_A,
                DIGEST_B,
                source_digest.as_deref(),
            ],
        )
        .unwrap();
    }

    fn seed_receipt(
        conn: &Connection,
        receipt_id: &str,
        attempt_id: &str,
        observation_id: &str,
        status_code: u64,
        manifest_digest: &str,
        fixtures_used: bool,
    ) {
        seed_receipt_with_outcome(
            conn,
            ReceiptSeed {
                receipt_id,
                attempt_id,
                obligation_id: "obl-coverage",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id,
                status_code,
                manifest_digest,
                fixtures_used,
                outcome: AttemptStatus::Passed,
            },
        );
    }

    fn seed_failed_receipt(
        conn: &Connection,
        receipt_id: &str,
        attempt_id: &str,
        observation_id: &str,
        manifest_digest: &str,
    ) {
        seed_receipt_with_outcome(
            conn,
            ReceiptSeed {
                receipt_id,
                attempt_id,
                obligation_id: "obl-coverage",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id,
                status_code: 500,
                manifest_digest,
                fixtures_used: false,
                outcome: AttemptStatus::Failed,
            },
        );
    }

    fn seed_unavailable_receipt(
        conn: &Connection,
        receipt_id: &str,
        attempt_id: &str,
        observation_id: &str,
        manifest_digest: &str,
    ) {
        seed_receipt_with_outcome(
            conn,
            ReceiptSeed {
                receipt_id,
                attempt_id,
                obligation_id: "obl-coverage",
                manifest_id: "manifest-coverage",
                instance_id: "instance-coverage",
                observation_id,
                status_code: 503,
                manifest_digest,
                fixtures_used: false,
                outcome: AttemptStatus::Unavailable,
            },
        );
    }

    fn seed_receipt_with_outcome(conn: &Connection, seed: ReceiptSeed<'_>) {
        seed_receipt_with_outcome_with_gaps(conn, seed, &[]);
    }

    fn seed_receipt_with_outcome_with_gaps(
        conn: &Connection,
        seed: ReceiptSeed<'_>,
        proof_gaps: &[&str],
    ) {
        let ReceiptSeed {
            receipt_id,
            attempt_id,
            obligation_id,
            manifest_id,
            instance_id,
            observation_id,
            status_code,
            manifest_digest,
            fixtures_used,
            outcome,
        } = seed;
        let (exit_code, exit_json, exit_error) = match outcome {
            AttemptStatus::Passed => (Some(0), json!(0), Value::Null),
            AttemptStatus::Failed => (Some(1), json!(1), json!("failed")),
            AttemptStatus::Unavailable => (None, Value::Null, json!("unavailable")),
            AttemptStatus::TimedOut => (None, Value::Null, json!("timed_out")),
            AttemptStatus::Skipped => (None, Value::Null, json!("skipped")),
            AttemptStatus::Aborted => (None, Value::Null, json!("aborted")),
            AttemptStatus::Inconclusive => (Some(0), json!(0), json!("inconclusive")),
        };
        let attempt_json = json!({
            "status": outcome.as_str(),
            "exit": {
                "exit_code": exit_json,
                "signal": null,
                "error": exit_error,
            }
        });
        conn.execute(
            "INSERT INTO evidence_attempts(
              id, project_id, obligation_id, capability_instance_id, attempt_status,
              execution_contract_digest, resolved_command_json, environment_digest,
              started_at, completed_at, exit_code, stdout_digest, stderr_digest,
              output_bounds_json, attempt_json, created_at
            ) VALUES (
              ?1, 'p-evidence', ?2, ?3, ?4, ?5, '{}', ?6,
              '2026-07-29T00:00:00Z', '2026-07-29T00:00:01Z', ?7, ?8, ?8, '{}', ?9, datetime('now')
            )",
            params![
                attempt_id,
                obligation_id,
                instance_id,
                outcome.as_str(),
                DIGEST_A,
                DIGEST_B,
                exit_code,
                DIGEST_C,
                attempt_json.to_string()
            ],
        )
        .unwrap();
        let receipt = build_trusted_receipt(TrustedReceiptInput {
            id: EvidenceId::parse(receipt_id.to_string()).unwrap(),
            criterion_id: EvidenceId::parse("crit-coverage".to_string()).unwrap(),
            obligation_id: EvidenceId::parse(obligation_id.to_string()).unwrap(),
            source: canonical_source(conn),
            target: TargetBinding {
                kind: "local_service".to_string(),
                uri: Some("http://127.0.0.1/health".to_string()),
                digest: None,
                deployment_id: None,
            },
            environment: EnvironmentBinding {
                kind: "local".to_string(),
                id: EvidenceId::parse("env-local".to_string()).unwrap(),
                digest: Sha256Digest::parse(DIGEST_B.to_string()).unwrap(),
            },
            vantage_point: VantagePoint {
                kind: "localhost".to_string(),
                identity: "127.0.0.1".to_string(),
            },
            capability: CapabilityBinding {
                manifest_id: EvidenceId::parse(manifest_id.to_string()).unwrap(),
                manifest_digest: Sha256Digest::parse(manifest_digest.to_string()).unwrap(),
                instance_id: EvidenceId::parse(instance_id.to_string()).unwrap(),
                instance_digest: Sha256Digest::parse(DIGEST_D.to_string()).unwrap(),
            },
            provenance: TrustedProvenance {
                source: ProvenanceSourceKind::PlanrObservedExecution,
                assigned_by: "planr".to_string(),
                execution_id: attempt_id.to_string(),
                tool_call_id: None,
            },
            observations: vec![crate::evidence::model::ObservationResult {
                requirement_id: EvidenceId::parse(observation_id.to_string()).unwrap(),
                observation_type: "planr.test.coverage".parse().unwrap(),
                outcome,
                predicate: [("status".to_string(), json!(status_code))]
                    .into_iter()
                    .collect(),
                actual: [
                    ("status".to_string(), json!(status_code)),
                    ("schema_ref".to_string(), json!("planr.test.coverage@v1")),
                ]
                .into_iter()
                .collect(),
            }],
            attempt_ids: vec![EvidenceId::parse(attempt_id.to_string()).unwrap()],
            retry_history: Vec::new(),
            artifacts: vec![ArtifactRef {
                id: EvidenceId::parse(format!("artifact-{attempt_id}")).unwrap(),
                kind: "stdout".to_string(),
                digest: Sha256Digest::parse(DIGEST_C.to_string()).unwrap(),
                uri: None,
                extra: Map::new(),
            }],
            raw_result: RawResultRef {
                kind: "artifact".to_string(),
                digest: Sha256Digest::parse(DIGEST_C.to_string()).unwrap(),
                artifact_id: Some(EvidenceId::parse(format!("artifact-{attempt_id}")).unwrap()),
                extra: Map::new(),
            },
            config_digest: Sha256Digest::parse(DIGEST_B.to_string()).unwrap(),
            fixture_disclosure: FixtureDisclosure {
                fixtures_used,
                mocks_used: false,
                fixture_refs: None,
                mock_refs: None,
            },
            permissions: PermissionState {
                network: "localhost_only".to_string(),
                filesystem: "read_workspace".to_string(),
                environment: None,
                secrets: None,
            },
            sandbox: SandboxState {
                mode: "bounded_process".to_string(),
                limits: SandboxLimits {
                    timeout_ms: 1000,
                    stdout_bytes: 1024,
                    stderr_bytes: 1024,
                },
            },
            proof_gaps: proof_gaps
                .iter()
                .map(|gap| GapReason::canonicalize(gap))
                .collect(),
            started_at: "2026-07-29T00:00:00Z".to_string(),
            ended_at: "2026-07-29T00:00:01Z".to_string(),
        })
        .unwrap();
        let receipt_value = serde_json::to_value(&receipt).unwrap();
        let digest =
            sha256_json_digest_without_top_level_field(&receipt_value, "receipt_digest").unwrap();
        let root = project_root(conn, "p-evidence").unwrap().unwrap();
        let policy_digest = super::super::policy::load_repository_policy_binding(&root)
            .unwrap()
            .unwrap()
            .digest
            .as_str()
            .to_string();
        let trusted_binding = json!({
            "source": receipt_value["source"],
            "target": receipt_value["target"],
            "environment": receipt_value["environment"],
            "capability": receipt_value["capability"],
            "policy_digest": policy_digest,
            "policy_source": "repository",
            "config_digest": DIGEST_B
        });
        conn.execute(
            "INSERT INTO evidence_receipts(
              id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
              trusted_binding_json, observations_json, provenance_json, receipt_json, created_at
            ) VALUES (?1, 'p-evidence', ?2, ?3, 'trusted', ?4, ?5, ?6, ?7, ?8, datetime('now'))",
            params![
                receipt_id,
                obligation_id,
                attempt_id,
                digest,
                trusted_binding.to_string(),
                receipt_value["observations"].to_string(),
                receipt_value["provenance"].to_string(),
                receipt_value.to_string(),
            ],
        )
        .unwrap();
    }

    fn canonical_source(conn: &Connection) -> SourceBinding {
        let root = project_root(conn, "p-evidence").unwrap().unwrap();
        capture_repository_snapshot(&root).unwrap().source
    }

    fn canonical_source_digest(conn: &Connection) -> Option<String> {
        let root = project_root(conn, "p-evidence").unwrap().unwrap();
        capture_repository_snapshot(&root)
            .ok()
            .map(|snapshot| snapshot.source.tree_digest.as_str().to_string())
    }

    fn write_repository_policy(root: &std::path::Path, max_age_seconds: i64) {
        fs::create_dir_all(root.join(".planr")).unwrap();
        let payload_schema = json!({
            "type": "com.example.coverage.check",
            "schema_ref": "com.example.coverage.check@v1",
            "schema_digest": DIGEST_C
        });
        let execution_contract = json!({
            "kind": "process",
            "executable": "sh",
            "args": ["-c", "true"],
            "timeout_ms": 1000,
            "stdout_limit_bytes": 1024,
            "stderr_limit_bytes": 1024,
            "payload_schema": payload_schema
        });
        let mut policy = json!({
            "id": "pol-coverage",
            "schema_version": crate::evidence::model::EVIDENCE_CONTRACT_V1,
            "policy_digest": DIGEST_A,
            "defaults": {
                "preset_id": "preset-coverage",
                "binding": true,
                "assurance_level": "standard"
            },
            "named_presets": [{
                "id": "preset-coverage",
                "schema_version": crate::evidence::model::EVIDENCE_CONTRACT_V1,
                "namespace": "com.example.coverage",
                "observations": [{
                    "id": "obs-one",
                    "type": "com.example.coverage.check",
                    "subject": "obs-one",
                    "expected": {"status": 200},
                    "target": {"kind": "local_service", "uri": "http://127.0.0.1/health"}
                }]
            }],
            "observation_schema_registrations": [{
                "type": "com.example.coverage.check",
                "schema_ref": "com.example.coverage.check@v1",
                "schema_digest": DIGEST_C,
                "owning_namespace": "com.example.coverage"
            }],
            "adapter_registrations": [{
                "manifest_id": "manifest-coverage",
                "manifest_path": ".planr/evidence/adapters/coverage.manifest.json",
                "manifest_digest": DIGEST_C,
                "observation_types": ["com.example.coverage.check"],
                "payload_schemas": [payload_schema],
                "provenance_path": "planr_observed_execution",
                "execution_contract": execution_contract
            }],
            "extension_namespaces": ["com.example.coverage"],
            "trust_policy": {
                "accepted_provenance": ["planr_observed_execution"],
                "min_receipt_status": "trusted",
                "allow_user_attestation": false
            },
            "freshness_policy": {
                "max_age_seconds": max_age_seconds,
                "invalidate_on": ["policy_change"]
            },
            "fixture_policy": {
                "fixtures_allowed": false,
                "mocks_allowed": false,
                "disclosure_required": true
            },
            "completion_policy": {
                "require_satisfied_or_waived": true,
                "allow_inconclusive_completion": false,
                "require_review_evidence": true
            },
            "layering_policy": {
                "mode": "monotonic_strengthening",
                "weakening_requires_waiver": true,
                "layers": [{
                    "scope": {"kind": "plan", "id": "pln-evidence"},
                    "policy_digest": DIGEST_A
                }]
            }
        });
        let digest = sha256_json_digest_without_top_level_field(&policy, "policy_digest").unwrap();
        policy["policy_digest"] = json!(digest);
        fs::write(
            root.join(".planr/evidence.yaml"),
            serde_yaml::to_string(&policy).unwrap(),
        )
        .unwrap();
    }

    fn init_git_repo(root: &std::path::Path) {
        if root.join(".git").exists() {
            return;
        }
        fs::write(root.join("repository-source.txt"), "clean\n").unwrap();
        git(root, &["init"]);
        git(
            root,
            &["config", "user.email", "planr-test@example.invalid"],
        );
        git(root, &["config", "user.name", "Planr Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial evidence snapshot"]);
    }

    fn git(root: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn seed_waiver(conn: &Connection, waiver_id: &str, observation_id: &str) {
        let root = project_root(conn, "p-evidence").unwrap().unwrap();
        let source = capture_repository_snapshot(&root).unwrap().source;
        seed_waiver_for_obligation_with_exact_source(
            conn,
            "obl-coverage",
            waiver_id,
            observation_id,
            &source,
        );
    }

    fn seed_waiver_with_source(
        conn: &Connection,
        waiver_id: &str,
        observation_id: &str,
        tree_digest: &str,
        dirty: bool,
    ) {
        seed_waiver_for_obligation_with_source(
            conn,
            "obl-coverage",
            waiver_id,
            observation_id,
            tree_digest,
            dirty,
        );
    }

    fn seed_waiver_for_obligation_with_source(
        conn: &Connection,
        obligation_id: &str,
        waiver_id: &str,
        observation_id: &str,
        tree_digest: &str,
        dirty: bool,
    ) {
        let source = SourceBinding {
            revision: "0123456789abcdef".to_string(),
            tree_digest: Sha256Digest::parse(tree_digest.to_string()).unwrap(),
            dirty,
        };
        seed_waiver_for_obligation_with_exact_source(
            conn,
            obligation_id,
            waiver_id,
            observation_id,
            &source,
        );
    }

    fn seed_waiver_for_obligation_with_exact_source(
        conn: &Connection,
        obligation_id: &str,
        waiver_id: &str,
        observation_id: &str,
        source: &SourceBinding,
    ) {
        let waiver_json = json!({
            "id": waiver_id,
            "schema_version": crate::evidence::model::EVIDENCE_CONTRACT_V1,
            "scope": {"kind": "criterion", "id": "crit-coverage"},
            "observation_ids": [observation_id],
            "source": source,
            "target": {
                "kind": "local_service",
                "uri": "http://127.0.0.1/health"
            },
            "reason": "temporary outage",
            "created_by": "reviewer",
            "created_at": "2026-07-29T00:00:00Z",
            "expires_at": "2026-08-01T00:00:00Z",
            "approval_ref": "i-evidence",
            "audit_trail": [{"event": "created", "at": "2026-07-29T00:00:00Z"}]
        });
        let waiver_digest = sha256_json_digest(&waiver_json).unwrap();
        conn.execute(
            "INSERT INTO evidence_waivers(
              id, project_id, approval_item_id, obligation_id, observation_id,
              scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
              waiver_json, created_at
            ) VALUES (
              ?1, 'p-evidence', 'i-evidence', ?2, ?3,
              'criterion', 'crit-coverage', ?4, 'temporary outage',
              '2026-08-01T00:00:00Z', 'reviewer', ?5, '2026-07-29T00:00:00Z'
            )",
            params![
                waiver_id,
                obligation_id,
                observation_id,
                waiver_digest,
                waiver_json.to_string()
            ],
        )
        .unwrap();
    }

    fn generated_verdict_for_coverage(coverage: ObservationCoverage) -> Value {
        let observation_status = coverage.status;
        let observation = coverage_to_value(coverage);
        let status = aggregate_observation_status(std::slice::from_ref(&observation));
        let mut validation = validation_scaffold("2026-07-29T00:00:00Z");
        if observation_status != CoverageObservationStatus::Waived
            && let Some(gap) = observation.get("gap_reason").and_then(Value::as_str)
        {
            push_validation_gap(
                &mut validation,
                validation_section_for_gap(gap),
                gap,
                &["receipt-one".to_string()],
            );
        }
        finalize_validation(
            &mut validation,
            &BTreeSet::from(["receipt-one".to_string()]),
        );
        json!({
            "id": "cverdict-generated",
            "schema_version": super::super::model::EVIDENCE_CONTRACT_V1,
            "scope": {
                "kind": "criterion",
                "id": "crit-coverage",
                "plan_id": "pln-evidence",
                "item_id": "i-evidence",
                "criterion_id": "crit-coverage"
            },
            "evaluated_at": "2026-07-29T00:00:00Z",
            "status": status.as_str(),
            "observation_coverage": [observation],
            "validation_details": validation,
            "suggested_next_action": suggested_next_action(&status),
            "actionable_now": matches!(
                status,
                CoverageStatus::Unsatisfied | CoverageStatus::Stale | CoverageStatus::Inconclusive
            ),
        })
    }

    fn assert_coverage_schema_valid(verdict: &Value) {
        serde_json::from_value::<CoverageVerdict>(verdict.clone()).unwrap();
        let schema = serde_json::from_str::<Value>(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json"
        ))
        .unwrap();
        let validator = jsonschema::draft202012::options().build(&schema).unwrap();
        let errors = validator
            .iter_errors(verdict)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        assert!(errors.is_empty(), "{errors:?}\n{verdict}");
    }
}
