#![allow(dead_code)] // Internal comparison engine; CLI/MCP adapters are downstream map items.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub(crate) struct EvalComparisonPolicy {
    pub(crate) required_case_coverage: f64,
    pub(crate) freshness_max_age_hours: i64,
    pub(crate) min_samples: usize,
    pub(crate) quality_non_inferiority_margin: f64,
    pub(crate) material_improvement: BTreeMap<String, f64>,
    pub(crate) material_regression: BTreeMap<String, f64>,
    pub(crate) protected_relative_max: BTreeMap<String, f64>,
    pub(crate) variance_cv_max: f64,
    pub(crate) bootstrap_iterations: usize,
}

impl Default for EvalComparisonPolicy {
    fn default() -> Self {
        let mut material_improvement = BTreeMap::new();
        material_improvement.insert("duration_ms_p95_relative".to_string(), -0.10);
        let mut material_regression = BTreeMap::new();
        material_regression.insert("duration_ms_p95_relative".to_string(), 0.10);
        Self {
            required_case_coverage: 1.0,
            freshness_max_age_hours: 168,
            min_samples: 3,
            quality_non_inferiority_margin: 0.0,
            material_improvement,
            material_regression,
            protected_relative_max: BTreeMap::new(),
            variance_cv_max: 1.0,
            bootstrap_iterations: 512,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EvalRunEvidence {
    pub(crate) run_id: String,
    pub(crate) suite_digest: String,
    pub(crate) subject_kind: String,
    pub(crate) subject_path: Option<String>,
    pub(crate) subject_argv: String,
    pub(crate) testbed_fingerprint: String,
    pub(crate) status: String,
    pub(crate) invalidated: bool,
    pub(crate) age_hours: i64,
    pub(crate) cases: Vec<EvalCaseEvidence>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalCaseEvidence {
    pub(crate) case_id: String,
    pub(crate) scorer_id: String,
    pub(crate) scorer_version: String,
    pub(crate) scorer_control: Option<String>,
    pub(crate) fixture_digest: String,
    pub(crate) status: String,
    pub(crate) repetition_outcomes: Vec<EvalRepetitionOutcome>,
    pub(crate) safety_pass: Option<bool>,
    pub(crate) quality_pass: Option<bool>,
    pub(crate) min_successful_samples: Option<usize>,
    pub(crate) required_repetitions: Option<usize>,
    pub(crate) samples: Vec<EvalNumericSample>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalRepetitionOutcome {
    pub(crate) repetition_index: usize,
    pub(crate) status: String,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalNumericSample {
    pub(crate) repetition_index: usize,
    pub(crate) warmup: bool,
    pub(crate) seed: i64,
    pub(crate) measure: String,
    pub(crate) value: f64,
    pub(crate) valid: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct EvalComparisonResult {
    pub(crate) verdict: String,
    pub(crate) reasons: Vec<String>,
    pub(crate) gates: Value,
    pub(crate) effect_estimates: Value,
    pub(crate) uncertainty: Value,
    pub(crate) protected_dimensions: Value,
}

pub(crate) fn compare_eval_runs(
    baseline: Option<&EvalRunEvidence>,
    candidate: Option<&EvalRunEvidence>,
    policy: &EvalComparisonPolicy,
) -> EvalComparisonResult {
    let mut gates = Vec::new();
    let mut reasons = Vec::<String>::new();
    let mut effects = serde_json::Map::new();
    let mut uncertainty = serde_json::Map::new();
    let mut protected = serde_json::Map::new();

    let Some(baseline) = baseline else {
        return result(
            "insufficient_evidence",
            vec!["missing_baseline".to_string()],
            gates,
            effects,
            uncertainty,
            protected,
        );
    };
    let Some(candidate) = candidate else {
        return result(
            "insufficient_evidence",
            vec!["missing_candidate".to_string()],
            gates,
            effects,
            uncertainty,
            protected,
        );
    };

    let compatibility = compatibility_reasons(baseline, candidate);
    push_gate(&mut gates, "compatibility", &compatibility);
    reasons.extend(compatibility);

    let freshness = freshness_reasons(baseline, candidate, policy);
    push_gate(&mut gates, "freshness", &freshness);
    reasons.extend(freshness);

    let coverage = coverage_reasons(baseline, candidate, policy);
    push_gate(&mut gates, "coverage", &coverage);
    reasons.extend(coverage);

    let scorer_controls = if reasons.is_empty() {
        scorer_control_reasons(baseline, candidate)
    } else {
        Vec::new()
    };
    push_gate(&mut gates, "scorer_controls", &scorer_controls);
    reasons.extend(scorer_controls);

    let safety = if reasons.is_empty() {
        safety_reasons(candidate)
    } else {
        Vec::new()
    };
    push_gate(&mut gates, "safety", &safety);
    reasons.extend(safety);

    let quality = if reasons.is_empty() {
        quality_reasons(baseline, candidate, policy, &mut uncertainty)
    } else {
        Vec::new()
    };
    push_gate(&mut gates, "quality", &quality);
    reasons.extend(quality);

    let correctness = if reasons.is_empty() {
        correctness_reasons(candidate)
    } else {
        Vec::new()
    };
    push_gate(&mut gates, "correctness", &correctness);
    reasons.extend(correctness);

    let mut performance_reasons = Vec::new();
    if reasons.is_empty() {
        let baseline_cases = ordinary_case_map(baseline);
        let candidate_cases = ordinary_case_map(candidate);
        let measures = compared_measures(baseline, candidate);
        if measures.is_empty() {
            performance_reasons.push("samples_below_minimum".to_string());
        }
        let mut material_improvement_measures = Vec::new();
        for measure in measures {
            let paired_values = paired_measure_values(&baseline_cases, &candidate_cases, &measure);
            if paired_values.baseline.len() < policy.min_samples
                || paired_values.candidate.len() < policy.min_samples
                || paired_values.case_pair_counts.iter().any(
                    |(_, paired_count, declared_required_count)| {
                        let required_count = (*declared_required_count).max(policy.min_samples);
                        paired_count < &required_count
                    },
                )
            {
                performance_reasons.push("samples_below_minimum".to_string());
                continue;
            }
            let baseline_median = median(&paired_values.baseline);
            let candidate_median = median(&paired_values.candidate);
            let baseline_p95 = percentile(&paired_values.baseline, 0.95);
            let candidate_p95 = percentile(&paired_values.candidate, 0.95);
            let relative = relative_delta(candidate_p95, baseline_p95);
            let unbounded_positive_delta = relative.is_infinite() && relative.is_sign_positive();
            if paired_values
                .baseline
                .iter()
                .chain(paired_values.candidate.iter())
                .any(|value| is_invalid_nonnegative_measure_value(&measure, *value))
            {
                performance_reasons.push("samples_below_minimum".to_string());
                continue;
            }
            let ci = paired_bootstrap_relative_delta(
                &paired_values.baseline,
                &paired_values.candidate,
                policy.bootstrap_iterations,
                0x5eed,
            );
            let variance = variance_classification(
                &paired_values.baseline,
                &paired_values.candidate,
                policy.variance_cv_max,
            );
            effects.insert(
                format!("{measure}_median_relative"),
                relative_value(relative_delta(candidate_median, baseline_median)),
            );
            effects.insert(format!("{measure}_p95_relative"), relative_value(relative));
            uncertainty.insert(format!("{measure}_p95_ci95"), relative_interval_value(ci));
            uncertainty.insert(format!("{measure}_variance"), json!(variance));
            if variance == "excessive" {
                performance_reasons.push("variance_too_high".to_string());
                continue;
            }
            if let Some(max_relative) = protected_threshold(policy, &measure) {
                protected.insert(format!("{measure}_relative"), relative_value(relative));
                if unbounded_positive_delta || ci.0 >= max_relative {
                    performance_reasons.push("protected_dimension_regressed".to_string());
                }
            }
            if let Some(threshold) = material_improvement_threshold(policy, &measure) {
                if !unbounded_positive_delta && ci.1 <= threshold {
                    material_improvement_measures.push(measure.clone());
                }
            }
            if let Some(threshold) = material_regression_threshold(policy, &measure) {
                if unbounded_positive_delta || ci.0 >= threshold {
                    performance_reasons.push(regression_reason_for_measure(&measure).to_string());
                }
            } else if let Some(max_relative) = protected_threshold(policy, &measure) {
                if unbounded_positive_delta || ci.0 >= max_relative {
                    performance_reasons.push(regression_reason_for_measure(&measure).to_string());
                }
            }
        }
        if performance_reasons.is_empty() && material_improvement_measures.is_empty() {
            performance_reasons.push("no_material_effect".to_string());
        } else if performance_reasons.is_empty() {
            effects.insert(
                "material_improvement_measures".to_string(),
                json!(material_improvement_measures),
            );
            performance_reasons.push("candidate_improved".to_string());
        }
    }
    push_gate(&mut gates, "performance", &performance_reasons);
    reasons.extend(performance_reasons);
    stable_dedup(&mut reasons);

    let verdict = if has_any(
        &reasons,
        &[
            "correctness_regressed",
            "safety_regressed",
            "quality_regressed",
            "performance_regressed",
            "cost_regressed",
            "protected_dimension_regressed",
        ],
    ) {
        "regressed"
    } else if has_any(
        &reasons,
        &[
            "suite_digest_mismatch",
            "fixture_digest_mismatch",
            "scorer_version_mismatch",
            "subject_identity_mismatch",
            "testbed_incompatible",
            "evidence_stale",
            "run_invalidated",
            "coverage_below_minimum",
            "samples_below_minimum",
            "scorer_control_missing_negative",
            "scorer_control_missing_oracle",
            "scorer_control_nondeterministic",
            "scorer_control_oracle_failed",
            "scorer_control_unexpected_no_op_success",
            "scorer_control_negative_passed",
            "variance_too_high",
            "missing_baseline",
            "missing_candidate",
        ],
    ) {
        "insufficient_evidence"
    } else if reasons == ["candidate_improved"] {
        "improved"
    } else {
        "no_material_difference"
    };
    result(verdict, reasons, gates, effects, uncertainty, protected)
}

fn result(
    verdict: &str,
    mut reasons: Vec<String>,
    gates: Vec<Value>,
    effects: serde_json::Map<String, Value>,
    uncertainty: serde_json::Map<String, Value>,
    protected: serde_json::Map<String, Value>,
) -> EvalComparisonResult {
    stable_dedup(&mut reasons);
    EvalComparisonResult {
        verdict: verdict.to_string(),
        reasons,
        gates: Value::Array(gates),
        effect_estimates: Value::Object(effects),
        uncertainty: Value::Object(uncertainty),
        protected_dimensions: Value::Object(protected),
    }
}

fn compatibility_reasons(baseline: &EvalRunEvidence, candidate: &EvalRunEvidence) -> Vec<String> {
    let mut reasons = Vec::new();
    if baseline.invalidated
        || candidate.invalidated
        || baseline.status == "invalidated"
        || candidate.status == "invalidated"
    {
        reasons.push("run_invalidated".to_string());
    }
    if baseline.status != "success" || candidate.status != "success" {
        reasons.push("coverage_below_minimum".to_string());
    }
    if baseline.suite_digest != candidate.suite_digest {
        reasons.push("suite_digest_mismatch".to_string());
    }
    if baseline.subject_kind != candidate.subject_kind
        || baseline.subject_path != candidate.subject_path
        || baseline.subject_argv != candidate.subject_argv
    {
        reasons.push("subject_identity_mismatch".to_string());
    }
    if baseline.testbed_fingerprint != candidate.testbed_fingerprint {
        reasons.push("testbed_incompatible".to_string());
    }
    let baseline_cases = case_map(baseline);
    let candidate_cases = case_map(candidate);
    for (case_id, baseline_case) in &baseline_cases {
        if let Some(candidate_case) = candidate_cases.get(case_id) {
            if baseline_case.scorer_id != candidate_case.scorer_id
                || baseline_case.scorer_version != candidate_case.scorer_version
            {
                reasons.push("scorer_version_mismatch".to_string());
            }
            if baseline_case.fixture_digest != candidate_case.fixture_digest {
                reasons.push("fixture_digest_mismatch".to_string());
            }
        }
    }
    stable_dedup(&mut reasons);
    reasons
}

fn freshness_reasons(
    baseline: &EvalRunEvidence,
    candidate: &EvalRunEvidence,
    policy: &EvalComparisonPolicy,
) -> Vec<String> {
    if baseline.age_hours > policy.freshness_max_age_hours
        || candidate.age_hours > policy.freshness_max_age_hours
    {
        vec!["evidence_stale".to_string()]
    } else {
        Vec::new()
    }
}

fn coverage_reasons(
    baseline: &EvalRunEvidence,
    candidate: &EvalRunEvidence,
    policy: &EvalComparisonPolicy,
) -> Vec<String> {
    let baseline_cases = case_map(baseline);
    let candidate_cases = case_map(candidate);
    if baseline_cases.is_empty() || candidate_cases.is_empty() {
        return vec!["coverage_below_minimum".to_string()];
    }
    let expected = baseline_cases.len().max(candidate_cases.len()) as f64;
    let covered = baseline_cases
        .keys()
        .filter(|case_id| candidate_cases.contains_key(*case_id))
        .count() as f64;
    if covered / expected < policy.required_case_coverage {
        vec!["coverage_below_minimum".to_string()]
    } else {
        Vec::new()
    }
}

fn correctness_reasons(candidate: &EvalRunEvidence) -> Vec<String> {
    let cases = ordinary_cases(candidate);
    if cases.iter().any(|case| case.status == "fail") {
        vec!["correctness_regressed".to_string()]
    } else if cases
        .iter()
        .any(|case| matches!(case.status.as_str(), "error" | "skipped" | "inconclusive"))
    {
        vec!["coverage_below_minimum".to_string()]
    } else {
        Vec::new()
    }
}

fn scorer_control_reasons(baseline: &EvalRunEvidence, candidate: &EvalRunEvidence) -> Vec<String> {
    let baseline_cases = case_map(baseline);
    let candidate_cases = case_map(candidate);
    let mut by_scorer = BTreeMap::<String, BTreeSet<String>>::new();
    let mut reasons = Vec::new();
    for (case_id, baseline_case) in &baseline_cases {
        let Some(control) = baseline_case.scorer_control.as_deref() else {
            continue;
        };
        let scorer = format!(
            "{}@{}",
            baseline_case.scorer_id, baseline_case.scorer_version
        );
        by_scorer
            .entry(scorer)
            .or_default()
            .insert(control.to_string());
        let Some(candidate_case) = candidate_cases.get(case_id) else {
            reasons.push("coverage_below_minimum".to_string());
            continue;
        };
        if candidate_case.scorer_control.as_deref() != Some(control) {
            reasons.push("scorer_control_nondeterministic".to_string());
            continue;
        }
        if let Some(reason) = control_repetition_integrity_reason(baseline_case) {
            reasons.push(reason.to_string());
        }
        if let Some(reason) = control_repetition_integrity_reason(candidate_case) {
            reasons.push(reason.to_string());
        }
        match control {
            "oracle" => {
                if has_mixed_control_outcomes(baseline_case)
                    || has_mixed_control_outcomes(candidate_case)
                {
                    reasons.push("scorer_control_nondeterministic".to_string());
                }
                if has_control_outcome_status_not(baseline_case, "pass")
                    || has_control_outcome_status_not(candidate_case, "pass")
                    || baseline_case.status != "pass"
                    || candidate_case.status != "pass"
                {
                    reasons.push("scorer_control_oracle_failed".to_string());
                }
            }
            "known_bad" => {
                if has_mixed_control_outcomes(baseline_case)
                    || has_mixed_control_outcomes(candidate_case)
                {
                    reasons.push("scorer_control_nondeterministic".to_string());
                }
                if has_control_outcome_status(baseline_case, "pass")
                    || has_control_outcome_status(candidate_case, "pass")
                {
                    reasons.push("scorer_control_negative_passed".to_string());
                }
                if baseline_case.status == "pass" || candidate_case.status == "pass" {
                    reasons.push("scorer_control_negative_passed".to_string());
                }
            }
            "no_op" => {
                if has_mixed_control_outcomes(baseline_case)
                    || has_mixed_control_outcomes(candidate_case)
                {
                    reasons.push("scorer_control_nondeterministic".to_string());
                }
                if has_control_outcome_status(baseline_case, "pass")
                    || has_control_outcome_status(candidate_case, "pass")
                {
                    reasons.push("scorer_control_unexpected_no_op_success".to_string());
                }
                if baseline_case.status == "pass" || candidate_case.status == "pass" {
                    reasons.push("scorer_control_unexpected_no_op_success".to_string());
                }
            }
            _ => reasons.push("scorer_control_nondeterministic".to_string()),
        }
    }
    for (scorer, controls) in by_scorer {
        if !controls.contains("oracle") {
            let _ = scorer;
            reasons.push("scorer_control_missing_oracle".to_string());
        }
        if !controls.contains("known_bad") && !controls.contains("no_op") {
            reasons.push("scorer_control_missing_negative".to_string());
        }
    }
    stable_dedup(&mut reasons);
    reasons
}

fn control_repetition_integrity_reason(case: &EvalCaseEvidence) -> Option<&'static str> {
    let required_repetitions = case
        .required_repetitions
        .or(case.min_successful_samples)
        .unwrap_or(case.repetition_outcomes.len())
        .max(2);
    let expected = (0..required_repetitions).collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut statuses = BTreeSet::new();
    for outcome in &case.repetition_outcomes {
        if !seen.insert(outcome.repetition_index) {
            return Some("scorer_control_nondeterministic");
        }
        statuses.insert(outcome.status.as_str());
    }
    if seen != expected {
        return Some("samples_below_minimum");
    }
    if statuses.len() > 1 {
        return Some("scorer_control_nondeterministic");
    }
    if statuses
        .iter()
        .next()
        .is_some_and(|status| *status != case.status)
    {
        return Some("scorer_control_nondeterministic");
    }
    None
}

fn has_mixed_control_outcomes(case: &EvalCaseEvidence) -> bool {
    let statuses = case
        .repetition_outcomes
        .iter()
        .map(|outcome| outcome.status.as_str())
        .collect::<BTreeSet<_>>();
    statuses.len() > 1
}

fn has_control_outcome_status(case: &EvalCaseEvidence, status: &str) -> bool {
    case.repetition_outcomes
        .iter()
        .any(|outcome| outcome.status == status)
}

fn has_control_outcome_status_not(case: &EvalCaseEvidence, status: &str) -> bool {
    case.repetition_outcomes
        .iter()
        .any(|outcome| outcome.status != status)
}

fn safety_reasons(candidate: &EvalRunEvidence) -> Vec<String> {
    if candidate
        .cases
        .iter()
        .any(|case| case.safety_pass == Some(false))
    {
        vec!["safety_regressed".to_string()]
    } else {
        Vec::new()
    }
}

fn quality_reasons(
    baseline: &EvalRunEvidence,
    candidate: &EvalRunEvidence,
    policy: &EvalComparisonPolicy,
    uncertainty: &mut serde_json::Map<String, Value>,
) -> Vec<String> {
    let baseline_cases = ordinary_cases(baseline);
    let candidate_cases = ordinary_cases(candidate);
    let baseline_total = baseline_cases.len();
    let candidate_total = candidate_cases.len();
    if baseline_total == 0 || candidate_total == 0 {
        return vec!["coverage_below_minimum".to_string()];
    }
    let baseline_passes = baseline_cases
        .iter()
        .filter(|case| quality_pass(case))
        .count();
    let candidate_passes = candidate_cases
        .iter()
        .filter(|case| quality_pass(case))
        .count();
    let baseline_ci = wilson_interval(baseline_passes, baseline_total, 1.96);
    let candidate_ci = wilson_interval(candidate_passes, candidate_total, 1.96);
    uncertainty.insert(
        "quality_baseline_wilson_ci95".to_string(),
        json!([baseline_ci.0, baseline_ci.1]),
    );
    uncertainty.insert(
        "quality_candidate_wilson_ci95".to_string(),
        json!([candidate_ci.0, candidate_ci.1]),
    );
    uncertainty.insert(
        "quality_non_inferiority_margin".to_string(),
        json!(policy.quality_non_inferiority_margin),
    );
    if candidate_ci.1 + policy.quality_non_inferiority_margin < baseline_ci.0 {
        vec!["quality_regressed".to_string()]
    } else {
        Vec::new()
    }
}

fn quality_pass(case: &EvalCaseEvidence) -> bool {
    case.quality_pass.unwrap_or(case.status == "pass")
}

fn regression_reason_for_measure(measure: &str) -> &'static str {
    if measure.contains("cost") || measure.contains("tool_calls") {
        "cost_regressed"
    } else {
        "performance_regressed"
    }
}

fn material_improvement_threshold(policy: &EvalComparisonPolicy, measure: &str) -> Option<f64> {
    policy_threshold(
        &policy.material_improvement,
        &canonical_p95_effect_key(measure),
        measure,
    )
}

fn material_regression_threshold(policy: &EvalComparisonPolicy, measure: &str) -> Option<f64> {
    policy_threshold(
        &policy.material_regression,
        &canonical_p95_effect_key(measure),
        measure,
    )
}

fn protected_threshold(policy: &EvalComparisonPolicy, measure: &str) -> Option<f64> {
    policy_threshold(
        &policy.protected_relative_max,
        &canonical_protected_budget_key(measure),
        measure,
    )
}

fn policy_threshold(
    thresholds: &BTreeMap<String, f64>,
    canonical_key: &str,
    legacy_measure_key: &str,
) -> Option<f64> {
    thresholds
        .get(canonical_key)
        .or_else(|| thresholds.get(legacy_measure_key))
        .copied()
}

fn canonical_p95_effect_key(measure: &str) -> String {
    format!("{measure}_p95_relative")
}

fn canonical_protected_budget_key(measure: &str) -> String {
    format!("{measure}_relative_max")
}

fn push_gate(gates: &mut Vec<Value>, gate: &str, reasons: &[String]) {
    gates.push(json!({
        "gate": gate,
        "status": if reasons.is_empty() || reasons == ["candidate_improved"] || reasons == ["no_material_effect"] { "pass" } else { "fail" },
        "reasons": reasons,
    }));
}

fn case_map(run: &EvalRunEvidence) -> BTreeMap<String, &EvalCaseEvidence> {
    run.cases
        .iter()
        .map(|case| (case.case_id.clone(), case))
        .collect()
}

fn ordinary_cases(run: &EvalRunEvidence) -> Vec<&EvalCaseEvidence> {
    run.cases
        .iter()
        .filter(|case| case.scorer_control.is_none())
        .collect()
}

fn ordinary_case_map(run: &EvalRunEvidence) -> BTreeMap<String, &EvalCaseEvidence> {
    ordinary_cases(run)
        .into_iter()
        .map(|case| (case.case_id.clone(), case))
        .collect()
}

fn compared_measures(baseline: &EvalRunEvidence, candidate: &EvalRunEvidence) -> BTreeSet<String> {
    ordinary_cases(baseline)
        .into_iter()
        .chain(ordinary_cases(candidate))
        .flat_map(|case| case.samples.iter())
        .filter(|sample| !sample.warmup && sample.valid)
        .map(|sample| sample.measure.clone())
        .collect()
}

struct PairedMeasureValues {
    baseline: Vec<f64>,
    candidate: Vec<f64>,
    case_pair_counts: Vec<(String, usize, usize)>,
}

fn paired_measure_values(
    baseline: &BTreeMap<String, &EvalCaseEvidence>,
    candidate: &BTreeMap<String, &EvalCaseEvidence>,
    measure: &str,
) -> PairedMeasureValues {
    let mut baseline_values = Vec::new();
    let mut candidate_values = Vec::new();
    let mut case_pair_counts = Vec::new();
    for (case_id, baseline_case) in baseline {
        let Some(candidate_case) = candidate.get(case_id) else {
            continue;
        };
        let required_count = baseline_case
            .min_successful_samples
            .unwrap_or(0)
            .max(candidate_case.min_successful_samples.unwrap_or(0));
        let mut case_pairs = Vec::<(usize, i64, f64, f64)>::new();
        for baseline_sample in baseline_case
            .samples
            .iter()
            .filter(|sample| !sample.warmup && sample.valid && sample.measure == measure)
        {
            let paired_candidate = candidate_case.samples.iter().find(|candidate_sample| {
                !candidate_sample.warmup
                    && candidate_sample.valid
                    && candidate_sample.measure == measure
                    && candidate_sample.repetition_index == baseline_sample.repetition_index
                    && candidate_sample.seed == baseline_sample.seed
            });
            if let Some(candidate_sample) = paired_candidate {
                case_pairs.push((
                    baseline_sample.repetition_index,
                    baseline_sample.seed,
                    baseline_sample.value,
                    candidate_sample.value,
                ));
            }
        }
        case_pairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        case_pair_counts.push((case_id.clone(), case_pairs.len(), required_count));
        for (_, _, baseline_value, candidate_value) in case_pairs {
            baseline_values.push(baseline_value);
            candidate_values.push(candidate_value);
        }
    }
    PairedMeasureValues {
        baseline: baseline_values,
        candidate: candidate_values,
        case_pair_counts,
    }
}

pub(crate) fn wilson_interval(successes: usize, total: usize, z: f64) -> (f64, f64) {
    if total == 0 {
        return (0.0, 0.0);
    }
    let n = total as f64;
    let phat = successes as f64 / n;
    let denom = 1.0 + z * z / n;
    let center = (phat + z * z / (2.0 * n)) / denom;
    let margin = z * ((phat * (1.0 - phat) + z * z / (4.0 * n)) / n).sqrt() / denom;
    (center - margin, center + margin)
}

fn median(values: &[f64]) -> f64 {
    percentile(values, 0.5)
}

fn percentile(values: &[f64], p: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() - 1) as f64 * p).ceil() as usize;
    sorted[rank]
}

fn relative_delta(candidate: f64, baseline: f64) -> f64 {
    if baseline == 0.0 {
        if candidate == 0.0 {
            0.0
        } else if candidate > 0.0 {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        }
    } else {
        (candidate - baseline) / baseline
    }
}

fn relative_value(value: f64) -> Value {
    if value.is_infinite() && value.is_sign_positive() {
        json!("unbounded_positive")
    } else if value.is_infinite() && value.is_sign_negative() {
        json!("unbounded_negative")
    } else {
        json!(value)
    }
}

fn relative_interval_value(interval: (f64, f64)) -> Value {
    Value::Array(vec![relative_value(interval.0), relative_value(interval.1)])
}

fn is_invalid_nonnegative_measure_value(measure: &str, value: f64) -> bool {
    value < 0.0
        && (measure.contains("duration")
            || measure.contains("latency")
            || measure.contains("cost")
            || measure.contains("tool_calls")
            || measure.contains("count"))
}

fn paired_bootstrap_relative_delta(
    baseline: &[f64],
    candidate: &[f64],
    iterations: usize,
    seed: u64,
) -> (f64, f64) {
    let count = baseline.len().min(candidate.len());
    if count == 0 || iterations == 0 {
        return (0.0, 0.0);
    }
    let mut rng = seed.max(1);
    let mut deltas = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut base = Vec::with_capacity(count);
        let mut cand = Vec::with_capacity(count);
        for _ in 0..count {
            rng = xorshift64(rng);
            let idx = (rng as usize) % count;
            base.push(baseline[idx]);
            cand.push(candidate[idx]);
        }
        deltas.push(relative_delta(
            percentile(&cand, 0.95),
            percentile(&base, 0.95),
        ));
    }
    deltas.sort_by(|a, b| a.total_cmp(b));
    let low = ((iterations - 1) as f64 * 0.025).floor() as usize;
    let high = ((iterations - 1) as f64 * 0.975).ceil() as usize;
    (deltas[low], deltas[high])
}

fn variance_classification(baseline: &[f64], candidate: &[f64], max_cv: f64) -> &'static str {
    let cv = coefficient_of_variation(baseline).max(coefficient_of_variation(candidate));
    if cv > max_cv {
        "excessive"
    } else {
        "acceptable"
    }
}

fn coefficient_of_variation(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    if mean == 0.0 {
        return 0.0;
    }
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt() / mean.abs()
}

fn xorshift64(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn stable_dedup(reasons: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    reasons.retain(|reason| seen.insert(reason.clone()));
}

fn has_any(reasons: &[String], needles: &[&str]) -> bool {
    reasons
        .iter()
        .any(|reason| needles.iter().any(|needle| reason == needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(repetition_index: usize, value: f64, measure: &str) -> EvalNumericSample {
        EvalNumericSample {
            repetition_index,
            warmup: false,
            seed: repetition_index as i64,
            measure: measure.to_string(),
            value,
            valid: true,
        }
    }

    fn run(run_id: &str, values: &[f64], case_status: &str) -> EvalRunEvidence {
        run_with_measure(run_id, values, case_status, "duration_ms")
    }

    fn run_with_measure(
        run_id: &str,
        values: &[f64],
        case_status: &str,
        measure: &str,
    ) -> EvalRunEvidence {
        EvalRunEvidence {
            run_id: run_id.to_string(),
            suite_digest: "sha256:suite".to_string(),
            subject_kind: "planr_cli".to_string(),
            subject_path: Some(".".to_string()),
            subject_argv: "[\"planr\"]".to_string(),
            testbed_fingerprint: "{\"os\":\"x\"}".to_string(),
            status: "success".to_string(),
            invalidated: false,
            age_hours: 1,
            cases: vec![EvalCaseEvidence {
                case_id: "case-a".to_string(),
                scorer_id: "scorer".to_string(),
                scorer_version: "1.0.0".to_string(),
                scorer_control: None,
                fixture_digest: "sha256:fixture".to_string(),
                status: case_status.to_string(),
                repetition_outcomes: values
                    .iter()
                    .enumerate()
                    .map(|(index, _)| EvalRepetitionOutcome {
                        repetition_index: index,
                        status: case_status.to_string(),
                    })
                    .collect(),
                safety_pass: None,
                quality_pass: None,
                min_successful_samples: Some(values.len()),
                required_repetitions: Some(values.len()),
                samples: values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| sample(index, *value, measure))
                    .collect(),
            }],
        }
    }

    #[test]
    fn eval_compare_wilson_interval_is_deterministic() {
        let (low, high) = wilson_interval(9, 10, 1.96);
        assert!((low - 0.5958).abs() < 0.001);
        assert!((high - 0.9821).abs() < 0.001);
    }

    #[test]
    fn eval_compare_material_regression_requires_bootstrap_confidence() {
        let baseline = run("base", &[100.0, 101.0, 99.0, 102.0, 100.0], "pass");
        let candidate = run("cand", &[105.0, 106.05, 103.95, 107.1, 105.0], "pass");
        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "no_material_difference");
        assert_eq!(result.reasons, vec!["no_material_effect"]);
        assert!(
            result.effect_estimates["duration_ms_p95_relative"]
                .as_f64()
                .unwrap()
                > 0.0
        );
        assert!(
            result.effect_estimates["duration_ms_p95_relative"]
                .as_f64()
                .unwrap()
                < 0.10
        );

        let baseline = run("base", &[100.0, 101.0, 99.0, 102.0, 100.0], "pass");
        let candidate = run("cand", &[112.0, 113.12, 110.88, 114.24, 112.0], "pass");
        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "regressed");
        assert_eq!(result.reasons, vec!["performance_regressed"]);
        assert!(
            result.uncertainty["duration_ms_p95_ci95"][0]
                .as_f64()
                .unwrap()
                >= 0.10
        );
    }

    #[test]
    fn eval_compare_improvement_requires_bootstrap_confidence_not_point_estimate() {
        let baseline = run("base", &[110.0, 120.0, 80.0, 130.0, 110.0], "pass");
        let candidate = run("cand", &[110.0, 70.0, 60.0, 50.0, 90.0], "pass");
        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "no_material_difference");
        assert_eq!(result.reasons, vec!["no_material_effect"]);
        assert!(
            result.effect_estimates["duration_ms_p95_relative"]
                .as_f64()
                .unwrap()
                <= -0.10
        );
        assert!(
            result.uncertainty["duration_ms_p95_ci95"][1]
                .as_f64()
                .unwrap()
                > -0.10
        );
    }

