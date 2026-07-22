use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const CONTRACT_PATH: &str = "docs/planr-spec/EVAL_CONTRACT_V1.md";

fn contract_text() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(CONTRACT_PATH))
        .expect("Eval Contract v1 must stay checked in")
}

fn json_fences(markdown: &str) -> Vec<String> {
    let mut fences = Vec::new();
    let mut current = Vec::new();
    let mut in_json = false;

    for line in markdown.lines() {
        if line.trim() == "```json" {
            in_json = true;
            current.clear();
            continue;
        }
        if in_json && line.trim() == "```" {
            in_json = false;
            fences.push(current.join("\n"));
            continue;
        }
        if in_json {
            current.push(line);
        }
    }

    fences
}

fn json_values() -> Vec<Value> {
    json_fences(&contract_text())
        .into_iter()
        .map(|fence| serde_json::from_str::<Value>(&fence).expect("contract JSON fence is valid"))
        .collect()
}

fn sha256_hex(bytes: &str) -> String {
    format!("{:x}", Sha256::digest(bytes.as_bytes()))
}

fn sha256_prefixed(bytes: &str) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

fn node_jcs(input_json: &str) -> String {
    let script = r#"
const input = process.argv[1];
function normalize(value) {
  if (Array.isArray(value)) return value.map(normalize);
  if (value && typeof value === "object") {
    const out = {};
    for (const key of Object.keys(value).sort()) out[key] = normalize(value[key]);
    return out;
  }
  return value;
}
process.stdout.write(JSON.stringify(normalize(JSON.parse(input))));
"#;
    let output = Command::new("node")
        .arg("-e")
        .arg(script)
        .arg(input_json)
        .stdin(Stdio::null())
        .output()
        .expect("node is available for RFC 8785 ECMAScript JSON.stringify vectors");
    assert!(
        output.status.success(),
        "node JCS helper failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("node emits UTF-8")
}

fn sorted_array_by_key(value: &mut Value, key: &str) {
    value
        .as_array_mut()
        .expect("field is array")
        .sort_by(|left, right| left[key].as_str().cmp(&right[key].as_str()));
}

fn object_mut(value: &mut Value) -> &mut Map<String, Value> {
    value.as_object_mut().expect("value is object")
}

fn normalize_eval_manifest(mut manifest: Value, fixture_bytes: &str) -> Value {
    let manifest_obj = object_mut(&mut manifest);
    manifest_obj.entry("metadata").or_insert_with(|| json!({}));

    let fixtures = manifest_obj
        .get_mut("fixtures")
        .expect("manifest has fixtures");
    for fixture in fixtures.as_array_mut().unwrap() {
        object_mut(fixture)
            .entry("role")
            .or_insert_with(|| json!("repository"));
    }
    sorted_array_by_key(fixtures, "id");

    let scorers = manifest_obj
        .get_mut("scorers")
        .expect("manifest has scorers");
    sorted_array_by_key(scorers, "id");

    let cases = manifest_obj.get_mut("cases").expect("manifest has cases");
    for case in cases.as_array_mut().unwrap() {
        let case_obj = object_mut(case);
        case_obj.entry("tags").or_insert_with(|| json!([]));
        case_obj.entry("measures").or_insert_with(|| json!([]));
        case_obj.entry("budgets").or_insert_with(|| json!({}));
        case_obj.entry("timeout_ms").or_insert_with(|| json!(30000));
        case_obj
            .entry("output_limit_bytes")
            .or_insert_with(|| json!(65536));
        let repetitions = 1;
        case_obj.entry("sampling").or_insert_with(|| {
            json!({
                "repetitions": repetitions,
                "warmups": 0,
                "seed": 0,
                "min_successful_samples": repetitions
            })
        });
    }
    sorted_array_by_key(cases, "case_id");

    let policy = manifest_obj
        .get_mut("comparison_policy")
        .expect("manifest has comparison_policy");
    let policy_obj = object_mut(policy);
    policy_obj
        .entry("freshness_max_age_hours")
        .or_insert_with(|| json!(168));
    policy_obj
        .entry("required_case_coverage")
        .or_insert_with(|| json!(1.0));
    policy_obj
        .entry("compatible_testbed_fields")
        .or_insert_with(|| json!(["os", "arch", "planr_version"]));
    policy_obj
        .entry("quality_non_inferiority_margin")
        .or_insert_with(|| json!(0.0));
    policy_obj
        .entry("material_improvement")
        .or_insert_with(|| json!({}));
    policy_obj
        .entry("material_regression")
        .or_insert_with(|| json!({}));
    policy_obj
        .entry("protected_dimensions")
        .or_insert_with(|| json!({}));

    let safety = manifest_obj.get_mut("safety").expect("manifest has safety");
    let safety_obj = object_mut(safety);
    safety_obj
        .entry("allow_network")
        .or_insert_with(|| json!(false));
    safety_obj
        .entry("allow_shell")
        .or_insert_with(|| json!(false));
    safety_obj
        .entry("max_concurrency")
        .or_insert_with(|| json!(1));
    safety_obj
        .entry("capture_stdout")
        .or_insert_with(|| json!("digest_and_bounded_excerpt"));
    safety_obj
        .entry("capture_stderr")
        .or_insert_with(|| json!("digest_and_bounded_excerpt"));
    safety_obj
        .entry("forbid_prompt_capture")
        .or_insert_with(|| json!(true));
    safety_obj
        .entry("forbid_source_capture")
        .or_insert_with(|| json!(true));
    safety_obj
        .entry("forbid_secret_capture")
        .or_insert_with(|| json!(true));

    let fixture_id = manifest["fixtures"][0]["id"].as_str().unwrap();
    let fixture_digest = sha256_prefixed(fixture_bytes);
    let scorer = &manifest["scorers"][0];
    let scorer_identity = format!(
        "{}@{}",
        scorer["id"].as_str().unwrap(),
        scorer["version"].as_str().unwrap()
    );
    let case_id = manifest["cases"][0]["case_id"].as_str().unwrap();

    json!({
        "schema_version": "eval.suite.snapshot.v1",
        "suite_id": manifest["suite_id"].clone(),
        "suite_version": manifest["suite_version"].clone(),
        "normalized_manifest": manifest,
        "fixture_digests": {
            fixture_id: fixture_digest
        },
        "scorer_identities": [scorer_identity],
        "case_order": [case_id]
    })
}

fn object_example_with_key<'a>(values: &'a [Value], key: &str) -> &'a Map<String, Value> {
    values
        .iter()
        .filter_map(|value| value.get("object").and_then(Value::as_object))
        .find(|object| object.contains_key(key))
        .unwrap_or_else(|| panic!("missing object example containing `{key}`"))
}

