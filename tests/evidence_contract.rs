use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use serde_json::json;
use sha2::{Digest, Sha256};

const CONTRACT_PATH: &str = "docs/contracts/EVIDENCE_CONTRACT_V1.md";
const FIXTURE_ROOT: &str = "docs/contracts/fixtures/evidence/v1";
const SCHEMA_PATH: &str =
    "docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap_or_else(|err| {
        panic!("{} must be readable: {err}", path.display());
    }))
    .unwrap_or_else(|err| panic!("{} must parse as JSON: {err}", path.display()))
}

fn evidence_schema_validator() -> jsonschema::Validator {
    let schema = read_json(&root().join(SCHEMA_PATH));
    jsonschema::draft202012::options()
        .build(&schema)
        .expect("Evidence contract schema must compile as Draft 2020-12")
}

fn schema_errors(validator: &jsonschema::Validator, value: &Value) -> Vec<String> {
    validator
        .iter_errors(value)
        .map(|error| format!("{}: {}", error.instance_path(), error))
        .collect()
}

fn fixture_files(kind: &str) -> Vec<PathBuf> {
    let dir = root().join(FIXTURE_ROOT).join(kind);
    let mut files = fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn object(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value.as_object()
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    object(value).and_then(|object| object.get(name))
}

fn assign_nested(value: &mut Value, path: &[&str], replacement: Value) {
    let Some((first, rest)) = path.split_first() else {
        *value = replacement;
        return;
    };
    if let Ok(index) = first.parse::<usize>() {
        if let Value::Array(values) = value {
            assign_nested(&mut values[index], rest, replacement);
            return;
        }
    }
    assign_nested(&mut value[*first], rest, replacement);
}

fn has_non_empty_string(value: &Value, name: &str) -> bool {
    field(value, name)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
}

fn has_non_empty_array(value: &Value, name: &str) -> bool {
    field(value, name)
        .and_then(Value::as_array)
        .is_some_and(|value| !value.is_empty())
}

fn has_object(value: &Value, name: &str) -> bool {
    field(value, name).and_then(Value::as_object).is_some()
}

fn required(value: &Value, errors: &mut Vec<String>, fields: &[&str]) {
    for field_name in fields {
        if field(value, field_name).is_none() {
            errors.push(format!("missing required field {field_name}"));
        }
    }
}

fn require_non_empty_array(value: &Value, errors: &mut Vec<String>, fields: &[&str]) {
    for field_name in fields {
        if !has_non_empty_array(value, field_name) {
            errors.push(format!("{field_name} must be a non-empty array"));
        }
    }
}

fn require_digest(value: &Value, errors: &mut Vec<String>, fields: &[&str]) {
    for field_name in fields {
        let valid = field(value, field_name)
            .and_then(Value::as_str)
            .is_some_and(is_sha256_digest);
        if !valid {
            errors.push(format!("{field_name} must be sha256:<64 lowercase hex>"));
        }
    }
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn is_namespaced(value: &str) -> bool {
    value.starts_with("planr.")
        || value.starts_with("mcp.")
        || value.starts_with("host.")
        || value.starts_with("project.")
        || value.split('.').count() >= 3
}

fn validate_observation_requirement(value: &Value, errors: &mut Vec<String>, label: &str) {
    required(
        value,
        errors,
        &[
            "id",
            "type",
            "subject",
            "expected",
            "target",
            "environment",
            "runtime_target",
        ],
    );
    if !field(value, "type")
        .and_then(Value::as_str)
        .is_some_and(is_namespaced)
    {
        errors.push(format!("{label}.type must use an allowed namespace"));
    }
    for object_field in ["expected", "target", "environment", "runtime_target"] {
        if !has_object(value, object_field) {
            errors.push(format!("{label}.{object_field} must be an object"));
        }
    }
}

fn validate_process_execution_contract(value: &Value, errors: &mut Vec<String>, label: &str) {
    required(
        value,
        errors,
        &[
            "kind",
            "executable",
            "args",
            "timeout_ms",
            "stdout_limit_bytes",
            "stderr_limit_bytes",
            "payload_schema",
        ],
    );
    if field(value, "kind") != Some(&Value::String("process".to_string())) {
        errors.push(format!("{label}.kind must be process"));
    }
    for limit in ["timeout_ms", "stdout_limit_bytes", "stderr_limit_bytes"] {
        if field(value, limit)
            .and_then(Value::as_i64)
            .is_none_or(|value| value < 1)
        {
            errors.push(format!("{label}.{limit} must be positive"));
        }
    }
}

fn validate_contract(value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if field(value, "schema_version") != Some(&Value::String("evidence.contract.v1".to_string())) {
        errors.push("schema_version must be evidence.contract.v1".to_string());
    }
    match contract_kind(value) {
        Some("ProofObligation") => validate_proof_obligation(value, &mut errors),
        Some("VerificationCapabilityManifest") => validate_capability_manifest(value, &mut errors),
        Some("VerificationCapabilityInstance") => validate_capability_instance(value, &mut errors),
        Some("EvidenceAttempt") => validate_attempt(value, &mut errors),
        Some("UntrustedEvidenceProposal") => validate_untrusted_proposal(value, &mut errors),
        Some("EvidenceReceipt") => validate_receipt(value, &mut errors),
        Some("CoverageVerdict") => validate_coverage_verdict(value, &mut errors),
        Some("EvidencePolicy") => validate_policy(value, &mut errors),
        Some("ProofPreset") => validate_preset(value, &mut errors),
        Some("EvidenceWaiver") => validate_waiver(value, &mut errors),
        _ => errors.push("fixture does not match one Evidence v1 object kind".to_string()),
    }
    errors
}

fn contract_kind(value: &Value) -> Option<&'static str> {
    if field(value, "receipt_status").is_some() {
        Some("EvidenceReceipt")
    } else if field(value, "source_kind").is_some() {
        Some("UntrustedEvidenceProposal")
    } else if field(value, "adapter_kind").is_some() {
        Some("VerificationCapabilityManifest")
    } else if field(value, "manifest_id").is_some() && field(value, "probe_result").is_some() {
        Some("VerificationCapabilityInstance")
    } else if field(value, "resolved_command").is_some() {
        Some("EvidenceAttempt")
    } else if field(value, "observation_coverage").is_some() {
        Some("CoverageVerdict")
    } else if field(value, "defaults").is_some() {
        Some("EvidencePolicy")
    } else if field(value, "namespace").is_some() {
        Some("ProofPreset")
    } else if field(value, "approval_ref").is_some() {
        Some("EvidenceWaiver")
    } else if field(value, "criterion_id").is_some() && field(value, "observations").is_some() {
        Some("ProofObligation")
    } else {
        None
    }
}

fn validate_proof_obligation(value: &Value, errors: &mut Vec<String>) {
    required(
        value,
        errors,
        &[
            "id",
            "criterion_id",
            "plan_id",
            "title",
            "binding",
            "observations",
            "fixture_policy",
            "freshness_policy",
            "assurance_policy",
            "policy_digest",
            "config_digest",
            "created_at",
        ],
    );
    require_non_empty_array(value, errors, &["observations"]);
    require_digest(value, errors, &["policy_digest", "config_digest"]);
    for observation in field(value, "observations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        validate_observation_requirement(observation, errors, "ProofObligation.observations[]");
    }
}

fn validate_capability_manifest(value: &Value, errors: &mut Vec<String>) {
    required(
        value,
        errors,
        &[
            "id",
            "version",
            "adapter_kind",
            "adapter_digest",
            "supported_surfaces",
            "supported_observations",
            "supported_interactions",
            "supported_artifacts",
            "runtime_targets",
            "provenance_path",
            "permissions",
            "costs",
            "determinism",
            "repeatability",
            "independence",
            "blind_spots",
            "availability_probe",
        ],
    );
    require_digest(value, errors, &["adapter_digest"]);
    require_non_empty_array(
        value,
        errors,
        &[
            "supported_surfaces",
            "supported_observations",
            "supported_interactions",
            "supported_artifacts",
            "runtime_targets",
            "blind_spots",
        ],
    );
    let execution = field(value, "availability_probe").and_then(|probe| field(probe, "execution"));
    match execution {
        Some(execution) => validate_process_execution_contract(
            execution,
            errors,
            "VerificationCapabilityManifest.availability_probe.execution",
        ),
        None => errors.push("availability_probe.execution is required".to_string()),
    }
}

fn validate_capability_instance(value: &Value, errors: &mut Vec<String>) {
    required(
        value,
        errors,
        &[
            "id",
            "manifest_id",
            "manifest_digest",
            "host",
            "surface",
            "host_version",
            "adapter_version",
            "environment",
            "permissions",
            "availability",
            "probe_result",
            "observed_payload_contract",
            "limitations",
            "captured_at",
        ],
    );
    if field(value, "probe_execution_id").is_some() {
        errors.push("probe_execution_id must live only under probe_result".to_string());
    }
    require_digest(value, errors, &["manifest_digest"]);
    if field(value, "availability")
        .and_then(|v| field(v, "status"))
        .and_then(Value::as_str)
        == Some("available")
    {
        let probe = field(value, "probe_result");
        if probe
            .and_then(|v| field(v, "outcome"))
            .and_then(Value::as_str)
            != Some("passed")
        {
            errors.push("available capability instances require a passed probe_result".to_string());
        }
        if !probe.is_some_and(|v| has_non_empty_string(v, "probe_execution_id")) {
            errors.push(
                "available capability instances require probe_result.probe_execution_id"
                    .to_string(),
            );
        }
        if !probe.is_some_and(|v| has_non_empty_array(v, "checks")) {
            errors.push(
                "available capability instances require non-empty probe_result.checks".to_string(),
            );
        }
    }
    require_non_empty_array(value, errors, &["limitations"]);
}

fn validate_attempt(value: &Value, errors: &mut Vec<String>) {
    required(
        value,
        errors,
        &[
            "id",
            "criterion_id",
            "obligation_id",
            "capability_instance_id",
            "started_at",
            "ended_at",
            "status",
            "resolved_command",
            "exit",
            "retry_lineage",
            "stdout_digest",
            "stderr_digest",
            "raw_result",
            "artifacts",
            "output_bounds",
        ],
    );
    require_digest(value, errors, &["stdout_digest", "stderr_digest"]);
    let status = field(value, "status").and_then(Value::as_str);
    let exit = field(value, "exit");
    let exit_code = exit.and_then(|exit| field(exit, "exit_code"));
    let signal = exit.and_then(|exit| field(exit, "signal"));
    let error = exit.and_then(|exit| field(exit, "error"));
    match status {
        Some("passed")
            if exit_code != Some(&Value::from(0))
                || signal != Some(&Value::Null)
                || error != Some(&Value::Null) =>
        {
            errors.push("passed attempts require clean exit code 0".to_string());
        }
        Some("failed")
            if exit_code
                .and_then(Value::as_i64)
                .is_none_or(|code| code <= 0)
                || signal != Some(&Value::Null) =>
        {
            errors.push("failed attempts require non-zero exit code and no signal".to_string());
        }
        Some("timed_out" | "skipped" | "unavailable")
            if exit_code != Some(&Value::Null)
                || signal != Some(&Value::Null)
                || error.and_then(Value::as_str) != status =>
        {
            errors.push(
                "terminal non-exit attempts require null code/signal and matching error"
                    .to_string(),
            );
        }
        Some("aborted")
            if exit_code != Some(&Value::Null)
                || signal
                    .and_then(Value::as_str)
                    .is_none_or(|value| value.is_empty())
                || error.and_then(Value::as_str) != Some("aborted") =>
        {
            errors.push("aborted attempts require an abort signal and matching error".to_string());
        }
        Some("inconclusive")
            if exit_code != Some(&Value::from(0))
                || signal != Some(&Value::Null)
                || error.and_then(Value::as_str) != Some("inconclusive") =>
        {
            errors.push(
                "inconclusive attempts require clean exit plus inconclusive marker".to_string(),
            );
        }
        Some(_) => {}
        None => errors.push("attempt status is required".to_string()),
    }
}

fn validate_untrusted_proposal(value: &Value, errors: &mut Vec<String>) {
    required(
        value,
        errors,
        &[
            "id",
            "source_kind",
            "submitted_at",
            "claims",
            "artifact_refs",
            "producer_metadata",
        ],
    );
    for forbidden in [
        "provenance",
        "execution_id",
        "receipt_digest",
        "closure_authority",
    ] {
        if field(value, forbidden).is_some() {
            errors.push(format!("untrusted proposals cannot include {forbidden}"));
        }
    }
}

fn validate_receipt(value: &Value, errors: &mut Vec<String>) {
    required(
        value,
        errors,
        &[
            "id",
            "criterion_id",
            "obligation_id",
            "receipt_status",
            "source",
            "target",
            "environment",
            "vantage_point",
            "capability",
            "provenance",
            "observations",
            "attempt_ids",
            "retry_history",
            "artifacts",
            "raw_result",
            "config_digest",
            "fixture_disclosure",
            "permissions",
            "sandbox",
            "proof_gaps",
            "started_at",
            "ended_at",
            "receipt_digest",
        ],
    );
    require_non_empty_array(value, errors, &["observations", "attempt_ids"]);
    require_digest(value, errors, &["config_digest", "receipt_digest"]);
    for object_field in [
        "source",
        "target",
        "environment",
        "vantage_point",
        "capability",
        "fixture_disclosure",
        "permissions",
        "sandbox",
    ] {
        if !has_object(value, object_field) {
            errors.push(format!("EvidenceReceipt.{object_field} must be an object"));
        }
    }
    let source = field(value, "source");
    if !source.is_some_and(|source| {
        has_non_empty_string(source, "revision")
            && field(source, "tree_digest")
                .and_then(Value::as_str)
                .is_some_and(is_sha256_digest)
            && field(source, "dirty").and_then(Value::as_bool).is_some()
    }) {
        errors.push(
            "EvidenceReceipt.source must bind revision, tree digest, and dirty state".to_string(),
        );
    }
    let target = field(value, "target");
    if !target.is_some_and(|target| {
        has_non_empty_string(target, "kind")
            && (has_non_empty_string(target, "uri")
                || has_non_empty_string(target, "deployment_id")
                || field(target, "digest")
                    .and_then(Value::as_str)
                    .is_some_and(is_sha256_digest))
    }) {
        errors.push("EvidenceReceipt.target must bind a target identity".to_string());
    }
    let environment = field(value, "environment");
    if !environment.is_some_and(|environment| {
        has_non_empty_string(environment, "kind")
            && has_non_empty_string(environment, "id")
            && field(environment, "digest")
                .and_then(Value::as_str)
                .is_some_and(is_sha256_digest)
    }) {
        errors.push("EvidenceReceipt.environment must bind kind, id, and digest".to_string());
    }
    let vantage_point = field(value, "vantage_point");
    if !vantage_point.is_some_and(|vantage| {
        has_non_empty_string(vantage, "kind") && has_non_empty_string(vantage, "identity")
    }) {
        errors.push("EvidenceReceipt.vantage_point must bind kind and identity".to_string());
    }
    let capability = field(value, "capability");
    if !capability.is_some_and(|capability| {
        has_non_empty_string(capability, "manifest_id")
            && has_non_empty_string(capability, "instance_id")
            && field(capability, "manifest_digest")
                .and_then(Value::as_str)
                .is_some_and(is_sha256_digest)
            && field(capability, "instance_digest")
                .and_then(Value::as_str)
                .is_some_and(is_sha256_digest)
    }) {
        errors
            .push("EvidenceReceipt.capability must bind manifest and instance digests".to_string());
    }
    let fixture = field(value, "fixture_disclosure");
    if !fixture.is_some_and(|fixture| {
        field(fixture, "fixtures_used")
            .and_then(Value::as_bool)
            .is_some()
            && field(fixture, "mocks_used")
                .and_then(Value::as_bool)
                .is_some()
    }) {
        errors.push(
            "EvidenceReceipt.fixture_disclosure must disclose fixtures and mocks".to_string(),
        );
    }
    let permissions = field(value, "permissions");
    if !permissions.is_some_and(|permissions| {
        has_non_empty_string(permissions, "network")
            && has_non_empty_string(permissions, "filesystem")
    }) {
        errors
            .push("EvidenceReceipt.permissions must bind network and filesystem state".to_string());
    }
    let sandbox = field(value, "sandbox");
    if !sandbox.is_some_and(|sandbox| {
        has_non_empty_string(sandbox, "mode")
            && field(sandbox, "limits").is_some_and(|limits| {
                field(limits, "timeout_ms")
                    .and_then(Value::as_i64)
                    .is_some_and(|value| value > 0)
                    && field(limits, "stdout_bytes")
                        .and_then(Value::as_i64)
                        .is_some_and(|value| value >= 0)
                    && field(limits, "stderr_bytes")
                        .and_then(Value::as_i64)
                        .is_some_and(|value| value >= 0)
            })
    }) {
        errors.push("EvidenceReceipt.sandbox must bind mode and limits".to_string());
    }
    let provenance = field(value, "provenance");
    if provenance
        .and_then(|value| field(value, "assigned_by"))
        .and_then(Value::as_str)
        != Some("planr")
        || !provenance.is_some_and(|value| has_non_empty_string(value, "execution_id"))
    {
        errors.push(
            "trusted receipt provenance must be Planr-assigned and execution-bound".to_string(),
        );
    }
}

fn validate_coverage_verdict(value: &Value, errors: &mut Vec<String>) {
    required(
        value,
        errors,
        &[
            "id",
            "scope",
            "evaluated_at",
            "status",
            "observation_coverage",
            "validation_details",
            "suggested_next_action",
            "actionable_now",
        ],
    );
    require_non_empty_array(value, errors, &["observation_coverage"]);
    let Some(status) = field(value, "status").and_then(Value::as_str) else {
        errors.push("coverage status is required".to_string());
        return;
    };
    let allowed = match status {
        "satisfied" => &["covered"][..],
        "unsatisfied" => &["missing", "unsatisfied", "covered"],
        "blocked" => &["blocked", "covered"],
        "inconclusive" => &["inconclusive", "covered"],
        "waived" => &["waived", "covered"],
        "stale" => &["stale", "covered"],
        _ => {
            errors.push(format!("unknown coverage status {status}"));
            return;
        }
    };
    let mut has_matching_gap = false;
    for coverage in field(value, "observation_coverage")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let coverage_status = field(coverage, "status")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !allowed.contains(&coverage_status) {
            errors.push(format!(
                "{status} verdict cannot contain {coverage_status} coverage"
            ));
        }
        if coverage_status == "covered" {
            if !has_non_empty_array(coverage, "covering_receipt_ids") {
                errors.push("covered observations require covering_receipt_ids".to_string());
            }
            continue;
        }
        has_matching_gap = true;
        validate_gap_reason(status, coverage, errors);
    }
    if status != "satisfied" && !has_matching_gap {
        errors.push(format!(
            "{status} verdict requires at least one matching gap"
        ));
    }
}