    #[test]
    fn eval_compare_improved_requires_material_bootstrap_effect() {
        let baseline = run("base", &[100.0, 101.0, 99.0, 102.0, 100.0], "pass");
        let candidate = run("cand", &[80.0, 81.0, 79.0, 82.0, 80.0], "pass");
        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "improved");
        assert_eq!(result.reasons, vec!["candidate_improved"]);
        assert!(
            result.effect_estimates["duration_ms_p95_relative"]
                .as_f64()
                .unwrap()
                < -0.1
        );
    }

    #[test]
    fn eval_compare_safety_regression_hard_gate_beats_material_improvement() {
        let baseline = run("base", &[100.0, 101.0, 99.0, 102.0, 100.0], "pass");
        let mut candidate = run("cand", &[80.0, 81.0, 79.0, 82.0, 80.0], "pass");
        candidate.cases[0].safety_pass = Some(false);
        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "regressed");
        assert_eq!(result.reasons, vec!["safety_regressed"]);
        assert!(
            result.effect_estimates.as_object().unwrap().is_empty(),
            "performance must not run after the hard safety gate fails"
        );
        let safety_gate = result
            .gates
            .as_array()
            .unwrap()
            .iter()
            .find(|gate| gate["gate"] == "safety")
            .expect("safety gate is recorded");
        assert_eq!(safety_gate["status"], "fail");
        assert_eq!(safety_gate["reasons"], json!(["safety_regressed"]));
    }

    #[test]
    fn eval_compare_blocks_promotion_when_scorer_controls_fail() {
        let mut baseline = run("base", &[100.0, 101.0, 99.0], "pass");
        let template = baseline.cases[0].clone();
        baseline.cases = vec![
            {
                let mut case = template.clone();
                case.case_id = "oracle".to_string();
                case.scorer_control = Some("oracle".to_string());
                case
            },
            {
                let mut case = template.clone();
                case.case_id = "no-op".to_string();
                case.scorer_control = Some("no_op".to_string());
                case.status = "fail".to_string();
                for outcome in &mut case.repetition_outcomes {
                    outcome.status = "fail".to_string();
                }
                case
            },
        ];
        let mut candidate = baseline.clone();
        candidate.run_id = "cand".to_string();
        candidate.cases[0].status = "fail".to_string();
        candidate.cases[1].status = "pass".to_string();

        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "insufficient_evidence");
        assert_eq!(
            result.reasons,
            vec![
                "scorer_control_nondeterministic",
                "scorer_control_unexpected_no_op_success",
                "scorer_control_oracle_failed"
            ]
        );
        assert!(
            result
                .effect_estimates
                .as_object()
                .expect("effect estimates object")
                .is_empty()
        );
    }

    #[test]
    fn eval_compare_blocks_mixed_no_op_control_repetition_outcomes() {
        let mut baseline = run("base", &[100.0, 101.0, 99.0], "pass");
        let template = baseline.cases[0].clone();
        baseline.cases = vec![
            {
                let mut case = template.clone();
                case.case_id = "oracle".to_string();
                case.scorer_control = Some("oracle".to_string());
                case.min_successful_samples = Some(2);
                case.required_repetitions = Some(2);
                case.repetition_outcomes.truncate(2);
                case
            },
            {
                let mut case = template.clone();
                case.case_id = "no-op".to_string();
                case.scorer_control = Some("no_op".to_string());
                case.status = "fail".to_string();
                case.min_successful_samples = Some(2);
                case.required_repetitions = Some(2);
                case.repetition_outcomes = vec![
                    EvalRepetitionOutcome {
                        repetition_index: 0,
                        status: "pass".to_string(),
                    },
                    EvalRepetitionOutcome {
                        repetition_index: 1,
                        status: "error".to_string(),
                    },
                ];
                case
            },
        ];
        let mut candidate = baseline.clone();
        candidate.run_id = "cand".to_string();
        candidate.cases[1].repetition_outcomes = vec![
            EvalRepetitionOutcome {
                repetition_index: 0,
                status: "fail".to_string(),
            },
            EvalRepetitionOutcome {
                repetition_index: 1,
                status: "inconclusive".to_string(),
            },
        ];

        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "insufficient_evidence");
        assert_eq!(
            result.reasons,
            vec![
                "scorer_control_nondeterministic",
                "scorer_control_unexpected_no_op_success"
            ]
        );
        assert!(
            result
                .effect_estimates
                .as_object()
                .expect("effect estimates object")
                .is_empty()
        );
    }

    #[test]
    fn eval_compare_blocks_zero_or_one_control_repetition_outcomes() {
        for (label, outcomes) in [
            ("zero", Vec::new()),
            (
                "one",
                vec![EvalRepetitionOutcome {
                    repetition_index: 0,
                    status: "pass".to_string(),
                }],
            ),
        ] {
            let mut baseline = run("base", &[100.0, 101.0, 99.0], "pass");
            let template = baseline.cases[0].clone();
            baseline.cases = vec![
                {
                    let mut case = template.clone();
                    case.case_id = "oracle".to_string();
                    case.scorer_control = Some("oracle".to_string());
                    case.repetition_outcomes = outcomes.clone();
                    case.min_successful_samples = Some(2);
                    case
                },
                {
                    let mut case = template.clone();
                    case.case_id = "no-op".to_string();
                    case.scorer_control = Some("no_op".to_string());
                    case.status = "fail".to_string();
                    for outcome in &mut case.repetition_outcomes {
                        outcome.status = "fail".to_string();
                    }
                    case
                },
            ];
            let mut candidate = baseline.clone();
            candidate.run_id = format!("cand-{label}");

            let result = compare_eval_runs(
                Some(&baseline),
                Some(&candidate),
                &EvalComparisonPolicy::default(),
            );
            assert_eq!(result.verdict, "insufficient_evidence");
            assert_eq!(result.reasons, vec!["samples_below_minimum"]);
        }
    }

    #[test]
    fn eval_compare_blocks_duplicate_or_status_mismatched_control_outcomes() {
        let mut baseline = run("base", &[100.0, 101.0, 99.0], "pass");
        let template = baseline.cases[0].clone();
        baseline.cases = vec![
            {
                let mut case = template.clone();
                case.case_id = "oracle".to_string();
                case.scorer_control = Some("oracle".to_string());
                case.repetition_outcomes = vec![
                    EvalRepetitionOutcome {
                        repetition_index: 0,
                        status: "pass".to_string(),
                    },
                    EvalRepetitionOutcome {
                        repetition_index: 0,
                        status: "pass".to_string(),
                    },
                ];
                case.min_successful_samples = Some(2);
                case.required_repetitions = Some(2);
                case
            },
            {
                let mut case = template.clone();
                case.case_id = "no-op".to_string();
                case.scorer_control = Some("no_op".to_string());
                case.status = "fail".to_string();
                case.repetition_outcomes = vec![
                    EvalRepetitionOutcome {
                        repetition_index: 0,
                        status: "pass".to_string(),
                    },
                    EvalRepetitionOutcome {
                        repetition_index: 1,
                        status: "pass".to_string(),
                    },
                ];
                case.min_successful_samples = Some(2);
                case.required_repetitions = Some(2);
                case
            },
        ];
        let mut candidate = baseline.clone();
        candidate.run_id = "cand".to_string();

        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "insufficient_evidence");
        assert_eq!(
            result.reasons,
            vec![
                "scorer_control_nondeterministic",
                "scorer_control_unexpected_no_op_success"
            ]
        );
    }

    #[test]
    fn eval_compare_requires_exact_frozen_control_repetition_indices_and_status() {
        let mut baseline = run("base", &[100.0, 101.0, 99.0, 102.0, 100.0], "pass");
        let template = baseline.cases[0].clone();
        baseline.cases = vec![
            {
                let mut case = template.clone();
                case.case_id = "oracle".to_string();
                case.scorer_control = Some("oracle".to_string());
                case.min_successful_samples = Some(2);
                case.required_repetitions = Some(5);
                case.repetition_outcomes = vec![
                    EvalRepetitionOutcome {
                        repetition_index: 3,
                        status: "pass".to_string(),
                    },
                    EvalRepetitionOutcome {
                        repetition_index: 4,
                        status: "pass".to_string(),
                    },
                ];
                case
            },
            {
                let mut case = template.clone();
                case.case_id = "no-op".to_string();
                case.scorer_control = Some("no_op".to_string());
                case.status = "fail".to_string();
                case.min_successful_samples = Some(2);
                case.required_repetitions = Some(5);
                case.repetition_outcomes = (0..5)
                    .map(|repetition_index| EvalRepetitionOutcome {
                        repetition_index,
                        status: "error".to_string(),
                    })
                    .collect();
                case
            },
        ];
        let mut candidate = baseline.clone();
        candidate.run_id = "cand".to_string();

        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "insufficient_evidence");
        assert_eq!(
            result.reasons,
            vec!["scorer_control_nondeterministic", "samples_below_minimum"]
        );
    }

    #[test]
    fn eval_compare_valid_scorer_controls_do_not_block_ordinary_verdict() {
        let mut baseline = run("base", &[100.0, 101.0, 99.0, 102.0, 100.0], "pass");
        let template = baseline.cases[0].clone();
        baseline.cases = vec![
            {
                let mut case = template.clone();
                case.case_id = "oracle".to_string();
                case.scorer_control = Some("oracle".to_string());
                case
            },
            {
                let mut case = template.clone();
                case.case_id = "no-op".to_string();
                case.scorer_control = Some("no_op".to_string());
                case.status = "fail".to_string();
                for outcome in &mut case.repetition_outcomes {
                    outcome.status = "fail".to_string();
                }
                case
            },
            {
                let mut case = template.clone();
                case.case_id = "ordinary".to_string();
                case
            },
        ];
        let mut candidate = baseline.clone();
        candidate.run_id = "cand".to_string();
        candidate.cases[2].samples = [80.0, 81.0, 79.0, 82.0, 80.0]
            .iter()
            .enumerate()
            .map(|(index, value)| sample(index, *value, "duration_ms"))
            .collect();

        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "improved");
        assert_eq!(result.reasons, vec!["candidate_improved"]);
        assert_eq!(
            result.effect_estimates["material_improvement_measures"],
            json!(["duration_ms"])
        );
    }

    #[test]
    fn eval_compare_canonical_policy_measure_keys_drive_all_material_gates() {
        let mut improvement_policy = EvalComparisonPolicy {
            material_improvement: BTreeMap::from([("duration_ms_p95_relative".to_string(), -0.10)]),
            material_regression: BTreeMap::new(),
            protected_relative_max: BTreeMap::new(),
            ..EvalComparisonPolicy::default()
        };
        let baseline = run("base", &[100.0, 101.0, 99.0, 102.0, 100.0], "pass");
        let candidate = run("cand", &[80.0, 81.0, 79.0, 82.0, 80.0], "pass");
        let result = compare_eval_runs(Some(&baseline), Some(&candidate), &improvement_policy);
        assert_eq!(result.verdict, "improved");
        assert_eq!(result.reasons, vec!["candidate_improved"]);

        improvement_policy.material_improvement.clear();
        improvement_policy.material_regression =
            BTreeMap::from([("duration_ms_p95_relative".to_string(), 0.10)]);
        let baseline = run("base", &[100.0, 101.0, 99.0, 102.0, 100.0], "pass");
        let candidate = run("cand", &[112.0, 113.12, 110.88, 114.24, 112.0], "pass");
        let result = compare_eval_runs(Some(&baseline), Some(&candidate), &improvement_policy);
        assert_eq!(result.verdict, "regressed");
        assert_eq!(result.reasons, vec!["performance_regressed"]);

        let protected_policy = EvalComparisonPolicy {
            material_improvement: BTreeMap::new(),
            material_regression: BTreeMap::new(),
            protected_relative_max: BTreeMap::from([("tool_calls_relative_max".to_string(), 0.05)]),
            ..EvalComparisonPolicy::default()
        };
        let baseline = run_with_measure(
            "base",
            &[10.0, 10.0, 10.0, 10.0, 10.0],
            "pass",
            "tool_calls",
        );
        let candidate = run_with_measure(
            "cand",
            &[11.0, 11.0, 11.0, 11.0, 11.0],
            "pass",
            "tool_calls",
        );
        let result = compare_eval_runs(Some(&baseline), Some(&candidate), &protected_policy);
        assert_eq!(result.verdict, "regressed");
        assert_eq!(
            result.reasons,
            vec!["protected_dimension_regressed", "cost_regressed"]
        );
        assert_eq!(
            result.protected_dimensions["tool_calls_relative"]
                .as_f64()
                .unwrap(),
            0.10
        );
    }

    #[test]
    fn eval_compare_zero_baseline_truth_table_is_explicit_and_serializable() {
        let policy = EvalComparisonPolicy {
            material_improvement: BTreeMap::new(),
            material_regression: BTreeMap::new(),
            protected_relative_max: BTreeMap::from([("tool_calls_relative_max".to_string(), 2.0)]),
            ..EvalComparisonPolicy::default()
        };

        let baseline = run_with_measure("base", &[0.0, 0.0, 0.0], "pass", "tool_calls");
        let candidate = run_with_measure("cand", &[0.0, 0.0, 0.0], "pass", "tool_calls");
        let result = compare_eval_runs(Some(&baseline), Some(&candidate), &policy);
        assert_eq!(result.verdict, "no_material_difference");
        assert_eq!(result.reasons, vec!["no_material_effect"]);
        assert_eq!(
            result.effect_estimates["tool_calls_p95_relative"]
                .as_f64()
                .unwrap(),
            0.0
        );
        assert_eq!(result.uncertainty["tool_calls_p95_ci95"], json!([0.0, 0.0]));

        let baseline = run_with_measure("base", &[0.0, 0.0, 0.0], "pass", "tool_calls");
        let candidate = run_with_measure("cand", &[1.0, 1.0, 1.0], "pass", "tool_calls");
        let result = compare_eval_runs(Some(&baseline), Some(&candidate), &policy);
        assert_eq!(result.verdict, "regressed");
        assert_eq!(
            result.reasons,
            vec!["protected_dimension_regressed", "cost_regressed"]
        );
        assert_eq!(
            result.effect_estimates["tool_calls_p95_relative"],
            json!("unbounded_positive")
        );
        assert_eq!(
            result.uncertainty["tool_calls_p95_ci95"],
            json!(["unbounded_positive", "unbounded_positive"])
        );
        assert_eq!(
            result.protected_dimensions["tool_calls_relative"],
            json!("unbounded_positive")
        );
    }

    #[test]
    fn eval_compare_rejects_negative_nonnegative_measure_samples() {
        let baseline = run("base", &[100.0, 101.0, 99.0], "pass");
        let candidate = run("cand", &[-1.0, -1.0, -1.0], "pass");
        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "insufficient_evidence");
        assert_eq!(result.reasons, vec!["samples_below_minimum"]);
        assert!(
            result
                .effect_estimates
                .get("duration_ms_p95_relative")
                .is_none()
        );
    }

    #[test]
    fn eval_compare_requires_minimum_samples_per_case_and_measure_before_pooling() {
        let mut baseline = run_with_measure("base", &[100.0], "pass", "duration_ms");
        let template_case = baseline.cases[0].clone();
        baseline.cases = (0..3)
            .map(|index| {
                let mut case = template_case.clone();
                case.case_id = format!("case-{index}");
                case.min_successful_samples = Some(3);
                case.samples = vec![sample(0, 100.0 + index as f64, "duration_ms")];
                case
            })
            .collect();
        let mut candidate = baseline.clone();
        candidate.run_id = "cand".to_string();
        for (index, case) in candidate.cases.iter_mut().enumerate() {
            case.samples = vec![sample(0, 80.0 + index as f64, "duration_ms")];
        }

        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "insufficient_evidence");
        assert_eq!(result.reasons, vec!["samples_below_minimum"]);
        assert!(
            result
                .effect_estimates
                .get("duration_ms_p95_relative")
                .is_none(),
            "pooled samples must not be estimated after a case-level shortfall"
        );

        let policy = EvalComparisonPolicy {
            min_samples: 1,
            ..EvalComparisonPolicy::default()
        };
        let mut baseline = run("base", &[100.0], "pass");
        baseline.cases[0].min_successful_samples = Some(1);
        let mut candidate = run("cand", &[80.0], "pass");
        candidate.cases[0].min_successful_samples = Some(4);
        let result = compare_eval_runs(Some(&baseline), Some(&candidate), &policy);
        assert_eq!(result.verdict, "insufficient_evidence");
        assert_eq!(result.reasons, vec!["samples_below_minimum"]);
    }

    #[test]
    fn eval_compare_faster_but_wrong_regresses_correctness_first() {
        let baseline = run("base", &[100.0, 101.0, 99.0], "pass");
        let candidate = run("cand", &[50.0, 51.0, 49.0], "fail");
        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "regressed");
        assert!(
            result
                .reasons
                .contains(&"correctness_regressed".to_string())
        );
    }

    #[test]
    fn eval_compare_quality_non_inferiority_uses_wilson_margin_boundary() {
        let mut baseline = run("base", &[100.0, 101.0, 99.0, 102.0, 100.0], "pass");
        let template_case = baseline.cases[0].clone();
        baseline.cases.extend((1..20).map(|index| {
            let mut case = template_case.clone();
            case.case_id = format!("case-{index}");
            case
        }));
        let mut candidate = baseline.clone();
        candidate.run_id = "cand".to_string();
        for case in baseline.cases.iter_mut() {
            case.quality_pass = Some(true);
        }
        for case in candidate.cases.iter_mut() {
            case.status = "pass".to_string();
            case.quality_pass = Some(true);
        }
        for case in candidate.cases.iter_mut().take(8) {
            case.quality_pass = Some(false);
        }
        let baseline_ci = wilson_interval(20, 20, 1.96);
        let candidate_ci = wilson_interval(12, 20, 1.96);
        let boundary_margin = baseline_ci.0 - candidate_ci.1;

        let mut policy = EvalComparisonPolicy {
            quality_non_inferiority_margin: boundary_margin - 0.000_001,
            ..EvalComparisonPolicy::default()
        };
        let result = compare_eval_runs(Some(&baseline), Some(&candidate), &policy);
        assert_eq!(result.verdict, "regressed");
        assert_eq!(result.reasons, vec!["quality_regressed"]);
        assert_eq!(
            result.uncertainty["quality_non_inferiority_margin"]
                .as_f64()
                .unwrap(),
            policy.quality_non_inferiority_margin
        );

        policy.quality_non_inferiority_margin = boundary_margin;
        let result = compare_eval_runs(Some(&baseline), Some(&candidate), &policy);
        assert_eq!(result.verdict, "no_material_difference");
        assert_eq!(result.reasons, vec!["no_material_effect"]);
        assert!(
            result
                .uncertainty
                .get("quality_baseline_wilson_ci95")
                .is_some()
        );
        assert!(
            result
                .uncertainty
                .get("quality_candidate_wilson_ci95")
                .is_some()
        );
    }

    #[test]
    fn eval_compare_stale_mismatch_missing_coverage_and_invalidated_are_insufficient() {
        let baseline = run("base", &[100.0, 101.0, 99.0], "pass");
        let mut candidate = run("cand", &[80.0, 81.0, 79.0], "pass");
        candidate.age_hours = 999;
        assert_eq!(
            compare_eval_runs(
                Some(&baseline),
                Some(&candidate),
                &EvalComparisonPolicy::default()
            )
            .verdict,
            "insufficient_evidence"
        );
        candidate = run("cand", &[80.0, 81.0, 79.0], "pass");
        candidate.testbed_fingerprint = "{\"os\":\"other\"}".to_string();
        assert_eq!(
            compare_eval_runs(
                Some(&baseline),
                Some(&candidate),
                &EvalComparisonPolicy::default()
            )
            .reasons,
            vec!["testbed_incompatible"]
        );
        candidate = run("cand", &[80.0, 81.0, 79.0], "pass");
        candidate.cases.clear();
        assert_eq!(
            compare_eval_runs(
                Some(&baseline),
                Some(&candidate),
                &EvalComparisonPolicy::default()
            )
            .reasons,
            vec!["coverage_below_minimum"]
        );
        candidate = run("cand", &[80.0, 81.0, 79.0], "pass");
        candidate.invalidated = true;
        assert_eq!(
            compare_eval_runs(
                Some(&baseline),
                Some(&candidate),
                &EvalComparisonPolicy::default()
            )
            .reasons,
            vec!["run_invalidated"]
        );
    }

    #[test]
    fn eval_compare_under_sampling_and_excessive_variance_cannot_improve() {
        let baseline = run("base", &[100.0, 101.0], "pass");
        let candidate = run("cand", &[80.0, 81.0], "pass");
        assert_eq!(
            compare_eval_runs(
                Some(&baseline),
                Some(&candidate),
                &EvalComparisonPolicy::default()
            )
            .reasons,
            vec!["samples_below_minimum"]
        );

        let baseline = run("base", &[10.0, 1000.0, 10.0, 1000.0, 10.0], "pass");
        let candidate = run("cand", &[8.0, 800.0, 8.0, 800.0, 8.0], "pass");
        assert_eq!(
            compare_eval_runs(
                Some(&baseline),
                Some(&candidate),
                &EvalComparisonPolicy::default()
            )
            .reasons,
            vec!["variance_too_high"]
        );
    }

    #[test]
    fn eval_compare_no_numeric_data_is_insufficient_evidence() {
        let mut baseline = run("base", &[100.0, 101.0, 99.0], "pass");
        let mut candidate = run("cand", &[80.0, 81.0, 79.0], "pass");
        baseline.cases[0].samples.clear();
        candidate.cases[0].samples.clear();
        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "insufficient_evidence");
        assert_eq!(result.reasons, vec!["samples_below_minimum"]);

        baseline = run("base", &[100.0, 101.0, 99.0], "pass");
        candidate = run("cand", &[80.0, 81.0, 79.0], "pass");
        for sample in baseline.cases[0]
            .samples
            .iter_mut()
            .chain(candidate.cases[0].samples.iter_mut())
        {
            sample.valid = false;
        }
        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "insufficient_evidence");
        assert_eq!(result.reasons, vec!["samples_below_minimum"]);
    }

    #[test]
    fn eval_compare_no_material_difference_when_effect_is_small() {
        let baseline = run("base", &[100.0, 101.0, 99.0, 102.0, 100.0], "pass");
        let candidate = run("cand", &[98.0, 99.0, 97.0, 100.0, 98.0], "pass");
        let result = compare_eval_runs(
            Some(&baseline),
            Some(&candidate),
            &EvalComparisonPolicy::default(),
        );
        assert_eq!(result.verdict, "no_material_difference");
        assert_eq!(result.reasons, vec!["no_material_effect"]);
    }
}