fn object_example_with_object_kind<'a>(values: &'a [Value], kind: &str) -> &'a Map<String, Value> {
    values
        .iter()
        .filter_map(|value| value.get("object").and_then(Value::as_object))
        .find(|object| object.get("object_kind").and_then(Value::as_str) == Some(kind))
        .unwrap_or_else(|| panic!("missing object example with object_kind `{kind}`"))
}

fn v1_1_vectors(values: &[Value]) -> &Map<String, Value> {
    values
        .iter()
        .find_map(|value| {
            value
                .pointer("/object/v1_1_contract_vectors")
                .and_then(Value::as_object)
        })
        .expect("contract must include executable v1.1 vectors")
}

fn treatment_field(value: &Value, field: &str) -> Option<String> {
    value[field].as_str().map(str::to_owned)
}

fn changed_treatment_fields(attempt: &Value) -> Vec<&'static str> {
    [
        "client",
        "provider",
        "runtime",
        "model",
        "effort",
        "profile_id",
        "profile_digest",
        "route_policy_digest",
    ]
    .into_iter()
    .filter(|field| {
        treatment_field(&attempt["requested_treatment"], field)
            != treatment_field(&attempt["effective_treatment"], field)
    })
    .collect()
}

fn attempt_has_reason(attempt: &Value, reason: &str) -> bool {
    attempt["reasons"]
        .as_array()
        .is_some_and(|reasons| reasons.iter().any(|value| value.as_str() == Some(reason)))
}

fn treatment_change_is_attributed(attempt: &Value, changed_fields: &[&str]) -> bool {
    if changed_fields.is_empty() {
        return true;
    }

    changed_fields.iter().all(|field| {
        let field_fallback = format!("{field}_fallback");
        let field_escalation = format!("{field}_escalation");
        attempt_has_reason(attempt, &field_fallback)
            || attempt_has_reason(attempt, &field_escalation)
            || (*field == "effort"
                && (attempt_has_reason(attempt, "effort_downgrade")
                    || attempt_has_reason(attempt, "quota_downgrade")))
    })
}