fn validate_gap_reason(verdict_status: &str, coverage: &Value, errors: &mut Vec<String>) {
    let reason = field(coverage, "gap_reason")
        .and_then(Value::as_str)
        .unwrap_or("");
    let allowed: &[&str] = match verdict_status {
        "unsatisfied" => &[
            "missing_observation",
            "product_failed",
            "verifier_failed",
            "target_mismatch",
            "schema_mismatch",
            "manifest_mismatch",
            "untrusted_provenance",
            "fixture_disallowed",
            "mock_disallowed",
            "insufficient_assurance",
            "waiver_missing",
            "waiver_expired",
            "unknown_observation_type",
            "unsupported_runtime_target",
        ],
        "blocked" => &[
            "missing_capability",
            "permission_denied",
            "sandbox_blocked",
            "environment_unavailable",
            "external_dependency_unavailable",
            "unsupported_runtime_target",
        ],
        "inconclusive" => &[
            "timed_out",
            "aborted",
            "inconclusive_result",
            "verifier_failed",
        ],
        "waived" => &[
            "missing_observation",
            "missing_capability",
            "permission_denied",
            "sandbox_blocked",
            "environment_unavailable",
            "external_dependency_unavailable",
            "product_failed",
            "verifier_failed",
            "target_mismatch",
            "schema_mismatch",
            "manifest_mismatch",
            "untrusted_provenance",
            "fixture_disallowed",
            "mock_disallowed",
            "insufficient_assurance",
            "unknown_observation_type",
            "unsupported_runtime_target",
        ],
        "stale" => &[
            "stale_source",
            "stale_target",
            "stale_environment",
            "stale_policy",
            "stale_adapter_schema",
            "stale_configuration",
        ],
        _ => &[],
    };
    if !allowed.contains(&reason) {
        errors.push(format!(
            "{verdict_status} verdict cannot use gap reason {reason}"
        ));
    }
    if verdict_status == "waived" && !has_non_empty_string(coverage, "waiver_id") {
        errors.push("waived coverage requires waiver_id".to_string());
    }
}

fn validate_policy(value: &Value, errors: &mut Vec<String>) {
    required(
        value,
        errors,
        &[
            "id",
            "policy_digest",
            "defaults",
            "named_presets",
            "observation_schema_registrations",
            "adapter_registrations",
            "extension_namespaces",
            "trust_policy",
            "freshness_policy",
            "fixture_policy",
            "completion_policy",
            "layering_policy",
        ],
    );
    require_digest(value, errors, &["policy_digest"]);
    require_non_empty_array(
        value,
        errors,
        &[
            "named_presets",
            "observation_schema_registrations",
            "adapter_registrations",
            "extension_namespaces",
        ],
    );
    if let Some(default_preset_id) = field(value, "defaults")
        .and_then(|defaults| field(defaults, "preset_id"))
        .and_then(Value::as_str)
    {
        let preset_exists = field(value, "named_presets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|preset| field(preset, "id").and_then(Value::as_str) == Some(default_preset_id));
        if !preset_exists {
            errors
                .push("policy defaults.preset_id must reference a named_presets entry".to_string());
        }
    }
    for adapter in field(value, "adapter_registrations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(execution) = field(adapter, "execution_contract") {
            validate_process_execution_contract(
                execution,
                errors,
                "EvidencePolicy.adapter_registrations[].execution_contract",
            );
        } else {
            errors.push("adapter registrations require execution_contract".to_string());
        }
    }
    if field(value, "layering_policy")
        .and_then(|layering| field(layering, "weakening_requires_waiver"))
        != Some(&Value::Bool(true))
    {
        errors.push("policy layering must require waiver for weakening".to_string());
    }
}