fn validate_v1_1_sample(sample: &Value) -> Result<(), &'static str> {
    let basis = sample["metering_basis"].as_str().unwrap();
    let value = &sample["value"];
    match basis {
        "unavailable" => {
            if !value.is_null() {
                return Err("unavailable_value_must_be_null");
            }
            if sample["basis_confidence"].as_str() != Some("unavailable") {
                return Err("unavailable_confidence_must_be_unavailable");
            }
        }
        "estimated" => {
            if value.is_null() {
                return Err("available_value_required");
            }
            if sample["estimate_provenance"].is_null() {
                return Err("estimate_provenance_missing");
            }
        }
        "actual_trusted" | "mixed" => {
            if value.is_null() {
                return Err("available_value_required");
            }
        }
        _ => return Err("unknown_metering_basis"),
    }
    Ok(())
}

fn unattributed_treatment_fallback_reason(attempt: &Value) -> Option<&'static str> {
    let changed_fields = changed_treatment_fields(attempt);
    if !changed_fields.is_empty() && !treatment_change_is_attributed(attempt, &changed_fields) {
        Some("treatment_fallback_unattributed")
    } else {
        None
    }
}

fn derive_efficiency(case: &Value, attempts: &[Value], samples: &[Value]) -> Value {
    let attempt_ids = case["attempt_ids"].as_array().unwrap();
    let measure = case["measure"].as_str().unwrap();
    let mut countable_attempts = 0usize;
    let mut verified_successes = 0usize;
    let mut total = 0i64;
    let mut has_total = false;
    let mut has_unavailable_sample = false;
    let mut bases = std::collections::BTreeSet::new();
    let mut reasons = Vec::new();

    for attempt_id in attempt_ids {
        let attempt_id = attempt_id.as_str().unwrap();
        let attempt = attempts
            .iter()
            .find(|attempt| attempt["attempt_id"].as_str() == Some(attempt_id))
            .unwrap_or_else(|| {
                panic!("efficiency vector references missing attempt `{attempt_id}`")
            });
        if attempt["countable"].as_bool() != Some(true) {
            continue;
        }

        countable_attempts += 1;
        if attempt["terminal_status"].as_str() == Some("verified_success") {
            verified_successes += 1;
        }

        let sample = samples.iter().find(|sample| {
            sample["attempt_id"].as_str() == Some(attempt_id)
                && sample["measure"].as_str() == Some(measure)
                && sample["warmup"].as_bool() == Some(false)
        });

        match sample {
            Some(sample) => {
                let basis = sample["metering_basis"].as_str().unwrap();
                if basis == "unavailable" {
                    has_unavailable_sample = true;
                    push_reason(&mut reasons, "samples_unavailable");
                } else {
                    bases.insert(basis.to_owned());
                }
                if let Some(value) = sample["value"].as_i64() {
                    total += value;
                    has_total = true;
                }
            }
            None => {
                has_unavailable_sample = true;
                push_reason(&mut reasons, "samples_unavailable");
            }
        }
    }

    if verified_successes == 0 {
        push_reason(&mut reasons, "zero_verified_successes");
    }
    if countable_attempts < 2 {
        push_reason(&mut reasons, "samples_below_minimum");
    }

    let total_is_available = has_total && !has_unavailable_sample;
    let total_value = if total_is_available {
        json!(total)
    } else {
        Value::Null
    };
    let per_verified_success = if verified_successes == 0 || !total_is_available {
        Value::Null
    } else {
        json!(total / verified_successes as i64)
    };
    let per_verified_success_state = if verified_successes == 0 || !total_is_available {
        "unavailable"
    } else {
        "available"
    };
    let basis = if verified_successes == 0 || has_unavailable_sample || bases.is_empty() {
        "unavailable".to_owned()
    } else if bases.len() == 1 {
        bases.into_iter().next().unwrap()
    } else {
        "mixed".to_owned()
    };

    json!({
        "countable_attempts": countable_attempts,
        "verified_successes": verified_successes,
        "total": total_value,
        "per_verified_success": per_verified_success,
        "per_verified_success_state": per_verified_success_state,
        "basis": basis,
        "reasons": reasons,
    })
}

fn push_reason(reasons: &mut Vec<&'static str>, reason: &'static str) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