fn validate_preset(value: &Value, errors: &mut Vec<String>) {
    required(value, errors, &["id", "namespace", "observations"]);
    if !field(value, "namespace")
        .and_then(Value::as_str)
        .is_some_and(is_namespaced)
    {
        errors.push("preset namespace must be namespaced".to_string());
    }
    require_non_empty_array(value, errors, &["observations"]);
    for observation in field(value, "observations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        validate_observation_requirement(observation, errors, "ProofPreset.observations[]");
    }
}

fn validate_waiver(value: &Value, errors: &mut Vec<String>) {
    required(
        value,
        errors,
        &[
            "id",
            "scope",
            "observation_ids",
            "source",
            "target",
            "reason",
            "created_by",
            "created_at",
            "expires_at",
            "approval_ref",
            "audit_trail",
        ],
    );
    require_non_empty_array(value, errors, &["observation_ids", "audit_trail"]);
    for object_field in ["source", "target"] {
        if !has_object(value, object_field) {
            errors.push(format!("EvidenceWaiver.{object_field} must be an object"));
        }
    }
}

#[test]
fn evidence_contract_examples_match_schema_and_semantics() {
    let validator = evidence_schema_validator();
    for path in fixture_files("examples") {
        let value = read_json(&path);
        let schema_errors = schema_errors(&validator, &value);
        assert!(
            schema_errors.is_empty(),
            "{} failed JSON Schema validation: {schema_errors:?}",
            path.display()
        );
        let errors = validate_contract(&value);
        assert!(
            errors.is_empty(),
            "{} failed validation: {errors:?}",
            path.display()
        );
    }
}