fn assert_efficiency_matches_vector(case: &Value, actual: &Value) {
    assert_eq!(
        actual["countable_attempts"], case["expected_countable_attempts"],
        "countable attempts mismatch for `{}`",
        case["name"]
    );
    assert_eq!(
        actual["verified_successes"], case["expected_verified_successes"],
        "verified successes mismatch for `{}`",
        case["name"]
    );
    assert_eq!(
        actual["total"], case["expected_total"],
        "total mismatch for `{}`",
        case["name"]
    );
    assert_eq!(
        actual["per_verified_success"], case["expected_per_verified_success"],
        "per-success value mismatch for `{}`",
        case["name"]
    );
    assert_eq!(
        actual["per_verified_success_state"], case["expected_per_verified_success_state"],
        "per-success state mismatch for `{}`",
        case["name"]
    );
    assert_eq!(
        actual["basis"], case["expected_basis"],
        "metering basis mismatch for `{}`",
        case["name"]
    );
    assert_eq!(
        actual["reasons"], case["expected_reasons"],
        "reasons mismatch for `{}`",
        case["name"]
    );
}

fn command_example<'a>(values: &'a [Value], command: &str) -> &'a Value {
    values
        .iter()
        .find(|value| value["command"].as_str() == Some(command))
        .unwrap_or_else(|| panic!("missing command example `{command}`"))
}

fn command_examples<'a>(values: &'a [Value], command: &str) -> Vec<&'a Value> {
    values
        .iter()
        .filter(|value| value["command"].as_str() == Some(command))
        .collect()
}

fn assert_effective_treatment_shape(value: &Value) {
    for field in [
        "client",
        "provider",
        "runtime",
        "model",
        "effort",
        "profile_id",
        "profile_digest",
        "route_policy_digest",
        "runner_version",
        "harness_version",
        "confidence",
    ] {
        assert!(
            value.get(field).and_then(Value::as_str).is_some(),
            "effective treatment missing `{field}`: {value}"
        );
    }
}

fn assert_sample_metering_shape(value: &Value) {
    for field in [
        "attempt_id",
        "measure",
        "metering_basis",
        "basis_source",
        "basis_confidence",
    ] {
        assert!(
            value.get(field).and_then(Value::as_str).is_some(),
            "sample metering projection missing `{field}`: {value}"
        );
    }
    assert!(
        value.get("estimate_provenance").is_some(),
        "sample metering projection must include estimate_provenance: {value}"
    );
}

#[test]
fn eval_contract_examples_are_valid_json() {
    let text = contract_text();
    let fences = json_fences(&text);
    assert!(
        fences.len() >= 5,
        "contract should include canonical manifest and response examples"
    );

    for fence in fences {
        serde_json::from_str::<serde_json::Value>(&fence)
            .unwrap_or_else(|err| panic!("invalid JSON example: {err}\n{fence}"));
    }
}

#[test]
fn eval_contract_defines_structural_examples_for_every_stored_object() {
    let values = json_values();

    for (marker, required_fields) in [
        (
            "schema_version",
            &[
                "schema_version",
                "suite_id",
                "suite_version",
                "digest",
                "normalized_manifest",
                "fixture_digests",
                "scorer_identities",
                "case_order",
                "baseline_selector",
                "comparison_policy_digest",
                "created_at",
            ][..],
        ),
        (
            "run_id",
            &[
                "run_id",
                "suite_digest",
                "suite_snapshot",
                "subject",
                "runner_version",
                "planr_version",
                "status",
                "created_at",
                "started_at",
                "completed_at",
                "testbed_fingerprint",
                "source_state",
                "case_counts",
                "lineage",
            ][..],
        ),
        (
            "eval_case_result",
            &[
                "object_kind",
                "run_id",
                "case_id",
                "scorer_id",
                "scorer_version",
                "fixture_digest",
                "status",
                "repetition_count",
                "warmup_count",
                "assertions",
                "command",
                "sample_ids",
                "reasons",
            ][..],
        ),
        (
            "eval_attempt",
            &[
                "object_kind",
                "attempt_id",
                "task_id",
                "run_id",
                "case_id",
                "repetition_index",
                "seed",
                "attempt_index",
                "countable",
                "retry_of",
                "fallback_of",
                "escalation_of",
                "resume_of",
                "terminal_status",
                "reasons",
                "requested_treatment",
                "effective_treatment",
            ][..],
        ),
        (
            "sample_id",
            &[
                "sample_id",
                "run_id",
                "case_id",
                "scorer_id",
                "scorer_version",
                "attempt_id",
                "repetition_index",
                "warmup",
                "seed",
                "measure",
                "value",
                "unit",
                "source",
                "metering_basis",
                "basis_source",
                "basis_confidence",
                "estimate_provenance",
                "captured_at",
            ][..],
        ),
        (
            "comparison_id",
            &[
                "comparison_id",
                "baseline_run_id",
                "candidate_run_id",
                "policy_digest",
                "runner_version",
                "created_at",
                "verdict",
                "reasons",
                "gates",
                "effect_estimates",
                "uncertainty",
                "protected_dimensions",
                "matched_effective_treatment",
                "efficiency_summary",
                "lineage",
            ][..],
        ),
        (
            "invalidation_id",
            &[
                "invalidation_id",
                "target_kind",
                "target_id",
                "reason",
                "reason_codes",
                "created_at",
                "created_by",
                "replacement_hint",
            ][..],
        ),
        (
            "evidence_ref",
            &[
                "evidence_ref",
                "target_kind",
                "target_id",
                "planr_attachment_kind",
                "planr_attachment_id",
                "item_id",
                "created_at",
                "closure_authority",
            ][..],
        ),
    ] {
        let object = match marker {
            "eval_case_result" | "eval_attempt" => object_example_with_object_kind(&values, marker),
            _ => object_example_with_key(&values, marker),
        };
        for field in required_fields {
            assert!(
                object.contains_key(*field),
                "object example `{marker}` is missing required field `{field}`"
            );
        }
    }
}

#[test]
fn eval_contract_uses_one_envelope_for_every_eval_command() {
    let values = json_values();
    let mut commands = Vec::new();
    for value in values.iter().filter(|value| value.get("command").is_some()) {
        for field in ["ok", "command", "object", "warnings", "reasons", "error"] {
            assert!(
                value.get(field).is_some(),
                "eval command envelope missing `{field}`: {value}"
            );
        }
        assert!(
            value["warnings"].is_array(),
            "warnings must be an array: {value}"
        );
        assert!(
            value["reasons"].is_array(),
            "reasons must be an array: {value}"
        );
        commands.push(value["command"].as_str().unwrap().to_string());
    }
    commands.sort();

    for required in [
        "eval.compare",
        "eval.gate",
        "eval.invalidate",
        "eval.rescore",
        "eval.run",
        "eval.show",
        "eval.suite.check",
    ] {
        assert!(
            commands.iter().any(|command| command == required),
            "missing envelope example for `{required}`; got {commands:?}"
        );
    }
}