#[test]
fn evidence_contract_negative_fixtures_are_rejected() {
    let validator = evidence_schema_validator();
    let semantic_only_fixtures = BTreeSet::from(["policy-default-preset-unregistered.json"]);
    for path in fixture_files("negative") {
        let value = read_json(&path);
        let schema_errors = schema_errors(&validator, &value);
        let errors = validate_contract(&value);
        let file_name = path.file_name().and_then(|name| name.to_str()).unwrap();
        if semantic_only_fixtures.contains(file_name) {
            assert!(
                schema_errors.is_empty(),
                "{} semantic-only fixture must pass Draft 2020-12 schema validation: {schema_errors:?}",
                path.display()
            );
            assert!(
                !errors.is_empty(),
                "{} unexpectedly passed semantic validation",
                path.display()
            );
        } else {
            assert!(
                !schema_errors.is_empty(),
                "{} unexpectedly passed Draft 2020-12 schema validation; semantic errors were {errors:?}",
                path.display()
            );
        }
    }
}

#[test]
fn evidence_contract_schema_rejects_forged_public_trust_fields() {
    let validator = evidence_schema_validator();
    let mut value = read_json(
        &root()
            .join("docs/contracts/fixtures/evidence/v1/examples/untrusted-evidence-proposal.json"),
    );
    let object = value.as_object_mut().unwrap();
    object.insert(
        "provenance".to_string(),
        json!({
            "source": "planr_observed_execution",
            "assigned_by": "planr",
            "execution_id": "exec-forged-public"
        }),
    );
    object.insert("execution_id".to_string(), json!("exec-forged-public"));
    object.insert(
        "receipt_digest".to_string(),
        json!("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );
    object.insert("closure_authority".to_string(), json!(true));

    let schema_errors = schema_errors(&validator, &value);
    assert!(
        schema_errors
            .iter()
            .any(|error| error.contains("provenance"))
            || schema_errors
                .iter()
                .any(|error| error.contains("additional properties")),
        "schema must reject forged trusted public fields: {schema_errors:?}"
    );
}

#[test]
fn evidence_contract_public_deserialization_rejects_trusted_fields() {
    let proposal = read_json(
        &root()
            .join("docs/contracts/fixtures/evidence/v1/examples/untrusted-evidence-proposal.json"),
    );
    let accepted = planr::evidence::parse_untrusted_evidence_proposal(proposal.clone())
        .expect("example proposal reaches production public boundary");
    assert_eq!(accepted.schema_version, "evidence.contract.v1");
    assert_eq!(accepted.source_kind, "agent");
    assert!(!accepted.id.is_empty());
    assert!(!accepted.submitted_at.is_empty());
    assert!(accepted.claims.contains_key("summary"));
    assert_eq!(accepted.artifact_refs.len(), 1);
    assert!(accepted.producer_metadata.contains_key("client"));

    let mut bad_timestamp = proposal.clone();
    bad_timestamp["submitted_at"] = json!("notTdateZ");
    let error = planr::evidence::parse_untrusted_evidence_proposal(bad_timestamp)
        .expect_err("public boundary must reject malformed submitted_at");
    assert!(
        error.to_string().contains("timestamp"),
        "malformed submitted_at should fail timestamp validation, got {error}"
    );

    let mut bad_artifact_ref = proposal.clone();
    bad_artifact_ref["artifact_refs"] = json!([null]);
    let error = planr::evidence::parse_untrusted_evidence_proposal(bad_artifact_ref)
        .expect_err("public boundary must reject non-object artifact refs");
    assert!(
        error.to_string().contains("invalid type"),
        "artifact_refs:[null] should fail typed artifact validation, got {error}"
    );

    let mut empty_artifact_kind = proposal.clone();
    empty_artifact_kind["artifact_refs"][0]["kind"] = json!("");
    let error = planr::evidence::parse_untrusted_evidence_proposal(empty_artifact_kind)
        .expect_err("public boundary must reject schema-invalid artifact refs");
    assert!(
        error.to_string().contains("artifact_refs[].kind"),
        "empty artifact kind should fail schema-parity validation, got {error}"
    );

    let mut null_artifact_uri = proposal.clone();
    null_artifact_uri["artifact_refs"][0]["uri"] = Value::Null;
    let error = planr::evidence::parse_untrusted_evidence_proposal(null_artifact_uri)
        .expect_err("public boundary must reject explicit null artifact uri");
    assert!(
        error.to_string().contains("optional string field"),
        "artifact_refs[].uri:null should fail schema-parity validation, got {error}"
    );

    let mut hostile = proposal;
    let object = hostile.as_object_mut().unwrap();
    object.insert(
        "provenance".to_string(),
        json!({
            "source": "planr_observed_execution",
            "assigned_by": "planr",
            "execution_id": "exec-forged-public"
        }),
    );
    object.insert("execution_id".to_string(), json!("exec-forged-public"));
    object.insert(
        "receipt_digest".to_string(),
        json!("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );
    object.insert("closure_authority".to_string(), json!(true));

    let error = planr::evidence::parse_untrusted_evidence_proposal(hostile)
        .expect_err("public JSON must not deserialize trusted Evidence provenance");
    let message = error.to_string();
    assert!(
        message.contains("unknown field")
            && (message.contains("provenance")
                || message.contains("execution_id")
                || message.contains("receipt_digest")
                || message.contains("closure_authority")),
        "hostile trusted fields must be rejected at the public boundary, got {message}"
    );
}

#[test]
fn evidence_contract_direct_untrusted_deserialization_rejects_schema_invalid_values() {
    let proposal = read_json(
        &root()
            .join("docs/contracts/fixtures/evidence/v1/examples/untrusted-evidence-proposal.json"),
    );

    let accepted: planr::evidence::UntrustedEvidenceProposal =
        serde_json::from_value(proposal.clone())
            .expect("direct public deserialization accepts schema-valid proposal");
    assert_eq!(accepted.id, "uep-agent-health-claim-v1");

    for (label, path, replacement) in [
        ("submitted_at", &["submitted_at"][..], json!("notTdateZ")),
        ("id", &["id"][..], json!("")),
        ("source_kind", &["source_kind"][..], json!("bogus")),
        (
            "artifact_refs.id",
            &["artifact_refs", "0", "id"][..],
            json!(""),
        ),
        (
            "artifact_refs.kind",
            &["artifact_refs", "0", "kind"][..],
            json!(""),
        ),
        (
            "artifact_refs.digest",
            &["artifact_refs", "0", "digest"][..],
            json!("sha256:BAD"),
        ),
        (
            "artifact_refs.uri",
            &["artifact_refs", "0", "uri"][..],
            json!(""),
        ),
    ] {
        let mut invalid = proposal.clone();
        assign_nested(&mut invalid, path, replacement);
        assert!(
            serde_json::from_value::<planr::evidence::UntrustedEvidenceProposal>(invalid).is_err(),
            "{label} should be rejected by direct public deserialization"
        );
    }
}

#[test]
fn evidence_contract_public_api_imports_only_untrusted_boundary() {
    let temp = tempfile::tempdir().expect("temp compile probe dir");
    let crate_dir = temp.path();
    fs::create_dir(crate_dir.join("src")).unwrap();
    fs::write(
        crate_dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "planr-untrusted-evidence-compile-probe"
version = "0.0.0"
edition = "2024"

[dependencies]
planr = {{ path = "{}" }}
serde_json = "1"
"#,
            root().display()
        ),
    )
    .unwrap();
    fs::write(
        crate_dir.join("src/main.rs"),
        r#"use planr::evidence::{parse_untrusted_evidence_proposal, UntrustedEvidenceProposal};

fn main() {
    let value = serde_json::json!({
        "id": "proposal-public",
        "schema_version": "evidence.contract.v1",
        "source_kind": "agent",
        "submitted_at": "2026-07-28T12:00:00Z",
        "claims": {"summary": "public input"},
        "artifact_refs": [],
        "producer_metadata": {"client": "compile-probe"}
    });
    let proposal: UntrustedEvidenceProposal =
        parse_untrusted_evidence_proposal(value).expect("public API parses untrusted proposal");
    assert_eq!(proposal.id, "proposal-public");
}
"#,
    )
    .unwrap();

    let output = Command::new("cargo")
        .arg("check")
        .arg("--quiet")
        .current_dir(crate_dir)
        .env("CARGO_TARGET_DIR", crate_dir.join("target"))
        .output()
        .expect("cargo check can run compile probe");
    assert!(
        output.status.success(),
        "untrusted Evidence public API must compile\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn evidence_contract_public_api_cannot_import_trusted_types() {
    let temp = tempfile::tempdir().expect("temp compile probe dir");
    let crate_dir = temp.path();
    fs::create_dir(crate_dir.join("src")).unwrap();
    fs::write(
        crate_dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "planr-trusted-evidence-compile-probe"
version = "0.0.0"
edition = "2024"

[dependencies]
planr = {{ path = "{}" }}
"#,
            root().display()
        ),
    )
    .unwrap();
    fs::write(
        crate_dir.join("src/main.rs"),
        r#"use planr::evidence::{EvidenceReceipt, TrustedProvenance};

fn main() {
    let _ = std::any::type_name::<EvidenceReceipt>();
    let _ = std::any::type_name::<TrustedProvenance>();
}
"#,
    )
    .unwrap();

    let output = Command::new("cargo")
        .arg("check")
        .arg("--quiet")
        .current_dir(crate_dir)
        .env("CARGO_TARGET_DIR", crate_dir.join("target"))
        .output()
        .expect("cargo check can run compile probe");
    assert!(
        !output.status.success(),
        "trusted Evidence types unexpectedly compiled through public API"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("no lib target") && stderr.contains("unresolved imports"),
        "compile probe should fail against the Evidence API, not because the crate is unavailable, stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("EvidenceReceipt") && stderr.contains("TrustedProvenance"),
        "compile probe should name unavailable trusted Evidence types, stderr:\n{stderr}"
    );
}

#[test]
fn evidence_contract_schema_lists_every_fixture_object_kind_once() {
    let schema = read_json(&root().join(SCHEMA_PATH));
    let defs = schema["$defs"].as_object().expect("schema has $defs");
    for required_def in [
        "ProofObligation",
        "ObservationRequirement",
        "VerificationCapabilityManifest",
        "VerificationCapabilityInstance",
        "EvidenceAttempt",
        "UntrustedEvidenceProposal",
        "EvidenceReceipt",
        "CoverageVerdict",
        "EvidencePolicy",
        "ProofPreset",
        "EvidenceWaiver",
        "TrustedProvenance",
    ] {
        assert!(
            defs.contains_key(required_def),
            "schema missing $defs/{required_def}"
        );
    }

    let mut kinds = BTreeSet::new();
    for path in fixture_files("examples") {
        let value = read_json(&path);
        kinds.insert(contract_kind(&value).unwrap_or("unknown"));
    }
    assert_eq!(
        kinds,
        BTreeSet::from([
            "CoverageVerdict",
            "EvidenceAttempt",
            "EvidencePolicy",
            "EvidenceReceipt",
            "EvidenceWaiver",
            "ProofObligation",
            "ProofPreset",
            "UntrustedEvidenceProposal",
            "VerificationCapabilityInstance",
            "VerificationCapabilityManifest",
        ])
    );
}

#[test]
fn evidence_contract_digest_vectors_are_executable_and_production_aligned() {
    let output = Command::new("node")
        .arg("scripts/verify-evidence-contract-vectors.mjs")
        .current_dir(root())
        .output()
        .expect("node can run evidence vector script");
    assert!(
        output.status.success(),
        "evidence vector script failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("verified 2 evidence contract digest vectors"),
        "script output should state vector count"
    );

    for (path, digest_field) in [
        (
            "docs/contracts/fixtures/evidence/v1/examples/evidence-receipt.json",
            "receipt_digest",
        ),
        (
            "docs/contracts/fixtures/evidence/v1/examples/evidence-policy.json",
            "policy_digest",
        ),
    ] {
        let mut value = read_json(&root().join(path));
        let expected = value[digest_field].as_str().unwrap().to_string();
        value.as_object_mut().unwrap().remove(digest_field);
        let canonical = canonical_json(&value);
        let actual = format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()));
        assert_eq!(actual, expected, "{path} digest vector drifted");
    }
}

#[test]
fn evidence_contract_forbids_public_trusted_provenance_construction_paths() {
    let contract = fs::read_to_string(root().join(CONTRACT_PATH)).unwrap();
    for required in [
        "Public deserialization always produces an `UntrustedEvidenceProposal`",
        "Public JSON, agent-authored JSON, adapter stdout, logs, and artifacts may propose claims",
        "but they cannot construct trusted provenance",
        "`src/canonical_json.rs` is the future single production owner",
        "`src/execution.rs` is the future shared owner",
        "Planr logs remain narrative and supporting records",
        "never automatically satisfies a binding observation",
    ] {
        assert!(
            contract.contains(required),
            "Evidence contract missing trust boundary: {required}"
        );
    }

    let lib_rs = fs::read_to_string(root().join("src/lib.rs")).unwrap();
    assert!(
        lib_rs.contains("pub mod evidence;"),
        "Evidence untrusted public boundary should be library-owned"
    );
    let evidence_rs = fs::read_to_string(root().join("src/evidence.rs")).unwrap();
    for required in [
        "pub struct UntrustedEvidenceProposal",
        "#[serde(deny_unknown_fields)]",
        "parse_untrusted_evidence_proposal",
    ] {
        assert!(
            evidence_rs.contains(required),
            "production Evidence boundary missing {required}"
        );
    }
    for forbidden in ["pub struct EvidenceReceipt", "pub struct TrustedProvenance"] {
        assert!(
            !evidence_rs.contains(forbidden),
            "trusted Evidence type must not be public yet: {forbidden}"
        );
    }
    let main_rs = fs::read_to_string(root().join("src/main.rs")).unwrap();
    assert!(
        !main_rs.contains("pub mod evidence"),
        "binary surface must not own the Evidence public boundary"
    );
    assert!(
        main_rs.contains("mod canonical_json;"),
        "canonical JSON owner should remain an internal module"
    );

    let repo_evidence = fs::read_to_string(root().join("src/app/repository/evidence.rs")).unwrap();
    for forbidden in [
        "EvidenceReceipt",
        "TrustedProvenance",
        "receipt_status",
        "closure_authority",
    ] {
        assert!(
            !repo_evidence.contains(forbidden),
            "repository evidence surface must not accept trusted Evidence provenance yet: {forbidden}"
        );
    }

    let proposal = read_json(
        &root()
            .join("docs/contracts/fixtures/evidence/v1/examples/untrusted-evidence-proposal.json"),
    );
    for forbidden in [
        "provenance",
        "execution_id",
        "receipt_digest",
        "closure_authority",
    ] {
        assert!(
            field(&proposal, forbidden).is_none(),
            "public proposal fixture unexpectedly contains authoritative {forbidden}"
        );
    }
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Array(values) => {
            format!(
                "[{}]",
                values
                    .iter()
                    .map(canonical_json)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Object(object) => {
            format!(
                "{{{}}}",
                object
                    .iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap(),
                        canonical_json(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        value => serde_json::to_string(value).unwrap(),
    }
}