#[test]
fn eval_contract_v1_1_eval_surfaces_project_efficiency_evidence() {
    let values = json_values();
    let run = &command_example(&values, "eval.run")["object"];
    assert!(
        run["attempt_lineage"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "eval.run must expose attempt lineage"
    );
    assert!(
        run["sample_metering"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "eval.run must expose sample metering provenance"
    );
    assert!(
        run["efficiency_summary"].is_object(),
        "eval.run must expose derived efficiency"
    );
    assert_effective_treatment_shape(&run["attempt_lineage"][0]["effective_treatment"]);
    assert_sample_metering_shape(&run["sample_metering"][0]);
    assert!(
        run["sample_metering"]
            .as_array()
            .unwrap()
            .iter()
            .any(|sample| sample["metering_basis"] == "estimated"
                && sample["estimate_provenance"].is_object()),
        "eval.run must retain estimate provenance for estimated samples"
    );

    let show = &command_example(&values, "eval.show")["object"];
    for field in [
        "matched_effective_treatment",
        "attempt_lineage",
        "sample_metering",
        "efficiency_summary",
    ] {
        assert!(
            show.get(field).is_some(),
            "eval.show projection missing `{field}`"
        );
    }
    assert_effective_treatment_shape(&show["matched_effective_treatment"]["candidate"]);
    assert!(
        show["attempt_lineage"][0]["reasons"].is_array(),
        "eval.show attempt lineage must project reasons as an array"
    );
    assert_sample_metering_shape(&show["sample_metering"][0]);

    let compare = &command_example(&values, "eval.compare")["object"];
    let compare_cost = &compare["efficiency_summary"]["cost_per_verified_success_micros"];
    assert!(
        compare_cost["basis"].as_str().is_some(),
        "eval.compare must project efficiency metric basis"
    );
    assert!(
        compare_cost["state"].as_str().is_some(),
        "eval.compare must project efficiency metric availability state"
    );
    assert_effective_treatment_shape(&compare["matched_effective_treatment"]["baseline"]);
    assert_effective_treatment_shape(&compare["matched_effective_treatment"]["candidate"]);
}

#[test]
fn eval_contract_comparison_projections_keep_verdict_reasons_and_promotion_consistent() {
    let values = json_values();
    let comparison_id = "evcmp-20260721-000001";
    let expected_reasons = json!(["samples_unavailable"]);

    let stored = values
        .iter()
        .filter(|value| value.get("command").is_none())
        .filter_map(|value| value.get("object"))
        .find(|object| object["comparison_id"].as_str() == Some(comparison_id))
        .unwrap_or_else(|| panic!("missing stored comparison `{comparison_id}`"));
    let show = &command_examples(&values, "eval.show")
        .into_iter()
        .find(|value| {
            value["object"]["target_kind"].as_str() == Some("comparison")
                && value["object"]["target_id"].as_str() == Some(comparison_id)
        })
        .unwrap_or_else(|| panic!("missing eval.show comparison `{comparison_id}`"))["object"];
    let compare = &command_examples(&values, "eval.compare")
        .into_iter()
        .find(|value| value["object"]["comparison_id"].as_str() == Some(comparison_id))
        .unwrap_or_else(|| panic!("missing eval.compare comparison `{comparison_id}`"))["object"];
    let gate_envelope = command_examples(&values, "eval.gate")
        .into_iter()
        .find(|value| value["object"]["comparison_id"].as_str() == Some(comparison_id))
        .unwrap_or_else(|| panic!("missing eval.gate comparison `{comparison_id}`"));
    let gate = &gate_envelope["object"];

    assert_eq!(stored["verdict"], "insufficient_evidence");
    assert_eq!(show["summary"]["verdict"], stored["verdict"]);
    assert_eq!(compare["verdict"], stored["verdict"]);
    assert_eq!(gate["verdict"], stored["verdict"]);

    assert_eq!(stored["reasons"], expected_reasons);
    assert_eq!(compare["reasons"], expected_reasons);
    assert_eq!(gate["reasons"], expected_reasons);
    assert_eq!(gate_envelope["reasons"], expected_reasons);
    assert_eq!(
        show["efficiency_summary"]["reasons"], expected_reasons,
        "eval.show must expose the same blocking reason in derived efficiency"
    );

    assert_eq!(show["summary"]["first_blocking_gate"], "performance");
    assert_eq!(gate["first_blocking_gate"], "performance");
    assert_eq!(gate["promotion_allowed"], false);

    for projected in [stored, compare] {
        let performance_gate = projected["gates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|gate| gate["gate"].as_str() == Some("performance"))
            .expect("comparison projection includes performance gate");
        assert_eq!(performance_gate["status"], "blocked");
        assert_eq!(performance_gate["reasons"], expected_reasons);
        assert_eq!(
            projected["efficiency_summary"]["cost_per_verified_success_micros"]["state"],
            "unavailable"
        );
        assert_eq!(
            projected["efficiency_summary"]["cost_per_verified_success_micros"]["basis"],
            "unavailable"
        );
    }
}

#[test]
fn eval_contract_records_typed_required_fields_for_evidence_objects() {
    let text = contract_text();
    for required in [
        "### EvalCaseResult",
        "### EvalAttempt",
        "`object_kind` string: exactly `eval_case_result`",
        "`object_kind` string: exactly `eval_attempt`",
        "`repetition_count` integer, required and non-negative",
        "`attempt_id` string: references EvalAttempt and is part of sample identity.",
        "`source` string enum: `process | planr_event | scorer | metering`",
        "`metering_basis` string enum: `actual_trusted | estimated | mixed | unavailable`",
        "The database uniqueness key is `(run_id, case_id, scorer_id, scorer_version, attempt_id, measure)`.",
        "`verdict` comparison verdict",
        "`matched_effective_treatment` object",
        "`efficiency_summary` object",
        "`target_kind` string enum: `run | comparison`",
        "`planr_attachment_kind` string enum: `log | review | artifact`",
        "`closure_authority` boolean, always `false` in V1",
    ] {
        assert!(
            text.contains(required),
            "Eval Contract v1 is missing typed field contract `{required}`"
        );
    }
}

#[test]
fn eval_contract_v1_1_freezes_failure_inclusive_efficiency_semantics() {
    let values = json_values();
    let vectors = v1_1_vectors(&values);
    let attempts = vectors["attempts"].as_array().unwrap();
    let samples = vectors["samples"].as_array().unwrap();

    let retry = attempts
        .iter()
        .find(|attempt| attempt["retry_of"].as_str() == Some("evatt-candidate-000001"))
        .expect("v1.1 vector must include a retry attempt");
    assert_eq!(retry["attempt_index"], json!(1));
    assert_eq!(retry["terminal_status"], "verified_success");

    let mut sample_keys = std::collections::BTreeSet::new();
    for sample in samples {
        validate_v1_1_sample(sample)
            .unwrap_or_else(|err| panic!("valid v1.1 sample rejected with {err}: {sample}"));
        let key = format!(
            "{}|{}|{}|{}|{}|{}",
            sample["run_id"].as_str().unwrap(),
            sample["case_id"].as_str().unwrap(),
            sample["scorer_id"].as_str().unwrap(),
            sample["scorer_version"].as_str().unwrap(),
            sample["attempt_id"].as_str().unwrap(),
            sample["measure"].as_str().unwrap()
        );
        assert!(
            sample_keys.insert(key),
            "sample identity key must be unique"
        );
    }

    assert!(
        samples
            .iter()
            .any(|sample| sample["attempt_id"] == "evatt-candidate-000001"
                && sample["measure"] == "cost_micros")
    );
    assert!(
        samples
            .iter()
            .any(|sample| sample["attempt_id"] == "evatt-candidate-000001"
                && sample["measure"] == "wall_time_ms"),
        "multiple measures for one attempt must remain representable"
    );

    for case in vectors["efficiency_cases"].as_array().unwrap() {
        let actual = derive_efficiency(case, attempts, samples);
        assert_efficiency_matches_vector(case, &actual);
        if case["name"].as_str() == Some("zero-success-is-unavailable-not-zero") {
            assert!(
                actual["per_verified_success"].is_null(),
                "zero verified successes must stay unavailable/unbounded, not zero"
            );
        }
    }

    for invalid in vectors["invalid_efficiency_cases"].as_array().unwrap() {
        let actual = derive_efficiency(invalid, attempts, samples);
        let claimed = json!({
            "countable_attempts": invalid["expected_countable_attempts"],
            "verified_successes": invalid["expected_verified_successes"],
            "total": invalid["expected_total"],
            "per_verified_success": invalid["expected_per_verified_success"],
            "per_verified_success_state": invalid["expected_per_verified_success_state"],
            "basis": invalid["expected_basis"],
        });
        assert_ne!(
            actual, claimed,
            "invalid efficiency vector `{}` must be rejected with `{}`",
            invalid["name"], invalid["expected_error"]
        );
    }
}

#[test]
fn eval_contract_v1_1_rejects_unknown_to_zero_and_silent_treatment_fallback() {
    let values = json_values();
    let vectors = v1_1_vectors(&values);

    for invalid in vectors["invalid_samples"].as_array().unwrap() {
        let err = validate_v1_1_sample(&invalid["sample"])
            .expect_err("invalid v1.1 sample vector must be rejected");
        assert_eq!(err, invalid["expected_error"].as_str().unwrap());
    }

    for invalid in vectors["invalid_attempts"].as_array().unwrap() {
        let reason = unattributed_treatment_fallback_reason(&invalid["attempt"])
            .expect("invalid v1.1 attempt vector must produce fallback reason");
        assert_eq!(reason, invalid["expected_reason"].as_str().unwrap());
    }

    for valid in vectors["valid_treatment_transitions"].as_array().unwrap() {
        let reason = unattributed_treatment_fallback_reason(&valid["attempt"]);
        assert_eq!(
            reason, None,
            "valid treatment transition vector `{}` must account for every changed field",
            valid["name"]
        );
    }

    let reason_text = contract_text();
    for reason in [
        "treatment_fallback_unattributed",
        "model_fallback",
        "provider_fallback",
        "route_policy_digest_fallback",
        "effort_downgrade",
        "model_escalation",
        "route_policy_digest_escalation",
        "unavailable_value_must_be_null",
        "samples_unavailable",
        "mixed_basis_not_allowed",
    ] {
        assert!(
            reason_text.contains(reason),
            "v1.1 reason vocabulary must include `{reason}`"
        );
    }
}

#[test]
fn eval_contract_digest_vectors_are_executable() {
    let values = json_values();
    let vectors = values
        .iter()
        .find_map(|value| {
            value
                .pointer("/object/canonicalization_vectors")
                .and_then(Value::as_array)
        })
        .expect("contract must include canonicalization vectors");

    for vector in vectors {
        let expected_canonical = vector["canonical_json"].as_str().unwrap();
        let expected_sha = vector["sha256"].as_str().unwrap();
        let actual_canonical = node_jcs(
            vector["input_json"]
                .as_str()
                .expect("every digest vector must carry raw JSON input"),
        );
        assert_eq!(actual_canonical, expected_canonical);
        assert_eq!(sha256_hex(&actual_canonical), expected_sha);
    }

    let equivalence = values
        .iter()
        .find_map(|value| value.pointer("/object/toml_equivalence"))
        .expect("contract must include TOML equivalence vector");
    let fixture_bytes = equivalence["fixture_bytes_utf8"].as_str().unwrap();
    let json_manifest = equivalence["json_manifest"].clone();
    let toml_manifest = serde_json::to_value(
        equivalence["toml_manifest"]
            .as_str()
            .unwrap()
            .parse::<toml::Value>()
            .expect("TOML vector parses"),
    )
    .expect("TOML vector converts to JSON");

    let json_snapshot = normalize_eval_manifest(json_manifest, fixture_bytes);
    let toml_snapshot = normalize_eval_manifest(toml_manifest, fixture_bytes);
    let json_canonical = node_jcs(&serde_json::to_string(&json_snapshot).unwrap());
    let toml_canonical = node_jcs(&serde_json::to_string(&toml_snapshot).unwrap());
    assert!(
        json_canonical.contains("\"required_case_coverage\":1"),
        "suite-equivalence digest must use JCS number serialization: {json_canonical}"
    );
    assert!(
        !json_canonical.contains("\"required_case_coverage\":1.0"),
        "suite-equivalence digest must not hash non-JCS serde_json float bytes: {json_canonical}"
    );
    let json_digest = sha256_prefixed(&json_canonical);
    let toml_digest = sha256_prefixed(&toml_canonical);

    assert_eq!(
        json_snapshot, toml_snapshot,
        "JSON and TOML vectors must normalize to one snapshot"
    );
    assert_eq!(
        json_digest,
        equivalence["json_manifest_digest"].as_str().unwrap()
    );
    assert_eq!(
        toml_digest,
        equivalence["toml_manifest_digest"].as_str().unwrap()
    );
}

#[test]
fn eval_contract_freezes_statuses_verdicts_and_reason_codes() {
    let text = contract_text();
    for required in [
        "started",
        "success",
        "error",
        "inconclusive",
        "invalidated",
        "pass",
        "fail",
        "skipped",
        "improved",
        "regressed",
        "no_material_difference",
        "insufficient_evidence",
        "manifest_invalid",
        "unsafe_path",
        "symlink_escape",
        "suite_digest_mismatch",
        "testbed_incompatible",
        "evidence_stale",
        "coverage_below_minimum",
        "samples_below_minimum",
        "variance_too_high",
        "correctness_regressed",
        "candidate_improved",
    ] {
        assert!(
            text.contains(required),
            "Eval Contract v1 is missing required vocabulary `{required}`"
        );
    }
}

#[test]
fn eval_contract_documents_deterministic_normalization_and_ownership_boundaries() {
    let text = contract_text();
    for required in [
        "Repository files own authored suite manifests",
        "SQLite owns immutable suite snapshots",
        "Existing Planr map state owns item status",
        "Sort maps by key",
        "fixtures by `id`",
        "cases by `case_id`",
        "Hash fixture bytes with `sha256`",
        "Reject absolute paths",
        "symlink escapes",
        "Execute only explicit argv arrays",
        "must not add item status columns",
        "eval verdict can be attached as evidence, but it cannot close or reopen work by itself",
    ] {
        assert!(
            text.contains(required),
            "Eval Contract v1 is missing boundary or normalization rule `{required}`"
        );
    }
}
